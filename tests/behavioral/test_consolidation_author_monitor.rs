// tests/behavioral/test_consolidation_author_monitor.rs
#![allow(dead_code, unused_imports)]

//! Behavioral tests for AuthorMonitorWorkflow trait (WF-MONITOR-001..002).
//! Covers: fn.author_monitor_workflow.run_monitor
//! Test obligations: test.monitor.*

use livrarr_behavioral::stubs::{create_second_test_user, create_test_user, StubHttpFetcher};
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::*;
use livrarr_domain::services::*;
use livrarr_domain::*;
use livrarr_metadata::author_monitor_workflow::AuthorMonitorWorkflowImpl;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

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
        _sort_by: WorkSortField,
        _sort_dir: SortDirection,
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

    async fn lookup_filtered(
        &self,
        req: LookupRequest,
        _raw: bool,
    ) -> Result<LookupResponse, WorkServiceError> {
        let results = self.lookup(req).await?;
        let count = results.len();
        Ok(LookupResponse {
            results,
            filtered_count: count,
            raw_count: count,
            raw_available: false,
        })
    }

    async fn search_works(
        &self,
        _user_id: UserId,
        _query: &str,
        _page: u32,
        _page_size: u32,
    ) -> Result<(Vec<Work>, i64), WorkServiceError> {
        Ok((vec![], 0))
    }

    async fn download_cover_from_url(
        &self,
        _user_id: i64,
        _work_id: i64,
        _cover_url: &str,
    ) -> Result<(), WorkServiceError> {
        Ok(())
    }
    fn try_start_bulk_refresh(&self, _user_id: i64) -> bool {
        true
    }
    fn finish_bulk_refresh(&self, _user_id: i64) {}
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

    let report = workflow
        .run_monitor(user_id, CancellationToken::new())
        .await
        .unwrap();

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

    let report = workflow
        .run_monitor(user_id, CancellationToken::new())
        .await
        .unwrap();

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

    let author =
        seed_monitored_author(&db, user_id, "Brandon Sanderson", "OL1234A", true, None).await;

    // Seed existing work with ol_key matching what OL will return
    // Must link to author so list_works_by_author_ol_keys JOIN finds it
    db.create_work(CreateWorkDbRequest {
        user_id,
        title: "Existing Book".into(),
        author_name: "Brandon Sanderson".into(),
        author_id: Some(author.id),
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

    let report = workflow
        .run_monitor(user_id, CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(report.authors_checked, 1);
    assert_eq!(report.new_works_found, 0);
    assert_eq!(report.works_added, 0);
    assert_eq!(work_svc.add_call_count().await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_monitor_ol_429_backs_off_and_retries() {
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

    let report = workflow
        .run_monitor(user_id, CancellationToken::new())
        .await
        .unwrap();
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

    let report = workflow
        .run_monitor(user_id, CancellationToken::new())
        .await
        .unwrap();

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

    let report = workflow
        .run_monitor(user_id, CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(report.authors_checked, 1);
    // Only the 2025 work should be found (2020 < 2024 monitor_since)
    assert_eq!(report.new_works_found, 1);
    assert_eq!(report.works_added, 1);
    assert_eq!(work_svc.add_call_count().await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_monitor_auto_add_passes_auto_added_provenance_setter() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    let _author =
        seed_monitored_author(&db, user_id, "Brandon Sanderson", "OL1234A", true, None).await;

    let http = StubHttpFetcher::with_ok(200, ol_works_json(&[("OL999W", "New Book", "2025")]));

    let work_svc = Arc::new(StubWorkService::succeeding());
    let db_arc = Arc::new(db);
    let workflow = AuthorMonitorWorkflowImpl::new(db_arc.clone(), work_svc.clone(), Arc::new(http));

    let report = workflow
        .run_monitor(user_id, CancellationToken::new())
        .await
        .unwrap();
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
    assert!(!req.defer_enrichment);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_monitor_disabled_does_not_call_work_service_add() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    let _author =
        seed_monitored_author(&db, user_id, "Brandon Sanderson", "OL1234A", false, None).await;

    let http = StubHttpFetcher::with_ok(200, ol_works_json(&[("OL999W", "New Book", "2025")]));

    let work_svc = Arc::new(StubWorkService::succeeding());
    let db_arc = Arc::new(db);
    let workflow = AuthorMonitorWorkflowImpl::new(db_arc.clone(), work_svc.clone(), Arc::new(http));

    let report = workflow
        .run_monitor(user_id, CancellationToken::new())
        .await
        .unwrap();

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
// Redesign contracts — now implemented
// =============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_monitor_run_per_user_processes_only_that_users_authors() {
    // run_monitor is now user-scoped. The scheduled job iterates users and
    // calls once per user. This test verifies each call only sees its user's authors.
    let db = create_test_db().await;
    let user1 = create_test_user(&db).await;
    let user2 = create_second_test_user(&db).await;

    // Each user has a monitored author with distinct OL key
    let _author1 = seed_monitored_author(&db, user1, "Author One", "OL_USER1_A", true, None).await;
    let _author2 = seed_monitored_author(&db, user2, "Author Two", "OL_USER2_A", true, None).await;

    let http = StubHttpFetcher::new();
    // user1 call returns a work for author1
    http.push_response(Ok(FetchResponse {
        status: 200,
        headers: vec![],
        body: ol_works_json(&[("OL_W1", "Book One", "2025")]),
    }));
    // user2 call returns a work for author2
    http.push_response(Ok(FetchResponse {
        status: 200,
        headers: vec![],
        body: ol_works_json(&[("OL_W2", "Book Two", "2025")]),
    }));

    let work_svc = Arc::new(StubWorkService::succeeding());
    let db_arc = Arc::new(db);
    // Use a non-atomic workflow: each call sees only that user's authors.
    // We need two separate workflow instances since the AtomicBool is per-instance.
    let workflow1 =
        AuthorMonitorWorkflowImpl::new(db_arc.clone(), work_svc.clone(), Arc::new(http))
            .with_backoff(Duration::from_millis(10), Duration::from_millis(10));

    // Process user1
    let report1 = workflow1
        .run_monitor(user1, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(report1.authors_checked, 1, "user1 has 1 monitored author");
    assert_eq!(report1.new_works_found, 1);
    assert_eq!(report1.works_added, 1);

    // Process user2 (same workflow instance is now unlocked)
    let report2 = workflow1
        .run_monitor(user2, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(report2.authors_checked, 1, "user2 has 1 monitored author");
    assert_eq!(report2.new_works_found, 1);
    assert_eq!(report2.works_added, 1);

    // Verify add() was called with correct user_ids
    let calls = work_svc.add_calls_snapshot().await;
    assert_eq!(calls.len(), 2);
    let user_ids: Vec<UserId> = calls.iter().map(|(uid, _)| *uid).collect();
    assert!(user_ids.contains(&user1), "should add work for user1");
    assert!(user_ids.contains(&user2), "should add work for user2");

    // Each user got their own notification
    let notifs1 = db_arc.list_notifications(user1, false).await.unwrap();
    let notifs2 = db_arc.list_notifications(user2, false).await.unwrap();
    assert_eq!(notifs1.len(), 1);
    assert_eq!(notifs2.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_monitor_every_db_mutation_carries_owning_user_id() {
    // Verifies that run_monitor(user_id, ..) only touches that user's data.
    let db = create_test_db().await;
    let user1 = create_test_user(&db).await;
    let user2 = create_second_test_user(&db).await;

    // User1 monitors (auto-add), user2 notification-only
    let _author1 = seed_monitored_author(&db, user1, "Author One", "OL_A1", true, None).await;
    let _author2 = seed_monitored_author(&db, user2, "Author Two", "OL_A2", false, None).await;

    let http = StubHttpFetcher::new();
    http.push_response(Ok(FetchResponse {
        status: 200,
        headers: vec![],
        body: ol_works_json(&[("OL_W1", "Book One", "2025")]),
    }));
    http.push_response(Ok(FetchResponse {
        status: 200,
        headers: vec![],
        body: ol_works_json(&[("OL_W2", "Book Two", "2025")]),
    }));

    let work_svc = Arc::new(StubWorkService::succeeding());
    let db_arc = Arc::new(db);
    let workflow = AuthorMonitorWorkflowImpl::new(db_arc.clone(), work_svc.clone(), Arc::new(http))
        .with_backoff(Duration::from_millis(10), Duration::from_millis(10));

    // Simulate what the scheduled job does: call once per user
    let _r1 = workflow
        .run_monitor(user1, CancellationToken::new())
        .await
        .unwrap();
    let _r2 = workflow
        .run_monitor(user2, CancellationToken::new())
        .await
        .unwrap();

    // User1's add() called with user1, not user2
    let calls = work_svc.add_calls_snapshot().await;
    assert_eq!(calls.len(), 1, "only user1 has monitor_new_items=true");
    assert_eq!(calls[0].0, user1);

    // Notifications: user1 gets WorkAutoAdded, user2 gets NewWorkDetected
    let notifs1 = db_arc.list_notifications(user1, false).await.unwrap();
    let notifs2 = db_arc.list_notifications(user2, false).await.unwrap();
    assert_eq!(notifs1.len(), 1);
    assert_eq!(
        notifs1[0].notification_type,
        NotificationType::WorkAutoAdded
    );
    assert_eq!(notifs2.len(), 1);
    assert_eq!(
        notifs2[0].notification_type,
        NotificationType::NewWorkDetected
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_monitor_429_uses_60s_backoff_max_3_retries_and_creates_rate_limit_notification() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    let _author =
        seed_monitored_author(&db, user_id, "Brandon Sanderson", "OL1234A", true, None).await;

    let http = StubHttpFetcher::new();
    // 4 x 429 — exceeds max 3 retries
    for _ in 0..4 {
        http.push_response(Ok(FetchResponse {
            status: 429,
            headers: vec![],
            body: vec![],
        }));
    }

    let work_svc = Arc::new(StubWorkService::succeeding());
    let http_ref = http.clone();
    let db_arc = Arc::new(db);
    let workflow = AuthorMonitorWorkflowImpl::new(db_arc.clone(), work_svc.clone(), Arc::new(http))
        .with_backoff(Duration::from_millis(10), Duration::from_millis(10));

    let report = workflow
        .run_monitor(user_id, CancellationToken::new())
        .await
        .unwrap();

    // Author skipped after max retries
    assert_eq!(report.authors_checked, 1);
    assert_eq!(report.new_works_found, 0);
    assert_eq!(report.works_added, 0);
    // 4 HTTP calls: initial + 3 retries, then skip
    assert_eq!(http_ref.call_count(), 4);

    let notifs = db_arc.list_notifications(user_id, false).await.unwrap();
    assert!(
        notifs
            .iter()
            .any(|n| n.notification_type == NotificationType::RateLimitHit),
        "should create RateLimitHit notification on first 429"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_monitor_honors_cancellation_token() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    // Seed multiple monitored authors
    let _a1 = seed_monitored_author(&db, user_id, "Author A", "OL_A", true, None).await;
    let _a2 = seed_monitored_author(&db, user_id, "Author B", "OL_B", true, None).await;
    let _a3 = seed_monitored_author(&db, user_id, "Author C", "OL_C", true, None).await;

    // Each author gets a successful response
    let http = StubHttpFetcher::new();
    for _ in 0..3 {
        http.push_response(Ok(FetchResponse {
            status: 200,
            headers: vec![],
            body: ol_works_json(&[]),
        }));
    }

    let work_svc = Arc::new(StubWorkService::succeeding());
    let db_arc = Arc::new(db);
    // Use a real inter-author delay so cancellation can fire during sleep
    let workflow = AuthorMonitorWorkflowImpl::new(db_arc.clone(), work_svc.clone(), Arc::new(http))
        .with_backoff(Duration::from_secs(60), Duration::from_millis(200));

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();

    // Cancel after a short delay (after first author processed but during inter-author sleep)
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel_clone.cancel();
    });

    let report = workflow.run_monitor(user_id, cancel).await.unwrap();

    // Should have stopped early — not all 3 authors processed
    assert!(
        report.authors_checked < 3,
        "expected early stop due to cancellation, got {} authors checked",
        report.authors_checked
    );
}
