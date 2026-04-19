// tests/behavioral/test_consolidation_author_monitor.rs
#![allow(dead_code, unused_imports)]

//! Behavioral tests for AuthorMonitorWorkflow trait (WF-MONITOR-001..002).
//! Covers: fn.author_monitor_workflow.run_monitor
//! Test obligations: test.monitor.*
//! Added for redesign phase:
//! - future global monitor / cancellation / provenance contracts captured as ignored tests

use livrarr_behavioral::stubs::{create_test_user, StubHttpFetcher};
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::*;
use livrarr_domain::services::*;
use livrarr_domain::*;
use livrarr_metadata::author_monitor_workflow::AuthorMonitorWorkflowImpl;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

// =============================================================================
// StubWorkService — tracks add() calls
// =============================================================================

struct StubWorkService {
    add_calls: Mutex<Vec<(UserId, AddWorkRequest)>>,
    should_fail: bool,
}

impl StubWorkService {
    fn succeeding() -> Self {
        Self {
            add_calls: Mutex::new(Vec::new()),
            should_fail: false,
        }
    }

    fn failing() -> Self {
        Self {
            add_calls: Mutex::new(Vec::new()),
            should_fail: true,
        }
    }

    async fn add_call_count(&self) -> usize {
        self.add_calls.lock().await.len()
    }

    async fn add_calls_snapshot(&self) -> Vec<(UserId, AddWorkRequest)> {
        self.add_calls.lock().await.drain(..).collect()
    }
}

impl WorkService for StubWorkService {
    async fn add(
        &self,
        user_id: UserId,
        req: AddWorkRequest,
    ) -> Result<AddWorkResult, WorkServiceError> {
        self.add_calls.lock().await.push((user_id, req));
        if self.should_fail {
            return Err(WorkServiceError::Enrichment("stub failure".into()));
        }
        Ok(AddWorkResult {
            work: Work::default(),
            author_created: false,
            author_id: None,
            messages: vec![],
        })
    }

    async fn get(&self, _user_id: UserId, _work_id: WorkId) -> Result<Work, WorkServiceError> {
        Ok(Work::default())
    }

    async fn get_detail(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
    ) -> Result<WorkDetailView, WorkServiceError> {
        Ok(WorkDetailView {
            work: Work::default(),
            library_items: vec![],
        })
    }

    async fn list(
        &self,
        _user_id: UserId,
        _filter: WorkFilter,
    ) -> Result<Vec<Work>, WorkServiceError> {
        Ok(vec![])
    }

    async fn list_paginated(
        &self,
        _user_id: UserId,
        _page: u32,
        _page_size: u32,
    ) -> Result<PaginatedWorksView, WorkServiceError> {
        Ok(PaginatedWorksView {
            works: vec![],
            total: 0,
            page: 1,
            page_size: 25,
        })
    }

    async fn update(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _req: UpdateWorkRequest,
    ) -> Result<Work, WorkServiceError> {
        Ok(Work::default())
    }

    async fn delete(&self, _user_id: UserId, _work_id: WorkId) -> Result<(), WorkServiceError> {
        Ok(())
    }

    async fn refresh(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
    ) -> Result<RefreshWorkResult, WorkServiceError> {
        Ok(RefreshWorkResult {
            work: Work::default(),
            messages: vec![],
            taggable_items: vec![],
            merge_deferred: false,
        })
    }

    async fn refresh_all(&self, _user_id: UserId) -> Result<RefreshAllHandle, WorkServiceError> {
        Ok(RefreshAllHandle { total_works: 0 })
    }

    async fn upload_cover(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _bytes: &[u8],
    ) -> Result<(), WorkServiceError> {
        Ok(())
    }

    async fn download_cover(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
    ) -> Result<Vec<u8>, WorkServiceError> {
        Ok(vec![])
    }

