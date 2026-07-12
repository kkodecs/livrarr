#![allow(dead_code, unused_imports)]

//! Behavioral tests for ImportWorkflow trait (WF-IMPORT-001..004).
//! Covers: fn.import_workflow.{import_grab, import_file}
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
use livrarr_handlers::context::{
    HasFileService, HasImportService, HasRootFolderService, HasWorkService,
};
use livrarr_handlers::root_folder::scan;
use livrarr_handlers::AuthContext;
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
    // Size must match the source file's on-disk size for the Copy-mode
    // adoption size gate (design §3.6) to adopt rather than reject as a
    // path collision.
    let target_file = target_dir.join(format!("{title_san}.epub"));
    std::fs::write(&target_file, b"test content for import").unwrap();

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
// import_grab orphan adoption (retry scenario — a previously-failed grab is
// re-driven through import_grab; retry_import itself is DELETED, see
// crates/livrarr-handlers/src/queue.rs for the retry route's
// try_set_importing + import_grab call)
// =============================================================================

#[tokio::test]
async fn test_import_grab_adopts_orphaned_file_on_retry() {
    // WF-IMPORT-001: a grab left in ImportFailed status, re-driven through
    // import_grab, adopts an orphaned file (present on disk, no library
    // item) instead of re-copying it.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (client_id, work_id) = setup_prereqs(&db, user_id).await;

    let source_dir = create_source_dir(&["book.epub"]);
    let source_path = source_dir.path().to_str().unwrap();

    let library_dir = tempdir().unwrap();
    db.create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    // Pre-create target file on disk (orphan: file exists, no DB record).
    // Size must match the source file's on-disk size for the Copy-mode
    // adoption size gate (design §3.6) to adopt rather than reject as a
    // path collision.
    let work = db.get_work(user_id, work_id).await.unwrap();
    let author_san = sanitize_path_component(&work.author_name, "Unknown Author");
    let title_san = sanitize_path_component(&work.title, "Unknown Title");
    let target_dir = library_dir
        .path()
        .join(user_id.to_string())
        .join(&author_san);
    std::fs::create_dir_all(&target_dir).unwrap();
    std::fs::write(
        target_dir.join(format!("{title_san}.epub")),
        b"test content for import",
    )
    .unwrap();

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
    let result = wf.import_grab(user_id, grab.id).await.unwrap();

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
// import_file — the shared core, driven directly (not via import_grab)
// =============================================================================

#[tokio::test]
async fn test_import_file_copy_fresh_imports() {
    // Copy mode, target absent: materializes via atomic_copy and creates a LibraryItem.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_client_id, work_id) = setup_prereqs(&db, user_id).await;

    let source_dir = create_source_dir(&["book.epub"]);
    let source_path = source_dir.path().join("book.epub");

    let library_dir = tempdir().unwrap();
    let rf = db
        .create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    let wf = make_workflow(db.clone());
    let result = wf
        .import_file(
            user_id,
            ImportFileRequest {
                work_id,
                root_folder_id: rf.id,
                source: source_path,
                target_relative: "book.epub".into(),
                media_type: MediaType::Ebook,
                materialization: Materialization::Copy,
                import_id: None,
                extract_chapters: false,
            },
        )
        .await
        .unwrap();

    let item_id = match result {
        ImportFileOutcome::Imported { item_id, path } => {
            assert_eq!(path, "book.epub");
            item_id
        }
        other => panic!("expected Imported, got {other:?}"),
    };

    let item = db.get_library_item(user_id, item_id).await.unwrap();
    assert_eq!(item.work_id, work_id);
    assert!(library_dir.path().join(&item.path).exists());
}

#[tokio::test]
async fn test_import_file_hardlink_first_fresh_imports() {
    // HardlinkFirst mode, target absent: the file lands at the target with
    // matching content. Whether it's an actual hard link or a copy-fallback
    // is a filesystem detail this test does not assert.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_client_id, work_id) = setup_prereqs(&db, user_id).await;

    let source_dir = create_source_dir(&["audio.m4b"]);
    let source_path = source_dir.path().join("audio.m4b");
    let source_bytes = std::fs::read(&source_path).unwrap();

    let library_dir = tempdir().unwrap();
    let rf = db
        .create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Audiobook)
        .await
        .unwrap();

    let wf = make_workflow(db.clone());
    let result = wf
        .import_file(
            user_id,
            ImportFileRequest {
                work_id,
                root_folder_id: rf.id,
                source: source_path,
                target_relative: "audio.m4b".into(),
                media_type: MediaType::Audiobook,
                materialization: Materialization::HardlinkFirst,
                import_id: None,
                extract_chapters: false,
            },
        )
        .await
        .unwrap();

    let item_id = match result {
        ImportFileOutcome::Imported { item_id, .. } => item_id,
        other => panic!("expected Imported, got {other:?}"),
    };

    let item = db.get_library_item(user_id, item_id).await.unwrap();
    let target = library_dir.path().join(&item.path);
    assert!(target.exists());
    assert_eq!(std::fs::read(&target).unwrap(), source_bytes);
}

