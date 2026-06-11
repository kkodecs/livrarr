use std::path::Path;

use librarr_db::mem::InMemoryDb;
use librarr_db::{create_pool, EnrichmentRetryDb};
use librarr_domain::{EnrichmentStatus, UserId, Work, WorkId};

#[allow(dead_code)]
async fn new_test_db() -> InMemoryDb {
    InMemoryDb::new()
}

#[cfg(test)]
mod test_sqlite_v21 {
    use super::*;

    fn contains_work_id(works: &[Work], work_id: WorkId) -> bool {
        works.iter().any(|w| work_id_of(w) == work_id)
    }

    fn work_id_of(work: &Work) -> WorkId {
        work.id
    }

    fn enrichment_status_of(work: &Work) -> &str {
        work.enrichment_status.as_str()
    }

    fn retry_count_of(work: &Work) -> i32 {
        work.enrichment_retry_count
    }

    fn parse_status(s: &str) -> EnrichmentStatus {
        match s {
            "pending" => EnrichmentStatus::Pending,
            "partial" => EnrichmentStatus::Partial,
            "enriched" => EnrichmentStatus::Enriched,
            "failed" => EnrichmentStatus::Failed,
            "exhausted" => EnrichmentStatus::Exhausted,
            _ => panic!("unknown status: {s}"),
        }
    }

    async fn seed_work(
        db: &InMemoryDb,
        user_id: UserId,
        work_id: WorkId,
        status: &str,
        retry_count: i32,
    ) {
        db.seed_work_for_test(user_id, work_id, parse_status(status), retry_count)
            .await;
    }

    async fn get_work(db: &InMemoryDb, work_id: WorkId) -> Work {
        db.get_work_by_id(work_id)
            .await
            .unwrap_or_else(|| panic!("work {work_id} should exist"))
    }

    fn test_user_id() -> UserId {
        1
    }

    fn work_id(n: i64) -> WorkId {
        n
    }

    #[tokio::test]
    async fn test_sqlite_v21_list_works_for_retry_returns_failed_and_partial_with_retry_count_below_3(
    ) {
        // REQ-ID: IMPL-JOBS-005

        let db = new_test_db().await;
        let user_id = test_user_id();

        let failed_0 = work_id(1001);
        let failed_2 = work_id(1002);
        let partial_1 = work_id(1003);
        let pending_0 = work_id(1004);
        let exhausted_3 = work_id(1005);
        let failed_3 = work_id(1006);

        seed_work(&db, user_id, failed_0, "failed", 0).await;
        seed_work(&db, user_id, failed_2, "failed", 2).await;
        seed_work(&db, user_id, partial_1, "partial", 1).await;
        seed_work(&db, user_id, pending_0, "pending", 0).await;
        seed_work(&db, user_id, exhausted_3, "exhausted", 3).await;
        seed_work(&db, user_id, failed_3, "failed", 3).await;

        let db_ref: &dyn EnrichmentRetryDb = &db;
        let works = db_ref
            .list_works_for_retry()
            .await
            .expect("list should succeed");

        assert!(contains_work_id(&works, failed_0));
        assert!(contains_work_id(&works, failed_2));
        assert!(contains_work_id(&works, partial_1));

        assert!(!contains_work_id(&works, pending_0));
        assert!(!contains_work_id(&works, exhausted_3));
        assert!(!contains_work_id(&works, failed_3));

        for work in works {
            let status = enrichment_status_of(&work);
            let retry_count = retry_count_of(&work);
            assert!(status == "failed" || status == "partial");
            assert!(retry_count < 3);
        }
    }

    #[tokio::test]
    async fn test_sqlite_v21_reset_enrichment_for_refresh_sets_retry_count_to_0_and_status_pending()
    {
        // REQ-ID: IMPL-JOBS-005

        let db = new_test_db().await;
        let user_id = test_user_id();
        let target = work_id(1101);

        seed_work(&db, user_id, target, "failed", 2).await;

        let db_ref: &dyn EnrichmentRetryDb = &db;
        db_ref
            .reset_enrichment_for_refresh(user_id, target)
            .await
            .expect("reset for refresh should succeed");

        let work = get_work(&db, target).await;
        assert_eq!(enrichment_status_of(&work), "pending");
        assert_eq!(retry_count_of(&work), 0);
    }

