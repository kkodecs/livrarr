// Behavioral tests for the identity-conflict remediation:
//   1. Respect user picks (User-set anchor → no conflict raised)
//   2. Resolve/dismiss take effect (badge + anchor changes)
//   3. raise_identity_conflict is atomic
//   4. No re-raise after resolve/dismiss

#[cfg(test)]
mod tests {
    use crate::{
        test_helpers::create_test_db, AuthorDb, ConflictApplyError, CreateAuthorDbRequest,
        CreateUserDbRequest, CreateWorkDbRequest, UserDb, WorkDb, WorkDbCreate,
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

        let (author, _) = db
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

        // Conflict with an incoming that also carries a canonical GR key (digits-only).
        // "goodreads:123456" is NOT canonical; "123456" is.
        let mut conflict = make_ol_conflict(s.user_id, s.work_id, "/works/OL999W");
        conflict.incoming.gr_key = Some("123456".to_string());

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

        // GR key from incoming is also confirmed + User-stamped (canonical form "123456")
        let gr = anchors
            .iter()
            .find(|a| {
                a.anchor_type.as_str() == AnchorType::GR_WORK
                    && a.confidence == livrarr_domain::identity::AnchorConfidence::Confirmed
            })
            .unwrap();
        assert_eq!(gr.anchor_value, "123456");
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

    // -------------------------------------------------------------------------
    // Fix 1 / R-1: Validation is enforced through conflict resolution
    // -------------------------------------------------------------------------

    /// A non-canonical primary anchor in the incoming payload (ReplaceAnchor) fails
    /// the whole resolution — the primary type failing is never a "skip + warn".
    #[tokio::test]
    async fn replace_anchor_with_invalid_primary_fails_resolution() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        db.confirm_anchor(
            s.work_id,
            AnchorType::new(AnchorType::GR_WORK),
            "111111",
            AnchorSetter::AutoSearch,
        )
        .await
        .unwrap();

        // Conflict whose incoming GR key is NOT canonical (starts with letters).
        let conflict_id = db
            .raise_identity_conflict(NewIdentityConflict {
                user_id: s.user_id,
                existing_work_id: s.work_id,
                kind: IdentityConflictKind::IncomingDifferentGrKey,
                incoming: IncomingConflictPayload {
                    gr_key: Some("goodreads:BAD".to_string()), // non-canonical
                    ol_key: None,
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
            })
            .await
            .unwrap();

        let conflict_row = db
            .get_identity_conflict(conflict_id, s.user_id)
            .await
            .unwrap()
            .unwrap();

        // ReplaceAnchor on a non-canonical primary → error (not a silent skip)
        let result = db
            .apply_conflict_resolution(
                &conflict_row,
                ConflictResolutionAction::ReplaceAnchor,
                None,
                chrono::Utc::now(),
            )
            .await;

        // R-008: must surface as InvalidAnchorValue, not a generic Db/Protocol error
        assert!(
            matches!(result, Err(ConflictApplyError::InvalidAnchorValue)),
            "ReplaceAnchor with non-canonical primary value must return ConflictApplyError::InvalidAnchorValue; \
             got: {result:?}"
        );

        // The conflict row must still be Open (the resolution was rolled back)
        let still_open = db
            .get_identity_conflict(conflict_id, s.user_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(still_open.status, ConflictStatus::Open);
    }

    /// A non-canonical SECONDARY anchor in a Merge is skipped with a warning;
    /// the primary anchor is still resolved and the resolution succeeds.
    #[tokio::test]
    async fn merge_secondary_invalid_skipped_primary_succeeds() {
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

        // Incoming: valid primary (OL), invalid secondary (GR is "goodreads:BAD")
        let mut conflict = make_ol_conflict(s.user_id, s.work_id, "/works/OL999W");
        conflict.incoming.gr_key = Some("goodreads:BAD".to_string()); // non-canonical secondary

        let conflict_id = db.raise_identity_conflict(conflict).await.unwrap();
        let conflict_row = db
            .get_identity_conflict(conflict_id, s.user_id)
            .await
            .unwrap()
            .unwrap();

        // Merge must succeed (secondary skip does not block primary)
        db.apply_conflict_resolution(
            &conflict_row,
            ConflictResolutionAction::Merge,
            None,
            chrono::Utc::now(),
        )
        .await
        .expect("merge must succeed despite an invalid secondary anchor");

        // Primary OL anchor is now confirmed at the incoming value
        let anchors = db.list_anchors(s.work_id).await.unwrap();
        let ol = anchors
            .iter()
            .find(|a| {
                a.anchor_type.as_str() == AnchorType::OL_WORK
                    && a.confidence == livrarr_domain::identity::AnchorConfidence::Confirmed
            })
            .unwrap();
        assert_eq!(ol.anchor_value, "/works/OL999W");

        // Invalid secondary was skipped — no GR anchor exists
        let has_gr = anchors
            .iter()
            .any(|a| a.anchor_type.as_str() == AnchorType::GR_WORK);
        assert!(!has_gr, "invalid secondary GR anchor must not be persisted");

        // Badge is no longer Conflict
        let work = db.get_work(s.user_id, s.work_id).await.unwrap();
        assert_ne!(
            work.identity_status,
            livrarr_domain::IdentityStatus::Conflict
        );
    }

    // -------------------------------------------------------------------------
    // Fix 2 / R-001: Merge never overwrites a User-set anchor of another type
    // -------------------------------------------------------------------------

    /// Merge resolves the conflict's own (primary) OL key, but must not touch an
    /// existing User-set GR anchor — even if the incoming payload carries a
    /// different GR key.
    #[tokio::test]
    async fn merge_preserves_user_set_other_type_anchor() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        // User has manually chosen both an OL key (AutoSearch) and a GR key (User).
        db.confirm_anchor(
            s.work_id,
            AnchorType::new(AnchorType::OL_WORK),
            "/works/OL123W",
            AnchorSetter::AutoSearch,
        )
        .await
        .unwrap();
        db.confirm_anchor(
            s.work_id,
            AnchorType::new(AnchorType::GR_WORK),
            "111111",
            AnchorSetter::User,
        )
        .await
        .unwrap();

        // Incoming payload conflicts on OL key AND also carries a different GR key.
        let mut conflict = make_ol_conflict(s.user_id, s.work_id, "/works/OL999W");
        conflict.incoming.gr_key = Some("999999".to_string()); // different from user's "111111"

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

        let anchors = db.list_anchors(s.work_id).await.unwrap();

        // Primary OL key was replaced
        let ol = anchors
            .iter()
            .find(|a| {
                a.anchor_type.as_str() == AnchorType::OL_WORK
                    && a.confidence == livrarr_domain::identity::AnchorConfidence::Confirmed
            })
            .unwrap();
        assert_eq!(
            ol.anchor_value, "/works/OL999W",
            "primary OL key must be replaced"
        );

        // User-set GR key is UNCHANGED (merge must never overwrite it)
        let gr = anchors
            .iter()
            .find(|a| {
                a.anchor_type.as_str() == AnchorType::GR_WORK
                    && a.confidence == livrarr_domain::identity::AnchorConfidence::Confirmed
            })
            .unwrap();
        assert_eq!(
            gr.anchor_value, "111111",
            "User-set GR anchor must be preserved by Merge (R-001 regression)"
        );
        assert_eq!(gr.setter, AnchorSetter::User);
    }

    /// When the work has NO existing GR anchor, Merge gap-fills it from the incoming payload.
    #[tokio::test]
    async fn merge_gap_fills_missing_type() {
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

        // Incoming carries a canonical GR key alongside the conflicting OL key
        let mut conflict = make_ol_conflict(s.user_id, s.work_id, "/works/OL999W");
        conflict.incoming.gr_key = Some("555555".to_string()); // no existing GR → gap-fill

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

        let anchors = db.list_anchors(s.work_id).await.unwrap();

        // GR anchor gap-filled from incoming
        let gr = anchors
            .iter()
            .find(|a| {
                a.anchor_type.as_str() == AnchorType::GR_WORK
                    && a.confidence == livrarr_domain::identity::AnchorConfidence::Confirmed
            })
            .expect("missing GR anchor should have been gap-filled");
        assert_eq!(gr.anchor_value, "555555");
        assert_eq!(gr.setter, AnchorSetter::User);
    }

    // -------------------------------------------------------------------------
    // Fix 3 / R-3: raise_identity_conflict deduplicates by (work, kind) not OL-only
    // -------------------------------------------------------------------------

    /// Raising a GR conflict twice via raise_identity_conflict returns the same id
    /// (dedup by (work_id, kind)) and the badge is stamped on first raise.
    #[tokio::test]
    async fn raise_identity_conflict_deduplicates_by_kind_not_ol() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        db.confirm_anchor(
            s.work_id,
            AnchorType::new(AnchorType::GR_WORK),
            "111111",
            AnchorSetter::AutoSearch,
        )
        .await
        .unwrap();

        let gr_conflict = NewIdentityConflict {
            user_id: s.user_id,
            existing_work_id: s.work_id,
            kind: IdentityConflictKind::IncomingDifferentGrKey,
            incoming: IncomingConflictPayload {
                gr_key: Some("999999".to_string()),
                ol_key: None,
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
        };

        let id1 = db
            .raise_identity_conflict(gr_conflict.clone())
            .await
            .unwrap();

        // Badge is set atomically
        let work = db.get_work(s.user_id, s.work_id).await.unwrap();
        assert_eq!(
            work.identity_status,
            livrarr_domain::IdentityStatus::Conflict
        );

        // Second raise with same (work, kind) returns the same id
        let id2 = db.raise_identity_conflict(gr_conflict).await.unwrap();
        assert_eq!(id1, id2, "raise must dedup by (work, kind)");
    }

    // -------------------------------------------------------------------------
    // Fix 5 / R-002: Work-scoped QuorumTie resolves through the standard flow
    // -------------------------------------------------------------------------

    fn make_quorum_tie_conflict(user_id: i64, work_id: i64) -> NewIdentityConflict {
        NewIdentityConflict {
            user_id,
            existing_work_id: work_id,
            kind: IdentityConflictKind::QuorumTie,
            incoming: IncomingConflictPayload {
                ol_key: Some("/works/OL999W".to_string()),
                gr_key: Some("999999".to_string()),
                hc_key: None,
                isbn_13: None,
                asin: None,
                title: "Test Work".to_string(),
                author_name: "Test Author".to_string(),
                year: None,
                cover_url: None,
                top_candidates: Vec::new(),
            },
            raised_by: ConflictSource::Convergence,
            raised_source_path: None,
        }
    }

    /// QuorumTie + KeepExisting: badge leaves Conflict; existing anchors are NOT changed.
    #[tokio::test]
    async fn quorum_tie_keep_existing_recomputes_badge() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        // Give the work a confirmed OL anchor so the badge can reach Confirmed.
        db.confirm_anchor(
            s.work_id,
            AnchorType::new(AnchorType::OL_WORK),
            "/works/OL123W",
            AnchorSetter::AutoSearch,
        )
        .await
        .unwrap();

        let conflict_id = db
            .raise_identity_conflict(make_quorum_tie_conflict(s.user_id, s.work_id))
            .await
            .unwrap();

        let conflict_row = db
            .get_identity_conflict(conflict_id, s.user_id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            db.get_work(s.user_id, s.work_id)
                .await
                .unwrap()
                .identity_status,
            livrarr_domain::IdentityStatus::Conflict,
            "badge must be Conflict before resolution"
        );

        db.apply_conflict_resolution(
            &conflict_row,
            ConflictResolutionAction::KeepExisting,
            None,
            chrono::Utc::now(),
        )
        .await
        .expect("QuorumTie KeepExisting must not error");

        let work = db.get_work(s.user_id, s.work_id).await.unwrap();
        assert_ne!(
            work.identity_status,
            livrarr_domain::IdentityStatus::Conflict,
            "badge must leave Conflict after QuorumTie KeepExisting"
        );

        // Existing OL anchor is unchanged (KeepExisting on QuorumTie makes no anchor change)
        let anchors = db.list_anchors(s.work_id).await.unwrap();
        let ol = anchors
            .iter()
            .find(|a| a.anchor_type.as_str() == AnchorType::OL_WORK)
            .unwrap();
        assert_eq!(ol.anchor_value, "/works/OL123W");
    }

