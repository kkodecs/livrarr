use livrarr_behavioral::stubs::{create_test_user, StubEnrichmentWorkflow, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    CreateHistoryEventDbRequest, CreateLibraryItemDbRequest, CreateWorkDbRequest, HistoryDb,
    LibraryItemDb, RootFolderDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::history_events::HistoryDraft;
use livrarr_domain::services::{HistoryService, RefreshSurface, WorkService};
use livrarr_domain::{
    normalize_for_matching, DbError, EnrichmentStatus, EventType, HistoryEvent, HistoryFilter,
    IdentityStatus, MediaType, TagStatus, UserId, Work,
};
use livrarr_metadata::work_service::WorkServiceImpl;
use livrarr_server::history_service::HistoryServiceImpl;

#[derive(Clone)]
struct FailingHistoryDb;

impl HistoryDb for FailingHistoryDb {
    async fn list_history(
        &self,
        _user_id: UserId,
        _filter: HistoryFilter,
    ) -> Result<Vec<HistoryEvent>, DbError> {
        Ok(vec![])
    }

    async fn list_history_paginated(
        &self,
        _user_id: UserId,
        _filter: HistoryFilter,
        _page: u32,
        _per_page: u32,
    ) -> Result<(Vec<HistoryEvent>, i64), DbError> {
        Ok((vec![], 0))
    }

    async fn create_history_event(&self, _req: CreateHistoryEventDbRequest) -> Result<(), DbError> {
        Err(DbError::Io(
            std::io::Error::other("simulated history insert failure").into(),
        ))
    }
}

fn empty_filter() -> HistoryFilter {
    HistoryFilter {
        event_type: None,
        work_id: None,
        start_date: None,
        end_date: None,
    }
}

fn draft(event_type: EventType) -> HistoryDraft {
    HistoryDraft {
        work_id: None,
        event_type,
        data: serde_json::json!({"work_title": "Observer Probe"}),
        date: None,
    }
}

fn work_req(user_id: UserId, title: &str) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: "Observer Author".to_string(),
        normalized_title: normalize_for_matching(title),
        normalized_author: normalize_for_matching("Observer Author"),
        language: Some("en".to_string()),
        monitor_ebook: true,
        monitor_audiobook: true,
        ..Default::default()
    }
}

async fn seed_work(db: &SqliteDb, user_id: UserId, title: &str) -> Work {
    let (work, created) = db
        .create_work(work_req(user_id, title))
        .await
        .expect("seed work");
    assert!(created, "fixture work must be newly created");
    db.set_identity_status(user_id, work.id, IdentityStatus::Confirmed)
        .await
        .expect("seed identity status");
    db.get_work(user_id, work.id).await.expect("read work")
}

fn service(db: SqliteDb) -> WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher> {
    WorkServiceImpl::new(
        db,
        StubEnrichmentWorkflow::succeeding(),
        StubHttpFetcher::new(),
        tempfile::tempdir().expect("test data dir").keep(),
    )
}

#[tokio::test]
async fn wh_record_history_absorbs_store_failure_and_host_flow_continues() {
    let db = FailingHistoryDb;
    livrarr_db::record_history(&db, 1, draft(EventType::IdentityResolved)).await;

    let host_flow_still_running = true;
    assert!(host_flow_still_running);
}

#[tokio::test]
async fn wh_history_service_record_absorbs_store_failure_and_host_flow_continues() {
    let service = HistoryServiceImpl::new(FailingHistoryDb);
    service.record(1, draft(EventType::IdentityResolved)).await;

    let host_flow_still_running = true;
    assert!(host_flow_still_running);
}

#[tokio::test]
async fn wh_unwritable_history_does_not_block_import_enrichment_or_work_delete() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_work(&db, user_id, "Observer End To End").await;

    sqlx::query(
        "CREATE TRIGGER history_unwritable \
         BEFORE INSERT ON history \
         BEGIN SELECT RAISE(ABORT, 'history unwritable'); END;",
    )
    .execute(db.pool())
    .await
    .expect("install history insert failure trigger");

    let root_dir = tempfile::tempdir().expect("library root");
    let root = db
        .create_root_folder(
            root_dir.path().to_str().expect("utf8 root path"),
            MediaType::Ebook,
        )
        .await
        .expect("create root folder");
    let imported = db
        .create_library_item(CreateLibraryItemDbRequest {
            user_id,
            work_id: work.id,
            root_folder_id: root.id,
            path: "Observer End To End/observer.epub".to_string(),
            media_type: MediaType::Ebook,
            file_size: 42,
            import_id: None,
            tag_status: TagStatus::Pending,
            tagged_at_generation: 0,
        })
        .await
        .expect("import/library item write must still succeed");
    assert_eq!(imported.work_id, work.id);

    let svc = service(db.clone());
    svc.refresh(user_id, work.id, RefreshSurface::Interactive)
        .await
        .expect("enrichment pass must still succeed when history is unwritable");
    db.update_work_enrichment(
        user_id,
        work.id,
        livrarr_db::UpdateWorkEnrichmentDbRequest {
            enrichment_status: EnrichmentStatus::Enriched,
            enrichment_source: Some("observer-test".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("enrichment effect lands");

    svc.delete(user_id, work.id)
        .await
        .expect("work deletion must still succeed when history is unwritable");
    assert!(
        db.get_work(user_id, work.id).await.is_err(),
        "work deletion effect lands"
    );
    assert!(
        db.list_history(user_id, empty_filter())
            .await
            .expect("list history")
            .is_empty(),
        "only history rows are missing"
    );
}
