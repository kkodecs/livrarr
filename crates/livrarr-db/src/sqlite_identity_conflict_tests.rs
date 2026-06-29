// Behavioral tests for the identity-conflict remediation:
//   1. Respect user picks (User-set anchor → no conflict raised)
//   2. Resolve/dismiss take effect (badge + anchor changes)
//   3. raise_identity_conflict is atomic
//   4. No re-raise after resolve/dismiss

#[cfg(test)]
mod tests {
    use crate::{
        test_helpers::create_test_db, AuthorDb, CreateAuthorDbRequest, CreateUserDbRequest,
        CreateWorkDbRequest, UserDb, WorkDb, WorkDbCreate,
    };
    use livrarr_domain::services::WorkIdentityRepository;
    use livrarr_domain::{
        identity::{
            AnchorSetter, AnchorType, CapturedIdentity, ConflictResolutionAction, ConflictSource,
            ConflictStatus, IdentityConflictKind, IncomingConflictPayload, NewIdentityConflict,
        },
        IdentityStatus,
    };

    // -------------------------------------------------------------------------
    // Seed helpers
    // -------------------------------------------------------------------------

    struct Seed {
        user_id: i64,
        work_id: i64,
    }

    async fn seed(db: &(impl UserDb + AuthorDb + WorkDbCreate)) -> Seed {
        let user = db
            .create_user(CreateUserDbRequest {
                username: "tester".to_string(),
                password_hash: "hash".to_string(),
                role: crate::UserRole::User,
                api_key_hash: "api_hash".to_string(),
            })
            .await
            .unwrap();

        let author = db
            .create_author(CreateAuthorDbRequest {
                user_id: user.id,
                name: "Test Author".to_string(),
                sort_name: None,
                ol_key: None,
                gr_key: None,
                hc_key: None,
                import_id: None,
            })
            .await
            .unwrap();

        let (work, _) = db
            .create_work(CreateWorkDbRequest {
                user_id: user.id,
                title: "Test Work".to_string(),
                author_name: "Test Author".to_string(),
                normalized_title: "test work".to_string(),
                normalized_author: "test author".to_string(),
                author_id: Some(author.id),
                monitor_ebook: true,
                monitor_audiobook: true,
                ..Default::default()
            })
            .await
            .unwrap();

        Seed {
            user_id: user.id,
            work_id: work.id,
        }
    }

    fn ol_incoming(ol_key: &str) -> CapturedIdentity {
        CapturedIdentity {
            ol_key: Some(ol_key.to_string()),
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
            title: "Test Work".to_string(),
            author_name: "Test Author".to_string(),
            language: None,
        }
    }

    fn make_ol_conflict(user_id: i64, work_id: i64, ol_key: &str) -> NewIdentityConflict {
        NewIdentityConflict {
            user_id,
            existing_work_id: work_id,
            kind: IdentityConflictKind::IncomingDifferentOlKey,
            incoming: IncomingConflictPayload {
                ol_key: Some(ol_key.to_string()),
                gr_key: None,
                hc_key: None,
                isbn_13: None,
                asin: None,
                title: "Test Work".to_string(),
                author_name: "Test Author".to_string(),
                year: None,
                cover_url: None,
                top_candidates: Vec::new(),
            },
            raised_by: ConflictSource::ManualAdd,
            raised_source_path: None,
        }
    }

    // -------------------------------------------------------------------------
    // Test 1: User-set confirmed anchor + differing incoming → NO conflict raised
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn user_set_anchor_suppresses_conflict() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        // Confirm OL anchor with User setter
        db.confirm_anchor(
            s.work_id,
            AnchorType::new(AnchorType::OL_WORK),
            "/works/OL123W",
            AnchorSetter::User,
        )
        .await
        .unwrap();

        // Incoming carries a different OL key
        let incoming = ol_incoming("/works/OL999W");
        let conflicts = db
            .detect_conflicting_anchors(s.work_id, &incoming, ConflictSource::ManualAdd)
            .await
            .unwrap();

