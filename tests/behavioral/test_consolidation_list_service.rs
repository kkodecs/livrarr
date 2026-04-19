// tests/behavioral/test_consolidation_list_service.rs
#![allow(dead_code, unused_imports)]

//! Behavioral tests for ListService trait (SVC-LIST-001..004).
//! Covers: fn.list_service.{preview, confirm, undo, list_imports}
//! Added for redesign phase:
//! - future DB-backed preview/confirm/complete/undo contracts captured as ignored tests

use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{ListImportDb, WorkDb};
use livrarr_domain::services::*;
use livrarr_metadata::list_service::ListServiceImpl;
use livrarr_metadata::work_service::WorkServiceImpl;

fn test_data_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("livrarr-test-{}", std::process::id()))
}

fn stub_http() -> StubHttpFetcher {
    StubHttpFetcher::new()
}

/// Helper: build a ListServiceImpl backed by a real in-memory DB.
/// Creates a user with id=USER and returns the service.
async fn make_service() -> ListServiceImpl<
    livrarr_db::sqlite::SqliteDb,
    WorkServiceImpl<
        livrarr_db::sqlite::SqliteDb,
        livrarr_metadata::work_service::StubNoEnrichment,
        StubHttpFetcher,
    >,
> {
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
    ListServiceImpl::new(db, work_svc)
}

/// Goodreads-style CSV with Title, Author, ISBN13 columns.
fn sample_csv() -> String {
    "Title,Author,ISBN13\n\
     The Great Gatsby,F. Scott Fitzgerald,9780743273565\n\
     1984,George Orwell,9780451524935\n\
     Invisible Man,Ralph Ellison,9780679732761\n"
        .to_string()
}

/// CSV with a single row.
fn single_row_csv() -> String {
    "Title,Author,ISBN13\nDune,Frank Herbert,9780441172719\n".to_string()
}

/// User ID for tests.
const USER: i64 = 1;

// =============================================================================
// preview
// =============================================================================

#[tokio::test]
async fn test_list_preview_valid_csv_returns_parsed_rows() {
    let svc = make_service().await;

    let req = ListPreviewRequest {
        source: ListSource::GoodreadsCsv,
        content: sample_csv(),
    };

    let resp = svc.preview(USER, req).await.unwrap();
    assert_eq!(resp.rows.len(), 3);
    assert!(!resp.import_id.is_empty());
    assert_eq!(resp.rows[0].title, "The Great Gatsby");
    assert_eq!(resp.rows[0].author.as_deref(), Some("F. Scott Fitzgerald"));
    assert_eq!(resp.rows[0].isbn.as_deref(), Some("9780743273565"));
    // No existing works in DB, so rows that can't be matched should show NotFound
    // (unless the service defaults to Matched for "ready to add" rows)
    for row in &resp.rows {
        assert!(
            row.match_status == ListMatchStatus::Matched
                || row.match_status == ListMatchStatus::NotFound,
            "row '{}' should be Matched or NotFound, got {:?}",
            row.title,
            row.match_status
        );
    }
}

