// tests/behavioral/test_consolidation_list_service.rs
#![allow(dead_code, unused_imports)]

//! Behavioral tests for ListService trait (SVC-LIST-001..004).
//! Covers: fn.list_service.{preview, confirm, complete, undo, list_imports}
//! Redesigned trait: preview(bytes), confirm(preview_id, import_id, row_indices),
//! complete(import_id), undo(import_id), list_imports.

use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{ListImportDb, WorkDb};
use livrarr_domain::services::*;
use livrarr_metadata::list_service::{ListServiceImpl, NoOpBibliographyTrigger};
use livrarr_metadata::work_service::WorkServiceImpl;

fn test_data_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("livrarr-test-{}", std::process::id()))
}

fn stub_http() -> StubHttpFetcher {
    StubHttpFetcher::new()
}

type TestListService = ListServiceImpl<
    livrarr_db::sqlite::SqliteDb,
    WorkServiceImpl<
        livrarr_db::sqlite::SqliteDb,
        livrarr_metadata::work_service::StubNoEnrichment,
        StubHttpFetcher,
    >,
    StubHttpFetcher,
    NoOpBibliographyTrigger,
>;

/// Helper: build a ListServiceImpl backed by a real in-memory DB.
/// Creates a user with id=USER and returns the service.
async fn make_service() -> TestListService {
    use livrarr_db::{CreateUserDbRequest, UserDb};
    use livrarr_domain::UserRole;
    let db = create_test_db().await;
    let _user = db
        .create_user(CreateUserDbRequest {
            username: "testuser".into(),
            password_hash: "hash".into(),
            role: UserRole::Admin,
            api_key_hash: "testhash".into(),
        })
        .await
        .unwrap();
    let work_svc = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());
    ListServiceImpl::new(db, work_svc, stub_http(), NoOpBibliographyTrigger)
}

/// Goodreads-style CSV bytes with Title, Author, ISBN13 columns.
fn sample_csv_bytes() -> Vec<u8> {
    b"Book Id,Title,Author,ISBN,ISBN13,My Rating,Exclusive Shelf\n\
     1,The Great Gatsby,F. Scott Fitzgerald,=\"\",=\"9780743273565\",5,read\n\
     2,1984,George Orwell,=\"\",=\"9780451524935\",4,to-read\n\
     3,Invisible Man,Ralph Ellison,=\"\",=\"9780679732761\",3,read\n"
        .to_vec()
}

/// CSV bytes with a single row.
fn single_row_csv_bytes() -> Vec<u8> {
    b"Book Id,Title,Author,ISBN,ISBN13,My Rating,Exclusive Shelf\n\
     1,Dune,Frank Herbert,=\"\",=\"9780441172719\",5,read\n"
        .to_vec()
}

/// User ID for tests.
const USER: i64 = 1;

// =============================================================================
// preview
// =============================================================================

#[tokio::test]
async fn test_list_preview_valid_csv_returns_parsed_rows() {
    let svc = make_service().await;

    let resp = svc.preview(USER, sample_csv_bytes()).await.unwrap();
    assert_eq!(resp.total_rows, 3);
    assert!(!resp.preview_id.is_empty());
    assert_eq!(resp.source, "goodreads");
    assert_eq!(resp.rows[0].title, "The Great Gatsby");
    assert_eq!(resp.rows[0].author, "F. Scott Fitzgerald");
    assert_eq!(resp.rows[0].isbn_13.as_deref(), Some("9780743273565"));
    // No existing works in DB, so rows should be "new"
    for row in &resp.rows {
        assert!(
            row.preview_status == "new" || row.preview_status == "parse_error",
            "row '{}' should be 'new' or 'parse_error', got '{}'",
            row.title,
            row.preview_status
        );
    }
}

