#![allow(dead_code, unused_variables)]

//! Behavioral contracts for `metadata-consistency-phase3b-6-7`.
//!
//! These tests are intentionally written before the implementation lands. They
//! are ignored so the current suite remains green, but each test states the
//! observable behavior that must be made executable when the matching contract
//! API exists.
//!
//! Scope:
//! - Phase 3b: unified enrichment inside `WorkService::add()`
//! - Phase 6: bypass migration to the single work creation gate
//! - Phase 7: enrichment retry and tag convergence background jobs

use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{CreateUserDbRequest, UserDb};
use livrarr_domain::{UserId, UserRole};

const READARR_ADD_CONCURRENCY_LIMIT: usize = 5;
const TAG_CONVERGENCE_BATCH_SIZE: usize = 50;
const STALE_UNENRICHED_MIN_AGE_MINUTES: i64 = 5;
const ORPHAN_FAILED_RETRY_LIMIT: i64 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedTagSyncItemResult {
    library_item_id: i64,
    succeeded: bool,
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedAddCall {
    title: String,
    author_name: String,
    has_source_provider_data: bool,
    has_series_context: bool,
    provenance_setter: &'static str,
}

#[derive(Debug, Default)]
struct ServiceProbe {
    add_calls: Vec<RecordedAddCall>,
    direct_create_work_calls: usize,
    refresh_calls: Vec<(UserId, i64)>,
    cover_downloads: Vec<(i64, String)>,
    tag_results: Vec<RecordedTagSyncItemResult>,
}

async fn sqlite_contract_fixture() -> (livrarr_db::sqlite::SqliteDb, UserId) {
    let db = create_test_db().await;
    let user = db
        .create_user(CreateUserDbRequest {
            username: "metadata-phase3b-6-7".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::Admin,
            api_key_hash: "api-key-hash".to_string(),
        })
        .await
        .expect("test user should be created");
    (db, user.id)
}

fn pending_contract(contract: &str) -> ! {
    panic!("pre-implementation behavioral contract pending: {contract}")
}

#[tokio::test]
#[ignore = "pk-implement: unified enrichment success path not landed"]
async fn phase3b_run_unified_enrichment_success_sets_enriched_syncs_all_tags_and_downloads_cover() {
    let (db, user_id) = sqlite_contract_fixture().await;
    let mut probe = ServiceProbe::default();

    // Given:
    // - WorkService::add() creates a new work through the single creation gate.
    // - provider scatter-gather and merge stubs return a successful merged work.
    // - the merged work has a cover_url.
    // - the DB has several taggable library_items for the work.
    // - TagService returns one successful TagSyncItemResult for each item.
    //
    // When:
    // - add() runs the synchronous run_unified_enrichment step.
    //
    // Then:
    // - add() returns Ok(AddWorkResult), not a deferred/background handle.
    // - AddWorkResult.enrichment_status is EnrichmentStatus::Enriched.
    // - the persisted work row is Enriched.
    // - cover download is attempted once for the merged cover URL.
    // - TagService::retag_library_items receives every taggable item for the work.
    // - every successful item result updates tag_status to Synced.
    pending_contract("Phase 3b success: Enriched + all-file tag sync + cover download");
}

#[tokio::test]
#[ignore = "pk-implement: enrichment failure status contract not landed"]
async fn phase3b_run_unified_enrichment_provider_failure_sets_failed_and_add_returns_ok() {
    let (db, user_id) = sqlite_contract_fixture().await;
    let probe = ServiceProbe::default();

    // Given:
    // - WorkService::add() creates a new work.
    // - provider dispatch, provenance read, merge, or another enrichment DB step
    //   fails.
    //
    // When:
    // - run_unified_enrichment observes the failure.
    //
    // Then:
    // - run_unified_enrichment catches the failure and never panics.
    // - add() still returns Ok(AddWorkResult).
    // - AddWorkResult.enrichment_status is EnrichmentStatus::Failed.
    // - the persisted work row is Failed, not Err/Pending/Unenriched.
    // - no WorkServiceError escapes for enrichment-only failures.
    pending_contract("Phase 3b provider/merge/DB enrichment failure is persisted as Failed");
}

#[tokio::test]
#[ignore = "pk-implement: enrichment conflict status contract not landed"]
async fn phase3b_run_unified_enrichment_conflict_sets_conflict_without_tag_or_cover_side_effects() {
    let (db, user_id) = sqlite_contract_fixture().await;
    let probe = ServiceProbe::default();

    // Given:
    // - provider dispatch succeeds.
    // - MergeEngine returns Ok(MergeOutput { conflict_detected: true, ... }).
    //
    // When:
    // - run_unified_enrichment handles the merge output.
    //
    // Then:
    // - add() returns Ok(AddWorkResult).
    // - AddWorkResult.enrichment_status is EnrichmentStatus::Conflict.
    // - the persisted work row is Conflict.
    // - apply_enrichment_merge does not mark the work Enriched.
    // - cover download and tag sync are skipped for the conflicted merge.
    pending_contract("Phase 3b conflict is a persisted enrichment status, not an error");
}

#[tokio::test]
#[ignore = "pk-implement: per-item TagSyncItemResult contract not landed"]
async fn phase3b_tag_sync_results_update_each_item_status_independently() {
    let (db, user_id) = sqlite_contract_fixture().await;
    let results = vec![
        RecordedTagSyncItemResult {
            library_item_id: 10,
            succeeded: true,
            error: None,
        },
        RecordedTagSyncItemResult {
            library_item_id: 30,
            succeeded: false,
            error: Some("tag write failed".to_string()),
        },
        RecordedTagSyncItemResult {
            library_item_id: 20,
            succeeded: true,
            error: None,
        },
    ];

    // Given:
    // - an enriched work with three taggable library_items.
    // - TagService::retag_library_items returns results out of slice order.
    //
    // When:
    // - run_unified_enrichment processes TagSyncItemResult values.
    //
    // Then:
    // - successful item IDs are updated to TagStatus::Synced.
    // - failed item IDs are updated to TagStatus::Failed.
    // - updates are keyed by TagSyncItemResult.library_item_id, not by item
    //   slice position.
    // - TagService does not update DB tag_status itself; WorkService owns those
    //   writes after receiving item results.
    pending_contract("Phase 3b per-item tag sync result drives per-item DB status");
}

#[tokio::test]
#[ignore = "pk-implement: post-merge generation tagging contract not landed"]
async fn phase3b_tag_sync_uses_post_merge_generation_not_pre_merge_generation() {
    let (db, user_id) = sqlite_contract_fixture().await;
    let pre_merge_generation = 1;
    let post_merge_generation = 2;

    // Given:
    // - a work starts with merge_generation = 1.
    // - apply_enrichment_merge increments the persisted work to generation 2.
    // - the work has taggable library_items.
    //
    // When:
    // - run_unified_enrichment applies the merge and then syncs tags.
    //
    // Then:
    // - WorkService re-reads the work after apply_enrichment_merge.
    // - update_library_item_tag_status writes tagged_at_generation = 2.
    // - no item is recorded at stale generation 1.
    pending_contract("Phase 3b tag sync records the post-merge merge_generation");
}

#[tokio::test]
#[ignore = "pk-implement: series monitor WorkService add migration not landed"]
async fn phase6_series_monitor_creates_via_add_and_links_existing_when_created_false() {
    let (db, user_id) = sqlite_contract_fixture().await;
    let mut probe = ServiceProbe::default();

    // Given:
    // - a monitored series contains one missing book and one book whose identity
    //   dedups to an existing work.
    // - WorkService::add is stubbed to record calls and can return created=false.
    // - direct DB create_work is guarded by the probe.
    //
    // When:
    // - SeriesQueryService processes missing series books.
    //
    // Then:
    // - every new-work path calls WorkService::add().
    // - SeriesQueryService does not call db.create_work() directly.
    // - created=true relies on add() to persist series_id/name/position.
    // - created=false explicitly links the existing work to the series.
    // - no manual addtime provenance, background enrich_work spawn, or separate
    //   cover download remains in the series monitor.
    pending_contract("Phase 6 series monitor uses add() and links created=false matches");
}

#[tokio::test]
#[ignore = "pk-implement: Readarr WorkService add/source_provider_data migration not landed"]
async fn phase6_readarr_import_builds_source_provider_data_and_enriches_each_work() {
    let (db, user_id) = sqlite_contract_fixture().await;
    let mut probe = ServiceProbe::default();

    // Given:
    // - Readarr books include overview, ISBN, ASIN, publisher, genres,
    //   page_count, ratings, cover image, and series metadata.
    // - WorkService::add is stubbed to record AddWorkRequest values.
    //
    // When:
    // - Readarr import creates or dedups works.
    //
    // Then:
    // - every creation attempt goes through WorkService::add().
    // - AddWorkRequest.source_provider_data is Some(...) and contains the
    //   mapped RdBook fields.
    // - AddWorkRequest.provenance_setter is ProvenanceSetter::Import.
    // - no readarr_import_service.create_work/db.create_work bypass is used.
    // - add() runs synchronous enrichment once per created work.
    // - result.created=false uses the existing work_id without treating dedup
    //   as an import error.
    pending_contract("Phase 6 Readarr import maps RdBook to SourceProviderData and add()");
}

#[tokio::test]
#[ignore = "pk-implement: Readarr bounded concurrency contract not landed"]
async fn phase6_readarr_bulk_import_uses_buffer_unordered_five() {
    let (db, user_id) = sqlite_contract_fixture().await;
    assert_eq!(READARR_ADD_CONCURRENCY_LIMIT, 5);

    // Given:
    // - a Readarr import with more than five books.
    // - WorkService::add is stubbed to block until released and records the
    //   maximum number of simultaneous calls.
    //
    // When:
    // - bulk work creation starts.
    //
    // Then:
    // - no more than five add() futures are in flight at any instant.
    // - all books are eventually processed.
    // - results may complete out of input order, matching buffer_unordered(5)
    //   rather than sequential or unbounded behavior.
    pending_contract("Phase 6 Readarr bulk add() calls are bounded by buffer_unordered(5)");
}

#[tokio::test]
#[ignore = "pk-implement: tag convergence job contract not landed"]
async fn phase7_tag_convergence_sweep_selects_pending_stale_and_new_generation_failed_items() {
    let (db, user_id) = sqlite_contract_fixture().await;
    assert_eq!(TAG_CONVERGENCE_BATCH_SIZE, 50);

    // Given:
    // - enriched works at merge_generation = 3.
    // - library_items include:
    //   * pending at any tagged_at_generation
    //   * synced at generation 2
    //   * failed at generation 2
    //   * synced at current generation 3
    //   * failed at current generation 3
    // - an unenriched work has pending items.
    //
    // When:
    // - tag_convergence_tick runs one batch.
    //
    // Then:
    // - pending items for enriched works are tagged.
    // - stale synced items are retagged.
    // - stale failed items are retried because the work has a newer generation.
    // - current-generation synced items are skipped.
    // - current-generation failed items are skipped to avoid retry loops.
    // - items whose work is not Enriched are skipped.
    // - the query returns root_folder_path/work metadata or otherwise avoids an
    //   N+1 get_work loop.
    pending_contract("Phase 7 tag convergence selects pending and stale items only");
}

#[tokio::test]
#[ignore = "pk-implement: enrichment retry expanded source contract not landed"]
async fn phase7_enrichment_retry_picks_stale_unenriched_and_orphan_failed_only() {
    let (db, user_id) = sqlite_contract_fixture().await;
    assert_eq!(STALE_UNENRICHED_MIN_AGE_MINUTES, 5);
    assert_eq!(ORPHAN_FAILED_RETRY_LIMIT, 3);

    // Given:
    // - an Unenriched work older than five minutes.
    // - an Unenriched work newer than five minutes.
    // - a Failed work with no provider_retry_state rows and retry_count < 3.
    // - a Failed work with provider_retry_state rows.
    // - a Failed work with no provider rows but retry_count >= 3.
    //
    // When:
    // - enrichment_retry_tick runs.
    //
    // Then:
    // - stale Unenriched work is recovered through the unified WorkService path.
    // - recent Unenriched work is not touched.
    // - orphan Failed work with retry_count < 3 is retried and increments
    //   enrichment_retry_count on each attempt.
    // - Failed works with provider retry rows stay on the existing provider
    //   retry path.
    // - orphan Failed works at retry_count >= 3 are skipped until manual refresh.
    // - persisted source_provider_json is used during crash recovery.
    pending_contract("Phase 7 retry covers stale crash recovery and orphan failed retry limits");
}
