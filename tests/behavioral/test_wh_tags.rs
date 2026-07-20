#![allow(dead_code, unused_imports)]

//! RED behavioral tests for work-history tag write events.

use std::sync::Arc;

use livrarr_behavioral::stubs::TagwriteChapterExtractor;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    CreateLibraryItemDbRequest, CreateUserDbRequest, CreateWorkDbRequest, HistoryDb, LibraryItemDb,
    RootFolderDb, TagStatus, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::services::{ImportService, TagService};
use livrarr_domain::{EnrichmentStatus, EventType, HistoryFilter, MediaType, UserRole, Work};
use livrarr_server::import_io_service::ImportIoServiceImpl;
use livrarr_server::import_service::LiveImportService;
use livrarr_server::state::{LiveImportIoService, LiveImportWorkflow, LiveSettingsService};
use livrarr_server::tag_service::LiveTagService;

fn empty_history_filter() -> HistoryFilter {
    HistoryFilter {
        event_type: None,
        work_id: None,
        start_date: None,
        end_date: None,
    }
}

async fn setup_user(db: &SqliteDb) -> i64 {
    db.create_user(CreateUserDbRequest {
        username: "wh_tag_user".into(),
        password_hash: "hash".into(),
        role: UserRole::Admin,
        api_key_hash: "api_hash".into(),
    })
    .await
    .unwrap()
    .id
}

async fn setup_work(db: &SqliteDb, user_id: i64, title: &str) -> Work {
    db.create_work(CreateWorkDbRequest {
        user_id,
        title: title.into(),
        author_name: "Tag Author".into(),
        ..Default::default()
    })
    .await
    .unwrap()
    .0
}

async fn setup_item(
    db: &SqliteDb,
    user_id: i64,
    work_id: i64,
    root_id: i64,
    path: &str,
) -> livrarr_domain::LibraryItem {
    db.create_library_item(CreateLibraryItemDbRequest {
        user_id,
        work_id,
        root_folder_id: root_id,
        path: path.into(),
        media_type: MediaType::Ebook,
        file_size: 4,
        import_id: None,
        tag_status: TagStatus::Pending,
        tagged_at_generation: 0,
    })
    .await
    .unwrap()
}

fn make_tag_service(
    db: SqliteDb,
    data_dir: &std::path::Path,
) -> LiveTagService<ImportIoServiceImpl<SqliteDb>> {
    LiveTagService::new(
        Arc::new(ImportIoServiceImpl::new(db.clone())),
        Arc::new(data_dir.to_path_buf()),
        db,
    )
}

#[tokio::test]
async fn wh_batch_retag_success_writes_one_tag_written_per_work_per_pass() {
    let tmp = tempfile::tempdir().unwrap();
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work = setup_work(&db, user_id, "Tagged Work").await;
    let root = db
        .create_root_folder(tmp.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    std::fs::write(tmp.path().join("unsupported.bin"), b"data").unwrap();
    let item = setup_item(&db, user_id, work.id, root.id, "unsupported.bin").await;

    let service = make_tag_service(db.clone(), tmp.path());
    let results = service.retag_library_items(&work, &[item]).await;
    assert_eq!(results.len(), 1);
    assert!(
        results[0].succeeded,
        "unsupported formats are successful no-op tag syncs"
    );

    let history = db
        .list_history(user_id, empty_history_filter())
        .await
        .unwrap();
    let written: Vec<_> = history
        .iter()
        .filter(|event| event.event_type == EventType::TagWritten)
        .collect();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].work_id, Some(work.id));
    assert_eq!(written[0].data["work_title"], "Tagged Work");
    assert_eq!(written[0].data["attempted"], 1);
    assert_eq!(written[0].data["succeeded"], 1);
}

#[tokio::test]
async fn wh_batch_retag_all_files_fail_writes_one_tag_write_failed_with_first_error() {
    let tmp = tempfile::tempdir().unwrap();
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work = setup_work(&db, user_id, "Failing Tag Work").await;
    let root = db
        .create_root_folder(tmp.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    let item_a = setup_item(&db, user_id, work.id, root.id, "missing-a.bin").await;
    let item_b = setup_item(&db, user_id, work.id, root.id, "missing-b.bin").await;

    let service = make_tag_service(db.clone(), tmp.path());
    let results = service
        .retag_library_items(&work, &[item_a.clone(), item_b])
        .await;
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|result| !result.succeeded));

    let history = db
        .list_history(user_id, empty_history_filter())
        .await
        .unwrap();
    let failed: Vec<_> = history
        .iter()
        .filter(|event| event.event_type == EventType::TagWriteFailed)
        .collect();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].work_id, Some(work.id));
    assert_eq!(failed[0].data["work_title"], "Failing Tag Work");
    assert_eq!(failed[0].data["attempted"], 2);
    assert!(
        failed[0].data["error"]
            .as_str()
            .unwrap()
            .contains("missing-a.bin"),
        "first failure should be carried verbatim"
    );
}

#[tokio::test]
async fn wh_batch_retag_empty_item_set_records_zero_events() {
    let tmp = tempfile::tempdir().unwrap();
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work = setup_work(&db, user_id, "Empty Tag Work").await;

    let service = make_tag_service(db.clone(), tmp.path());
    let results = service.retag_library_items(&work, &[]).await;
    assert!(results.is_empty());

    let history = db
        .list_history(user_id, empty_history_filter())
        .await
        .unwrap();
    assert!(history.is_empty());
}

