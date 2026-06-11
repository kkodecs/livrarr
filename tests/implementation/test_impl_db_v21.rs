use librarr_db::mem::InMemoryDb;
use librarr_db::EnrichmentRetryDb;
use librarr_domain::{EnrichmentStatus, WorkId};

#[tokio::test]
async fn test_impl_db_v21_increment_on_enriched_status_increments_count_without_state_transition() {
    // Targets state machine edge case: incrementing a work in Enriched status.
    // The implementation increments the counter regardless of status and should
    // not transition Enriched to any other state.
    let db = InMemoryDb::default();
    let user_id = 1i64;
    let work_id = 100i64;

    db.seed_work_for_test(user_id, work_id, EnrichmentStatus::Enriched, 0)
        .await;

    db.increment_retry_count(user_id, work_id).await.unwrap();

    let work = db.get_work_by_id(work_id).await.unwrap();
    assert_eq!(work.enrichment_status, EnrichmentStatus::Enriched);
    assert_eq!(work.enrichment_retry_count, 1);
}

#[tokio::test]
async fn test_impl_db_v21_increment_on_pending_status_increments_count_without_state_transition() {
    // Targets state machine edge case: incrementing a work in Pending status.
    // Pending should remain Pending, but the retry count still increments.
    let db = InMemoryDb::default();
    let user_id = 2i64;
    let work_id = 101i64;

    db.seed_work_for_test(user_id, work_id, EnrichmentStatus::Pending, 0)
        .await;

    db.increment_retry_count(user_id, work_id).await.unwrap();

    let work = db.get_work_by_id(work_id).await.unwrap();
    assert_eq!(work.enrichment_status, EnrichmentStatus::Pending);
    assert_eq!(work.enrichment_retry_count, 1);
}

#[tokio::test]
async fn test_impl_db_v21_boundary_retry_count_exactly_3_excludes_all_statuses_from_retry_listing()
{
    // Targets boundary behavior at exactly retry_count == 3 across multiple statuses.
    // Even Failed/Partial at exactly 3 must be excluded because listing requires < 3.
    let db = InMemoryDb::default();
    let user_id = 3i64;

    let failed_id = 200i64;
    let partial_id = 201i64;
    let pending_id = 202i64;
    let enriched_id = 203i64;
    let exhausted_id = 204i64;

    db.seed_work_for_test(user_id, failed_id, EnrichmentStatus::Failed, 3)
        .await;
    db.seed_work_for_test(user_id, partial_id, EnrichmentStatus::Partial, 3)
        .await;
    db.seed_work_for_test(user_id, pending_id, EnrichmentStatus::Pending, 3)
        .await;
    db.seed_work_for_test(user_id, enriched_id, EnrichmentStatus::Enriched, 3)
        .await;
    db.seed_work_for_test(user_id, exhausted_id, EnrichmentStatus::Exhausted, 3)
        .await;

    let works = db.list_works_for_retry().await.unwrap();
    let ids: Vec<_> = works.into_iter().map(|w| w.id).collect();

    assert!(!ids.contains(&failed_id));
    assert!(!ids.contains(&partial_id));
    assert!(!ids.contains(&pending_id));
    assert!(!ids.contains(&enriched_id));
    assert!(!ids.contains(&exhausted_id));
    assert!(ids.is_empty());
}

#[tokio::test]
async fn test_impl_db_v21_large_mixed_dataset_filters_only_failed_or_partial_below_threshold() {
    // Targets filtering correctness with a larger mixed dataset.
    // Verifies only Failed/Partial with retry_count < 3 are returned.
    let db = InMemoryDb::default();
    let user_a = 10i64;
    let user_b = 11i64;

    let mut expected_ids = Vec::new();

    for i in 0u64..60 {
        let user_id = if i % 2 == 0 { user_a } else { user_b };
        let work_id = (1000 + i) as WorkId;

        let (status, retry_count, should_include) = match i % 10 {
            0 => (EnrichmentStatus::Failed, 0, true),
            1 => (EnrichmentStatus::Failed, 1, true),
            2 => (EnrichmentStatus::Failed, 2, true),
            3 => (EnrichmentStatus::Failed, 3, false),
            4 => (EnrichmentStatus::Partial, 0, true),
            5 => (EnrichmentStatus::Partial, 2, true),
            6 => (EnrichmentStatus::Partial, 3, false),
            7 => (EnrichmentStatus::Pending, 0, false),
            8 => (EnrichmentStatus::Enriched, 1, false),
            _ => (EnrichmentStatus::Exhausted, 0, false),
        };

        db.seed_work_for_test(user_id, work_id, status, retry_count)
            .await;
        if should_include {
            expected_ids.push(work_id);
        }
    }

    let works = db.list_works_for_retry().await.unwrap();
    let mut actual_ids: Vec<_> = works.into_iter().map(|w| w.id).collect();

    expected_ids.sort_by_key(|id| format!("{:?}", id));
    actual_ids.sort_by_key(|id| format!("{:?}", id));

    assert_eq!(actual_ids.len(), expected_ids.len());
    assert_eq!(actual_ids, expected_ids);
}

