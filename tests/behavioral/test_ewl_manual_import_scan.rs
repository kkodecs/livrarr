#![allow(dead_code, unused_imports)]
//! Behavioral tests for english-work-lifecycle `manual_import_scan` directives.

/// REQ-IDs: REQ-001, REQ-002
/// Directive: Single-file user-selected manual import: persisted work carries every selected OL field.
#[tokio::test]
#[ignore = "stage-5: requires implementation or integration infrastructure not available in unit tests"]
async fn test_ewl_workflow_manual_import_scan_service_single_file_user_selected_manual_import_persisted_work_carries(
) {
    todo!()
}

/// REQ-IDs: REQ-001, REQ-002
/// Directive: Multi-file auto-resolution batch: bulk_resolver invoked with concurrency=4.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_workflow_manual_import_scan_service_multi_file_auto_resolution_batch_bulk_resolver_invoked_concurrency(
) {
    todo!()
}