#[tokio::test]
async fn wh_reorganize_road_retag_yields_exactly_one_tag_event() {
    let lib_dir = tempfile::tempdir().unwrap();
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work = setup_work(&db, user_id, "Reorganize Work").await;
    db.update_work_enrichment(
        user_id,
        work.id,
        livrarr_db::UpdateWorkEnrichmentDbRequest {
            enrichment_status: EnrichmentStatus::Enriched,
            enrichment_source: Some("wh-reorganize-test".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let root = db
        .create_root_folder(lib_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    // Seed the item at a non-canonical path with a real file on disk so the
    // reorganize road actually moves it. The Ebook branch of build_target_path
    // preserves the source file's extension, so an unsupported one (.bin)
    // survives the move and tag-syncs as a no-op success — exactly like the
    // batch success test above.
    let stray_path = lib_dir.path().join("stray").join("misfiled.bin");
    std::fs::create_dir_all(stray_path.parent().unwrap()).unwrap();
    std::fs::write(&stray_path, b"data").unwrap();
    setup_item(&db, user_id, work.id, root.id, "stray/misfiled.bin").await;

    let import_io = Arc::new(LiveImportIoService::new(db.clone()));
    let import_workflow = Arc::new(LiveImportWorkflow::new(
        db.clone(),
        Arc::new(tokio::sync::Semaphore::new(2)),
        Arc::new(lib_dir.path().to_path_buf()),
        Arc::new(TagwriteChapterExtractor),
    ));
    let tag_service = Arc::new(LiveTagService::new(
        import_io.clone(),
        Arc::new(lib_dir.path().to_path_buf()),
        db.clone(),
    ));
    let settings_service = Arc::new(LiveSettingsService::new(db.clone()));
    let http_client_safe = livrarr_http::HttpClientBuilder::default().build().unwrap();
    let svc = LiveImportService::new(
        import_io,
        import_workflow,
        tag_service,
        settings_service,
        http_client_safe,
    );

    let warnings = svc
        .reorganize_work_files(user_id, work.id)
        .await
        .expect("reorganize should succeed");
    assert!(
        warnings.is_empty(),
        "expected a clean move, got {warnings:?}"
    );

    let history = db
        .list_history(user_id, empty_history_filter())
        .await
        .unwrap();
    let written: Vec<_> = history
        .iter()
        .filter(|event| event.event_type == EventType::TagWritten)
        .collect();
    assert_eq!(written.len(), 1, "expected exactly one tagWritten event");
    assert_eq!(written[0].work_id, Some(work.id));
    assert_eq!(written[0].data["work_title"], "Reorganize Work");
    assert_eq!(written[0].data["attempted"], 1);
    assert_eq!(written[0].data["succeeded"], 1);
    assert_eq!(
        history
            .iter()
            .filter(|event| event.event_type == EventType::TagWriteFailed)
            .count(),
        0,
        "no tagWriteFailed events expected"
    );
}

#[tokio::test]
async fn wh_convergence_recovered_item_writes_single_file_tag_written_payload() {
    let tmp = tempfile::tempdir().unwrap();
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work = setup_work(&db, user_id, "Recovered Work").await;
    db.update_work_enrichment(
        user_id,
        work.id,
        livrarr_db::UpdateWorkEnrichmentDbRequest {
            enrichment_status: EnrichmentStatus::Enriched,
            enrichment_source: Some("wh-tag-convergence-test".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let root = db
        .create_root_folder(tmp.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    std::fs::write(tmp.path().join("recover.bin"), b"data").unwrap();
    let item = setup_item(&db, user_id, work.id, root.id, "recover.bin").await;

    livrarr_server::jobs::tag_convergence::recover_item_tags(
        &db,
        &ImportIoServiceImpl::new(db.clone()),
        tmp.path(),
        &item,
    )
    .await;

    let history = db
        .list_history(user_id, empty_history_filter())
        .await
        .unwrap();
    let written: Vec<_> = history
        .iter()
        .filter(|event| event.event_type == EventType::TagWritten)
        .collect();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].work_id, Some(work.id));
    assert_eq!(written[0].data["work_title"], "Recovered Work");
    assert_eq!(written[0].data["path"], "recover.bin");
    assert!(written[0].data.get("attempted").is_none());
    assert!(written[0].data.get("succeeded").is_none());

    let failed_count = history
        .iter()
        .filter(|event| event.event_type == EventType::TagWriteFailed)
        .count();
    assert_eq!(failed_count, 0);
}

#[tokio::test]
async fn wh_convergence_still_failing_item_in_same_sweep_records_zero_new_events() {
    let tmp = tempfile::tempdir().unwrap();
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work = setup_work(&db, user_id, "Still Failing Work").await;
    db.update_work_enrichment(
        user_id,
        work.id,
        livrarr_db::UpdateWorkEnrichmentDbRequest {
            enrichment_status: EnrichmentStatus::Enriched,
            enrichment_source: Some("wh-tag-convergence-test".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let root = db
        .create_root_folder(tmp.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    // No file written at "gone.bin" — tag sync fails, item stays Failed.
    let item = setup_item(&db, user_id, work.id, root.id, "gone.bin").await;

    livrarr_server::jobs::tag_convergence::recover_item_tags(
        &db,
        &ImportIoServiceImpl::new(db.clone()),
        tmp.path(),
        &item,
    )
    .await;

    let history = db
        .list_history(user_id, empty_history_filter())
        .await
        .unwrap();
    let written = history
        .iter()
        .filter(|event| event.event_type == EventType::TagWritten)
        .count();
    let failed = history
        .iter()
        .filter(|event| event.event_type == EventType::TagWriteFailed)
        .count();
    assert_eq!(written, 0);
    assert_eq!(failed, 0);
}
