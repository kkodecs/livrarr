#![allow(dead_code, unused_imports)]

use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    CreateLibraryItemDbRequest, CreateUserDbRequest, CreateWorkDbRequest, LibraryItemDb,
    ListImportDb, PlaybackProgressDb, RootFolderDb, UserDb, WorkDb,
};
use livrarr_domain::services::*;
use livrarr_domain::{MediaType, UserRole};
use livrarr_library::file_service::FileServiceImpl;
use livrarr_metadata::list_service::{ListServiceImpl, NoOpBibliographyTrigger};
use livrarr_metadata::work_service::WorkServiceImpl;
use std::path::PathBuf;

fn stub_http() -> StubHttpFetcher {
    StubHttpFetcher::new()
}

fn test_data_dir() -> PathBuf {
    std::env::temp_dir().join(format!(
        "livrarr-stress-phase4a-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

async fn setup_user(db: &SqliteDb) -> i64 {
    db.create_user(CreateUserDbRequest {
        username: "testuser".into(),
        password_hash: "hash".into(),
        role: UserRole::Admin,
        api_key_hash: "testhash".into(),
    })
    .await
    .unwrap()
    .id
}

async fn setup_second_user(db: &SqliteDb) -> i64 {
    db.create_user(CreateUserDbRequest {
        username: "otheruser".into(),
        password_hash: "hash".into(),
        role: UserRole::User,
        api_key_hash: "othertesthash".into(),
    })
    .await
    .unwrap()
    .id
}

async fn seed_work(db: &SqliteDb, user_id: i64, title: &str, author_name: &str) -> i64 {
    db.create_work(CreateWorkDbRequest {
        user_id,
        title: title.into(),
        author_name: author_name.into(),
        ..Default::default()
    })
    .await
    .unwrap()
    .id
}

async fn seed_library_item(
    db: &SqliteDb,
    user_id: i64,
    root_path: &str,
    relative_path: &str,
    media_type: MediaType,
) -> (i64, i64, i64) {
    let root = db.create_root_folder(root_path, media_type).await.unwrap();
    let work_id = seed_work(db, user_id, "Test Work", "Test Author").await;
    let item = db
        .create_library_item(CreateLibraryItemDbRequest {
            user_id,
            work_id,
            root_folder_id: root.id,
            path: relative_path.into(),
            media_type,
            file_size: 1024,
            import_id: None,
        })
        .await
        .unwrap();

    (root.id, work_id, item.id)
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

async fn make_list_service() -> TestListService {
    let db = create_test_db().await;
    let _user_id = setup_user(&db).await;
    let work_svc = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());
    ListServiceImpl::new(db, work_svc, stub_http(), NoOpBibliographyTrigger)
}

fn single_row_csv_bytes() -> Vec<u8> {
    b"Book Id,Title,Author,ISBN,ISBN13,My Rating,Exclusive Shelf\n\
     1,Dune,Frank Herbert,=\"\",=\"9780441172719\",5,read\n"
        .to_vec()
}

#[tokio::test]
async fn test_work_add_empty_author_persists_without_author_record() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());

    let result = svc
        .add(
            user_id,
            AddWorkRequest {
                title: "No Author".into(),
                author_name: "   ".into(),
                author_ol_key: None,
                ol_key: None,
                gr_key: None,
                year: None,
                cover_url: None,
                metadata_source: None,
                language: None,
                detail_url: None,
                series_name: None,
                series_position: None,
                defer_enrichment: false,
                provenance_setter: None,
            },
        )
        .await
        .unwrap();

    assert!(!result.author_created);
    assert_eq!(result.author_id, None);
    assert_eq!(result.work.author_name, "");
    assert_eq!(result.work.author_id, None);
}

#[tokio::test]
async fn test_work_add_long_title_round_trips() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());
    let title = "A".repeat(8192);
    let expected_title = format!("A{}", "a".repeat(8191));

    let result = svc
        .add(
            user_id,
            AddWorkRequest {
                title,
                author_name: "Long Author".into(),
                author_ol_key: None,
                ol_key: None,
                gr_key: None,
                year: None,
                cover_url: None,
                metadata_source: None,
                language: None,
                detail_url: None,
                series_name: None,
                series_position: None,
                defer_enrichment: false,
                provenance_setter: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(result.work.title, expected_title);
    let persisted = svc.get(user_id, result.work.id).await.unwrap();
    assert_eq!(persisted.title.len(), 8192);
    assert_eq!(persisted.title, expected_title);
}

#[tokio::test]
async fn test_work_upload_cover_zero_bytes_returns_error() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());
    let work_id = seed_work(&db, user_id, "Coverless", "Author").await;

    let result = svc.upload_cover(user_id, work_id, &[]).await;

    assert!(matches!(result, Err(WorkServiceError::Enrichment(_))));
}