#[tokio::test]
async fn test_impl_db_v21_double_reset_on_already_pending_work_is_idempotent() {
    // Targets double-reset behavior on a work already in Pending.
    // Reset should leave it Pending with retry_count 0, and repeating it should be harmless.
    let db = InMemoryDb::default();
    let user_id = 20i64;
    let work_id = 300i64;

    db.seed_work_for_test(user_id, work_id, EnrichmentStatus::Pending, 2)
        .await;

    db.reset_enrichment_for_refresh(user_id, work_id)
        .await
        .unwrap();
    let after_first = db.get_work_by_id(work_id).await.unwrap();
    assert_eq!(after_first.enrichment_status, EnrichmentStatus::Pending);
    assert_eq!(after_first.enrichment_retry_count, 0);

    db.reset_enrichment_for_refresh(user_id, work_id)
        .await
        .unwrap();
    let after_second = db.get_work_by_id(work_id).await.unwrap();
    assert_eq!(after_second.enrichment_status, EnrichmentStatus::Pending);
    assert_eq!(after_second.enrichment_retry_count, 0);
}

#[tokio::test]
async fn test_impl_db_v21_increment_past_three_on_exhausted_keeps_exhausted_and_continues_counting()
{
    // Targets incrementing past the threshold on an already Exhausted work.
    // Count should continue increasing beyond 3 while status remains Exhausted.
    let db = InMemoryDb::default();
    let user_id = 21i64;
    let work_id = 301i64;

    db.seed_work_for_test(user_id, work_id, EnrichmentStatus::Exhausted, 3)
        .await;

    db.increment_retry_count(user_id, work_id).await.unwrap();
    let work = db.get_work_by_id(work_id).await.unwrap();
    assert_eq!(work.enrichment_status, EnrichmentStatus::Exhausted);
    assert_eq!(work.enrichment_retry_count, 4);

    db.increment_retry_count(user_id, work_id).await.unwrap();
    let work = db.get_work_by_id(work_id).await.unwrap();
    assert_eq!(work.enrichment_status, EnrichmentStatus::Exhausted);
    assert_eq!(work.enrichment_retry_count, 5);
}

#[tokio::test]
async fn test_impl_db_v21_increment_past_three_on_failed_transitions_once_then_keeps_exhausted() {
    // Targets incrementing a Failed work already at the boundary.
    // First increment should move it to Exhausted with count 4, and later increments
    // should keep Exhausted while continuing to increment.
    let db = InMemoryDb::default();
    let user_id = 22i64;
    let work_id = 302i64;

    db.seed_work_for_test(user_id, work_id, EnrichmentStatus::Failed, 3)
        .await;

    db.increment_retry_count(user_id, work_id).await.unwrap();
    let work = db.get_work_by_id(work_id).await.unwrap();
    assert_eq!(work.enrichment_status, EnrichmentStatus::Exhausted);
    assert_eq!(work.enrichment_retry_count, 4);

    db.increment_retry_count(user_id, work_id).await.unwrap();
    let work = db.get_work_by_id(work_id).await.unwrap();
    assert_eq!(work.enrichment_status, EnrichmentStatus::Exhausted);
    assert_eq!(work.enrichment_retry_count, 5);
}

#[tokio::test]
async fn test_impl_db_v21_increment_past_three_on_partial_keeps_partial_and_continues_counting() {
    // Targets incrementing past 3 on Partial.
    // Unlike Failed, Partial should not transition to Exhausted even at 4+.
    let db = InMemoryDb::default();
    let user_id = 23i64;
    let work_id = 303i64;

    db.seed_work_for_test(user_id, work_id, EnrichmentStatus::Partial, 3)
        .await;

    db.increment_retry_count(user_id, work_id).await.unwrap();
    let work = db.get_work_by_id(work_id).await.unwrap();
    assert_eq!(work.enrichment_status, EnrichmentStatus::Partial);
    assert_eq!(work.enrichment_retry_count, 4);

    db.increment_retry_count(user_id, work_id).await.unwrap();
    let work = db.get_work_by_id(work_id).await.unwrap();
    assert_eq!(work.enrichment_status, EnrichmentStatus::Partial);
    assert_eq!(work.enrichment_retry_count, 5);
}

