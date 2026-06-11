#![allow(dead_code, unused_imports)]
//! Behavioral tests for english-work-lifecycle `readarr_import` directives.

/// REQ-IDs: REQ-002, REQ-003, REQ-013
/// Directive: 10-book Readarr batch: 8 Confirmed, 1 Pending, 1 Conflict → BulkImportReport reflects all four counts; conflict row created.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_workflow_readarr_import_10_book_readarr_batch_8_confirmed_1_pending_1() {
    todo!()
}

/// REQ-IDs: REQ-002, REQ-003
/// Directive: Author with 5 newly-surfaced works: 5 candidates produced; 5 work_service.add invocations.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_workflow_author_monitor_author_5_newly_surfaced_works_5_candidates_produced_5() {
    todo!()
}

/// REQ-IDs: REQ-002
/// Directive: List with 100 rows + concurrency=4: bulk_resolver respects cap.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_workflow_list_service_confirm_list_100_rows_concurrency_4_bulk_resolver_respects_cap(
) {
    todo!()
}
