#![allow(dead_code)]

//! RED behavioral tests for work-history merge repointing and worksMerged events.

mod common;

use common::create_test_db;
use livrarr_db::{
    CreateHistoryEventDbRequest, CreateUserDbRequest, CreateWorkDbRequest, HistoryDb,
    HistoryFilter, MergeWorksDbRequest, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::services::WorkService;
use livrarr_domain::{EventType, HistoryEvent, UserId, UserRole, WorkId};
use livrarr_metadata::work_service::WorkServiceImpl;

const SURVIVOR_TITLE: &str = "Merge Survivor";
const LOSER_TITLE: &str = "Merge Loser";
const AUTHOR: &str = "Merge Author";

async fn seed_user(db: &livrarr_db::sqlite::SqliteDb) -> UserId {
    db.create_user(CreateUserDbRequest {
        username: "wh-merge-user".to_string(),
        password_hash: "hash".to_string(),
        role: UserRole::Admin,
        api_key_hash: "wh-merge-api-key".to_string(),
    })
    .await
    .unwrap()
    .id
}

fn work_req(user_id: UserId, title: &str) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: AUTHOR.to_string(),
        normalized_title: livrarr_domain::normalize_for_matching(title),
        normalized_author: livrarr_domain::normalize_for_matching(AUTHOR),
        ..Default::default()
    }
}

async fn seed_work(db: &livrarr_db::sqlite::SqliteDb, user_id: UserId, title: &str) -> WorkId {
    db.create_work(work_req(user_id, title)).await.unwrap().0.id
}

async fn seed_loser_history(db: &livrarr_db::sqlite::SqliteDb, user_id: UserId, loser_id: WorkId) {
    for (event_type, path) in [
        (EventType::Imported, "loser/imported.epub"),
        (EventType::TagWritten, "loser/tagged.epub"),
    ] {
        db.create_history_event(CreateHistoryEventDbRequest {
            user_id,
            work_id: Some(loser_id),
            event_type,
            data: serde_json::json!({
                "work_title": LOSER_TITLE,
                "path": path,
                "media_type": "ebook"
            }),
            date: None,
        })
        .await
        .unwrap();
    }
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

#[tokio::test]
async fn wh_work_db_merge_repoints_loser_history_to_survivor_without_changing_row_count() {
    let db = create_test_db().await;
    let user_id = seed_user(&db).await;
    let survivor_id = seed_work(&db, user_id, SURVIVOR_TITLE).await;
    let loser_id = seed_work(&db, user_id, LOSER_TITLE).await;
    seed_loser_history(&db, user_id, loser_id).await;
    let before = history(&db, user_id).await;

    db.merge_works(MergeWorksDbRequest {
        user_id,
        survivor_id,
        loser_id,
        monitor_ebook: false,
        monitor_audiobook: false,
        series_name: None,
        series_position: None,
    })
    .await
    .unwrap();

    let after = history(&db, user_id).await;
    assert_eq!(
        after.len(),
        before.len(),
        "merge repoints history rows, never deletes or adds in the DB half"
    );
    assert!(
        after.iter().all(|event| event.work_id == Some(survivor_id)),
        "every former-loser row should now point at the survivor"
    );
    assert!(
        after.iter().all(|event| event.work_id != Some(loser_id)),
        "no history row may remain on the deleted loser id"
    );
}

#[tokio::test]
async fn wh_work_service_merge_repoints_history_and_records_one_works_merged_event() {
    let db = create_test_db().await;
    let user_id = seed_user(&db).await;
    let survivor_id = seed_work(&db, user_id, SURVIVOR_TITLE).await;
    let loser_id = seed_work(&db, user_id, LOSER_TITLE).await;
    seed_loser_history(&db, user_id, loser_id).await;
    let before = history(&db, user_id).await;

    let svc = WorkServiceImpl::new(
        db.clone(),
        livrarr_behavioral::stubs::StubEnrichmentWorkflow::succeeding(),
        livrarr_behavioral::stubs::StubHttpFetcher::new(),
        tempfile::tempdir().expect("test data dir").keep(),
    );
    svc.merge_works(user_id, survivor_id, loser_id, Vec::new())
        .await
        .unwrap();

    let after = history(&db, user_id).await;
    assert_eq!(
        after.len(),
        before.len() + 1,
        "service merge repoints prior rows and adds exactly one worksMerged event"
    );
    assert!(
        after.iter().all(|event| event.work_id != Some(loser_id)),
        "no history row may remain on the loser after service merge"
    );

    let loser_rows: Vec<&HistoryEvent> = after
        .iter()
        .filter(|event| event.data["work_title"].as_str() == Some(LOSER_TITLE))
        .collect();
    assert_eq!(loser_rows.len(), 2);
    assert!(
        loser_rows
            .iter()
            .all(|event| event.work_id == Some(survivor_id)),
        "former-loser history should list under survivor after merge"
    );

    let merge_events = events_of(&after, EventType::WorksMerged);
    assert_eq!(merge_events.len(), 1);
    let merge_event = merge_events[0];
    assert_eq!(merge_event.work_id, Some(survivor_id));
    assert_eq!(
        merge_event.data["work_title"].as_str(),
        Some(SURVIVOR_TITLE)
    );
    assert_eq!(merge_event.data["merged_title"].as_str(), Some(LOSER_TITLE));
    assert_eq!(merge_event.data["merged_work_id"].as_i64(), Some(loser_id));
}
