#![allow(dead_code, unused_imports)]

//! Behavioral tests for ImportWorkflow trait (WF-IMPORT-001..004).
//! Covers: fn.import_workflow.{import_grab, retry_import, confirm_scan}
//! Test obligations: test.import.*

use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    ChapterDb, CreateDownloadClientDbRequest, CreateGrabDbRequest, CreateLibraryItemDbRequest,
    CreateUserDbRequest, CreateWorkDbRequest, DownloadClientDb, GrabDb, LibraryItemDb,
    RootFolderDb, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::services::*;
use livrarr_domain::*;
use livrarr_library::import_workflow::ImportWorkflowImpl;
use std::io::Write;
use tempfile::tempdir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn setup_user(db: &SqliteDb) -> i64 {
    let user = db
        .create_user(CreateUserDbRequest {
            username: "testuser".into(),
            password_hash: "hash".into(),
            role: UserRole::Admin,
            api_key_hash: "testhash".into(),
        })
        .await
        .unwrap();
    user.id
}

/// Create a download client and work, return (client_id, work_id).
async fn setup_prereqs(db: &SqliteDb, user_id: i64) -> (i64, i64) {
    let client = db
        .create_download_client(CreateDownloadClientDbRequest {
            name: "test-qbit".into(),
            implementation: DownloadClientImplementation::QBittorrent,
            host: "localhost".into(),
            port: 8080,
            use_ssl: false,
            skip_ssl_validation: false,
            url_base: None,
            username: None,
            password: None,
            category: "livrarr".into(),
            download_dir: None,
            enabled: true,
            api_key: None,
        })
        .await
        .unwrap();

    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Test Book".into(),
            author_name: "Test Author".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    (client.id, work.id)
}

/// Create a grab with content_path pointing to a source directory.
async fn seed_grab_with_content_path(
    db: &SqliteDb,
    user_id: i64,
    work_id: i64,
    download_client_id: i64,
    content_path: &str,
    status: GrabStatus,
    size: Option<i64>,
) -> Grab {
    let grab = db
        .upsert_grab(CreateGrabDbRequest {
            user_id,
            work_id,
            download_client_id,
            title: "Test Grab".into(),
            indexer: "test-indexer".into(),
            guid: format!("guid-{}", rand_suffix()),
            size,
            download_url: "magnet:?xt=urn:btih:abc123".into(),
            download_id: Some("hash-abc123".into()),
            status,
            media_type: None,
        })
        .await
        .unwrap();
    db.set_grab_content_path(user_id, grab.id, content_path)
        .await
        .unwrap();
    db.get_grab(user_id, grab.id).await.unwrap()
}

fn rand_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{n:x}")
}

/// Create a tempdir with source files of the given names. Returns the tempdir path.
fn create_source_dir(filenames: &[&str]) -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    for name in filenames {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(&path).unwrap();
        // Write some bytes so files aren't zero-length
        f.write_all(b"test content for import").unwrap();
    }
    dir
}

fn make_workflow(db: SqliteDb) -> ImportWorkflowImpl<SqliteDb> {
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(2));
    let data_dir = std::sync::Arc::new(std::path::PathBuf::from("/tmp/livrarr-test"));
    ImportWorkflowImpl::new(
        db,
        semaphore,
        data_dir,
        std::sync::Arc::new(livrarr_behavioral::stubs::TagwriteChapterExtractor),
    )
}

// =============================================================================
// import_grab
// =============================================================================

#[tokio::test]
async fn test_import_grab_happy_path_imports_all_files() {
    // WF-IMPORT-002, test.import.happy_path: Given complete download, imports all files and returns Imported
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (client_id, work_id) = setup_prereqs(&db, user_id).await;

    // Create source directory with importable files
    let source_dir = create_source_dir(&["book.epub"]);
    let source_path = source_dir.path().to_str().unwrap();

    // Create root folder for ebooks
    let library_dir = tempdir().unwrap();
    db.create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    let grab = seed_grab_with_content_path(
        &db,
        user_id,
        work_id,
        client_id,
        source_path,
        GrabStatus::Confirmed,
        None,
    )
    .await;

    let wf = make_workflow(db.clone());
    let result = wf.import_grab(user_id, grab.id).await.unwrap();

    assert_eq!(result.final_status, GrabStatus::Imported);
    assert_eq!(result.imported_files.len(), 1);
    assert!(result.failed_files.is_empty());
    assert_eq!(result.imported_files[0].media_type, MediaType::Ebook);

    // Verify library item exists in DB
    let items = db
        .list_library_items_by_work(user_id, work_id)
        .await
        .unwrap();
    assert_eq!(items.len(), 1);

    // Verify file was actually copied
    let target_path = library_dir.path().join(&items[0].path);
    assert!(target_path.exists());
}

