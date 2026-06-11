#![allow(dead_code, unused_imports)]

//! Behavioral tests for FileService trait (redesigned).
//! Covers: list_paginated, get, delete, resolve_path, prepare_email, get_progress, update_progress

use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    CreateLibraryItemDbRequest, CreateUserDbRequest, CreateWorkDbRequest, LibraryItemDb,
    RootFolderDb, TagStatus, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::services::*;
use livrarr_domain::{MediaType, UserRole};
use livrarr_library::file_service::FileServiceImpl;
use std::io::Write;

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

async fn setup_second_user(db: &SqliteDb) -> i64 {
    let user = db
        .create_user(CreateUserDbRequest {
            username: "otheruser".into(),
            password_hash: "hash".into(),
            role: UserRole::User,
            api_key_hash: "testhash2".into(),
        })
        .await
        .unwrap();
    user.id
}

/// Create a root folder, a work, and a library item pointing at the given path.
/// Returns (root_folder_id, work_id, library_item_id).
async fn seed_library_item(
    db: &SqliteDb,
    user_id: i64,
    root_path: &str,
    relative_path: &str,
    media_type: MediaType,
) -> (i64, i64, i64) {
    let root = db.create_root_folder(root_path, media_type).await.unwrap();

    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Test Work".into(),
            author_name: "Test Author".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let item = db
        .create_library_item(CreateLibraryItemDbRequest {
            user_id,
            work_id: work.id,
            root_folder_id: root.id,
            path: relative_path.into(),
            media_type,
            file_size: 1024,
            import_id: None,
            tag_status: TagStatus::Pending,
            tagged_at_generation: 0,
        })
        .await
        .unwrap();

    (root.id, work.id, item.id)
}

// =============================================================================
// list_paginated
// =============================================================================

#[tokio::test]
async fn test_file_list_paginated_returns_items_and_total() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;

    let root = db
        .create_root_folder("/tmp/test-root", MediaType::Ebook)
        .await
        .unwrap();

    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Work".into(),
            author_name: "Author".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    for i in 0..3 {
        db.create_library_item(CreateLibraryItemDbRequest {
            user_id,
            work_id: work.id,
            root_folder_id: root.id,
            path: format!("file{i}.epub"),
            media_type: MediaType::Ebook,
            file_size: 100,
            import_id: None,
            tag_status: TagStatus::Pending,
            tagged_at_generation: 0,
        })
        .await
        .unwrap();
    }

    let svc = FileServiceImpl::new(db);
    let (items, total) = svc.list_paginated(user_id, 1, 2).await.unwrap();

    assert_eq!(total, 3);
    assert_eq!(items.len(), 2);
}

#[tokio::test]
async fn test_file_list_paginated_user_scoped() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let user_b = setup_second_user(&db).await;

    let root = db
        .create_root_folder("/tmp/test-root", MediaType::Ebook)
        .await
        .unwrap();

    let (work_a, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Work A".into(),
            author_name: "Author".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let (work_b, _) = db
        .create_work(CreateWorkDbRequest {
            user_id: user_b,
            title: "Work B".into(),
            author_name: "Author".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    db.create_library_item(CreateLibraryItemDbRequest {
        user_id,
        work_id: work_a.id,
        root_folder_id: root.id,
        path: "a.epub".into(),
        media_type: MediaType::Ebook,
        file_size: 100,
        import_id: None,
        tag_status: TagStatus::Pending,
        tagged_at_generation: 0,
    })
    .await
    .unwrap();

    db.create_library_item(CreateLibraryItemDbRequest {
        user_id: user_b,
        work_id: work_b.id,
        root_folder_id: root.id,
        path: "b.epub".into(),
        media_type: MediaType::Ebook,
        file_size: 200,
        import_id: None,
        tag_status: TagStatus::Pending,
        tagged_at_generation: 0,
    })
    .await
    .unwrap();

    let svc = FileServiceImpl::new(db);
    let (items, total) = svc.list_paginated(user_id, 1, 50).await.unwrap();

    assert_eq!(total, 1);
    assert_eq!(items.len(), 1);
    assert!(items.iter().all(|i| i.user_id == user_id));
}

// =============================================================================
// get
// =============================================================================

#[tokio::test]
async fn test_file_get_existing_returns_item() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_root_id, work_id, item_id) = seed_library_item(
        &db,
        user_id,
        "/tmp/root",
        "test/file.epub",
        MediaType::Ebook,
    )
    .await;

    let svc = FileServiceImpl::new(db);
    let item = svc.get(user_id, item_id).await.unwrap();

    assert_eq!(item.id, item_id);
    assert_eq!(item.user_id, user_id);
    assert_eq!(item.work_id, work_id);
    assert_eq!(item.path, "test/file.epub");
    assert_eq!(item.media_type, MediaType::Ebook);
}