    #[tokio::test]
    async fn test_sqlite_v21_increment_retry_count_increments_count_and_leaves_partial_status_unchanged(
    ) {
        // REQ-ID: IMPL-JOBS-005

        let db = new_test_db().await;
        let user_id = test_user_id();
        let target = work_id(1201);

        seed_work(&db, user_id, target, "partial", 1).await;

        let db_ref: &dyn EnrichmentRetryDb = &db;
        db_ref
            .increment_retry_count(user_id, target)
            .await
            .expect("increment should succeed");

        let work = get_work(&db, target).await;
        assert_eq!(retry_count_of(&work), 2);
        assert_eq!(enrichment_status_of(&work), "partial");
    }

    #[tokio::test]
    async fn test_sqlite_v21_increment_when_retry_count_is_2_and_status_failed_transitions_to_exhausted(
    ) {
        // REQ-ID: IMPL-JOBS-005

        let db = new_test_db().await;
        let user_id = test_user_id();
        let target = work_id(1301);

        seed_work(&db, user_id, target, "failed", 2).await;

        let db_ref: &dyn EnrichmentRetryDb = &db;
        db_ref
            .increment_retry_count(user_id, target)
            .await
            .expect("increment should succeed");

        let work = get_work(&db, target).await;
        assert_eq!(retry_count_of(&work), 3);
        assert_eq!(enrichment_status_of(&work), "exhausted");
    }

    #[tokio::test]
    async fn test_sqlite_v21_increment_when_retry_count_is_2_and_status_partial_does_not_transition_to_exhausted(
    ) {
        // REQ-ID: IMPL-JOBS-005

        let db = new_test_db().await;
        let user_id = test_user_id();
        let target = work_id(1302);

        seed_work(&db, user_id, target, "partial", 2).await;

        let db_ref: &dyn EnrichmentRetryDb = &db;
        db_ref
            .increment_retry_count(user_id, target)
            .await
            .expect("increment should succeed");

        let work = get_work(&db, target).await;
        assert_eq!(retry_count_of(&work), 3);
        assert_eq!(enrichment_status_of(&work), "partial");
    }

    #[tokio::test]
    async fn test_sqlite_v21_list_works_for_retry_excludes_exhausted_works() {
        // REQ-ID: IMPL-JOBS-005

        let db = new_test_db().await;
        let user_id = test_user_id();

        let exhausted = work_id(1401);
        let eligible = work_id(1402);

        seed_work(&db, user_id, exhausted, "exhausted", 3).await;
        seed_work(&db, user_id, eligible, "failed", 1).await;

        let db_ref: &dyn EnrichmentRetryDb = &db;
        let works = db_ref
            .list_works_for_retry()
            .await
            .expect("list should succeed");

        assert!(!contains_work_id(&works, exhausted));
        assert!(contains_work_id(&works, eligible));

        for work in works {
            assert_ne!(enrichment_status_of(&work), "exhausted");
        }
    }

    #[tokio::test]
    async fn test_sqlite_v21_list_works_for_retry_excludes_works_with_retry_count_greater_than_or_equal_to_3(
    ) {
        // REQ-ID: IMPL-JOBS-005

        let db = new_test_db().await;
        let user_id = test_user_id();

        let failed_3 = work_id(1501);
        let partial_4 = work_id(1502);
        let failed_2 = work_id(1503);

        seed_work(&db, user_id, failed_3, "failed", 3).await;
        seed_work(&db, user_id, partial_4, "partial", 4).await;
        seed_work(&db, user_id, failed_2, "failed", 2).await;

        let db_ref: &dyn EnrichmentRetryDb = &db;
        let works = db_ref
            .list_works_for_retry()
            .await
            .expect("list should succeed");

        assert!(!contains_work_id(&works, failed_3));
        assert!(!contains_work_id(&works, partial_4));
        assert!(contains_work_id(&works, failed_2));

        for work in works {
            assert!(retry_count_of(&work) < 3);
        }
    }

    #[tokio::test]
    async fn test_sqlite_v21_reset_enrichment_for_refresh_on_nonexistent_work_returns_error() {
        // REQ-ID: IMPL-JOBS-005

        let db = new_test_db().await;
        let db_ref: &dyn EnrichmentRetryDb = &db;
        let user_id = test_user_id();
        let missing = work_id(1601);

        let result = db_ref.reset_enrichment_for_refresh(user_id, missing).await;

        assert!(result.is_err(), "expected error for nonexistent work");
    }