#[tokio::test]
async fn test_import_grab_partial_failure_in_failed_files() {
    // WF-IMPORT-003, test.import.partial_failure: Given some file failures, returns ImportedWithErrors with failed_files
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (client_id, work_id) = setup_prereqs(&db, user_id).await;

    // Create source dir with one valid ebook and one valid audiobook
    let source_dir = create_source_dir(&["book.epub", "audio.m4b"]);
    let source_path = source_dir.path().to_str().unwrap();

    // Only create ebook root folder — audiobook will fail with no root folder
    let library_dir = tempdir().unwrap();
    db.create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    let grab = seed_grab_with_content_path(
        &db,
        user_id,
        work_id,
        client_id,
        source_path,
        GrabStatus::Confirmed,
        None,
    )
    .await;

    let wf = make_workflow(db.clone());
    let result = wf.import_grab(user_id, grab.id).await.unwrap();

    // Should have imported the ebook and failed the audiobook
    assert_eq!(result.imported_files.len(), 1);
    assert_eq!(result.failed_files.len(), 1);
    assert!(result.failed_files[0].error.contains("no root folder"));

    // Overall status should still be Imported (some succeeded)
    assert_eq!(result.final_status, GrabStatus::Imported);
}

#[tokio::test]
async fn test_import_grab_nonexistent_returns_error() {
    // WF-IMPORT-001: Given nonexistent grab, returns GrabNotFound
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;

    let wf = make_workflow(db);
    let result = wf.import_grab(user_id, 99999).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, ImportWorkflowError::GrabNotFound),
        "expected GrabNotFound, got: {err:?}"
    );
}

#[tokio::test]
async fn test_import_grab_inaccessible_source_returns_error() {
    // WF-IMPORT-001: Given inaccessible source, returns SourceInaccessible
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (client_id, work_id) = setup_prereqs(&db, user_id).await;

    // Point to a nonexistent path
    let grab = seed_grab_with_content_path(
        &db,
        user_id,
        work_id,
        client_id,
        "/nonexistent/path/that/does/not/exist",
        GrabStatus::Confirmed,
        None,
    )
    .await;

    let wf = make_workflow(db);
    let result = wf.import_grab(user_id, grab.id).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, ImportWorkflowError::SourceInaccessible),
        "expected SourceInaccessible, got: {err:?}"
    );
}

#[tokio::test]
async fn test_import_grab_duplicate_file_skipped() {
    // WF-IMPORT-004, test.import.dedup: Given duplicate file, skips it
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (client_id, work_id) = setup_prereqs(&db, user_id).await;

    let source_dir = create_source_dir(&["book.epub"]);
    let source_path = source_dir.path().to_str().unwrap();

    let library_dir = tempdir().unwrap();
    db.create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    // First import
    let grab1 = seed_grab_with_content_path(
        &db,
        user_id,
        work_id,
        client_id,
        source_path,
        GrabStatus::Confirmed,
        None,
    )
    .await;

    let wf = make_workflow(db.clone());
    let result1 = wf.import_grab(user_id, grab1.id).await.unwrap();
    assert_eq!(result1.imported_files.len(), 1);

    // Second import of same source (new grab pointing to same work)
    let grab2 = seed_grab_with_content_path(
        &db,
        user_id,
        work_id,
        client_id,
        source_path,
        GrabStatus::Confirmed,
        None,
    )
    .await;

    let result2 = wf.import_grab(user_id, grab2.id).await.unwrap();
    // File should be skipped as duplicate (target exists + library item exists)
    assert!(
        !result2.skipped_files.is_empty(),
        "expected at least one skipped file, got: imported={} skipped={} failed={}",
        result2.imported_files.len(),
        result2.skipped_files.len(),
        result2.failed_files.len()
    );

    // Should not create a second library item
    let items = db
        .list_library_items_by_work(user_id, work_id)
        .await
        .unwrap();
    assert_eq!(items.len(), 1, "dedup should prevent second library item");
}