#[tokio::test]
async fn test_file_get_nonexistent_returns_not_found() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;

    let svc = FileServiceImpl::new(db);
    let result = svc.get(user_id, 99999).await;

    assert!(
        matches!(result, Err(FileServiceError::NotFound)),
        "expected NotFound, got {result:?}"
    );
}

// =============================================================================
// delete
// =============================================================================

#[tokio::test]
async fn test_file_delete_removes_db_record() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_root_id, _work_id, item_id) = seed_library_item(
        &db,
        user_id,
        "/tmp/root",
        "test/file.epub",
        MediaType::Ebook,
    )
    .await;

    let svc = FileServiceImpl::new(db.clone());
    svc.delete(user_id, item_id).await.unwrap();

    // DB record gone
    let svc2 = FileServiceImpl::new(db);
    let result = svc2.get(user_id, item_id).await;
    assert!(matches!(result, Err(FileServiceError::NotFound)));
}

// =============================================================================
// resolve_path
// =============================================================================

#[tokio::test]
async fn test_resolve_path_returns_canonical_path() {
    let tmp = tempfile::tempdir().unwrap();
    let root_path = tmp.path().to_str().unwrap();

    // Create physical file
    let file_dir = tmp.path().join("books");
    std::fs::create_dir_all(&file_dir).unwrap();
    std::fs::write(file_dir.join("book.epub"), b"epub data").unwrap();

    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_root_id, _work_id, item_id) =
        seed_library_item(&db, user_id, root_path, "books/book.epub", MediaType::Ebook).await;

    let svc = FileServiceImpl::new(db);
    let path = svc.resolve_path(user_id, item_id).await.unwrap();

    assert!(path.is_absolute());
    assert!(path.exists());
    assert!(path.to_string_lossy().contains("book.epub"));
}

#[tokio::test]
async fn test_resolve_path_missing_file_returns_not_found() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_root_id, _work_id, item_id) = seed_library_item(
        &db,
        user_id,
        "/tmp/nonexistent-root",
        "missing/file.epub",
        MediaType::Ebook,
    )
    .await;

    let svc = FileServiceImpl::new(db);
    let result = svc.resolve_path(user_id, item_id).await;
    assert!(result.is_err(), "expected error for missing file");
}

#[tokio::test]
async fn test_resolve_path_traversal_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let root_path = tmp.path().to_str().unwrap();

    // Create the root dir so canonicalize works for root
    std::fs::create_dir_all(tmp.path()).unwrap();

    let db = create_test_db().await;
    let user_id = setup_user(&db).await;

    let root = db
        .create_root_folder(root_path, MediaType::Ebook)
        .await
        .unwrap();
    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Evil".into(),
            author_name: "Hacker".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let item = db
        .create_library_item(CreateLibraryItemDbRequest {
            user_id,
            work_id: work.id,
            root_folder_id: root.id,
            path: "../../../etc/passwd".into(),
            media_type: MediaType::Ebook,
            file_size: 100,
            import_id: None,
            tag_status: TagStatus::Pending,
            tagged_at_generation: 0,
        })
        .await
        .unwrap();

    let svc = FileServiceImpl::new(db);
    let result = svc.resolve_path(user_id, item.id).await;
    // Path traversal should be rejected (either NotFound because canonicalize fails,
    // or Forbidden because it escapes root).
    assert!(result.is_err(), "path traversal should be rejected");
}

// =============================================================================
// prepare_email
// =============================================================================

#[tokio::test]
async fn test_prepare_email_valid_epub() {
    let tmp = tempfile::tempdir().unwrap();
    let root_path = tmp.path().to_str().unwrap();

    let file_dir = tmp.path().join("books");
    std::fs::create_dir_all(&file_dir).unwrap();
    let content = b"fake epub content for testing";
    std::fs::write(file_dir.join("book.epub"), content).unwrap();

    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_root_id, _work_id, item_id) =
        seed_library_item(&db, user_id, root_path, "books/book.epub", MediaType::Ebook).await;

    let svc = FileServiceImpl::new(db);
    let payload = svc.prepare_email(user_id, item_id).await.unwrap();

    assert_eq!(payload.file_bytes, content);
    assert_eq!(payload.filename, "book.epub");
    assert_eq!(payload.extension, "epub");
}