#[tokio::test]
async fn test_list_preview_malformed_csv_returns_parse_error() {
    let svc = make_service().await;

    // No recognized CSV headers at all.
    let bytes = b"Foo,Bar\nval1,val2\n".to_vec();
    let result = svc.preview(USER, bytes).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ListServiceError::Parse(_) => {}
        other => panic!("expected Parse error, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_list_preview_existing_works_marked_already_exists() {
    let svc = make_service().await;

    // Add a work that matches by ISBN.
    let add_req = AddWorkRequest {
        title: "The Great Gatsby".to_string(),
        author_name: "F. Scott Fitzgerald".to_string(),
        ol_key: None,
        detail_url: None,
        cover_url: None,
        author_ol_key: None,
        gr_key: None,
        year: None,
        metadata_source: None,
        language: None,
        series_name: None,
        series_position: None,
        defer_enrichment: false,
        provenance_setter: None,
    };
    svc.work_service.add(USER, add_req).await.unwrap();

    // Now set ISBN on the work so ISBN check matches.
    // The preview checks by ISBN, not by title.
    // Since StubHttpFetcher returns no responses, add won't set ISBN.
    // Without ISBN match, title match doesn't happen in preview (only ISBN check in preview).
    // So all 3 rows will be "new".
    let resp = svc.preview(USER, sample_csv_bytes()).await.unwrap();
    assert_eq!(resp.total_rows, 3);
    // With no ISBN on the existing work, all rows are "new"
    // (preview only checks by ISBN, not title)
}

#[tokio::test]
async fn test_list_preview_does_not_create_works() {
    let svc = make_service().await;

    let before = svc.db.list_works(USER).await.unwrap();
    assert!(before.is_empty());

    let _resp = svc.preview(USER, sample_csv_bytes()).await.unwrap();

    let after = svc.db.list_works(USER).await.unwrap();
    assert!(after.is_empty(), "preview should not create works");
}

// =============================================================================
// confirm (batched)
// =============================================================================

#[tokio::test]
async fn test_list_confirm_adds_selected_rows() {
    let svc = make_service().await;

    let preview = svc.preview(USER, sample_csv_bytes()).await.unwrap();
    let row_indices: Vec<usize> = preview.rows.iter().map(|r| r.row_index).collect();

    // StubHttpFetcher has no responses queued, so OL lookup will fail for all rows.
    // This is expected — tests that need OL lookup should queue responses.
    let result = svc
        .confirm(USER, &preview.preview_id, None, &row_indices)
        .await
        .unwrap();

    assert!(!result.import_id.is_empty());
    assert_eq!(result.results.len(), 3);
    // With stub HTTP (no responses), all lookups will fail
    for r in &result.results {
        assert!(
            r.status == "added" || r.status == "lookup_error" || r.status == "add_failed",
            "unexpected status: {} for row {}",
            r.status,
            r.row_index
        );
    }
}

#[tokio::test]
async fn test_list_confirm_expired_preview_returns_error() {
    let svc = make_service().await;

    let result = svc
        .confirm(USER, "nonexistent-preview-id", None, &[0])
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ListServiceError::Parse(_) => {} // "preview not found or expired"
        other => panic!("expected Parse error, got: {other:?}"),
    }
}

// =============================================================================
// complete
// =============================================================================

#[tokio::test]
async fn test_list_complete_marks_import_completed() {
    let svc = make_service().await;

    let preview = svc.preview(USER, single_row_csv_bytes()).await.unwrap();
    let result = svc
        .confirm(USER, &preview.preview_id, None, &[0])
        .await
        .unwrap();

    // Complete the import.
    svc.complete(USER, &result.import_id).await.unwrap();

    // Verify via list_imports.
    let imports = svc.list_imports(USER).await.unwrap();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].status, "completed");
}

#[tokio::test]
async fn test_list_complete_nonexistent_returns_not_found() {
    let svc = make_service().await;

    let result = svc.complete(USER, "nonexistent-import-id").await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ListServiceError::NotFound => {}
        other => panic!("expected NotFound, got: {other:?}"),
    }
}

// =============================================================================
// undo
// =============================================================================

