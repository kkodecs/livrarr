#![allow(dead_code, unused_imports)]

//! Behavioral tests for search-fallback-chain async add service contracts.

use assert_matches::assert_matches;
use livrarr_behavioral::stubs::create_test_user;
use livrarr_db::test_helpers::create_test_db;
use livrarr_domain::services::SourceProviderData;
use livrarr_domain::{EnrichmentStatus, Work};

fn not_yet_implemented() -> ! {
    todo!("search-fallback-chain async add service implementation is not yet wired")
}

/// REQ-IDs: REQ-008, REQ-009
/// AC-IDs: AC-006, AC-008
/// Directive: cover_manual is written atomically in CREATE INSERT, never via post-add call.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_async_add_service_cover_manual_written_in_initial_insert() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let insert_columns = ["title", "author_name", "cover_url", "cover_manual"];
    let post_add_cover_manual_call_count = 0;

    assert!(user_id > 0);
    assert!(insert_columns.contains(&"cover_manual"));
    assert_eq!(post_add_cover_manual_call_count, 0);
    not_yet_implemented();
}

/// REQ-IDs: REQ-009
/// AC-IDs: AC-008
/// Directive: skip=true, no source data -> Unenriched, enrichment not called, work reloaded from DB.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_async_add_service_skip_true_without_source_data_returns_unenriched_reloaded_work()
{
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let source_provider_data: Option<SourceProviderData> = None;
    let enrichment_called = false;
    let work_reloaded_from_db = true;
    let status = EnrichmentStatus::Unenriched;

    assert!(user_id > 0);
    assert!(source_provider_data.is_none());
    assert!(!enrichment_called);
    assert!(work_reloaded_from_db);
    assert_eq!(status, EnrichmentStatus::Unenriched);
    not_yet_implemented();
}

/// REQ-IDs: REQ-009
/// AC-IDs: AC-008
/// Directive: skip=true, has source data -> enrichment runs (Readarr path).
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_async_add_service_skip_true_with_source_data_still_runs_enrichment() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let source_provider_data = Some(SourceProviderData {
        isbn: Some("9780441013593".into()),
        ..SourceProviderData::default()
    });
    let enrichment_called = true;

    assert!(user_id > 0);
    assert!(source_provider_data.is_some());
    assert!(enrichment_called);
    not_yet_implemented();
}

/// REQ-IDs: REQ-009
/// AC-IDs: AC-008
/// Directive: skip=false -> existing behavior.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_async_add_service_skip_false_runs_existing_sync_enrichment_behavior() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let skip_sync_enrichment = false;
    let enrichment_called = true;

    assert!(user_id > 0);
    assert!(!skip_sync_enrichment);
    assert!(enrichment_called);
    not_yet_implemented();
}

/// REQ-IDs: REQ-009
/// AC-IDs: AC-015
/// Directive: incomplete enrichment survives restart and retry job completes from durable provider_retry_state.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_async_add_service_restart_retry_job_completes_incomplete_enrichment() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let before_restart = EnrichmentStatus::Unenriched;
    let retry_state_rows = 1;
    let after_retry = EnrichmentStatus::Enriched;

    assert!(user_id > 0);
    assert_eq!(before_restart, EnrichmentStatus::Unenriched);
    assert!(retry_state_rows > 0);
    assert_matches!(after_retry, EnrichmentStatus::Enriched);
    not_yet_implemented();
}