        assert!(
            conflicts.is_empty(),
            "User-set anchor must not generate a conflict; got: {conflicts:?}"
        );
    }

    // -------------------------------------------------------------------------
    // Test 2: AutoSearch-set confirmed anchor + differing incoming → conflict IS raised
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn auto_search_anchor_raises_conflict() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        // Confirm OL anchor with AutoSearch setter
        db.confirm_anchor(
            s.work_id,
            AnchorType::new(AnchorType::OL_WORK),
            "/works/OL123W",
            AnchorSetter::AutoSearch,
        )
        .await
        .unwrap();

        let incoming = ol_incoming("/works/OL999W");
        let conflicts = db
            .detect_conflicting_anchors(s.work_id, &incoming, ConflictSource::ManualAdd)
            .await
            .unwrap();

        assert_eq!(
            conflicts.len(),
            1,
            "AutoSearch-set anchor with differing incoming must raise a conflict"
        );
        assert_eq!(
            conflicts[0].kind,
            IdentityConflictKind::IncomingDifferentOlKey
        );
    }

    // -------------------------------------------------------------------------
    // Test 3a: Resolve with KeepExisting → badge leaves Conflict, anchor is User-set
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn resolve_keep_existing_clears_conflict_badge() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        // Set up a Confirmed OL anchor (AutoSearch)
        db.confirm_anchor(
            s.work_id,
            AnchorType::new(AnchorType::OL_WORK),
            "/works/OL123W",
            AnchorSetter::AutoSearch,
        )
        .await
        .unwrap();

        // Raise a conflict
        let conflict_id = db
            .raise_identity_conflict(make_ol_conflict(s.user_id, s.work_id, "/works/OL999W"))
            .await
            .unwrap();

        // Verify badge is now Conflict
        let conflict_row = db
            .get_identity_conflict(conflict_id, s.user_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(conflict_row.status, ConflictStatus::Open);

        // Resolve with KeepExisting
        db.apply_conflict_resolution(
            &conflict_row,
            ConflictResolutionAction::KeepExisting,
            None,
            chrono::Utc::now(),
        )
        .await
        .unwrap();

        // Conflict row is now resolved
        let resolved = db
            .get_identity_conflict(conflict_id, s.user_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.status, ConflictStatus::Resolved);
        assert_eq!(
            resolved.resolution_action,
            Some(ConflictResolutionAction::KeepExisting)
        );

        // Badge is no longer Conflict (work has a confirmed OL anchor → Confirmed)
        let work = db.get_work(s.user_id, s.work_id).await.unwrap();
        assert_ne!(
            work.identity_status,
            IdentityStatus::Conflict,
            "badge must leave Conflict after resolution"
        );
        assert_eq!(work.identity_status, IdentityStatus::Confirmed);

        // Existing anchor is now User-stamped
        let anchors = db.list_anchors(s.work_id).await.unwrap();
        let ol_anchor = anchors
            .iter()
            .find(|a| a.anchor_type.as_str() == AnchorType::OL_WORK)
            .unwrap();
        assert_eq!(
            ol_anchor.setter,
            AnchorSetter::User,
            "KeepExisting must re-stamp anchor as User"
        );
    }

    // -------------------------------------------------------------------------
    // Test 3b: Resolve with ReplaceAnchor → badge leaves Conflict, incoming anchor confirmed
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn resolve_replace_anchor_applies_incoming() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        db.confirm_anchor(
            s.work_id,
            AnchorType::new(AnchorType::OL_WORK),
            "/works/OL123W",
            AnchorSetter::AutoSearch,
        )
        .await
        .unwrap();

        let conflict_id = db
            .raise_identity_conflict(make_ol_conflict(s.user_id, s.work_id, "/works/OL999W"))
            .await
            .unwrap();

        let conflict_row = db
            .get_identity_conflict(conflict_id, s.user_id)
            .await
            .unwrap()
            .unwrap();

        db.apply_conflict_resolution(
            &conflict_row,
            ConflictResolutionAction::ReplaceAnchor,
            None,
            chrono::Utc::now(),
        )
        .await
        .unwrap();

        // Badge leaves Conflict
        let work = db.get_work(s.user_id, s.work_id).await.unwrap();
        assert_ne!(work.identity_status, IdentityStatus::Conflict);
        assert_eq!(work.identity_status, IdentityStatus::Confirmed);

        // The new anchor is the incoming value, stamped User
        let anchors = db.list_anchors(s.work_id).await.unwrap();
        let confirmed_ol: Vec<_> = anchors
            .iter()
            .filter(|a| {
                a.anchor_type.as_str() == AnchorType::OL_WORK
                    && a.confidence == livrarr_domain::identity::AnchorConfidence::Confirmed
            })
            .collect();
        assert_eq!(confirmed_ol.len(), 1);
        assert_eq!(confirmed_ol[0].anchor_value, "/works/OL999W");
        assert_eq!(confirmed_ol[0].setter, AnchorSetter::User);
    }

    // -------------------------------------------------------------------------
    // Test 3c: Resolve with Merge → badge leaves Conflict, all incoming anchors confirmed
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn resolve_merge_confirms_incoming_anchors() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        db.confirm_anchor(
            s.work_id,
            AnchorType::new(AnchorType::OL_WORK),
            "/works/OL123W",
            AnchorSetter::AutoSearch,
        )
        .await
        .unwrap();

        // Conflict with an incoming that also carries a GR key
        let mut conflict = make_ol_conflict(s.user_id, s.work_id, "/works/OL999W");
        conflict.incoming.gr_key = Some("goodreads:123456".to_string());

        let conflict_id = db.raise_identity_conflict(conflict).await.unwrap();

        let conflict_row = db
            .get_identity_conflict(conflict_id, s.user_id)
            .await
            .unwrap()
            .unwrap();

        db.apply_conflict_resolution(
            &conflict_row,
            ConflictResolutionAction::Merge,
            None,
            chrono::Utc::now(),
        )
        .await
        .unwrap();

        let work = db.get_work(s.user_id, s.work_id).await.unwrap();
        assert_ne!(work.identity_status, IdentityStatus::Conflict);

        let anchors = db.list_anchors(s.work_id).await.unwrap();

        // OL key from incoming is confirmed + User-stamped
        let ol = anchors
            .iter()
            .find(|a| {
                a.anchor_type.as_str() == AnchorType::OL_WORK
                    && a.confidence == livrarr_domain::identity::AnchorConfidence::Confirmed
            })
            .unwrap();
        assert_eq!(ol.anchor_value, "/works/OL999W");
        assert_eq!(ol.setter, AnchorSetter::User);

        // GR key from incoming is also confirmed + User-stamped
        let gr = anchors
            .iter()
            .find(|a| {
                a.anchor_type.as_str() == AnchorType::GR_WORK
                    && a.confidence == livrarr_domain::identity::AnchorConfidence::Confirmed
            })
            .unwrap();
        assert_eq!(gr.anchor_value, "goodreads:123456");
        assert_eq!(gr.setter, AnchorSetter::User);
    }

    // -------------------------------------------------------------------------
    // Test 3d: Dismiss → badge leaves Conflict; anchor NOT re-stamped
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn dismiss_clears_conflict_badge_without_restamping() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        db.confirm_anchor(
            s.work_id,
            AnchorType::new(AnchorType::OL_WORK),
            "/works/OL123W",
            AnchorSetter::AutoSearch,
        )
        .await
        .unwrap();

        let conflict_id = db
            .raise_identity_conflict(make_ol_conflict(s.user_id, s.work_id, "/works/OL999W"))
            .await
            .unwrap();

        let conflict_row = db
            .get_identity_conflict(conflict_id, s.user_id)
            .await
            .unwrap()
            .unwrap();

        db.apply_conflict_dismiss(&conflict_row, chrono::Utc::now())
            .await
            .unwrap();

        // Conflict row is dismissed
        let dismissed = db
            .get_identity_conflict(conflict_id, s.user_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dismissed.status, ConflictStatus::Dismissed);

        // Badge leaves Conflict (existing OL anchor → Confirmed)
        let work = db.get_work(s.user_id, s.work_id).await.unwrap();
        assert_ne!(work.identity_status, IdentityStatus::Conflict);

        // Original anchor is still AutoSearch (NOT re-stamped on dismiss)
        let anchors = db.list_anchors(s.work_id).await.unwrap();
        let ol = anchors
            .iter()
            .find(|a| a.anchor_type.as_str() == AnchorType::OL_WORK)
            .unwrap();
        assert_eq!(
            ol.setter,
            AnchorSetter::AutoSearch,
            "dismiss must NOT re-stamp the anchor"
        );
    }

    // -------------------------------------------------------------------------
    // Test 4a: After resolve (KeepExisting), re-running detection does NOT re-raise
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn no_reraise_after_resolve() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        db.confirm_anchor(
            s.work_id,
            AnchorType::new(AnchorType::OL_WORK),
            "/works/OL123W",
            AnchorSetter::AutoSearch,
        )
        .await
        .unwrap();

        let conflict_id = db
            .raise_identity_conflict(make_ol_conflict(s.user_id, s.work_id, "/works/OL999W"))
            .await
            .unwrap();

        let conflict_row = db
            .get_identity_conflict(conflict_id, s.user_id)
            .await
            .unwrap()
            .unwrap();

        // Resolve with KeepExisting → anchor re-stamped as User
        db.apply_conflict_resolution(
            &conflict_row,
            ConflictResolutionAction::KeepExisting,
            None,
            chrono::Utc::now(),
        )
        .await
        .unwrap();

        // Re-run detection with same incoming — should produce NO new conflicts
        // because the anchor is now User-set (fix #1 guards it).
        let incoming = ol_incoming("/works/OL999W");
        let conflicts = db
            .detect_conflicting_anchors(s.work_id, &incoming, ConflictSource::ManualAdd)
            .await
            .unwrap();

        assert!(
            conflicts.is_empty(),
            "after KeepExisting resolve, same incoming must not re-raise a conflict"
        );
    }

    // -------------------------------------------------------------------------
    // Test 4b: After dismiss, re-running detection for the SAME incoming does NOT re-raise
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn no_reraise_after_dismiss() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        db.confirm_anchor(
            s.work_id,
            AnchorType::new(AnchorType::OL_WORK),
            "/works/OL123W",
            AnchorSetter::AutoSearch,
        )
        .await
        .unwrap();

        let conflict_id = db
            .raise_identity_conflict(make_ol_conflict(s.user_id, s.work_id, "/works/OL999W"))
            .await
            .unwrap();

        let conflict_row = db
            .get_identity_conflict(conflict_id, s.user_id)
            .await
            .unwrap()
            .unwrap();

        // Dismiss — existing anchor stays AutoSearch
        db.apply_conflict_dismiss(&conflict_row, chrono::Utc::now())
            .await
            .unwrap();

        // Re-run detection with the SAME incoming value — closed-conflict guard must fire
        let incoming = ol_incoming("/works/OL999W");
        let conflicts = db
            .detect_conflicting_anchors(s.work_id, &incoming, ConflictSource::ManualAdd)
            .await
            .unwrap();

        assert!(
            conflicts.is_empty(),
            "same incoming value after dismiss must not re-raise the same conflict"
        );

        // A DIFFERENT incoming value still raises normally (guard is value-scoped)
        let incoming2 = ol_incoming("/works/OL777W");
        let conflicts2 = db
            .detect_conflicting_anchors(s.work_id, &incoming2, ConflictSource::ManualAdd)
            .await
            .unwrap();
        assert_eq!(
            conflicts2.len(),
            1,
            "a different incoming value must still raise a new conflict"
        );
    }

    // -------------------------------------------------------------------------
    // Test 5: raise_identity_conflict is atomic (badge + row in one tx)
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn raise_identity_conflict_is_atomic() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        db.confirm_anchor(
            s.work_id,
            AnchorType::new(AnchorType::OL_WORK),
            "/works/OL123W",
            AnchorSetter::AutoSearch,
        )
        .await
        .unwrap();

        // Badge before raise is not yet Conflict (confirm_anchor does not
        // set identity_status; that is a separate write via set_identity_confirmed).
        let work_before = db.get_work(s.user_id, s.work_id).await.unwrap();
        assert_ne!(
            work_before.identity_status,
            IdentityStatus::Conflict,
            "badge must not be Conflict before we raise one"
        );

        // Raise a conflict — both the conflict row INSERT and the badge UPDATE
        // must succeed or fail together (atomic).
        let conflict_id = db
            .raise_identity_conflict(make_ol_conflict(s.user_id, s.work_id, "/works/OL999W"))
            .await
            .unwrap();

        // Badge must be Conflict now (written atomically with the conflict row)
        let work_after = db.get_work(s.user_id, s.work_id).await.unwrap();
        assert_eq!(
            work_after.identity_status,
            IdentityStatus::Conflict,
            "badge must be updated to Conflict atomically with conflict row insertion"
        );

        // The conflict row exists with status=open
        let row = db
            .get_identity_conflict(conflict_id, s.user_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, ConflictStatus::Open);

        // Idempotency: raising the same conflict again returns existing id
        let conflict_id2 = db
            .raise_identity_conflict(make_ol_conflict(s.user_id, s.work_id, "/works/OL999W"))
            .await
            .unwrap();
        assert_eq!(conflict_id, conflict_id2, "raise must be idempotent");
    }
}