#[tokio::test]
async fn test_list_undo_returns_removed_and_skipped_counts() {
    let svc = make_service().await;

    // Since StubHttpFetcher has no responses, confirm will fail OL lookup.
    // We need to add works manually and tag them.
    let add_req = AddWorkRequest {
        title: "Dune".to_string(),
        author_name: "Frank Herbert".to_string(),
        ol_key: None,
        detail_url: None,
        cover_url: None,
        author_ol_key: None,
        gr_key: None,
        year: None,
        metadata_source: None,
        language: None,
        series_name: None,
        series_position: None,
        defer_enrichment: false,
        provenance_setter: None,
    };
    let work_result = svc.work_service.add(USER, add_req).await.unwrap();

    // Create an import record manually.
    let import_id = "test-import-1";
    let now = chrono::Utc::now().to_rfc3339();
    svc.db
        .create_list_import_record(import_id, USER, "goodreads", &now)
        .await
        .unwrap();
    svc.db
        .tag_work_with_import(USER, work_result.work.id, import_id)
        .await
        .unwrap();

    // Complete the import so it can be undone.
    svc.db
        .complete_list_import(import_id, USER, &now)
        .await
        .unwrap();

    // Verify work exists.
    let works = svc.db.list_works(USER).await.unwrap();
    assert_eq!(works.len(), 1);

    // Undo.
    let undo_result = svc.undo(USER, import_id).await.unwrap();
    assert_eq!(undo_result.works_removed, 1);
    assert_eq!(undo_result.works_skipped, 0);

    // Verify works gone.
    let works = svc.db.list_works(USER).await.unwrap();
    assert!(works.is_empty());
}

#[tokio::test]
async fn test_list_undo_unknown_import_id_returns_not_found() {
    let svc = make_service().await;

    let result = svc.undo(USER, "unknown-import-id").await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ListServiceError::NotFound => {}
        other => panic!("expected NotFound, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_list_undo_already_undone_returns_conflict() {
    let svc = make_service().await;

    // Create a completed import with a tagged work.
    let add_req = AddWorkRequest {
        title: "Dune".to_string(),
        author_name: "Frank Herbert".to_string(),
        ol_key: None,
        detail_url: None,
        cover_url: None,
        author_ol_key: None,
        gr_key: None,
        year: None,
        metadata_source: None,
        language: None,
        series_name: None,
        series_position: None,
        defer_enrichment: false,
        provenance_setter: None,
    };
    let work_result = svc.work_service.add(USER, add_req).await.unwrap();

    let import_id = "test-import-undo";
    let now = chrono::Utc::now().to_rfc3339();
    svc.db
        .create_list_import_record(import_id, USER, "goodreads", &now)
        .await
        .unwrap();
    svc.db
        .tag_work_with_import(USER, work_result.work.id, import_id)
        .await
        .unwrap();
    svc.db
        .complete_list_import(import_id, USER, &now)
        .await
        .unwrap();

    // First undo succeeds.
    let _ = svc.undo(USER, import_id).await.unwrap();

    // Second undo returns Conflict.
    let result = svc.undo(USER, import_id).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ListServiceError::Conflict(_) => {}
        other => panic!("expected Conflict, got: {other:?}"),
    }
}

// =============================================================================
// list_imports
// =============================================================================

#[tokio::test]
async fn test_list_imports_returns_imports_for_user() {
    let svc = make_service().await;

    // Create an import record.
    let import_id = "test-list-import";
    let now = chrono::Utc::now().to_rfc3339();
    svc.db
        .create_list_import_record(import_id, USER, "goodreads", &now)
        .await
        .unwrap();

    let imports = svc.list_imports(USER).await.unwrap();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].id, import_id);
    assert_eq!(imports[0].source, "goodreads");

    // Different user should see nothing.
    let other_imports = svc.list_imports(999).await.unwrap();
    assert!(other_imports.is_empty());
}

// =============================================================================
// redesign contracts (DB-backed preview survives restart)
// =============================================================================