#[tokio::test]
async fn test_prepare_email_rejects_unsupported_extension() {
    let tmp = tempfile::tempdir().unwrap();
    let root_path = tmp.path().to_str().unwrap();

    let file_dir = tmp.path().join("audio");
    std::fs::create_dir_all(&file_dir).unwrap();
    std::fs::write(file_dir.join("book.m4b"), b"audio data").unwrap();

    let db = create_test_db().await;
    let user_id = setup_user(&db).await;

    let root = db
        .create_root_folder(root_path, MediaType::Audiobook)
        .await
        .unwrap();
    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Audio Book".into(),
            author_name: "Author".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let item = db
        .create_library_item(CreateLibraryItemDbRequest {
            user_id,
            work_id: work.id,
            root_folder_id: root.id,
            path: "audio/book.m4b".into(),
            media_type: MediaType::Audiobook,
            file_size: 100,
            import_id: None,
            tag_status: TagStatus::Pending,
            tagged_at_generation: 0,
        })
        .await
        .unwrap();

    let svc = FileServiceImpl::new(db);
    let result = svc.prepare_email(user_id, item.id).await;
    assert!(
        matches!(result, Err(FileServiceError::BadRequest(_))),
        "expected BadRequest for unsupported extension, got {result:?}"
    );
}

#[tokio::test]
async fn test_prepare_email_rejects_oversized_file() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;

    let root = db
        .create_root_folder("/tmp/root", MediaType::Ebook)
        .await
        .unwrap();
    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Big Book".into(),
            author_name: "Author".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let item = db
        .create_library_item(CreateLibraryItemDbRequest {
            user_id,
            work_id: work.id,
            root_folder_id: root.id,
            path: "big.epub".into(),
            media_type: MediaType::Ebook,
            file_size: 60 * 1024 * 1024, // 60 MB — exceeds 50 MB limit
            import_id: None,
            tag_status: TagStatus::Pending,
            tagged_at_generation: 0,
        })
        .await
        .unwrap();

    let svc = FileServiceImpl::new(db);
    let result = svc.prepare_email(user_id, item.id).await;
    assert!(
        matches!(result, Err(FileServiceError::BadRequest(_))),
        "expected BadRequest for oversized file, got {result:?}"
    );
}

// =============================================================================
// get_progress / update_progress
// =============================================================================

#[tokio::test]
async fn test_progress_roundtrip() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_root_id, _work_id, item_id) = seed_library_item(
        &db,
        user_id,
        "/tmp/root",
        "test/file.epub",
        MediaType::Ebook,
    )
    .await;

    let svc = FileServiceImpl::new(db);

    // No progress initially
    let progress = svc.get_progress(user_id, item_id).await.unwrap();
    assert!(progress.is_none());

    // Update progress
    svc.update_progress(
        user_id,
        item_id,
        "epubcfi(/6/4)",
        0.25,
        livrarr_domain::services::ProgressKind::Progress,
        None,
    )
    .await
    .unwrap();

    // Read back
    let progress = svc.get_progress(user_id, item_id).await.unwrap();
    assert!(progress.is_some());
    let p = progress.unwrap();
    assert_eq!(p.position, "epubcfi(/6/4)");
    assert!((p.progress_pct - 0.25).abs() < f64::EPSILON);
}

#[tokio::test]
async fn test_update_progress_clamps_pct() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_root_id, _work_id, item_id) = seed_library_item(
        &db,
        user_id,
        "/tmp/root",
        "test/file.epub",
        MediaType::Ebook,
    )
    .await;

    let svc = FileServiceImpl::new(db);

    // Try to set progress > 1.0
    svc.update_progress(
        user_id,
        item_id,
        "page 100",
        1.5,
        livrarr_domain::services::ProgressKind::Progress,
        None,
    )
    .await
    .unwrap();

    let progress = svc.get_progress(user_id, item_id).await.unwrap().unwrap();
    assert!((progress.progress_pct - 1.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn test_update_progress_nonexistent_item_returns_not_found() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;

    let svc = FileServiceImpl::new(db);
    let result = svc
        .update_progress(
            user_id,
            99999,
            "page 1",
            0.1,
            livrarr_domain::services::ProgressKind::Progress,
            None,
        )
        .await;
    assert!(
        matches!(result, Err(FileServiceError::NotFound)),
        "expected NotFound, got {result:?}"
    );
}
