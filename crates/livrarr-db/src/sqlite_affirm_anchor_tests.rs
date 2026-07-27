// Behavioral tests for M-020: affirming a pending anchor must write the
// identity_status badge atomically — no waiting on background refresh.

#[cfg(test)]
mod tests {
    use crate::{
        test_helpers::create_test_db, AuthorDb, CreateAuthorDbRequest, CreateUserDbRequest,
        CreateWorkDbRequest, UserDb, WorkDb, WorkDbCreate,
    };
    use livrarr_domain::services::WorkIdentityRepository;
    use livrarr_domain::{
        identity::{AnchorSetter, AnchorType},
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

    // -------------------------------------------------------------------------
    // Test 1: Affirm the last (only) chaseable pending anchor on a Pending work
    // → badge becomes Confirmed immediately.
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn affirm_last_pending_anchor_sets_confirmed() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        // Record a pending OL_WORK anchor (the only chaseable anchor).
        db.record_pending_anchor(
            s.work_id,
            AnchorType::new(AnchorType::OL_WORK),
            "/works/OL123W",
        )
        .await
        .unwrap();

        // Sanity-check: badge starts Pending.
        let before = db.get_work(s.user_id, s.work_id).await.unwrap();
        assert_eq!(before.identity_status, IdentityStatus::Pending);

        // Affirm: confirm the anchor and recompute badge atomically.
        db.confirm_anchor_and_recompute_badge(
            s.work_id,
            AnchorType::new(AnchorType::OL_WORK),
            "/works/OL123W",
            AnchorSetter::User,
        )
        .await
        .unwrap();

        let after = db.get_work(s.user_id, s.work_id).await.unwrap();
        assert_eq!(
            after.identity_status,
            IdentityStatus::Confirmed,
            "badge must be Confirmed after affirming the last pending work anchor"
        );
    }

    // -------------------------------------------------------------------------
    // Test 2: Affirm one pending work anchor when another chaseable anchor
    // still exists as pending → badge still becomes Confirmed (a confirmed work
    // anchor is enough; chaseable remainder must not block the badge).
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn affirm_one_anchor_when_others_pending_still_sets_confirmed() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        // Two pending work-type anchors.
        db.record_pending_anchor(
            s.work_id,
            AnchorType::new(AnchorType::OL_WORK),
            "/works/OL123W",
        )
        .await
        .unwrap();
        db.record_pending_anchor(s.work_id, AnchorType::new(AnchorType::GR_WORK), "12345")
            .await
            .unwrap();

        // Affirm only OL_WORK — GR_WORK remains pending (still chaseable).
        db.confirm_anchor_and_recompute_badge(
            s.work_id,
            AnchorType::new(AnchorType::OL_WORK),
            "/works/OL123W",
            AnchorSetter::User,
        )
        .await
        .unwrap();

        let after = db.get_work(s.user_id, s.work_id).await.unwrap();
        assert_eq!(
            after.identity_status,
            IdentityStatus::Confirmed,
            "badge must be Confirmed when at least one work anchor is confirmed, \
             even while other anchors remain pending"
        );
    }

    // -------------------------------------------------------------------------
    // Test 3: Affirm a pending anchor on a NeedsReview work → badge reaches
    // Confirmed (NeedsReview must not act as a permanent block).
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn affirm_on_needs_review_work_reaches_confirmed() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        // Drive the work into NeedsReview.
        db.set_needs_review(s.work_id).await.unwrap();

        // Record a pending OL_WORK anchor (the resolver put it there before
        // exhausting resolution and surfacing NeedsReview).
        db.record_pending_anchor(
            s.work_id,
            AnchorType::new(AnchorType::OL_WORK),
            "/works/OL456W",
        )
        .await
        .unwrap();

        let before = db.get_work(s.user_id, s.work_id).await.unwrap();
        assert_eq!(before.identity_status, IdentityStatus::NeedsReview);

        // User affirms the pending guess.
        db.confirm_anchor_and_recompute_badge(
            s.work_id,
            AnchorType::new(AnchorType::OL_WORK),
            "/works/OL456W",
            AnchorSetter::User,
        )
        .await
        .unwrap();

        let after = db.get_work(s.user_id, s.work_id).await.unwrap();
        assert_eq!(
            after.identity_status,
            IdentityStatus::Confirmed,
            "NeedsReview must not block badge promotion when the user affirms a work anchor"
        );
    }
}