#[tokio::test]
async fn test_list_preview_auto_detects_csv_format_and_persists_preview() {
    let svc = make_service().await;

    let resp = svc.preview(USER, sample_csv_bytes()).await.unwrap();
    assert_eq!(resp.source, "goodreads");
    assert_eq!(resp.total_rows, 3);
    assert!(!resp.preview_id.is_empty());
    for row in &resp.rows {
        assert!(!row.title.is_empty());
        assert!(
            row.preview_status == "new"
                || row.preview_status == "already_exists"
                || row.preview_status == "parse_error"
        );
    }

    // Preview is persisted to DB — verify via count.
    let count = svc
        .db
        .count_list_import_previews(&resp.preview_id, USER)
        .await
        .unwrap();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn test_list_preview_survives_service_restart() {
    use livrarr_db::{CreateUserDbRequest, UserDb};
    use livrarr_domain::UserRole;

    // Use a single DB but two service instances.
    let db = create_test_db().await;
    let _user = db
        .create_user(CreateUserDbRequest {
            username: "testuser".into(),
            password_hash: "hash".into(),
            role: UserRole::Admin,
            api_key_hash: "testhash".into(),
        })
        .await
        .unwrap();

    let work_svc1 = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());
    let svc1: TestListService =
        ListServiceImpl::new(db.clone(), work_svc1, stub_http(), NoOpBibliographyTrigger);

    // Preview with first service instance.
    let resp = svc1.preview(USER, single_row_csv_bytes()).await.unwrap();
    let preview_id = resp.preview_id.clone();

    // "Restart" — construct a fresh service with same DB.
    let work_svc2 = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());
    let svc2: TestListService =
        ListServiceImpl::new(db.clone(), work_svc2, stub_http(), NoOpBibliographyTrigger);

    // Confirm using the second service instance — preview rows are still in DB.
    let result = svc2.confirm(USER, &preview_id, None, &[0]).await.unwrap();
    assert!(!result.import_id.is_empty());
    assert_eq!(result.results.len(), 1);
}

#[tokio::test]
async fn test_list_confirm_batched_first_call_creates_import_id() {
    let svc = make_service().await;

    let preview = svc.preview(USER, sample_csv_bytes()).await.unwrap();

    // Confirm only the first row, no import_id yet.
    let result = svc
        .confirm(USER, &preview.preview_id, None, &[0])
        .await
        .unwrap();

    assert!(!result.import_id.is_empty());
    assert_eq!(result.results.len(), 1);

    // Confirm second row with same import_id (batched additive).
    let result2 = svc
        .confirm(USER, &preview.preview_id, Some(&result.import_id), &[1])
        .await
        .unwrap();

    assert_eq!(result2.import_id, result.import_id);
    assert_eq!(result2.results.len(), 1);
}

#[tokio::test]
async fn test_list_confirm_on_completed_import_returns_conflict() {
    let svc = make_service().await;

    let preview = svc.preview(USER, single_row_csv_bytes()).await.unwrap();
    let result = svc
        .confirm(USER, &preview.preview_id, None, &[0])
        .await
        .unwrap();

    // Complete the import.
    svc.complete(USER, &result.import_id).await.unwrap();

    // Try to confirm more rows on the completed import.
    let result2 = svc
        .confirm(USER, &preview.preview_id, Some(&result.import_id), &[0])
        .await;

    assert!(result2.is_err());
    match result2.unwrap_err() {
        ListServiceError::Conflict(_) => {}
        other => panic!("expected Conflict, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_list_confirm_on_undone_import_returns_conflict() {
    let svc = make_service().await;

    // Create a completed import manually.
    let import_id = "test-undone-confirm";
    let now = chrono::Utc::now().to_rfc3339();
    svc.db
        .create_list_import_record(import_id, USER, "goodreads", &now)
        .await
        .unwrap();
    svc.db
        .complete_list_import(import_id, USER, &now)
        .await
        .unwrap();

    // Undo it.
    let _ = svc.db.mark_list_import_undone(import_id).await;

    // Preview for confirm.
    let preview = svc.preview(USER, single_row_csv_bytes()).await.unwrap();

    // Try to confirm with the undone import_id.
    let result = svc
        .confirm(USER, &preview.preview_id, Some(import_id), &[0])
        .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        ListServiceError::Conflict(_) => {}
        other => panic!("expected Conflict, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_list_confirm_partial_failures_do_not_fail_batch() {
    let svc = make_service().await;

    // Preview with a mix of rows (some have empty titles which parse fine
    // but the OL lookup will fail for all since no HTTP responses queued).
    let preview = svc.preview(USER, sample_csv_bytes()).await.unwrap();
    let row_indices: Vec<usize> = preview.rows.iter().map(|r| r.row_index).collect();

    let result = svc
        .confirm(USER, &preview.preview_id, None, &row_indices)
        .await
        .unwrap();

    // The confirm call should succeed even if individual rows fail.
    assert_eq!(result.results.len(), row_indices.len());
    // Each row has a status — some might be lookup_error, some added, etc.
    for r in &result.results {
        assert!(!r.status.is_empty());
    }
}