#[tokio::test]
async fn test_import_grab_orphan_adoption() {
    // WF-IMPORT-004, test.import.orphan_adoption: Given orphaned file (exists but no DB record), adopts it
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (client_id, work_id) = setup_prereqs(&db, user_id).await;

    let source_dir = create_source_dir(&["book.epub"]);
    let source_path = source_dir.path().to_str().unwrap();

    let library_dir = tempdir().unwrap();
    db.create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    // Pre-create the target file on disk (simulating a crash where file was copied but DB insert failed)
    let work = db.get_work(user_id, work_id).await.unwrap();
    let author_san = sanitize_path_component(&work.author_name, "Unknown Author");
    let title_san = sanitize_path_component(&work.title, "Unknown Title");
    let target_dir = library_dir
        .path()
        .join(user_id.to_string())
        .join(&author_san);
    std::fs::create_dir_all(&target_dir).unwrap();
    let target_file = target_dir.join(format!("{title_san}.epub"));
    std::fs::write(&target_file, b"orphaned file content").unwrap();

    // No library item in DB for this file
    let items_before = db
        .list_library_items_by_work(user_id, work_id)
        .await
        .unwrap();
    assert!(items_before.is_empty());

    let grab = seed_grab_with_content_path(
        &db,
        user_id,
        work_id,
        client_id,
        source_path,
        GrabStatus::Confirmed,
        None,
    )
    .await;

    let wf = make_workflow(db.clone());
    let result = wf.import_grab(user_id, grab.id).await.unwrap();

    // Should adopt the orphaned file (create DB record without re-copying)
    assert_eq!(result.final_status, GrabStatus::Imported);
    assert!(
        !result.imported_files.is_empty(),
        "should have adopted the orphaned file"
    );
    assert!(result
        .warnings
        .iter()
        .any(|w| w.contains("adopted") || w.contains("orphan")));

    let items_after = db
        .list_library_items_by_work(user_id, work_id)
        .await
        .unwrap();
    assert_eq!(
        items_after.len(),
        1,
        "should have created DB record for orphaned file"
    );
}

#[tokio::test]
async fn test_import_grab_path_traversal_rejected() {
    // WF-IMPORT-004, test.import.path_traversal: Given path traversal in torrent name, rejects the file
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (client_id, _) = setup_prereqs(&db, user_id).await;

    // Create a work with a malicious author name containing ..
    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "../../etc/passwd".into(),
            author_name: "../../root".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let source_dir = create_source_dir(&["book.epub"]);
    let source_path = source_dir.path().to_str().unwrap();

    let library_dir = tempdir().unwrap();
    db.create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    let grab = seed_grab_with_content_path(
        &db,
        user_id,
        work.id,
        client_id,
        source_path,
        GrabStatus::Confirmed,
        None,
    )
    .await;

    let wf = make_workflow(db.clone());
    let result = wf.import_grab(user_id, grab.id).await.unwrap();

    // sanitize_path_component strips .. so the path won't actually contain traversal.
    // The file should either be imported safely or rejected. The key invariant is:
    // no file written outside the root folder.
    // With sanitization, ".." becomes "__" so the file is imported safely.
    // Verify no file exists outside the root folder.
    let root_path = library_dir.path();
    if !result.imported_files.is_empty() {
        for f in &result.imported_files {
            let full = root_path.join(&f.target_relative_path);
            // The resolved path must be within root
            assert!(
                full.starts_with(root_path),
                "imported file path escapes root: {}",
                full.display()
            );
        }
    }
    // No files should exist outside root
    assert!(
        result.failed_files.is_empty()
            || result.imported_files.iter().all(|f| {
                root_path
                    .join(&f.target_relative_path)
                    .starts_with(root_path)
            }),
        "path traversal should be blocked or sanitized"
    );
}

