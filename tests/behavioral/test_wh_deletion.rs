#![allow(dead_code)]

//! RED behavioral tests for work-history deletion events.

mod common;

use common::create_test_db;
use livrarr_db::{
    CreateHistoryEventDbRequest, CreateImportDbRequest, CreateLibraryItemDbRequest,
    CreateUserDbRequest, CreateWorkDbRequest, HistoryDb, HistoryFilter, ImportDb, LibraryItemDb,
    RootFolderDb, TagStatus, UserDb, WorkDbCreate,
};
use livrarr_domain::services::{FileService, ManualImportService, WorkService};
use livrarr_domain::{EventType, HistoryEvent, MediaType, UserId, UserRole, WorkId};
use livrarr_library::file_service::FileServiceImpl;
use livrarr_metadata::work_service::WorkServiceImpl;
use livrarr_server::manual_import_service::ManualImportServiceImpl;

const WORK_TITLE: &str = "History Delete Work";
const WORK_AUTHOR: &str = "History Delete Author";

async fn seed_user(db: &livrarr_db::sqlite::SqliteDb) -> UserId {
    db.create_user(CreateUserDbRequest {
        username: "wh-user".to_string(),
        password_hash: "hash".to_string(),
        role: UserRole::Admin,
        api_key_hash: "wh-api-key".to_string(),
    })
    .await
    .unwrap()
    .id
}

fn work_req(user_id: UserId, title: &str, import_id: Option<&str>) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: WORK_AUTHOR.to_string(),
        normalized_title: livrarr_domain::normalize_for_matching(title),
        normalized_author: livrarr_domain::normalize_for_matching(WORK_AUTHOR),
        import_id: import_id.map(str::to_string),
        ..Default::default()
    }
}

async fn seed_work(db: &livrarr_db::sqlite::SqliteDb, user_id: UserId, title: &str) -> WorkId {
    db.create_work(work_req(user_id, title, None))
        .await
        .unwrap()
        .0
        .id
}

async fn seed_library_item(
    db: &livrarr_db::sqlite::SqliteDb,
    user_id: UserId,
    work_id: WorkId,
    path: &str,
    media_type: MediaType,
    import_id: Option<&str>,
) -> i64 {
    let root_path = format!("/tmp/livrarr-wh-{}", path.replace('/', "-"));
    // root_folders.media_type is UNIQUE — a second item of the same type must
    // reuse the existing root instead of creating another.
    let root = match db.create_root_folder(&root_path, media_type).await {
        Ok(root) => root,
        Err(_) => db
            .list_root_folders()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.media_type == media_type)
            .expect("existing root folder of this media type"),
    };
    db.create_library_item(CreateLibraryItemDbRequest {
        user_id,
        work_id,
        root_folder_id: root.id,
        path: path.to_string(),
        media_type,
        file_size: 1024,
        import_id: import_id.map(str::to_string),
        tag_status: TagStatus::Pending,
        tagged_at_generation: 0,
    })
    .await
    .unwrap()
    .id
}

async fn seed_prior_history(db: &livrarr_db::sqlite::SqliteDb, user_id: UserId, work_id: WorkId) {
    db.create_history_event(CreateHistoryEventDbRequest {
        user_id,
        work_id: Some(work_id),
        event_type: EventType::Imported,
        data: serde_json::json!({
            "work_title": WORK_TITLE,
            "path": "prior.epub",
            "media_type": "ebook"
        }),
        date: None,
    })
    .await
    .unwrap();
}

async fn history(db: &livrarr_db::sqlite::SqliteDb, user_id: UserId) -> Vec<HistoryEvent> {
    db.list_history(
        user_id,
        HistoryFilter {
            event_type: None,
            work_id: None,
            start_date: None,
            end_date: None,
        },
    )
    .await
    .unwrap()
}

fn events_of(events: &[HistoryEvent], event_type: EventType) -> Vec<&HistoryEvent> {
    events
        .iter()
        .filter(|event| event.event_type == event_type)
        .collect()
}