    /// QuorumTie + AcceptSeparate: badge leaves Conflict; existing anchors are NOT changed.
    #[tokio::test]
    async fn quorum_tie_accept_separate_recomputes_badge() {
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
            .raise_identity_conflict(make_quorum_tie_conflict(s.user_id, s.work_id))
            .await
            .unwrap();

        let conflict_row = db
            .get_identity_conflict(conflict_id, s.user_id)
            .await
            .unwrap()
            .unwrap();

        db.apply_conflict_resolution(
            &conflict_row,
            ConflictResolutionAction::AcceptSeparate,
            None,
            chrono::Utc::now(),
        )
        .await
        .expect("QuorumTie AcceptSeparate must not error");

        let work = db.get_work(s.user_id, s.work_id).await.unwrap();
        assert_ne!(
            work.identity_status,
            livrarr_domain::IdentityStatus::Conflict,
            "badge must leave Conflict after QuorumTie AcceptSeparate"
        );
    }

    /// QuorumTie + Merge: badge leaves Conflict; incoming anchors are gap-filled where
    /// no confirmed anchor existed before.
    #[tokio::test]
    async fn quorum_tie_merge_gap_fills_and_recomputes_badge() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        // Work has no anchors at all initially
        let conflict_id = db
            .raise_identity_conflict(make_quorum_tie_conflict(s.user_id, s.work_id))
            .await
            .unwrap();

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
        .expect("QuorumTie Merge must not error");