#[tokio::test]
async fn test_work_upload_cover_one_mebibyte_boundary_succeeds() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());
    let work_id = seed_work(&db, user_id, "Boundary Cover", "Author").await;
    let bytes = vec![0x5a; 1_024 * 1_024];

    svc.upload_cover(user_id, work_id, &bytes).await.unwrap();

    let downloaded = svc.download_cover(user_id, work_id).await.unwrap();
    let work = svc.get(user_id, work_id).await.unwrap();
    assert_eq!(downloaded.len(), 1_024 * 1_024);
    assert_eq!(downloaded, bytes);
    assert!(work.cover_manual);
}

#[tokio::test]
async fn test_work_download_cover_missing_file_returns_not_found() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());
    let work_id = seed_work(&db, user_id, "Missing Cover", "Author").await;

    let result = svc.download_cover(user_id, work_id).await;

    assert!(matches!(result, Err(WorkServiceError::NotFound)));
}

#[tokio::test]
async fn test_work_lookup_empty_term_returns_empty_results() {
    let db = create_test_db().await;
    let http = stub_http();
    let svc = WorkServiceImpl::without_enrichment(db, http.clone(), test_data_dir());

    let result = svc
        .lookup(LookupRequest {
            term: "   ".into(),
            lang_override: None,
        })
        .await
        .unwrap();

    assert!(result.is_empty());
    assert_eq!(http.call_count(), 0);
}

#[tokio::test]
async fn test_work_lookup_unsupported_lang_returns_error() {
    let db = create_test_db().await;
    let http = stub_http();
    let svc = WorkServiceImpl::without_enrichment(db, http.clone(), test_data_dir());

    let result = svc
        .lookup(LookupRequest {
            term: "Dune".into(),
            lang_override: Some("zz".into()),
        })
        .await;

    assert!(matches!(result, Err(WorkServiceError::Enrichment(_))));
    assert_eq!(http.call_count(), 0);
}

#[tokio::test]
async fn test_list_preview_empty_csv_returns_parse_error() {
    let svc = make_list_service().await;

    let result = svc.preview(1, Vec::new()).await;

    assert!(matches!(result, Err(ListServiceError::Parse(_))));
}

#[tokio::test]
async fn test_list_confirm_on_completed_import_returns_conflict() {
    let svc = make_list_service().await;
    let preview = svc.preview(1, single_row_csv_bytes()).await.unwrap();
    let confirmed = svc
        .confirm(1, &preview.preview_id, None, &[0])
        .await
        .unwrap();
    svc.complete(1, &confirmed.import_id).await.unwrap();

    let result = svc
        .confirm(1, &preview.preview_id, Some(&confirmed.import_id), &[0])
        .await;

    assert!(matches!(result, Err(ListServiceError::Conflict(_))));
}

#[tokio::test]
async fn test_list_undo_on_undone_import_returns_conflict() {
    let svc = make_list_service().await;
    let import_id = "stress-import-undone";
    let now = chrono::Utc::now().to_rfc3339();
    svc.db
        .create_list_import_record(import_id, 1, "goodreads", &now)
        .await
        .unwrap();
    svc.db
        .complete_list_import(import_id, 1, &now)
        .await
        .unwrap();
    svc.undo(1, import_id).await.unwrap();

    let result = svc.undo(1, import_id).await;

    assert!(matches!(result, Err(ListServiceError::Conflict(_))));
}