fn assert_one_file_deleted(events: &[HistoryEvent], path: &str, media_type: &str) {
    let deleted = events_of(events, EventType::FileDeleted);
    assert_eq!(deleted.len(), 1, "expected exactly one fileDeleted event");
    let event = deleted[0];
    assert_eq!(event.data["path"].as_str(), Some(path));
    assert_eq!(event.data["media_type"].as_str(), Some(media_type));
    assert_eq!(event.data["work_title"].as_str(), Some(WORK_TITLE));
    assert!(
        event.data.get("undo").is_none(),
        "non-undo delete payload must omit undo"
    );
}

#[tokio::test]
async fn wh_file_service_delete_records_one_file_deleted() {
    let db = create_test_db().await;
    let user_id = seed_user(&db).await;
    let work_id = seed_work(&db, user_id, WORK_TITLE).await;
    let item_id = seed_library_item(
        &db,
        user_id,
        work_id,
        "library-road/book.epub",
        MediaType::Ebook,
        None,
    )
    .await;

    FileServiceImpl::new(db.clone())
        .delete(user_id, item_id)
        .await
        .unwrap();

    let events = history(&db, user_id).await;
    assert_one_file_deleted(&events, "library-road/book.epub", "ebook");
}

#[tokio::test]
async fn wh_manual_import_delete_library_item_records_one_file_deleted() {
    let db = create_test_db().await;
    let user_id = seed_user(&db).await;
    let work_id = seed_work(&db, user_id, WORK_TITLE).await;
    let item_id = seed_library_item(
        &db,
        user_id,
        work_id,
        "manual-road/book.m4b",
        MediaType::Audiobook,
        None,
    )
    .await;

    ManualImportServiceImpl::new(db.clone())
        .delete_library_item(user_id, item_id)
        .await
        .unwrap();

    let events = history(&db, user_id).await;
    assert_one_file_deleted(&events, "manual-road/book.m4b", "audiobook");
}

// REQ-005 door (c) "secondary API delete" — NO TEST, BY DISPOSITION (tests-review
// r1, google R-1 CONFIRMED and extended): the test as authored drove the raw
// `SqliteDb::delete_library_item` seam and asserted a fileDeleted event there,
// which the design forbids (writers live at the doors, never inside DB methods —
// ir-v2 D-WRITE-PATH). Deeper: `api_secondary_impl` is `#[cfg(test)]` in
// livrarr-server's lib.rs, `SecondaryApiImpl` has zero production references, and
// no other type implements `LibraryFileApi` — the "door" has no production
// surface today (same shape as the proven-empty REQ-008 door (d)). No writer is
// built for it and no event can be pinned; flagged to the PO as a spec-level
// stale enumeration. Any future productionization of the secondary API must add
// the fileDeleted writer, its doors row, and this test in the same change.

#[tokio::test]
async fn wh_work_delete_records_one_unattached_work_deleted_and_preserves_prior_history() {
    let db = create_test_db().await;
    let user_id = seed_user(&db).await;
    let work_id = seed_work(&db, user_id, WORK_TITLE).await;
    seed_library_item(
        &db,
        user_id,
        work_id,
        "work-delete/one.epub",
        MediaType::Ebook,
        None,
    )
    .await;
    seed_library_item(
        &db,
        user_id,
        work_id,
        "work-delete/two.epub",
        MediaType::Ebook,
        None,
    )
    .await;
    seed_prior_history(&db, user_id, work_id).await;

    let svc = WorkServiceImpl::new(
        db.clone(),
        livrarr_behavioral::stubs::StubEnrichmentWorkflow::succeeding(),
        livrarr_behavioral::stubs::StubHttpFetcher::new(),
        tempfile::tempdir().expect("test data dir").keep(),
    );
    svc.delete(user_id, work_id).await.unwrap();

    let events = history(&db, user_id).await;
    assert_eq!(
        events_of(&events, EventType::FileDeleted).len(),
        0,
        "whole-work delete is composite-only"
    );

    let work_deleted = events_of(&events, EventType::WorkDeleted);
    assert_eq!(work_deleted.len(), 1, "expected one workDeleted event");
    let event = work_deleted[0];
    assert_eq!(event.work_id, None, "workDeleted row must end unattached");
    assert_eq!(event.data["work_title"].as_str(), Some(WORK_TITLE));
    assert_eq!(event.data["work_author"].as_str(), Some(WORK_AUTHOR));
    assert_eq!(event.data["files_removed"].as_u64(), Some(2));
    assert!(event.data.get("undo").is_none());

    let prior = events
        .iter()
        .find(|event| event.event_type == EventType::Imported)
        .expect("prior history remains listable after ON DELETE SET NULL");
    assert_eq!(prior.work_id, None);
    assert_eq!(prior.data["work_title"].as_str(), Some(WORK_TITLE));
}