        // Badge must leave Conflict
        let work = db.get_work(s.user_id, s.work_id).await.unwrap();
        assert_ne!(
            work.identity_status,
            livrarr_domain::IdentityStatus::Conflict,
            "badge must leave Conflict after QuorumTie Merge"
        );

        // Incoming OL and GR anchors gap-filled
        let anchors = db.list_anchors(s.work_id).await.unwrap();
        let ol = anchors
            .iter()
            .find(|a| {
                a.anchor_type.as_str() == AnchorType::OL_WORK
                    && a.confidence == livrarr_domain::identity::AnchorConfidence::Confirmed
            })
            .expect("OL anchor should be gap-filled by QuorumTie Merge");
        assert_eq!(ol.anchor_value, "/works/OL999W");

        let gr = anchors
            .iter()
            .find(|a| {
                a.anchor_type.as_str() == AnchorType::GR_WORK
                    && a.confidence == livrarr_domain::identity::AnchorConfidence::Confirmed
            })
            .expect("GR anchor should be gap-filled by QuorumTie Merge");
        assert_eq!(gr.anchor_value, "999999");
    }

    /// QuorumTie + ReplaceAnchor: treated as Merge — badge leaves Conflict;
    /// incoming anchors are gap-filled (same semantics as Merge for QuorumTie).
    #[tokio::test]
    async fn quorum_tie_replace_anchor_treated_as_merge() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        // Work has an existing OL anchor (AutoSearch) — gap-fill must not overwrite it
        db.confirm_anchor(
            s.work_id,
            AnchorType::new(AnchorType::OL_WORK),
            "/works/OL123W",
            AnchorSetter::AutoSearch,
        )
        .await
        .unwrap();

        let conflict_id = db
            .raise_identity_conflict(make_quorum_tie_conflict(s.user_id, s.work_id))
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
        .expect("QuorumTie ReplaceAnchor (treated as Merge) must not error");

        // Badge must leave Conflict
        let work = db.get_work(s.user_id, s.work_id).await.unwrap();
        assert_ne!(
            work.identity_status,
            livrarr_domain::IdentityStatus::Conflict,
            "badge must leave Conflict after QuorumTie ReplaceAnchor"
        );

        // Existing OL anchor is preserved (gap-fill skips if confirmed anchor exists)
        let anchors = db.list_anchors(s.work_id).await.unwrap();
        let confirmed_ol: Vec<_> = anchors
            .iter()
            .filter(|a| {
                a.anchor_type.as_str() == AnchorType::OL_WORK
                    && a.confidence == livrarr_domain::identity::AnchorConfidence::Confirmed
            })
            .collect();
        assert_eq!(confirmed_ol.len(), 1);
        assert_eq!(confirmed_ol[0].anchor_value, "/works/OL123W");

        // GR anchor from incoming was gap-filled (none existed before)
        let gr = anchors
            .iter()
            .find(|a| {
                a.anchor_type.as_str() == AnchorType::GR_WORK
                    && a.confidence == livrarr_domain::identity::AnchorConfidence::Confirmed
            })
            .expect("GR anchor should be gap-filled by QuorumTie ReplaceAnchor-as-Merge");
        assert_eq!(gr.anchor_value, "999999");
    }

    // -------------------------------------------------------------------------
    // R-003: Known-gap marker — User-set anchor suppression from Refresh/Convergence
    // -------------------------------------------------------------------------

    /// A User-set confirmed anchor is suppressed even when the differing value comes
    /// from a Refresh or Convergence source. This is the CURRENT (limited) behavior:
    /// redirect detection requires Phase 2-3 provider re-fetch machinery that does
    /// not exist yet. This test pins the behavior so the limitation is tracked.
    ///
    /// See the TODO(phase2-3) comment in detect_conflicting_anchors.
    #[tokio::test]
    async fn user_set_anchor_suppressed_from_refresh_and_convergence() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        // User explicitly confirmed this OL key.
        db.confirm_anchor(
            s.work_id,
            AnchorType::new(AnchorType::OL_WORK),
            "/works/OL123W",
            AnchorSetter::User,
        )
        .await
        .unwrap();

        // A Refresh pass carries a different OL key — currently suppressed.
        let incoming_refresh = ol_incoming("/works/OL999W");
        let refresh_conflicts = db
            .detect_conflicting_anchors(s.work_id, &incoming_refresh, ConflictSource::Refresh)
            .await
            .unwrap();

        assert!(
            refresh_conflicts.is_empty(),
            "Known-gap (R-003): User-set anchor must be suppressed from Refresh source \
             until Phase 2-3 redirect machinery exists; got: {refresh_conflicts:?}"
        );

        // Convergence source: same behavior.
        let convergence_conflicts = db
            .detect_conflicting_anchors(s.work_id, &incoming_refresh, ConflictSource::Convergence)
            .await
            .unwrap();

        assert!(
            convergence_conflicts.is_empty(),
            "Known-gap (R-003): User-set anchor must be suppressed from Convergence source \
             until Phase 2-3 redirect machinery exists; got: {convergence_conflicts:?}"
        );
    }

    // -------------------------------------------------------------------------
    // R-007: TOCTOU — second resolve of an already-resolved conflict must fail
    // -------------------------------------------------------------------------

    /// A second call to `apply_conflict_resolution` on an already-resolved conflict
    /// must return `ConflictApplyError::AlreadyResolved` and must NOT write any
    /// additional anchor mutations (rows_affected guard catches the race).
    #[tokio::test]
    async fn double_resolve_returns_already_resolved_and_no_double_apply() {
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

        // First resolve succeeds
        db.apply_conflict_resolution(
            &conflict_row,
            ConflictResolutionAction::KeepExisting,
            None,
            chrono::Utc::now(),
        )
        .await
        .expect("first resolve must succeed");

        // Snapshot anchors after the first resolve
        let anchors_after_first = db.list_anchors(s.work_id).await.unwrap();

        // Second resolve on the same (now-stale) conflict row must fail typed
        let result = db
            .apply_conflict_resolution(
                &conflict_row,
                ConflictResolutionAction::Merge,
                None,
                chrono::Utc::now(),
            )
            .await;

        assert!(
            matches!(result, Err(ConflictApplyError::AlreadyResolved)),
            "second resolve must return ConflictApplyError::AlreadyResolved; got: {result:?}"
        );

        // Anchors must be unchanged — no double-apply
        let anchors_after_second = db.list_anchors(s.work_id).await.unwrap();
        assert_eq!(
            anchors_after_first.len(),
            anchors_after_second.len(),
            "second resolve must not mutate anchors"
        );
    }

    // -------------------------------------------------------------------------
    // R-007: TOCTOU — second dismiss of an already-dismissed conflict must fail
    // -------------------------------------------------------------------------

    /// A second call to `apply_conflict_dismiss` on an already-dismissed conflict
    /// must return `ConflictApplyError::AlreadyResolved`.
    #[tokio::test]
    async fn double_dismiss_returns_already_resolved() {
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

        // First dismiss succeeds
        db.apply_conflict_dismiss(&conflict_row, chrono::Utc::now())
            .await
            .expect("first dismiss must succeed");

        // Second dismiss on the same stale conflict row must fail typed
        let result = db
            .apply_conflict_dismiss(&conflict_row, chrono::Utc::now())
            .await;

        assert!(
            matches!(result, Err(ConflictApplyError::AlreadyResolved)),
            "second dismiss must return ConflictApplyError::AlreadyResolved; got: {result:?}"
        );
    }

    // -------------------------------------------------------------------------
    // R-008: Merge with invalid primary anchor yields typed InvalidAnchorValue
    // -------------------------------------------------------------------------

    /// A Merge resolution where the primary anchor value fails canonical validation
    /// must return `ConflictApplyError::InvalidAnchorValue` (not a generic Db error),
    /// and must NOT commit anchor mutations or flip the conflict to resolved.
    ///
    /// Uses a GR conflict because OL/HC work keys have no canonical form restriction
    /// (any non-empty string is accepted), whereas GR keys must be numeric.
    #[tokio::test]
    async fn merge_invalid_primary_anchor_returns_typed_error() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        db.confirm_anchor(
            s.work_id,
            AnchorType::new(AnchorType::GR_WORK),
            "111111",
            AnchorSetter::AutoSearch,
        )
        .await
        .unwrap();

        // Conflict with a non-canonical GR key (starts with letters — not numeric)
        let conflict_id = db
            .raise_identity_conflict(NewIdentityConflict {
                user_id: s.user_id,
                existing_work_id: s.work_id,
                kind: IdentityConflictKind::IncomingDifferentGrKey,
                incoming: IncomingConflictPayload {
                    ol_key: None,
                    gr_key: Some("goodreads:BAD".to_string()), // non-canonical
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
            })
            .await
            .unwrap();

        let conflict_row = db
            .get_identity_conflict(conflict_id, s.user_id)
            .await
            .unwrap()
            .unwrap();

        // Merge on a non-canonical primary → typed error
        let result = db
            .apply_conflict_resolution(
                &conflict_row,
                ConflictResolutionAction::Merge,
                None,
                chrono::Utc::now(),
            )
            .await;

        assert!(
            matches!(result, Err(ConflictApplyError::InvalidAnchorValue)),
            "Merge with non-canonical primary must return ConflictApplyError::InvalidAnchorValue; \
             got: {result:?}"
        );

        // Conflict row must still be Open (the tx was rolled back)
        let still_open = db
            .get_identity_conflict(conflict_id, s.user_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            still_open.status,
            ConflictStatus::Open,
            "conflict must remain Open after a failed Merge"
        );
    }
}