#[tokio::test]
async fn test_import_grab_partial_sync_rejected() {
    // WF-IMPORT-004, test.import.size_precheck: Given partial sync (< 90% size), returns ImportFailed
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (client_id, work_id) = setup_prereqs(&db, user_id).await;

    // Create source dir with a small file
    let source_dir = create_source_dir(&["book.epub"]);
    let source_path = source_dir.path().to_str().unwrap();

    // The actual file is ~22 bytes ("test content for import")
    // Set grab.size to 1000 bytes — local is well under 90%
    let library_dir = tempdir().unwrap();
    db.create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    let grab = seed_grab_with_content_path(
        &db,
        user_id,
        work_id,
        client_id,
        source_path,
        GrabStatus::Confirmed,
        Some(1000), // declared size much larger than actual
    )
    .await;

    let wf = make_workflow(db.clone());
    let result = wf.import_grab(user_id, grab.id).await.unwrap();

    assert_eq!(
        result.final_status,
        GrabStatus::ImportFailed,
        "partial sync should result in ImportFailed"
    );
    assert!(result.imported_files.is_empty());
    assert!(
        result.warnings.iter().any(|w| w.contains("synced")),
        "should mention sync issue in warnings"
    );
}

#[tokio::test]
async fn test_import_grab_concurrent_prevents_duplicates() {
    // WF-IMPORT-004, test.import.concurrent_lock: Concurrent imports for same work produce no duplicates or corruption
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (client_id, work_id) = setup_prereqs(&db, user_id).await;

    let source_dir = create_source_dir(&["book.epub"]);
    let source_path = source_dir.path().to_str().unwrap().to_string();

    let library_dir = tempdir().unwrap();
    db.create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    let grab1 = seed_grab_with_content_path(
        &db,
        user_id,
        work_id,
        client_id,
        &source_path,
        GrabStatus::Confirmed,
        None,
    )
    .await;

    let grab2 = seed_grab_with_content_path(
        &db,
        user_id,
        work_id,
        client_id,
        &source_path,
        GrabStatus::Confirmed,
        None,
    )
    .await;

    let wf = std::sync::Arc::new(make_workflow(db.clone()));

    let wf1 = wf.clone();
    let wf2 = wf.clone();
    let g1_id = grab1.id;
    let g2_id = grab2.id;

    // Run both imports concurrently
    let (r1, r2) = tokio::join!(
        wf1.import_grab(user_id, g1_id),
        wf2.import_grab(user_id, g2_id)
    );

    // Both should complete without panicking
    r1.unwrap();
    r2.unwrap();
    // Should have exactly one library item in DB
    let items = db
        .list_library_items_by_work(user_id, work_id)
        .await
        .unwrap();
    assert_eq!(
        items.len(),
        1,
        "concurrent imports should produce exactly one library item, got {}",
        items.len()
    );
}

#[tokio::test]
async fn test_import_grab_mp3_batch_processing() {
    // WF-IMPORT-002: MP3 audiobook files are processed as a batch
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (client_id, work_id) = setup_prereqs(&db, user_id).await;

    // Create source dir with multiple MP3 files
    let source_dir = create_source_dir(&["chapter01.mp3", "chapter02.mp3", "chapter03.mp3"]);
    let source_path = source_dir.path().to_str().unwrap();

    let library_dir = tempdir().unwrap();
    db.create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Audiobook)
        .await
        .unwrap();

    let grab = seed_grab_with_content_path(
        &db,
        user_id,
        work_id,
        client_id,
        source_path,
        GrabStatus::Confirmed,
        None,
    )
    .await;

    let wf = make_workflow(db.clone());
    let result = wf.import_grab(user_id, grab.id).await.unwrap();

    assert_eq!(result.final_status, GrabStatus::Imported);
    // All MP3 files should be imported
    assert_eq!(
        result.imported_files.len(),
        3,
        "all 3 MP3 files should be imported"
    );

    // Verify library items in DB
    let items = db
        .list_library_items_by_work(user_id, work_id)
        .await
        .unwrap();
    assert_eq!(items.len(), 3, "each MP3 file gets a library item");
}

// =============================================================================
// retry_import
// =============================================================================