#[tokio::test]
async fn wh_readarr_import_undo_marks_file_and_orphan_work_deletions_as_undo() {
    let db = create_test_db().await;
    let user_id = seed_user(&db).await;
    let import_id = "wh-undo-import-1";

    db.create_import(CreateImportDbRequest {
        id: import_id.to_string(),
        user_id,
        source: "readarr".to_string(),
        source_url: None,
        target_root_folder_id: None,
    })
    .await
    .unwrap();
    // create_import always lands as "running"; undo refuses a running import.
    db.update_import_status(import_id, "completed")
        .await
        .unwrap();

    let work_id = db
        .create_work(work_req(user_id, WORK_TITLE, Some(import_id)))
        .await
        .unwrap()
        .0
        .id;
    seed_library_item(
        &db,
        user_id,
        work_id,
        "undo/one.epub",
        MediaType::Ebook,
        Some(import_id),
    )
    .await;
    seed_library_item(
        &db,
        user_id,
        work_id,
        "undo/two.m4b",
        MediaType::Audiobook,
        Some(import_id),
    )
    .await;

    let service = livrarr_server::readarr_import_service::LiveReadarrImportService::new(db.clone());
    let tmp = tempfile::tempdir().expect("test data dir");

    let response = livrarr_server::readarr_import_workflow::undo_import(
        &service,
        tmp.path(),
        &db,
        user_id,
        import_id,
    )
    .await
    .unwrap();

    assert_eq!(
        response.files_deleted, 2,
        "fixture-reality: 2 items removed"
    );
    assert_eq!(
        response.works_deleted, 1,
        "fixture-reality: 1 orphan work removed"
    );

    let events = history(&db, user_id).await;

    let file_deleted = events_of(&events, EventType::FileDeleted);
    assert_eq!(
        file_deleted.len(),
        2,
        "expected one fileDeleted per undone item"
    );
    for event in &file_deleted {
        let path = event.data["path"].as_str().expect("path");
        let expected_media_type = match path {
            "undo/one.epub" => "ebook",
            "undo/two.m4b" => "audiobook",
            other => panic!("unexpected path in fileDeleted event: {other}"),
        };
        assert_eq!(event.data["media_type"].as_str(), Some(expected_media_type));
        assert_eq!(event.data["work_title"].as_str(), Some(WORK_TITLE));
        assert_eq!(event.data["undo"].as_bool(), Some(true));
    }

    let work_deleted = events_of(&events, EventType::WorkDeleted);
    assert_eq!(
        work_deleted.len(),
        1,
        "expected one workDeleted for the orphaned work"
    );
    let event = work_deleted[0];
    assert_eq!(event.work_id, None, "workDeleted row must end unattached");
    assert_eq!(event.data["work_title"].as_str(), Some(WORK_TITLE));
    assert_eq!(event.data["files_removed"].as_u64(), Some(0));
    assert_eq!(event.data["undo"].as_bool(), Some(true));
}