#[tokio::test]
async fn test_impl_db_v21_user_isolation_reset_uses_work_id_not_user_id() {
    // Targets user isolation semantics.
    // The implementation ignores user_id and looks up by work_id only, so using a different
    // user_id should still mutate the targeted work and not affect another user's work.
    let db = InMemoryDb::default();
    let owner_user = 30i64;
    let other_user = 31i64;

    let target_work = 400i64;
    let untouched_work = 401i64;

    db.seed_work_for_test(owner_user, target_work, EnrichmentStatus::Failed, 2)
        .await;
    db.seed_work_for_test(other_user, untouched_work, EnrichmentStatus::Partial, 1)
        .await;

    db.reset_enrichment_for_refresh(other_user, target_work)
        .await
        .unwrap();

    let target = db.get_work_by_id(target_work).await.unwrap();
    let untouched = db.get_work_by_id(untouched_work).await.unwrap();

    assert_eq!(target.enrichment_status, EnrichmentStatus::Pending);
    assert_eq!(target.enrichment_retry_count, 0);

    assert_eq!(untouched.enrichment_status, EnrichmentStatus::Partial);
    assert_eq!(untouched.enrichment_retry_count, 1);
}

#[tokio::test]
async fn test_impl_db_v21_user_isolation_increment_uses_work_id_not_user_id() {
    // Targets user isolation semantics for increment.
    // Passing a mismatched user_id should still increment the work identified by work_id.
    let db = InMemoryDb::default();
    let owner_user = 32i64;
    let other_user = 33i64;

    let target_work = 402i64;
    let untouched_work = 403i64;

    db.seed_work_for_test(owner_user, target_work, EnrichmentStatus::Failed, 1)
        .await;
    db.seed_work_for_test(other_user, untouched_work, EnrichmentStatus::Failed, 1)
        .await;

    db.increment_retry_count(other_user, target_work)
        .await
        .unwrap();

    let target = db.get_work_by_id(target_work).await.unwrap();
    let untouched = db.get_work_by_id(untouched_work).await.unwrap();

    assert_eq!(target.enrichment_status, EnrichmentStatus::Failed);
    assert_eq!(target.enrichment_retry_count, 2);

    assert_eq!(untouched.enrichment_status, EnrichmentStatus::Failed);
    assert_eq!(untouched.enrichment_retry_count, 1);
}

#[tokio::test]
async fn test_impl_db_v21_concurrent_operations_on_different_works_are_independent() {
    // Targets concurrent access across different works.
    // Simultaneous operations should complete without interfering with each other's final state.
    let db = InMemoryDb::default();
    let user_id = 40i64;

    let work_a = 500i64;
    let work_b = 501i64;
    let work_c = 502i64;
    let work_d = 503i64;

    db.seed_work_for_test(user_id, work_a, EnrichmentStatus::Failed, 1)
        .await;
    db.seed_work_for_test(user_id, work_b, EnrichmentStatus::Partial, 2)
        .await;
    db.seed_work_for_test(user_id, work_c, EnrichmentStatus::Pending, 5)
        .await;
    db.seed_work_for_test(user_id, work_d, EnrichmentStatus::Exhausted, 3)
        .await;

    let f1 = db.increment_retry_count(user_id, work_a);
    let f2 = db.increment_retry_count(user_id, work_b);
    let f3 = db.reset_enrichment_for_refresh(user_id, work_c);
    let f4 = db.increment_retry_count(user_id, work_d);

    let (r1, r2, r3, r4) = tokio::join!(f1, f2, f3, f4);
    r1.unwrap();
    r2.unwrap();
    r3.unwrap();
    r4.unwrap();

    let a = db.get_work_by_id(work_a).await.unwrap();
    let b = db.get_work_by_id(work_b).await.unwrap();
    let c = db.get_work_by_id(work_c).await.unwrap();
    let d = db.get_work_by_id(work_d).await.unwrap();

    assert_eq!(a.enrichment_status, EnrichmentStatus::Failed);
    assert_eq!(a.enrichment_retry_count, 2);

    assert_eq!(b.enrichment_status, EnrichmentStatus::Partial);
    assert_eq!(b.enrichment_retry_count, 3);

    assert_eq!(c.enrichment_status, EnrichmentStatus::Pending);
    assert_eq!(c.enrichment_retry_count, 0);

    assert_eq!(d.enrichment_status, EnrichmentStatus::Exhausted);
    assert_eq!(d.enrichment_retry_count, 4);
}

#[tokio::test]
async fn test_impl_db_v21_list_works_for_retry_on_empty_db_returns_empty_vec() {
    // Targets empty database behavior.
    // Listing retryable works from an empty DB should succeed and return an empty vector.
    let db = InMemoryDb::default();

    let works = db.list_works_for_retry().await.unwrap();

    assert!(works.is_empty());
}