#[tokio::test]
async fn test_retry_import_reimports_failed_files() {
    // WF-IMPORT-001: Given failed import, retries and imports remaining files
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (client_id, work_id) = setup_prereqs(&db, user_id).await;

    // First import with inaccessible source (will fail)
    let grab = seed_grab_with_content_path(
        &db,
        user_id,
        work_id,
        client_id,
        "/nonexistent/path",
        GrabStatus::Confirmed,
        None,
    )
    .await;

    let wf = make_workflow(db.clone());
    let result1 = wf.import_grab(user_id, grab.id).await;
    assert!(result1.is_err()); // SourceInaccessible

    // Now fix the source path — create real files
    let source_dir = create_source_dir(&["book.epub"]);
    let source_path = source_dir.path().to_str().unwrap();
    db.set_grab_content_path(user_id, grab.id, source_path)
        .await
        .unwrap();

    let library_dir = tempdir().unwrap();
    db.create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    // Retry
    let result2 = wf.retry_import(user_id, grab.id).await.unwrap();
    assert_eq!(result2.final_status, GrabStatus::Imported);
    assert_eq!(result2.imported_files.len(), 1);
}

#[tokio::test]
async fn test_retry_import_skips_already_imported() {
    // WF-IMPORT-001: Already-imported files are skipped on retry
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (client_id, work_id) = setup_prereqs(&db, user_id).await;

    let source_dir = create_source_dir(&["book.epub"]);
    let source_path = source_dir.path().to_str().unwrap();

    let library_dir = tempdir().unwrap();
    db.create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    let grab = seed_grab_with_content_path(
        &db,
        user_id,
        work_id,
        client_id,
        source_path,
        GrabStatus::Confirmed,
        None,
    )
    .await;

    let wf = make_workflow(db.clone());

    // First import succeeds
    let result1 = wf.import_grab(user_id, grab.id).await.unwrap();
    assert_eq!(result1.imported_files.len(), 1);

    // Retry — file already exists with library item
    let result2 = wf.retry_import(user_id, grab.id).await.unwrap();
    assert!(
        !result2.skipped_files.is_empty(),
        "retry should skip already-imported files"
    );
    assert!(
        result2.imported_files.is_empty(),
        "no new files should be imported on retry"
    );

    // Still only one library item
    let items = db
        .list_library_items_by_work(user_id, work_id)
        .await
        .unwrap();
    assert_eq!(items.len(), 1);
}

#[tokio::test]
async fn test_retry_import_adopts_orphaned_files() {
    // WF-IMPORT-001: Orphaned files are adopted
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (client_id, work_id) = setup_prereqs(&db, user_id).await;

    let source_dir = create_source_dir(&["book.epub"]);
    let source_path = source_dir.path().to_str().unwrap();

    let library_dir = tempdir().unwrap();
    db.create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    // Pre-create target file on disk (orphan: file exists, no DB record)
    let work = db.get_work(user_id, work_id).await.unwrap();
    let author_san = sanitize_path_component(&work.author_name, "Unknown Author");
    let title_san = sanitize_path_component(&work.title, "Unknown Title");
    let target_dir = library_dir
        .path()
        .join(user_id.to_string())
        .join(&author_san);
    std::fs::create_dir_all(&target_dir).unwrap();
    std::fs::write(target_dir.join(format!("{title_san}.epub")), b"orphan").unwrap();

    let grab = seed_grab_with_content_path(
        &db,
        user_id,
        work_id,
        client_id,
        source_path,
        GrabStatus::ImportFailed,
        None,
    )
    .await;

    let wf = make_workflow(db.clone());
    let result = wf.retry_import(user_id, grab.id).await.unwrap();

    assert_eq!(result.final_status, GrabStatus::Imported);
    // Should adopt the orphaned file
    let items = db
        .list_library_items_by_work(user_id, work_id)
        .await
        .unwrap();
    assert_eq!(items.len(), 1, "orphaned file should be adopted");
}