#[tokio::test]
async fn test_import_file_adopt_in_place_existing_file_adopts() {
    // AdoptInPlace: source == target, the file is already on disk (e.g. a
    // scan match) — no file I/O, just a LibraryItem row.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_client_id, work_id) = setup_prereqs(&db, user_id).await;

    let library_dir = tempdir().unwrap();
    let rf = db
        .create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    let target = library_dir.path().join("existing.epub");
    std::fs::write(&target, b"already on disk").unwrap();

    let wf = make_workflow(db.clone());
    let result = wf
        .import_file(
            user_id,
            ImportFileRequest {
                work_id,
                root_folder_id: rf.id,
                source: target.clone(),
                target_relative: "existing.epub".into(),
                media_type: MediaType::Ebook,
                materialization: Materialization::AdoptInPlace,
                import_id: None,
                extract_chapters: false,
            },
        )
        .await
        .unwrap();

    match result {
        ImportFileOutcome::Adopted { path, .. } => assert_eq!(path, "existing.epub"),
        other => panic!("expected Adopted, got {other:?}"),
    }

    let items = db
        .list_library_items_by_work(user_id, work_id)
        .await
        .unwrap();
    assert_eq!(items.len(), 1);
}

#[tokio::test]
async fn test_import_file_same_work_dedup_skips() {
    // Importing the same target twice for the same work skips the second
    // time instead of re-copying or erroring.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_client_id, work_id) = setup_prereqs(&db, user_id).await;

    let source_dir = create_source_dir(&["book.epub"]);
    let source_path = source_dir.path().join("book.epub");

    let library_dir = tempdir().unwrap();
    let rf = db
        .create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    let build_req = |source: std::path::PathBuf| ImportFileRequest {
        work_id,
        root_folder_id: rf.id,
        source,
        target_relative: "book.epub".into(),
        media_type: MediaType::Ebook,
        materialization: Materialization::Copy,
        import_id: None,
        extract_chapters: false,
    };

    let wf = make_workflow(db.clone());
    wf.import_file(user_id, build_req(source_path.clone()))
        .await
        .unwrap();

    let result = wf
        .import_file(user_id, build_req(source_path))
        .await
        .unwrap();
    assert!(
        matches!(
            result,
            ImportFileOutcome::Skipped {
                reason: SkipReason::AlreadyImported
            }
        ),
        "expected Skipped(AlreadyImported), got {result:?}"
    );

    let items = db
        .list_library_items_by_work(user_id, work_id)
        .await
        .unwrap();
    assert_eq!(items.len(), 1, "dedup must not create a second row");
}