    async fn lookup(&self, _req: LookupRequest) -> Result<Vec<LookupResult>, WorkServiceError> {
        Ok(vec![])
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Build canned OL works.json response body.
fn ol_works_json(entries: &[(&str, &str, &str)]) -> Vec<u8> {
    // entries: [(ol_key, title, first_publish_date), ...]
    let entries_json: Vec<String> = entries
        .iter()
        .map(|(key, title, date)| {
            format!(
                r#"{{"key": "/works/{}", "title": "{}", "first_publish_date": "{}"}}"#,
                key, title, date
            )
        })
        .collect();
    format!(r#"{{"entries": [{}]}}"#, entries_json.join(",")).into_bytes()
}

async fn seed_monitored_author(
    db: &livrarr_db::sqlite::SqliteDb,
    user_id: UserId,
    name: &str,
    ol_key: &str,
    monitor_new_items: bool,
    monitor_since: Option<chrono::DateTime<chrono::Utc>>,
) -> Author {
    let author = db
        .create_author(CreateAuthorDbRequest {
            user_id,
            name: name.into(),
            sort_name: None,
            ol_key: Some(ol_key.into()),
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .unwrap();

    // Update monitoring settings
    db.update_author(
        user_id,
        author.id,
        UpdateAuthorDbRequest {
            name: None,
            sort_name: None,
            ol_key: None,
            gr_key: None,
            monitored: Some(true),
            monitor_new_items: Some(monitor_new_items),
            monitor_since,
        },
    )
    .await
    .unwrap()
}

// =============================================================================
// Tests
// =============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_monitor_auto_adds_new_work() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    let _author =
        seed_monitored_author(&db, user_id, "Brandon Sanderson", "OL1234A", true, None).await;

    let http = StubHttpFetcher::with_ok(200, ol_works_json(&[("OL999W", "New Book", "2025")]));

    let work_svc = Arc::new(StubWorkService::succeeding());
    let db_arc = Arc::new(db);
    let workflow = AuthorMonitorWorkflowImpl::new(db_arc.clone(), work_svc.clone(), Arc::new(http));

    let report = workflow.run_monitor(user_id).await.unwrap();

    assert_eq!(report.authors_checked, 1);
    assert_eq!(report.new_works_found, 1);
    assert_eq!(report.works_added, 1);
    assert_eq!(work_svc.add_call_count().await, 1);

    // Notification created
    let notifs = db_arc.list_notifications(user_id, false).await.unwrap();
    assert!(!notifs.is_empty());
    assert_eq!(notifs[0].notification_type, NotificationType::WorkAutoAdded);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_monitor_notification_only_when_disabled() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    let _author = seed_monitored_author(
        &db,
        user_id,
        "Brandon Sanderson",
        "OL1234A",
        false, // monitor_new_items = false
        None,
    )
    .await;

    let http = StubHttpFetcher::with_ok(200, ol_works_json(&[("OL999W", "New Book", "2025")]));

    let work_svc = Arc::new(StubWorkService::succeeding());
    let db_arc = Arc::new(db);
    let workflow = AuthorMonitorWorkflowImpl::new(db_arc.clone(), work_svc.clone(), Arc::new(http));

    let report = workflow.run_monitor(user_id).await.unwrap();

    assert_eq!(report.new_works_found, 1);
    assert_eq!(report.works_added, 0);
    assert_eq!(work_svc.add_call_count().await, 0);

    // Notification created — NewWorkDetected, not WorkAutoAdded
    let notifs = db_arc.list_notifications(user_id, false).await.unwrap();
    assert!(!notifs.is_empty());
    assert_eq!(
        notifs[0].notification_type,
        NotificationType::NewWorkDetected
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_monitor_skips_existing_work() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    let _author =
        seed_monitored_author(&db, user_id, "Brandon Sanderson", "OL1234A", true, None).await;

    // Seed existing work with ol_key matching what OL will return
    db.create_work(CreateWorkDbRequest {
        user_id,
        title: "Existing Book".into(),
        author_name: "Brandon Sanderson".into(),
        author_id: None,
        ol_key: Some("OL999W".into()),
        gr_key: None,
        year: None,
        cover_url: None,
        metadata_source: None,
        detail_url: None,
        language: None,
        import_id: None,
        series_id: None,
        series_name: None,
        series_position: None,
        monitor_ebook: true,
        monitor_audiobook: false,
    })
    .await
    .unwrap();

    let http = StubHttpFetcher::with_ok(200, ol_works_json(&[("OL999W", "Existing Book", "2025")]));

    let work_svc = Arc::new(StubWorkService::succeeding());
    let db_arc = Arc::new(db);
    let workflow = AuthorMonitorWorkflowImpl::new(db_arc.clone(), work_svc.clone(), Arc::new(http));

    let report = workflow.run_monitor(user_id).await.unwrap();

    assert_eq!(report.authors_checked, 1);
    assert_eq!(report.new_works_found, 0);
    assert_eq!(report.works_added, 0);
    assert_eq!(work_svc.add_call_count().await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_monitor_ol_429_backs_off_and_retries() {
    use std::time::Duration;
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    let _author =
        seed_monitored_author(&db, user_id, "Brandon Sanderson", "OL1234A", true, None).await;

    let http = StubHttpFetcher::new();
    // First two calls: 429, third: success with a new work
    http.push_response(Ok(FetchResponse {
        status: 429,
        headers: vec![],
        body: vec![],
    }));
    http.push_response(Ok(FetchResponse {
        status: 429,
        headers: vec![],
        body: vec![],
    }));
    http.push_response(Ok(FetchResponse {
        status: 200,
        headers: vec![],
        body: ol_works_json(&[("OL999W", "After Retry", "2025")]),
    }));

    let work_svc = Arc::new(StubWorkService::succeeding());
    let http_ref = http.clone();
    let db_arc = Arc::new(db);
    let workflow = AuthorMonitorWorkflowImpl::new(db_arc.clone(), work_svc.clone(), Arc::new(http))
        .with_backoff(Duration::from_millis(10), Duration::from_millis(10));

    let report = workflow.run_monitor(user_id).await.unwrap();
    assert_eq!(
        report.new_works_found, 1,
        "should find work after retry succeeds"
    );
    assert_eq!(report.works_added, 1, "should add work after retry");
    assert_eq!(
        http_ref.call_count(),
        3,
        "should have made 3 HTTP calls (429, 429, success)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_monitor_ol_error_continues_to_next_author() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    // Two monitored authors
    let _author1 = seed_monitored_author(&db, user_id, "Author One", "OL_FAIL_A", true, None).await;
    let _author2 = seed_monitored_author(&db, user_id, "Author Two", "OL_OK_A", true, None).await;

    let http = StubHttpFetcher::new();
    // First author: 500 error
    http.push_response(Ok(FetchResponse {
        status: 500,
        headers: vec![],
        body: b"Internal Server Error".to_vec(),
    }));
    // Second author: success
    http.push_response(Ok(FetchResponse {
        status: 200,
        headers: vec![],
        body: ol_works_json(&[("OL_NEW_W", "Good Book", "2025")]),
    }));

    let work_svc = Arc::new(StubWorkService::succeeding());
    let db_arc = Arc::new(db);
    let workflow = AuthorMonitorWorkflowImpl::new(db_arc.clone(), work_svc.clone(), Arc::new(http));

    let report = workflow.run_monitor(user_id).await.unwrap();

    // First author fails, second succeeds
    assert_eq!(report.authors_checked, 2);
    assert_eq!(report.new_works_found, 1);
    assert_eq!(report.works_added, 1);
    assert_eq!(work_svc.add_call_count().await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_monitor_publish_year_filter() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    // Author monitors since 2024
    let since = chrono::Utc::now()
        .with_year(2024)
        .unwrap()
        .with_month(1)
        .unwrap()
        .with_day(1)
        .unwrap();

    let _author = seed_monitored_author(
        &db,
        user_id,
        "Brandon Sanderson",
        "OL1234A",
        true,
        Some(since),
    )
    .await;

    let http = StubHttpFetcher::with_ok(
        200,
        ol_works_json(&[
            ("OL_OLD_W", "Old Book", "2020"),
            ("OL_NEW_W", "New Book", "2025"),
        ]),
    );

    let work_svc = Arc::new(StubWorkService::succeeding());
    let db_arc = Arc::new(db);
    let workflow = AuthorMonitorWorkflowImpl::new(db_arc.clone(), work_svc.clone(), Arc::new(http));

    let report = workflow.run_monitor(user_id).await.unwrap();

    assert_eq!(report.authors_checked, 1);
    // Only the 2025 work should be found (2020 < 2024 monitor_since)
    assert_eq!(report.new_works_found, 1);
    assert_eq!(report.works_added, 1);
    assert_eq!(work_svc.add_call_count().await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "pk-implement: AuthorMonitorImpl must pass provenance_setter=AutoAdded to WorkService::add()"]
async fn test_monitor_auto_add_passes_auto_added_provenance_setter() {
    // Redesign contract expressed against current trait shape:
    // monitor_new_items=true should call WorkService::add() with provenance_setter=AutoAdded
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    let _author =
        seed_monitored_author(&db, user_id, "Brandon Sanderson", "OL1234A", true, None).await;

    let http = StubHttpFetcher::with_ok(200, ol_works_json(&[("OL999W", "New Book", "2025")]));

    let work_svc = Arc::new(StubWorkService::succeeding());
    let db_arc = Arc::new(db);
    let workflow = AuthorMonitorWorkflowImpl::new(db_arc.clone(), work_svc.clone(), Arc::new(http));

    let report = workflow.run_monitor(user_id).await.unwrap();
    assert_eq!(report.works_added, 1);

    let calls = work_svc.add_calls_snapshot().await;
    assert_eq!(calls.len(), 1);
    let (called_user_id, req) = &calls[0];
    assert_eq!(*called_user_id, user_id);
    assert_eq!(
        req.provenance_setter,
        Some(ProvenanceSetter::AutoAdded),
        "auto-added monitor work should carry AutoAdded provenance"
    );
    assert_eq!(req.title, "New Book");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_monitor_disabled_does_not_call_work_service_add() {
    // Redesign contract expressed against current trait shape:
    // monitor_new_items=false should notify only, no add call
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    let _author =
        seed_monitored_author(&db, user_id, "Brandon Sanderson", "OL1234A", false, None).await;

    let http = StubHttpFetcher::with_ok(200, ol_works_json(&[("OL999W", "New Book", "2025")]));

    let work_svc = Arc::new(StubWorkService::succeeding());
    let db_arc = Arc::new(db);
    let workflow = AuthorMonitorWorkflowImpl::new(db_arc.clone(), work_svc.clone(), Arc::new(http));

    let report = workflow.run_monitor(user_id).await.unwrap();

    assert_eq!(report.new_works_found, 1);
    assert_eq!(report.works_added, 0);
    assert_eq!(work_svc.add_call_count().await, 0);

    let notifs = db_arc.list_notifications(user_id, false).await.unwrap();
    assert_eq!(notifs.len(), 1);
    assert_eq!(
        notifs[0].notification_type,
        NotificationType::NewWorkDetected
    );
}

use chrono::Datelike;

// =============================================================================
// redesign contracts (future trait/API)
// =============================================================================

#[tokio::test]
#[ignore = "pk-implement: redesigned AuthorMonitorWorkflow removes user_id and runs globally across all users"]
async fn test_monitor_run_globally_across_all_users() {
    todo!("Create monitored authors for multiple users in same DB, call redesigned run_monitor(cancel_token), assert report spans all users and per-user notifications/adds are created.")
}

#[tokio::test]
#[ignore = "pk-implement: multi-user compliance needs explicit verification hooks in redesigned workflow"]
async fn test_monitor_every_db_mutation_carries_owning_user_id() {
    todo!("Seed multiple users and monitored authors, run global monitor, assert all created works/notifications/import mutations are attached to correct owning user_id with no cross-user leakage.")
}

#[tokio::test]
#[ignore = "pk-implement: redesigned 429 behavior requires 60s backoff policy and explicit rate-limit notification"]
async fn test_monitor_429_uses_60s_backoff_max_3_retries_and_creates_rate_limit_notification() {
    todo!("Stub three 429 responses and timing controls, assert exactly 3 retries/max attempts, no infinite loop, and a rate-limit notification is created for affected user/global run context.")
}

#[tokio::test]
#[ignore = "pk-implement: redesigned AuthorMonitorWorkflow needs CancellationToken support"]
async fn test_monitor_honors_cancellation_token() {
    todo!("Seed enough authors to ensure looping, cancel token during run, assert workflow stops early and returns partial report.")
}