#[tokio::test]
async fn test_import_grab_copy_failure_no_orphaned_db_record() {
    // WF-IMPORT-003: must not create library items without copying first
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (client_id, work_id) = setup_prereqs(&db, user_id).await;

    // Create source dir, then delete the source file to simulate copy failure
    let source_dir = create_source_dir(&["book.epub"]);
    let source_path = source_dir.path().to_str().unwrap().to_string();

    let library_dir = tempdir().unwrap();
    db.create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    // Remove the source file after creating grab — copy will fail
    std::fs::remove_file(source_dir.path().join("book.epub")).unwrap();

    let grab = seed_grab_with_content_path(
        &db,
        user_id,
        work_id,
        client_id,
        &source_path,
        GrabStatus::Confirmed,
        None,
    )
    .await;

    let wf = make_workflow(db.clone());
    let result = wf.import_grab(user_id, grab.id).await.unwrap();

    // Import should report failure (no recognized files since file was deleted)
    assert_eq!(result.final_status, GrabStatus::ImportFailed);

    // No orphaned library items
    let items = db
        .list_library_items_by_work(user_id, work_id)
        .await
        .unwrap();
    assert!(
        items.is_empty(),
        "no library items should exist when copy fails"
    );
}

// =============================================================================
// extract_chapters_for_item (manual-import chapter regression)
// =============================================================================
//
// Regression: manual-imported audiobooks (the #97 single-file path) never ran
// chapter extraction, so `audiobook_chapters` stayed empty and the player
// showed no chapters. `extract_chapters_for_item` is the shared hook the import
// service now invokes after creating the library item.

/// Create an audiobook library item and return its id.
async fn seed_audiobook_item(
    db: &SqliteDb,
    user_id: i64,
    work_id: i64,
    root_folder_id: i64,
    relative_path: &str,
) -> i64 {
    let item = db
        .create_library_item(CreateLibraryItemDbRequest {
            user_id,
            work_id,
            root_folder_id,
            path: relative_path.into(),
            media_type: MediaType::Audiobook,
            file_size: 1024,
            import_id: None,
            tag_status: livrarr_db::TagStatus::Pending,
            tagged_at_generation: 0,
        })
        .await
        .unwrap();
    item.id
}

#[tokio::test]
async fn test_extract_chapters_for_item_marks_non_m4b_audiobook_scanned() {
    // A non-M4B audiobook has no extractable chapters but must still be marked
    // scanned ("no_chapters") so it drops out of the unscanned backlog instead
    // of being rescanned forever.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_client_id, work_id) = setup_prereqs(&db, user_id).await;

    let library_dir = tempdir().unwrap();
    let rf = db
        .create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Audiobook)
        .await
        .unwrap();

    let item_id = seed_audiobook_item(&db, user_id, work_id, rf.id, "audio.mp3").await;

    // Before extraction the item is part of the unscanned backlog.
    let unscanned_before = db.list_unscanned_audiobook_items().await.unwrap();
    assert!(
        unscanned_before.iter().any(|(id, _)| *id == item_id),
        "newly created audiobook item should start unscanned"
    );

    let wf = make_workflow(db.clone());
    let target = library_dir.path().join("audio.mp3");
    wf.extract_chapters_for_item(item_id, &target, MediaType::Audiobook, user_id, work_id)
        .await;

    // After extraction the item is marked scanned and no longer unscanned.
    let unscanned_after = db.list_unscanned_audiobook_items().await.unwrap();
    assert!(
        !unscanned_after.iter().any(|(id, _)| *id == item_id),
        "audiobook item should be marked scanned after extraction"
    );

    let item = db.get_library_item(user_id, item_id).await.unwrap();
    assert_eq!(
        item.chapter_scan_status.as_deref(),
        Some("no_chapters"),
        "non-M4B audiobook should be recorded as no_chapters"
    );
}

#[tokio::test]
async fn test_extract_chapters_for_item_missing_m4b_stays_unscanned() {
    // A missing M4B is an I/O error (transient): scan_status must stay NULL so
    // backfill retries it, rather than being marked terminally.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_client_id, work_id) = setup_prereqs(&db, user_id).await;

    let library_dir = tempdir().unwrap();
    let rf = db
        .create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Audiobook)
        .await
        .unwrap();

    let item_id = seed_audiobook_item(&db, user_id, work_id, rf.id, "missing.m4b").await;

    let wf = make_workflow(db.clone());
    let target = library_dir.path().join("missing.m4b");
    wf.extract_chapters_for_item(item_id, &target, MediaType::Audiobook, user_id, work_id)
        .await;

    let item = db.get_library_item(user_id, item_id).await.unwrap();
    assert_eq!(
        item.chapter_scan_status, None,
        "missing M4B is an I/O error and must remain unscanned for retry"
    );
}