#[tokio::test]
async fn test_import_file_other_work_row_is_path_collision() {
    // A different work already owns a LibraryItem at this path — the DB's
    // cross-work rejection must surface as PathCollision, never a silent
    // reassignment or a generic error.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_client_id, work_id) = setup_prereqs(&db, user_id).await;

    // normalized_title/normalized_author (not title/author_name) back the
    // UNIQUE(user_id, normalized_title, normalized_author) constraint that
    // create_work dedupes on — they must differ from setup_prereqs's work
    // (which also defaults them to "") or this "second" work silently
    // resolves to the SAME row.
    let (other_work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Other Book".into(),
            author_name: "Other Author".into(),
            normalized_title: "other book".into(),
            normalized_author: "other author".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let library_dir = tempdir().unwrap();
    let rf = db
        .create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    let source_dir = create_source_dir(&["book.epub"]);
    let source_path = source_dir.path().join("book.epub");

    let wf = make_workflow(db.clone());

    // First work claims the path.
    wf.import_file(
        user_id,
        ImportFileRequest {
            work_id,
            root_folder_id: rf.id,
            source: source_path.clone(),
            target_relative: "shared.epub".into(),
            media_type: MediaType::Ebook,
            materialization: Materialization::Copy,
            import_id: None,
            extract_chapters: false,
        },
    )
    .await
    .unwrap();

    // Same source (same on-disk size) but a different work claims the same
    // target — size check passes, so the DB's own uniqueness check must
    // reject the reassignment.
    let result = wf
        .import_file(
            user_id,
            ImportFileRequest {
                work_id: other_work.id,
                root_folder_id: rf.id,
                source: source_path,
                target_relative: "shared.epub".into(),
                media_type: MediaType::Ebook,
                materialization: Materialization::Copy,
                import_id: None,
                extract_chapters: false,
            },
        )
        .await;

    assert!(
        matches!(&result, Err(ImportWorkflowError::PathCollision(p)) if p == "shared.epub"),
        "expected PathCollision, got: {result:?}"
    );
}

#[tokio::test]
async fn test_import_file_copy_adopts_orphan_when_size_matches() {
    // Target exists with no DB row and matches the source's size — adopt
    // without touching the file's bytes.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_client_id, work_id) = setup_prereqs(&db, user_id).await;

    let source_dir = create_source_dir(&["book.epub"]);
    let source_path = source_dir.path().join("book.epub");
    let source_size = std::fs::metadata(&source_path).unwrap().len();

    let library_dir = tempdir().unwrap();
    let rf = db
        .create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    let target = library_dir.path().join("book.epub");
    let orphan_bytes = vec![b'x'; source_size as usize];
    std::fs::write(&target, &orphan_bytes).unwrap();

    let wf = make_workflow(db.clone());
    let result = wf
        .import_file(
            user_id,
            ImportFileRequest {
                work_id,
                root_folder_id: rf.id,
                source: source_path,
                target_relative: "book.epub".into(),
                media_type: MediaType::Ebook,
                materialization: Materialization::Copy,
                import_id: None,
                extract_chapters: false,
            },
        )
        .await
        .unwrap();

    assert!(
        matches!(result, ImportFileOutcome::Adopted { .. }),
        "expected Adopted, got {result:?}"
    );
    let items = db
        .list_library_items_by_work(user_id, work_id)
        .await
        .unwrap();
    assert_eq!(items.len(), 1);
    // Adoption must not touch the file's bytes.
    assert_eq!(std::fs::read(&target).unwrap(), orphan_bytes);
}

#[tokio::test]
async fn test_import_file_copy_size_mismatch_is_path_collision() {
    // Target exists with no DB row but a DIFFERENT size than the source —
    // never silently adopt; a colliding different book virtually never
    // matches byte-for-byte size.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_client_id, work_id) = setup_prereqs(&db, user_id).await;

    let source_dir = create_source_dir(&["book.epub"]);
    let source_path = source_dir.path().join("book.epub");

    let library_dir = tempdir().unwrap();
    let rf = db
        .create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    let target = library_dir.path().join("book.epub");
    std::fs::write(
        &target,
        b"a totally different, longer file body than the source",
    )
    .unwrap();

    let wf = make_workflow(db.clone());
    let result = wf
        .import_file(
            user_id,
            ImportFileRequest {
                work_id,
                root_folder_id: rf.id,
                source: source_path,
                target_relative: "book.epub".into(),
                media_type: MediaType::Ebook,
                materialization: Materialization::Copy,
                import_id: None,
                extract_chapters: false,
            },
        )
        .await;

    assert!(
        matches!(&result, Err(ImportWorkflowError::PathCollision(p)) if p == "book.epub"),
        "expected PathCollision, got: {result:?}"
    );

    let items = db
        .list_library_items_by_work(user_id, work_id)
        .await
        .unwrap();
    assert!(items.is_empty());
}

#[tokio::test]
async fn test_import_file_foreign_row_missing_file_no_bytes_at_target() {
    // A foreign work's row claims (root, path) but its backing file is gone
    // from disk (e.g. deleted out-of-band). A second work's Copy-mode import
    // to that same path must still be rejected as PathCollision by the DB's
    // own uniqueness check on (user_id, root_folder_id, path) — and, since
    // materialization now lands in a staging file first, no bytes may reach
    // `target` before that check runs, and the staging file must not be left
    // behind.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_client_id, work_id) = setup_prereqs(&db, user_id).await;

    let (other_work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Other Book".into(),
            author_name: "Other Author".into(),
            normalized_title: "other book".into(),
            normalized_author: "other author".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let library_dir = tempdir().unwrap();
    let rf = db
        .create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    // work_id's row claims "book.epub" but has no backing file on disk —
    // e.g. the file was removed out-of-band after the row was created.
    db.create_library_item(CreateLibraryItemDbRequest {
        user_id,
        work_id,
        root_folder_id: rf.id,
        path: "book.epub".into(),
        media_type: MediaType::Ebook,
        file_size: 4,
        import_id: None,
        tag_status: livrarr_db::TagStatus::Pending,
        tagged_at_generation: 0,
    })
    .await
    .unwrap();

    let source_dir = create_source_dir(&["book.epub"]);
    let source_path = source_dir.path().join("book.epub");

    let wf = make_workflow(db.clone());
    let result = wf
        .import_file(
            user_id,
            ImportFileRequest {
                work_id: other_work.id,
                root_folder_id: rf.id,
                source: source_path,
                target_relative: "book.epub".into(),
                media_type: MediaType::Ebook,
                materialization: Materialization::Copy,
                import_id: None,
                extract_chapters: false,
            },
        )
        .await;

    assert!(
        matches!(&result, Err(ImportWorkflowError::PathCollision(p)) if p == "book.epub"),
        "expected PathCollision, got: {result:?}"
    );

    let target = library_dir.path().join("book.epub");
    assert!(
        !target.exists(),
        "no bytes may reach the target path when the DB row collides — the \
         row committed first is the authority for the path"
    );

    let leftovers: Vec<_> = std::fs::read_dir(library_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".stg"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "staging file must be cleaned up after a DB collision, found: {leftovers:?}"
    );
}

#[tokio::test]
async fn test_import_file_same_work_second_root_imports() {
    // The dedup gate now scopes by (work, root_folder, path): the same work
    // already having a row in one root folder must not block — or wrongly
    // Skip — a fresh import of that same work into a SECOND root folder at
    // the same relative path.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_client_id, work_id) = setup_prereqs(&db, user_id).await;

    let library_dir_1 = tempdir().unwrap();
    let rf1 = db
        .create_root_folder(library_dir_1.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    // Work A's existing row lives only in root folder 1.
    db.create_library_item(CreateLibraryItemDbRequest {
        user_id,
        work_id,
        root_folder_id: rf1.id,
        path: "book.epub".into(),
        media_type: MediaType::Ebook,
        file_size: 24,
        import_id: None,
        tag_status: livrarr_db::TagStatus::Pending,
        tagged_at_generation: 0,
    })
    .await
    .unwrap();

    // root_folders enforces one root folder per media type (schema-level
    // UNIQUE(media_type), no user_id column) — a second root folder must be
    // the other media type. This also reflects a normal real scenario: the
    // same work has both an ebook and an audiobook edition.
    let library_dir_2 = tempdir().unwrap();
    let rf2 = db
        .create_root_folder(library_dir_2.path().to_str().unwrap(), MediaType::Audiobook)
        .await
        .unwrap();

    let source_dir = create_source_dir(&["book.epub"]);
    let source_path = source_dir.path().join("book.epub");

    let wf = make_workflow(db.clone());
    let result = wf
        .import_file(
            user_id,
            ImportFileRequest {
                work_id,
                root_folder_id: rf2.id,
                source: source_path,
                target_relative: "book.epub".into(),
                media_type: MediaType::Audiobook,
                materialization: Materialization::Copy,
                import_id: None,
                extract_chapters: false,
            },
        )
        .await
        .unwrap();

    assert!(
        matches!(result, ImportFileOutcome::Imported { .. }),
        "expected Imported (not Skipped) for the same work in a second root folder, got {result:?}"
    );
    assert!(
        library_dir_2.path().join("book.epub").exists(),
        "file should exist under root folder 2"
    );

    let items = db
        .list_library_items_by_work(user_id, work_id)
        .await
        .unwrap();
    assert_eq!(
        items.len(),
        2,
        "work A should now have one row per root folder"
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

// =============================================================================
// scan door (R9) — root_folder::scan calls ImportService::adopt_scanned_file,
// which must ride the shared core. Minimal stubs below satisfy scan's
// Has* bounds; only the methods scan actually calls do real work, everything
// else is unreachable!() (scan never calls it). StubImportServiceForScan
// wraps a REAL ImportWorkflowImpl<SqliteDb> so adopt_scanned_file exercises
// genuine core behavior (dedup/adopt/PathCollision), not a fake.
// =============================================================================

async fn setup_user_full(db: &SqliteDb) -> User {
    db.create_user(CreateUserDbRequest {
        username: format!("scanuser{}", rand_suffix()),
        password_hash: "hash".into(),
        role: UserRole::Admin,
        api_key_hash: format!("keyhash{}", rand_suffix()),
    })
    .await
    .unwrap()
}

fn test_auth(user: User) -> livrarr_handlers::middleware::RequireAdmin {
    livrarr_handlers::middleware::RequireAdmin(AuthContext {
        user,
        auth_type: livrarr_domain::AuthType::Session,
        session_token_hash: None,
    })
}

#[derive(Clone)]
struct StubRootFolderService {
    folders: Vec<RootFolder>,
}

impl RootFolderService for StubRootFolderService {
    async fn get_root_folder(&self, id: RootFolderId) -> Result<RootFolder, DbError> {
        self.folders
            .iter()
            .find(|rf| rf.id == id)
            .cloned()
            .ok_or(DbError::NotFound {
                entity: "root_folder",
            })
    }
    async fn list_root_folders(&self) -> Result<Vec<RootFolder>, DbError> {
        unreachable!("not used by scan")
    }
    async fn create_root_folder(&self, _: &str, _: MediaType) -> Result<RootFolder, DbError> {
        unreachable!("not used by scan")
    }
    async fn delete_root_folder(&self, _: RootFolderId) -> Result<(), DbError> {
        unreachable!("not used by scan")
    }
}

#[derive(Clone)]
struct StubWorkService {
    works: Vec<Work>,
}

impl WorkService for StubWorkService {
    async fn add(
        &self,
        _: UserId,
        _: livrarr_domain::identity::WorkCandidate,
    ) -> Result<AddWorkResult, WorkServiceError> {
        unreachable!("not used by scan")
    }
    async fn resolve_identity(
        &self,
        _: UserId,
        _: livrarr_domain::identity::RawHarvest,
        _: livrarr_domain::identity::LatencyTier,
    ) -> Result<livrarr_domain::identity::ResolvedIdentity, WorkServiceError> {
        unreachable!("not used by scan")
    }
    fn resolve_identity_local(
        &self,
        _: livrarr_domain::identity::RawHarvest,
    ) -> Result<livrarr_domain::identity::ResolvedIdentity, WorkServiceError> {
        unreachable!("not used by scan")
    }
    async fn add_fast(
        &self,
        _: UserId,
        _: livrarr_domain::identity::WorkCandidate,
    ) -> Result<AddWorkResult, WorkServiceError> {
        unreachable!("not used by scan")
    }
    async fn complete_add(
        &self,
        _: UserId,
        _: WorkId,
        _: Option<livrarr_domain::services::SourceProviderData>,
        _: Option<livrarr_domain::identity::CandidateId>,
        _: livrarr_domain::identity::IdentityMode,
        _: livrarr_domain::identity::ConflictSource,
    ) {
        unreachable!("not used by scan")
    }
    fn is_enriching(&self, _: UserId, _: WorkId) -> bool {
        unreachable!("not used by scan")
    }
    async fn get(&self, _: UserId, _: WorkId) -> Result<Work, WorkServiceError> {
        unreachable!("not used by scan")
    }
    async fn get_detail(&self, _: UserId, _: WorkId) -> Result<WorkDetailView, WorkServiceError> {
        unreachable!("not used by scan")
    }
    async fn list(&self, _: UserId, _: WorkFilter) -> Result<Vec<Work>, WorkServiceError> {
        Ok(self.works.clone())
    }
    async fn list_paginated(
        &self,
        _: UserId,
        _: u32,
        _: u32,
        _: WorkSortField,
        _: SortDirection,
        _: Option<MediaType>,
        _: Option<&str>,
    ) -> Result<PaginatedWorksView, WorkServiceError> {
        unreachable!("not used by scan")
    }
    async fn update(
        &self,
        _: UserId,
        _: WorkId,
        _: UpdateWorkRequest,
    ) -> Result<Work, WorkServiceError> {
        unreachable!("not used by scan")
    }
    async fn delete(&self, _: UserId, _: WorkId) -> Result<(), WorkServiceError> {
        unreachable!("not used by scan")
    }
    async fn refresh(
        &self,
        _: UserId,
        _: WorkId,
        _: RefreshSurface,
    ) -> Result<RefreshWorkResult, WorkServiceError> {
        unreachable!("not used by scan")
    }
    async fn retry_all_incomplete(&self, _: UserId) -> Result<RetrySummary, WorkServiceError> {
        unreachable!("not used by scan")
    }
    async fn upload_cover(&self, _: UserId, _: WorkId, _: &[u8]) -> Result<(), WorkServiceError> {
        unreachable!("not used by scan")
    }
    async fn download_cover(&self, _: UserId, _: WorkId) -> Result<Vec<u8>, WorkServiceError> {
        unreachable!("not used by scan")
    }
    async fn search_works(
        &self,
        _: UserId,
        _: &str,
        _: u32,
        _: u32,
    ) -> Result<(Vec<Work>, i64), WorkServiceError> {
        unreachable!("not used by scan")
    }
    fn try_start_bulk_refresh(&self, _: i64) -> Option<BulkRefreshGuard> {
        unreachable!("not used by scan")
    }
    async fn converge_work(
        &self,
        _: UserId,
        _: WorkId,
        _: u32,
    ) -> Result<ConvergeOutcome, WorkServiceError> {
        unreachable!("not used by scan")
    }
    async fn preview_merge_works(
        &self,
        _: UserId,
        _: WorkId,
        _: WorkId,
    ) -> Result<MergePreview, WorkServiceError> {
        unreachable!("not used by scan")
    }
    async fn merge_works(
        &self,
        _: UserId,
        _: WorkId,
        _: WorkId,
        _: Vec<MergeFieldChoiceEntry>,
    ) -> Result<MergeWorksResult, WorkServiceError> {
        unreachable!("not used by scan")
    }
}

#[derive(Clone)]
struct StubFileService {
    items: Vec<LibraryItem>,
}

impl FileService for StubFileService {
    async fn list(&self, _: UserId) -> Result<Vec<LibraryItem>, FileServiceError> {
        Ok(self.items.clone())
    }
    async fn list_paginated(
        &self,
        _: UserId,
        _: u32,
        _: u32,
    ) -> Result<(Vec<LibraryItem>, i64), FileServiceError> {
        unreachable!("not used by scan")
    }
    async fn get(&self, _: UserId, _: i64) -> Result<LibraryItem, FileServiceError> {
        unreachable!("not used by scan")
    }
    async fn delete(&self, _: UserId, _: i64) -> Result<(), FileServiceError> {
        unreachable!("not used by scan")
    }
    async fn resolve_path(
        &self,
        _: UserId,
        _: i64,
    ) -> Result<std::path::PathBuf, FileServiceError> {
        unreachable!("not used by scan")
    }
    async fn prepare_email(&self, _: UserId, _: i64) -> Result<EmailPayload, FileServiceError> {
        unreachable!("not used by scan")
    }
    async fn get_progress(
        &self,
        _: UserId,
        _: i64,
    ) -> Result<Option<PlaybackProgress>, FileServiceError> {
        unreachable!("not used by scan")
    }
    async fn update_progress(
        &self,
        _: UserId,
        _: i64,
        _: &str,
        _: f64,
        _: ProgressKind,
        _: Option<f64>,
    ) -> Result<(), FileServiceError> {
        unreachable!("not used by scan")
    }
    async fn get_progress_for_items(
        &self,
        _: UserId,
        _: &[LibraryItemId],
    ) -> Result<Vec<ItemProgress>, FileServiceError> {
        unreachable!("not used by scan")
    }
}

/// Wraps the real core (`ImportWorkflowImpl<SqliteDb>`) so `adopt_scanned_file`
/// exercises genuine core behavior (dedup/adopt/PathCollision) through the
/// scan door; every other `ImportService` method is unreachable from `scan`.
struct StubImportServiceForScan {
    workflow: ImportWorkflowImpl<SqliteDb>,
}

impl ImportService for StubImportServiceForScan {
    async fn import_grab(&self, _: i64, _: i64) -> Result<ImportGrabResult, ServiceError> {
        unreachable!("not used by scan")
    }
    async fn import_single_file(&self, _: ImportSingleFileRequest) -> ImportFileResult {
        unreachable!("not used by scan")
    }
    async fn adopt_scanned_file(
        &self,
        user_id: i64,
        req: AdoptScannedFileRequest,
    ) -> Result<ImportFileOutcome, ImportWorkflowError> {
        self.workflow
            .import_file(
                user_id,
                ImportFileRequest {
                    work_id: req.work_id,
                    root_folder_id: req.root_folder_id,
                    source: req.path,
                    target_relative: req.target_relative,
                    media_type: req.media_type,
                    materialization: Materialization::AdoptInPlace,
                    import_id: None,
                    extract_chapters: false,
                },
            )
            .await
    }
    async fn reorganize_work_files(&self, _: i64, _: i64) -> Result<Vec<String>, ServiceError> {
        unreachable!("not used by scan")
    }
    fn build_target_path(
        &self,
        _: &str,
        _: i64,
        _: &str,
        _: &str,
        _: MediaType,
        _: &std::path::Path,
        _: &std::path::Path,
    ) -> String {
        unreachable!("not used by scan")
    }
}

#[derive(Clone)]
struct ScanTestState {
    root_folder: std::sync::Arc<StubRootFolderService>,
    work: std::sync::Arc<StubWorkService>,
    file: std::sync::Arc<StubFileService>,
    import: std::sync::Arc<StubImportServiceForScan>,
}

impl HasRootFolderService for ScanTestState {
    type RootFolderSvc = StubRootFolderService;
    fn root_folder_service(&self) -> &Self::RootFolderSvc {
        &self.root_folder
    }
}
impl HasWorkService for ScanTestState {
    type WorkSvc = StubWorkService;
    fn work_service(&self) -> &Self::WorkSvc {
        &self.work
    }
}
impl HasFileService for ScanTestState {
    type FileSvc = StubFileService;
    fn file_service(&self) -> &Self::FileSvc {
        &self.file
    }
}
impl HasImportService for ScanTestState {
    type ImportSvc = StubImportServiceForScan;
    fn import_service(&self) -> &Self::ImportSvc {
        &self.import
    }
}

#[tokio::test]
async fn scan_adopts_new_file_via_import_service() {
    // A file sits on disk matching a work by (author, title); no LibraryItem
    // row exists anywhere. scan must route the adoption through
    // ImportService::adopt_scanned_file (AdoptInPlace) rather than creating
    // the row itself, and the core must actually create it.
    let db = create_test_db().await;
    let user = setup_user_full(&db).await;
    let user_id = user.id;

    let library_dir = tempdir().unwrap();
    let rf = db
        .create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Scan Title".into(),
            author_name: "Scan Author".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let author_dir = library_dir
        .path()
        .join(user_id.to_string())
        .join("Scan Author");
    std::fs::create_dir_all(&author_dir).unwrap();
    std::fs::write(
        author_dir.join("Scan Title.epub"),
        b"test content for import",
    )
    .unwrap();

    let workflow = make_workflow(db.clone());
    let state = ScanTestState {
        root_folder: std::sync::Arc::new(StubRootFolderService {
            folders: vec![rf.clone()],
        }),
        work: std::sync::Arc::new(StubWorkService {
            works: vec![work.clone()],
        }),
        file: std::sync::Arc::new(StubFileService { items: vec![] }),
        import: std::sync::Arc::new(StubImportServiceForScan { workflow }),
    };

    let result = scan(
        axum::extract::State(state),
        test_auth(user),
        axum::extract::Path(rf.id),
    )
    .await
    .unwrap();

    assert_eq!(result.0.matched, 1, "expected the new file to be adopted");
    assert!(
        result.0.errors.is_empty(),
        "expected no scan errors, got: {:?}",
        result.0.errors
    );

    let items = db
        .list_library_items_by_work(user_id, work.id)
        .await
        .unwrap();
    assert_eq!(
        items.len(),
        1,
        "adopt_scanned_file must create the row via the core"
    );
    assert_eq!(items[0].root_folder_id, rf.id);
    assert_eq!(items[0].media_type, MediaType::Ebook);
}

#[tokio::test]
async fn scan_path_collision_lands_in_scan_errors_and_walk_continues() {
    // Two files on disk: one whose computed path is already claimed by a
    // DIFFERENT work (a path collision the core's DB constraint must catch),
    // and one clean file for an unrelated work. The collision must surface
    // as a ScanErrorEntry — never abort the walk — and the clean file must
    // still be matched.
    let db = create_test_db().await;
    let user = setup_user_full(&db).await;
    let user_id = user.id;

    let library_dir = tempdir().unwrap();
    let rf = db
        .create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    // work_owner already owns the path the colliding file resolves to.
    let (work_owner, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Owner Work".into(),
            author_name: "Owner Author".into(),
            normalized_title: "owner work".into(),
            normalized_author: "owner author".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    // work_collide is what the on-disk file's path identity-matches.
    let (work_collide, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Collide Title".into(),
            author_name: "Collide Author".into(),
            normalized_title: "collide title".into(),
            normalized_author: "collide author".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    // work_clean is the uncontested "walk continues" case.
    let (work_clean, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Clean Title".into(),
            author_name: "Clean Author".into(),
            normalized_title: "clean title".into(),
            normalized_author: "clean author".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let colliding_path = format!("{user_id}/Collide Author/Collide Title.epub");
    db.create_library_item(CreateLibraryItemDbRequest {
        user_id,
        work_id: work_owner.id,
        root_folder_id: rf.id,
        path: colliding_path,
        media_type: MediaType::Ebook,
        file_size: 4,
        import_id: None,
        tag_status: livrarr_db::TagStatus::Pending,
        tagged_at_generation: 0,
    })
    .await
    .unwrap();

    let collide_dir = library_dir
        .path()
        .join(user_id.to_string())
        .join("Collide Author");
    std::fs::create_dir_all(&collide_dir).unwrap();
    std::fs::write(
        collide_dir.join("Collide Title.epub"),
        b"test content for import",
    )
    .unwrap();

    let clean_dir = library_dir
        .path()
        .join(user_id.to_string())
        .join("Clean Author");
    std::fs::create_dir_all(&clean_dir).unwrap();
    std::fs::write(
        clean_dir.join("Clean Title.epub"),
        b"test content for import",
    )
    .unwrap();

    let workflow = make_workflow(db.clone());
    let state = ScanTestState {
        root_folder: std::sync::Arc::new(StubRootFolderService {
            folders: vec![rf.clone()],
        }),
        work: std::sync::Arc::new(StubWorkService {
            works: vec![work_collide.clone(), work_clean.clone()],
        }),
        file: std::sync::Arc::new(StubFileService { items: vec![] }),
        import: std::sync::Arc::new(StubImportServiceForScan { workflow }),
    };

    let result = scan(
        axum::extract::State(state),
        test_auth(user),
        axum::extract::Path(rf.id),
    )
    .await
    .unwrap();

    assert_eq!(
        result.0.matched, 1,
        "the clean file must still be matched despite the collision"
    );
    assert_eq!(
        result.0.errors.len(),
        1,
        "the collision must surface as exactly one scan error, got: {:?}",
        result.0.errors
    );
    assert!(
        result.0.errors[0].message.contains("path collision"),
        "unexpected message: {}",
        result.0.errors[0].message
    );

    let items_owner = db
        .list_library_items_by_work(user_id, work_owner.id)
        .await
        .unwrap();
    assert_eq!(items_owner.len(), 1, "the owning work's row is untouched");

    let items_collide = db
        .list_library_items_by_work(user_id, work_collide.id)
        .await
        .unwrap();
    assert!(
        items_collide.is_empty(),
        "a path collision must not create a row for the colliding work"
    );

    let items_clean = db
        .list_library_items_by_work(user_id, work_clean.id)
        .await
        .unwrap();
    assert_eq!(
        items_clean.len(),
        1,
        "the clean file's work should get its row despite the other collision"
    );
}

#[tokio::test]
async fn scan_foreign_work_item_in_file_list_still_surfaces_collision() {
    // Variant of scan_path_collision_lands_in_scan_errors_and_walk_continues
    // where the stubbed file service's list() actually RETURNS the foreign
    // work's item (mirroring what the real file_service reports), exercising
    // the `already_tracked` predicate directly instead of relying solely on
    // the DB's own constraint as a backstop. Before the work_id scoping fix,
    // a root_folder_id+path match alone (regardless of which work owns the
    // row) made `already_tracked` true, silently counting the file as
    // "matched" without ever calling adopt_scanned_file — hiding the
    // collision entirely. Scoped by work_id, the foreign item no longer
    // short-circuits the match, so the file falls through to
    // adopt_scanned_file and the real PathCollision surfaces as a scan error.
    let db = create_test_db().await;
    let user = setup_user_full(&db).await;
    let user_id = user.id;

    let library_dir = tempdir().unwrap();
    let rf = db
        .create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    let (work_owner, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Owner Work".into(),
            author_name: "Owner Author".into(),
            normalized_title: "owner work".into(),
            normalized_author: "owner author".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let (work_collide, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Collide Title".into(),
            author_name: "Collide Author".into(),
            normalized_title: "collide title".into(),
            normalized_author: "collide author".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let (work_clean, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Clean Title".into(),
            author_name: "Clean Author".into(),
            normalized_title: "clean title".into(),
            normalized_author: "clean author".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let colliding_path = format!("{user_id}/Collide Author/Collide Title.epub");
    let owner_item = db
        .create_library_item(CreateLibraryItemDbRequest {
            user_id,
            work_id: work_owner.id,
            root_folder_id: rf.id,
            path: colliding_path,
            media_type: MediaType::Ebook,
            file_size: 4,
            import_id: None,
            tag_status: livrarr_db::TagStatus::Pending,
            tagged_at_generation: 0,
        })
        .await
        .unwrap();

    let collide_dir = library_dir
        .path()
        .join(user_id.to_string())
        .join("Collide Author");
    std::fs::create_dir_all(&collide_dir).unwrap();
    std::fs::write(
        collide_dir.join("Collide Title.epub"),
        b"test content for import",
    )
    .unwrap();

    let clean_dir = library_dir
        .path()
        .join(user_id.to_string())
        .join("Clean Author");
    std::fs::create_dir_all(&clean_dir).unwrap();
    std::fs::write(
        clean_dir.join("Clean Title.epub"),
        b"test content for import",
    )
    .unwrap();

    let workflow = make_workflow(db.clone());
    let state = ScanTestState {
        root_folder: std::sync::Arc::new(StubRootFolderService {
            folders: vec![rf.clone()],
        }),
        work: std::sync::Arc::new(StubWorkService {
            works: vec![work_collide.clone(), work_clean.clone()],
        }),
        // The foreign work's item IS visible via file_service().list() here —
        // this is what makes `already_tracked` evaluate against it at all.
        file: std::sync::Arc::new(StubFileService {
            items: vec![owner_item],
        }),
        import: std::sync::Arc::new(StubImportServiceForScan { workflow }),
    };

    let result = scan(
        axum::extract::State(state),
        test_auth(user),
        axum::extract::Path(rf.id),
    )
    .await
    .unwrap();

    assert_eq!(
        result.0.matched, 1,
        "the clean file must still be matched despite the collision"
    );
    assert_eq!(
        result.0.errors.len(),
        1,
        "the collision must surface as exactly one scan error, got: {:?}",
        result.0.errors
    );
    assert!(
        result.0.errors[0].message.contains("path collision"),
        "unexpected message: {}",
        result.0.errors[0].message
    );

    let items_owner = db
        .list_library_items_by_work(user_id, work_owner.id)
        .await
        .unwrap();
    assert_eq!(items_owner.len(), 1, "the owning work's row is untouched");

    let items_collide = db
        .list_library_items_by_work(user_id, work_collide.id)
        .await
        .unwrap();
    assert!(
        items_collide.is_empty(),
        "a path collision must not create a row for the colliding work"
    );

    let items_clean = db
        .list_library_items_by_work(user_id, work_clean.id)
        .await
        .unwrap();
    assert_eq!(
        items_clean.len(),
        1,
        "the clean file's work should get its row despite the other collision"
    );
}