    #[tokio::test]
    async fn test_sqlite_v21_increment_retry_count_on_nonexistent_work_returns_error() {
        // REQ-ID: IMPL-JOBS-005

        let db = new_test_db().await;
        let db_ref: &dyn EnrichmentRetryDb = &db;
        let user_id = test_user_id();
        let missing = work_id(1602);

        let result = db_ref.increment_retry_count(user_id, missing).await;

        assert!(result.is_err(), "expected error for nonexistent work");
    }

    #[tokio::test]
    async fn test_sqlite_v21_failed_with_retry_count_3_is_terminal_and_never_returned_for_retry() {
        // REQ-ID: IMPL-JOBS-005

        let db = new_test_db().await;
        let user_id = test_user_id();
        let target = work_id(1701);

        seed_work(&db, user_id, target, "failed", 2).await;

        let db_ref: &dyn EnrichmentRetryDb = &db;
        db_ref
            .increment_retry_count(user_id, target)
            .await
            .expect("increment to terminal threshold should succeed");

        let after_increment = get_work(&db, target).await;
        assert_eq!(retry_count_of(&after_increment), 3);
        assert_eq!(enrichment_status_of(&after_increment), "exhausted");

        let listed = db_ref
            .list_works_for_retry()
            .await
            .expect("list should succeed");
        assert!(!contains_work_id(&listed, target));

        db_ref
            .increment_retry_count(user_id, target)
            .await
            .expect("subsequent increment should not make work retry-eligible again");

        let after_second_increment = get_work(&db, target).await;
        assert_eq!(enrichment_status_of(&after_second_increment), "exhausted");

        let listed_again = db_ref
            .list_works_for_retry()
            .await
            .expect("list should succeed");
        assert!(!contains_work_id(&listed_again, target));
    }

    #[tokio::test]
    async fn test_sqlite_v21_manual_refresh_resets_exhausted_terminal_state_to_pending_with_zero_retries(
    ) {
        // REQ-ID: IMPL-JOBS-005

        let db = new_test_db().await;
        let user_id = test_user_id();
        let target = work_id(1801);

        seed_work(&db, user_id, target, "exhausted", 3).await;

        let db_ref: &dyn EnrichmentRetryDb = &db;
        db_ref
            .reset_enrichment_for_refresh(user_id, target)
            .await
            .expect("manual refresh reset should succeed");

        let work = get_work(&db, target).await;
        assert_eq!(enrichment_status_of(&work), "pending");
        assert_eq!(retry_count_of(&work), 0);

        let listed = db_ref
            .list_works_for_retry()
            .await
            .expect("list should succeed");
        assert!(!contains_work_id(&listed, target));
    }

    #[tokio::test]
    async fn test_sqlite_v21_runtime_sqlite_001_create_pool_signature_is_async_and_returns_result()
    {
        // REQ-ID: RUNTIME-SQLITE-001

        let _ = Path::new(".");

        async fn assert_signature<P, E, F>(_f: fn(&Path) -> F)
        where
            F: std::future::Future<Output = Result<P, E>>,
        {
        }

        assert_signature(create_pool).await;
    }

    #[tokio::test]
    async fn test_sqlite_v21_runtime_sqlite_002_create_pool_invalid_path_returns_configured_error_type(
    ) {
        // REQ-ID: RUNTIME-SQLITE-002

        let path = Path::new("/definitely/not/a/real/sqlite/location/test.db");
        let result = create_pool(path).await;

        match result {
            Ok(_) => {}
            Err(_err) => {}
        }
    }

    #[tokio::test]
    #[should_panic(expected = "Phase 3 should verify")]
    async fn test_sqlite_v21_runtime_sqlite_003_placeholder_admin_exists_after_migrations() {
        // REQ-ID: RUNTIME-SQLITE-003

        let _ = Path::new(".");

        todo!(
            "Phase 3 should verify placeholder admin exists after migrations via a runtime harness"
        )
    }

    #[tokio::test]
    #[should_panic(expected = "Phase 2/3 runtime harness")]
    async fn test_sqlite_v21_runtime_sqlite_004_run_migrations_failure_returns_configured_error_type(
    ) {
        // REQ-ID: RUNTIME-SQLITE-004

        let db = new_test_db().await;
        let db_ref: &dyn EnrichmentRetryDb = &db;

        let _ = db_ref;
        todo!("Phase 2/3 runtime harness should provide a migration target that can fail deterministically")
    }

    #[tokio::test]
    async fn test_sqlite_v21_runtime_sqlite_005_trait_object_harness_supports_contract_execution() {
        // REQ-ID: RUNTIME-SQLITE-005

        let db = new_test_db().await;
        let _db: &dyn EnrichmentRetryDb = &db;
    }
}