#[tokio::test]
async fn test_list_preview_malformed_csv_returns_parse_error() {
    let svc = make_service().await;

    // No Title column at all.
    let req = ListPreviewRequest {
        source: ListSource::GoodreadsCsv,
        content: "Foo,Bar\nval1,val2\n".to_string(),
    };

    let result = svc.preview(USER, req).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ListServiceError::Parse(_) => {}
        other => panic!("expected Parse error, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_list_preview_existing_works_marked_already_exists() {
    let svc = make_service().await;

    // Add a work that matches by title.
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

    let req = ListPreviewRequest {
        source: ListSource::GoodreadsCsv,
        content: sample_csv(),
    };

    let resp = svc.preview(USER, req).await.unwrap();
    assert_eq!(resp.rows.len(), 3);

    // "The Great Gatsby" should be AlreadyExists (matched by title).
    let gatsby = &resp.rows[0];
    assert_eq!(gatsby.title, "The Great Gatsby");
    assert_eq!(gatsby.match_status, ListMatchStatus::AlreadyExists);

    // Other rows should be Matched.
    assert_eq!(resp.rows[1].match_status, ListMatchStatus::Matched);
    assert_eq!(resp.rows[2].match_status, ListMatchStatus::Matched);
}

#[tokio::test]
async fn test_list_preview_does_not_create_works() {
    let svc = make_service().await;

    let before = svc.db.list_works(USER).await.unwrap();
    assert!(before.is_empty());

    let req = ListPreviewRequest {
        source: ListSource::GoodreadsCsv,
        content: sample_csv(),
    };
    let _resp = svc.preview(USER, req).await.unwrap();

    let after = svc.db.list_works(USER).await.unwrap();
    assert!(after.is_empty(), "preview should not create works");
}

// =============================================================================
// confirm
// =============================================================================

#[tokio::test]
async fn test_list_confirm_adds_matched_works() {
    let svc = make_service().await;

    let req = ListPreviewRequest {
        source: ListSource::GoodreadsCsv,
        content: sample_csv(),
    };
    let preview = svc.preview(USER, req).await.unwrap();

    let result = svc.confirm(USER, &preview.import_id).await.unwrap();
    assert_eq!(result.added, 3);
    assert_eq!(result.skipped, 0);
    assert!(result.failed.is_empty());

    let works = svc.db.list_works(USER).await.unwrap();
    assert_eq!(works.len(), 3);
}

#[tokio::test]
async fn test_list_confirm_skips_already_existing() {
    let svc = make_service().await;

    // Pre-add a work that will match by title.
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

    let req = ListPreviewRequest {
        source: ListSource::GoodreadsCsv,
        content: sample_csv(),
    };
    let preview = svc.preview(USER, req).await.unwrap();

    let result = svc.confirm(USER, &preview.import_id).await.unwrap();
    // Gatsby was already exists at preview time, so skipped.
    assert_eq!(result.skipped, 1);
    assert_eq!(result.added, 2);
    assert!(result.failed.is_empty());

    // Total works = 1 (pre-existing) + 2 (added) = 3.
    let works = svc.db.list_works(USER).await.unwrap();
    assert_eq!(works.len(), 3);
}

#[tokio::test]
async fn test_list_confirm_single_row_failure_continues() {
    let svc = make_service().await;

    // CSV with one valid row and one with spaces-only title (will fail WorkService::add)
    let csv = "Title,Author,ISBN13\n\
               Dune,Frank Herbert,9780441172719\n\
                  ,George Orwell,9780451524935\n";

    let req = ListPreviewRequest {
        source: ListSource::GoodreadsCsv,
        content: csv.to_string(),
    };
    let preview = svc.preview(USER, req).await.unwrap();

    let result = svc.confirm(USER, &preview.import_id).await.unwrap();
    // One row should succeed, one should fail (empty title) — partial success
    assert!(
        result.added >= 1,
        "at least one row should succeed: added={}",
        result.added
    );
    // The overall confirm call should not have errored — partial success is valid
    let total = result.added + result.skipped + result.failed.len();
    assert!(total > 0, "should have processed at least some rows");
}

#[tokio::test]
async fn test_list_confirm_expired_import_id_returns_not_found() {
    let svc = make_service().await;

    let result = svc.confirm(USER, "nonexistent-import-id").await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ListServiceError::NotFound => {}
        other => panic!("expected NotFound, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_list_confirm_single_use_second_call_not_found() {
    let svc = make_service().await;

    let req = ListPreviewRequest {
        source: ListSource::GoodreadsCsv,
        content: single_row_csv(),
    };
    let preview = svc.preview(USER, req).await.unwrap();

    // First confirm succeeds.
    let result = svc.confirm(USER, &preview.import_id).await.unwrap();
    assert_eq!(result.added, 1);

    // Second confirm fails with NotFound.
    let result2 = svc.confirm(USER, &preview.import_id).await;
    assert!(result2.is_err());
    match result2.unwrap_err() {
        ListServiceError::NotFound => {}
        other => panic!("expected NotFound on second confirm, got: {other:?}"),
    }
}

// =============================================================================
// undo
// =============================================================================

#[tokio::test]
async fn test_list_undo_deletes_imported_works() {
    let svc = make_service().await;

    let req = ListPreviewRequest {
        source: ListSource::GoodreadsCsv,
        content: sample_csv(),
    };
    let preview = svc.preview(USER, req).await.unwrap();
    let import_id = preview.import_id.clone();
    let result = svc.confirm(USER, &import_id).await.unwrap();
    assert_eq!(result.added, 3);

    // Verify works exist.
    let works = svc.db.list_works(USER).await.unwrap();
    assert_eq!(works.len(), 3);

    // Undo.
    let deleted = svc.undo(USER, &import_id).await.unwrap();
    assert_eq!(deleted, 3);

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

// =============================================================================
// list_imports
// =============================================================================

#[tokio::test]
async fn test_list_imports_returns_sessions_for_user() {
    let svc = make_service().await;

    // Create an import by doing preview + confirm.
    let req = ListPreviewRequest {
        source: ListSource::GoodreadsCsv,
        content: single_row_csv(),
    };
    let preview = svc.preview(USER, req).await.unwrap();
    let _result = svc.confirm(USER, &preview.import_id).await.unwrap();

    let imports = svc.list_imports(USER).await.unwrap();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].import_id, preview.import_id);
    assert!(matches!(imports[0].source, ListSource::GoodreadsCsv));
    assert_eq!(imports[0].added_count, 1);

    // Different user should see nothing.
    let other_imports = svc.list_imports(999).await.unwrap();
    assert!(other_imports.is_empty());
}

// =============================================================================
// redesign contracts (future trait/API)
// =============================================================================

#[tokio::test]
#[ignore = "pk-implement: ListService redesign not yet landed; current trait takes ListPreviewRequest { source, content }"]
async fn test_list_preview_auto_detects_csv_format_and_persists_preview() {
    todo!("Call redesigned preview(bytes) with Goodreads-style CSV bytes and no explicit source. Assert format auto-detected, preview persisted to DB, and preview_id plus per-row statuses returned.")
}

#[tokio::test]
#[ignore = "pk-implement: redesigned confirm(preview_id, import_id, row_indices) not yet available"]
async fn test_list_confirm_batched_first_call_creates_import_id() {
    todo!("Create preview, confirm subset of row_indices with import_id=None/first call semantics, assert import_id created and only selected rows imported.")
}

#[tokio::test]
#[ignore = "pk-implement: redesigned complete(import_id) not yet available"]
async fn test_list_complete_marks_import_completed() {
    todo!("Preview and confirm at least one batch, call complete(import_id), assert import marked completed in persistent storage and visible via list_imports/details.")
}

#[tokio::test]
#[ignore = "pk-implement: redesigned undo(import_id) should call WorkService::delete() per work and report removed/skipped"]
async fn test_list_undo_calls_work_service_delete_per_work_and_reports_removed_skipped() {
    todo!("Use stub WorkService tracking delete() calls. Import works, make one delete succeed and one return NotFound/already gone, call undo, assert per-work deletes invoked and result {{works_removed, works_skipped}} matches.")
}

#[tokio::test]
#[ignore = "pk-implement: Conflict error and completed/undone state model not yet available in current trait"]
async fn test_list_confirm_on_completed_import_returns_conflict() {
    todo!("Complete an import, then call confirm again for same import context. Assert Conflict error rather than NotFound or silent success.")
}

#[tokio::test]
#[ignore = "pk-implement: Conflict error and undone state model not yet available in current trait"]
async fn test_list_confirm_on_undone_import_returns_conflict() {
    todo!("Undo an import, then call confirm again for same import context. Assert Conflict error.")
}

#[tokio::test]
#[ignore = "pk-implement: row-level idempotent retry API not yet available"]
async fn test_list_confirm_same_row_retry_returns_already_exists_idempotently() {
    todo!("Confirm a row once, then retry same row in a later confirm batch. Assert row result reports already_exists and no duplicate work is created.")
}

#[tokio::test]
#[ignore = "pk-implement: redesigned preview should be DB-backed and survive service restart"]
async fn test_list_preview_survives_service_restart() {
    todo!("Create preview using one service instance over a real SQLite DB, construct a fresh service with same DB, confirm using preview_id, assert preview rows are still available.")
}

#[tokio::test]
#[ignore = "pk-implement: redesigned batched confirm response shape not yet available"]
async fn test_list_confirm_partial_failures_do_not_fail_batch() {
    todo!("Create preview with a mix of valid and invalid rows, confirm selected rows, assert overall call succeeds and returns per-row statuses with both successes and failures.")
}