#[tokio::test]
async fn test_file_resolve_path_missing_root_returns_root_folder_not_found() {
    let root_dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root_dir.path().join("books")).unwrap();
    std::fs::write(root_dir.path().join("books/book.epub"), b"epub").unwrap();
    let root_path = root_dir.path().to_string_lossy().into_owned();
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (root_id, _work_id, item_id) = seed_library_item(
        &db,
        user_id,
        &root_path,
        "books/book.epub",
        MediaType::Ebook,
    )
    .await;
    let mut conn = db.pool().acquire().await.unwrap();
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *conn)
        .await
        .unwrap();
    sqlx::query("UPDATE library_items SET root_folder_id = ? WHERE id = ?")
        .bind(root_id + 999_999)
        .bind(item_id)
        .execute(&mut *conn)
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *conn)
        .await
        .unwrap();
    drop(conn);
    let svc = FileServiceImpl::new(db);

    let result = svc.resolve_path(user_id, item_id).await;

    match result {
        Err(FileServiceError::RootFolderNotFound) => {}
        other => panic!("expected RootFolderNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn test_file_prepare_email_bad_extension_returns_bad_request() {
    let root_dir = tempfile::tempdir().unwrap();
    let root_path = root_dir.path().to_string_lossy().into_owned();
    std::fs::create_dir_all(root_dir.path().join("books")).unwrap();
    std::fs::write(root_dir.path().join("books/book.exe"), b"binary").unwrap();
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_root_id, _work_id, item_id) =
        seed_library_item(&db, user_id, &root_path, "books/book.exe", MediaType::Ebook).await;
    let svc = FileServiceImpl::new(db);

    let result = svc.prepare_email(user_id, item_id).await;

    assert!(matches!(result, Err(FileServiceError::BadRequest(_))));
}

#[tokio::test]
async fn test_file_update_progress_clamps_pct() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_root_id, _work_id, item_id) = seed_library_item(
        &db,
        user_id,
        "/tmp/root",
        "books/book.epub",
        MediaType::Ebook,
    )
    .await;
    let svc = FileServiceImpl::new(db);

    svc.update_progress(user_id, item_id, "middle", -0.5)
        .await
        .unwrap();
    let low = svc.get_progress(user_id, item_id).await.unwrap().unwrap();
    svc.update_progress(user_id, item_id, "end", 1.5)
        .await
        .unwrap();
    let high = svc.get_progress(user_id, item_id).await.unwrap().unwrap();

    assert_eq!(low.progress_pct, 0.0);
    assert_eq!(high.progress_pct, 1.0);
}

#[tokio::test]
async fn test_file_list_paginated_page_zero_behaves_like_first_page() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let root = db
        .create_root_folder("/tmp/paginated-root", MediaType::Ebook)
        .await
        .unwrap();
    let work_id = seed_work(&db, user_id, "Paged Work", "Author").await;

    for idx in 0..3 {
        db.create_library_item(CreateLibraryItemDbRequest {
            user_id,
            work_id,
            root_folder_id: root.id,
            path: format!("file-{idx}.epub"),
            media_type: MediaType::Ebook,
            file_size: 100,
            import_id: None,
        })
        .await
        .unwrap();
    }

    let svc = FileServiceImpl::new(db);
    let (page_zero_items, page_zero_total) = svc.list_paginated(user_id, 0, 2).await.unwrap();
    let (page_one_items, page_one_total) = svc.list_paginated(user_id, 1, 2).await.unwrap();
    let page_zero_ids: Vec<_> = page_zero_items.iter().map(|item| item.id).collect();
    let page_one_ids: Vec<_> = page_one_items.iter().map(|item| item.id).collect();

    assert_eq!(page_zero_total, 3);
    assert_eq!(page_zero_total, page_one_total);
    assert_eq!(page_zero_items.len(), 2);
    assert_eq!(page_zero_ids, page_one_ids);
}

#[tokio::test]
async fn test_file_get_wrong_user_returns_not_found() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let other_user_id = setup_second_user(&db).await;
    let (_root_id, _work_id, item_id) = seed_library_item(
        &db,
        user_id,
        "/tmp/root",
        "books/book.epub",
        MediaType::Ebook,
    )
    .await;
    let svc = FileServiceImpl::new(db);

    let result = svc.get(other_user_id, item_id).await;

    assert!(matches!(result, Err(FileServiceError::NotFound)));
}
