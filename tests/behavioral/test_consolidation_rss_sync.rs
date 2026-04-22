#![allow(dead_code, unused_imports)]

//! Behavioral tests for RssSyncWorkflow trait (WF-RSS-001..002).
//! Covers: fn.rss_sync_workflow.run_sync
//! Test obligations: test.rss.*

use livrarr_behavioral::stubs::{create_test_user, StubHttpFetcher};
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::*;
use livrarr_domain::services::*;
use livrarr_domain::*;
use livrarr_metadata::rss_sync_workflow::RssSyncWorkflowImpl;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

// =============================================================================
// StubReleaseService — tracks grab() calls
// =============================================================================

struct StubReleaseService {
    grab_calls: Mutex<Vec<(UserId, GrabRequest)>>,
    should_fail: bool,
}

impl StubReleaseService {
    fn succeeding() -> Self {
        Self {
            grab_calls: Mutex::new(Vec::new()),
            should_fail: false,
        }
    }

    fn failing() -> Self {
        Self {
            grab_calls: Mutex::new(Vec::new()),
            should_fail: true,
        }
    }

    async fn grab_call_count(&self) -> usize {
        self.grab_calls.lock().await.len()
    }
}

impl ReleaseService for StubReleaseService {
    async fn search(
        &self,
        _user_id: UserId,
        _req: SearchReleasesRequest,
    ) -> Result<ReleaseSearchResponse, ReleaseServiceError> {
        Ok(ReleaseSearchResponse {
            results: vec![],
            warnings: vec![],
            cache_age_seconds: None,
        })
    }

    async fn grab(&self, user_id: UserId, req: GrabRequest) -> Result<Grab, ReleaseServiceError> {
        self.grab_calls.lock().await.push((user_id, req));
        if self.should_fail {
            return Err(ReleaseServiceError::ClientUnreachable(
                "stub failure".into(),
            ));
        }
        Ok(Grab {
            id: 1,
            user_id,
            work_id: 0,
            download_client_id: 1,
            title: "test".into(),
            indexer: "test".into(),
            guid: "test".into(),
            size: None,
            download_url: "test".into(),
            download_id: None,
            status: GrabStatus::Sent,
            import_error: None,
            media_type: None,
            content_path: None,
            grabbed_at: chrono::Utc::now(),
            import_retry_count: 0,
            import_failed_at: None,
        })
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Build a minimal Torznab RSS XML response.
fn rss_xml(items: &[(&str, &str, &str, i64)]) -> Vec<u8> {
    // items: [(title, guid, link, size)]
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
<channel>
<title>Test Feed</title>
"#,
    );
    for (title, guid, link, size) in items {
        xml.push_str(&format!(
            r#"<item>
<title>{}</title>
<guid>{}</guid>
<link>{}</link>
<enclosure length="{}" type="application/x-bittorrent" url="{}"/>
<newznab:attr name="category" value="7020"/>
<pubDate>Sun, 01 Jan 2028 00:00:00 +0000</pubDate>
</item>
"#,
            title, guid, link, size, link
        ));
    }
    xml.push_str("</channel>\n</rss>");
    xml.into_bytes()
}

async fn seed_rss_indexer(db: &livrarr_db::sqlite::SqliteDb, name: &str, url: &str) -> Indexer {
    db.create_indexer(CreateIndexerDbRequest {
        name: name.into(),
        protocol: "torrent".into(),
        url: url.into(),
        api_path: "/api".into(),
        api_key: Some("testkey".into()),
        categories: vec![7000],
        priority: 25,
        enable_automatic_search: true,
        enable_interactive_search: true,
        enable_rss: true,
        enabled: true,
    })
    .await
    .unwrap()
}

async fn seed_monitored_work(
    db: &livrarr_db::sqlite::SqliteDb,
    user_id: UserId,
    title: &str,
    author: &str,
) -> Work {
    db.create_work(CreateWorkDbRequest {
        user_id,
        title: title.into(),
        author_name: author.into(),
        author_id: None,
        ol_key: None,
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
    .unwrap()
}

// =============================================================================
// Tests
// =============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_rss_sync_matches_and_grabs_release() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    let _indexer = seed_rss_indexer(&db, "TestIndexer", "http://indexer.test").await;

    // Seed RSS state so it's NOT a first sync
    db.upsert_rss_state(_indexer.id, Some("2024-12-01"), "old-guid")
        .await
        .unwrap();

    let _work = seed_monitored_work(&db, user_id, "The Way of Kings", "Brandon Sanderson").await;

    // Need a download client for protocol eligibility
    db.create_download_client(CreateDownloadClientDbRequest {
        name: "TestClient".into(),
        implementation: DownloadClientImplementation::QBittorrent,
        host: "localhost".into(),
        port: 8080,
        use_ssl: false,
        skip_ssl_validation: false,
        url_base: None,
        username: None,
        password: None,
        category: "books".into(),
        enabled: true,
        api_key: None,
    })
    .await
    .unwrap();

    // RSS feed contains a release matching the work
    let feed = rss_xml(&[(
        "The Way of Kings Brandon Sanderson EPUB",
        "guid-001",
        "http://indexer.test/dl/1",
        1_000_000,
    )]);

    let http = StubHttpFetcher::with_ok(200, feed);
    let release_svc = Arc::new(StubReleaseService::succeeding());
    let db_arc = Arc::new(db);

    let workflow = RssSyncWorkflowImpl::new(db_arc.clone(), Arc::new(http), release_svc.clone());

    let report = workflow.run_sync().await.unwrap();

    assert_eq!(report.feeds_checked, 1);
    assert!(report.releases_matched >= 1);
    assert_eq!(report.grabs_attempted, 1);
    assert_eq!(report.grabs_succeeded, 1);
    assert_eq!(release_svc.grab_call_count().await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_rss_sync_first_sync_records_state_no_grab() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    let indexer = seed_rss_indexer(&db, "TestIndexer", "http://indexer.test").await;
    // No RSS state seeded — this is a first sync

    let _work = seed_monitored_work(&db, user_id, "The Way of Kings", "Brandon Sanderson").await;

    let feed = rss_xml(&[(
        "The Way of Kings Brandon Sanderson EPUB",
        "guid-001",
        "http://indexer.test/dl/1",
        1_000_000,
    )]);

    let http = StubHttpFetcher::with_ok(200, feed);
    let release_svc = Arc::new(StubReleaseService::succeeding());
    let db_arc = Arc::new(db);

    let workflow = RssSyncWorkflowImpl::new(db_arc.clone(), Arc::new(http), release_svc.clone());

    let report = workflow.run_sync().await.unwrap();

    // Feeds were checked but no grabs on first sync
    assert_eq!(report.feeds_checked, 1);
    assert_eq!(report.grabs_attempted, 0);
    assert_eq!(report.grabs_succeeded, 0);
    assert_eq!(release_svc.grab_call_count().await, 0);

    // RSS state should be recorded
    let state = db_arc.get_rss_state(indexer.id).await.unwrap();
    assert!(
        state.is_some(),
        "RSS state should be recorded after first sync"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_rss_sync_skips_active_grab() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    let indexer = seed_rss_indexer(&db, "TestIndexer", "http://indexer.test").await;
    db.upsert_rss_state(indexer.id, Some("2024-12-01"), "old-guid")
        .await
        .unwrap();

    let work = seed_monitored_work(&db, user_id, "The Way of Kings", "Brandon Sanderson").await;

    // Need a download client for the grab
    let dc = db
        .create_download_client(CreateDownloadClientDbRequest {
            name: "TestClient".into(),
            implementation: DownloadClientImplementation::QBittorrent,
            host: "localhost".into(),
            port: 8080,
            use_ssl: false,
            skip_ssl_validation: false,
            url_base: None,
            username: None,
            password: None,
            category: "books".into(),
            enabled: true,
            api_key: None,
        })
        .await
        .unwrap();

    // Seed an active grab for this work
    db.upsert_grab(CreateGrabDbRequest {
        user_id,
        work_id: work.id,
        download_client_id: dc.id,
        title: "Existing grab".into(),
        indexer: "TestIndexer".into(),
        guid: "existing-grab-guid".into(),
        size: Some(1_000_000),
        download_url: "http://example.com/dl".into(),
        download_id: None,
        status: GrabStatus::Sent,
        media_type: Some(MediaType::Ebook),
    })
    .await
    .unwrap();

    let feed = rss_xml(&[(
        "The Way of Kings Brandon Sanderson EPUB",
        "guid-new",
        "http://indexer.test/dl/2",
        1_000_000,
    )]);

    let http = StubHttpFetcher::with_ok(200, feed);
    let release_svc = Arc::new(StubReleaseService::succeeding());
    let db_arc = Arc::new(db);

    let workflow = RssSyncWorkflowImpl::new(db_arc.clone(), Arc::new(http), release_svc.clone());

    let report = workflow.run_sync().await.unwrap();

    // Should skip because active grab exists
    assert_eq!(report.grabs_attempted, 0);
    assert_eq!(release_svc.grab_call_count().await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_rss_sync_skips_work_with_library_item() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    let indexer = seed_rss_indexer(&db, "TestIndexer", "http://indexer.test").await;
    db.upsert_rss_state(indexer.id, Some("2024-12-01"), "old-guid")
        .await
        .unwrap();

    let work = seed_monitored_work(&db, user_id, "The Way of Kings", "Brandon Sanderson").await;

    // Create a root folder and library item
    let rf = db
        .create_root_folder("/books", MediaType::Ebook)
        .await
        .unwrap();

    db.create_library_item(CreateLibraryItemDbRequest {
        user_id,
        work_id: work.id,
        root_folder_id: rf.id,
        path: "Brandon Sanderson/The Way of Kings.epub".into(),
        media_type: MediaType::Ebook,
        file_size: 500_000,
        import_id: None,
    })
    .await
    .unwrap();

    let feed = rss_xml(&[(
        "The Way of Kings Brandon Sanderson EPUB",
        "guid-new",
        "http://indexer.test/dl/2",
        1_000_000,
    )]);

    let http = StubHttpFetcher::with_ok(200, feed);
    let release_svc = Arc::new(StubReleaseService::succeeding());
    let db_arc = Arc::new(db);

    let workflow = RssSyncWorkflowImpl::new(db_arc.clone(), Arc::new(http), release_svc.clone());

    let report = workflow.run_sync().await.unwrap();

    // Should skip because library item exists
    assert_eq!(report.grabs_attempted, 0);
    assert_eq!(release_svc.grab_call_count().await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_rss_sync_partial_indexer_failure_continues() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    // Two indexers
    let idx1 = seed_rss_indexer(&db, "FailIndexer", "http://fail.test").await;
    let idx2 = seed_rss_indexer(&db, "GoodIndexer", "http://good.test").await;

    // Both have existing state (not first sync)
    db.upsert_rss_state(idx1.id, Some("2024-12-01"), "old1")
        .await
        .unwrap();
    db.upsert_rss_state(idx2.id, Some("2024-12-01"), "old2")
        .await
        .unwrap();

    let _work = seed_monitored_work(&db, user_id, "The Way of Kings", "Brandon Sanderson").await;

    db.create_download_client(CreateDownloadClientDbRequest {
        name: "TestClient".into(),
        implementation: DownloadClientImplementation::QBittorrent,
        host: "localhost".into(),
        port: 8080,
        use_ssl: false,
        skip_ssl_validation: false,
        url_base: None,
        username: None,
        password: None,
        category: "books".into(),
        enabled: true,
        api_key: None,
    })
    .await
    .unwrap();

    let http = StubHttpFetcher::new();
    // First indexer fetch fails
    http.push_response(Ok(FetchResponse {
        status: 500,
        headers: vec![],
        body: b"Internal Error".to_vec(),
    }));
    // Second indexer returns valid feed
    http.push_response(Ok(FetchResponse {
        status: 200,
        headers: vec![],
        body: rss_xml(&[(
            "The Way of Kings Brandon Sanderson EPUB",
            "guid-good",
            "http://good.test/dl/1",
            1_000_000,
        )]),
    }));

    let release_svc = Arc::new(StubReleaseService::succeeding());
    let db_arc = Arc::new(db);

    let workflow = RssSyncWorkflowImpl::new(db_arc.clone(), Arc::new(http), release_svc.clone());

    let report = workflow.run_sync().await.unwrap();

    // One indexer failed, one succeeded
    assert_eq!(report.feeds_checked, 1);
    assert!(!report.warnings.is_empty()); // Should have a warning for failed indexer
    assert_eq!(report.grabs_attempted, 1);
    assert_eq!(report.grabs_succeeded, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_rss_sync_below_threshold_skipped() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    let indexer = seed_rss_indexer(&db, "TestIndexer", "http://indexer.test").await;
    db.upsert_rss_state(indexer.id, Some("2024-12-01"), "old-guid")
        .await
        .unwrap();

    let _work = seed_monitored_work(&db, user_id, "The Way of Kings", "Brandon Sanderson").await;

    // Release title has very low similarity to the work
    let feed = rss_xml(&[(
        "Totally Unrelated Book By Someone Else",
        "guid-nomatch",
        "http://indexer.test/dl/99",
        1_000_000,
    )]);

    let http = StubHttpFetcher::with_ok(200, feed);
    let release_svc = Arc::new(StubReleaseService::succeeding());
    let db_arc = Arc::new(db);

    let workflow = RssSyncWorkflowImpl::new(db_arc.clone(), Arc::new(http), release_svc.clone());

    let report = workflow.run_sync().await.unwrap();

    // No match above threshold
    assert_eq!(report.releases_matched, 0);
    assert_eq!(report.grabs_attempted, 0);
    assert_eq!(release_svc.grab_call_count().await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_rss_sync_creates_notifications() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    let indexer = seed_rss_indexer(&db, "TestIndexer", "http://indexer.test").await;
    db.upsert_rss_state(indexer.id, Some("2024-12-01"), "old-guid")
        .await
        .unwrap();

    let _work = seed_monitored_work(&db, user_id, "The Way of Kings", "Brandon Sanderson").await;

    db.create_download_client(CreateDownloadClientDbRequest {
        name: "TestClient".into(),
        implementation: DownloadClientImplementation::QBittorrent,
        host: "localhost".into(),
        port: 8080,
        use_ssl: false,
        skip_ssl_validation: false,
        url_base: None,
        username: None,
        password: None,
        category: "books".into(),
        enabled: true,
        api_key: None,
    })
    .await
    .unwrap();

    let feed = rss_xml(&[(
        "The Way of Kings Brandon Sanderson EPUB",
        "guid-notif",
        "http://indexer.test/dl/1",
        1_000_000,
    )]);

    // Use a failing release service to test both success and failure notifications
    let http = StubHttpFetcher::with_ok(200, feed);
    let release_svc = Arc::new(StubReleaseService::failing());
    let db_arc = Arc::new(db);

    let workflow = RssSyncWorkflowImpl::new(db_arc.clone(), Arc::new(http), release_svc.clone());

    let report = workflow.run_sync().await.unwrap();

    assert_eq!(report.grabs_attempted, 1);
    assert_eq!(report.grabs_succeeded, 0);

    // Check that a notification was created for the failed grab
    let notifs = db_arc.list_notifications(user_id, false).await.unwrap();
    assert!(!notifs.is_empty());
    assert_eq!(notifs[0].notification_type, NotificationType::RssGrabFailed);

    // Now test success notification path
    let db2 = create_test_db().await;
    let user_id2 = create_test_user(&db2).await;
    let indexer2 = seed_rss_indexer(&db2, "TestIndexer", "http://indexer.test").await;
    db2.upsert_rss_state(indexer2.id, Some("2024-12-01"), "old-guid")
        .await
        .unwrap();
    let _work2 = seed_monitored_work(&db2, user_id2, "The Way of Kings", "Brandon Sanderson").await;

    db2.create_download_client(CreateDownloadClientDbRequest {
        name: "TestClient".into(),
        implementation: DownloadClientImplementation::QBittorrent,
        host: "localhost".into(),
        port: 8080,
        use_ssl: false,
        skip_ssl_validation: false,
        url_base: None,
        username: None,
        password: None,
        category: "books".into(),
        enabled: true,
        api_key: None,
    })
    .await
    .unwrap();

    let feed2 = rss_xml(&[(
        "The Way of Kings Brandon Sanderson EPUB",
        "guid-notif-ok",
        "http://indexer.test/dl/1",
        1_000_000,
    )]);

    let http2 = StubHttpFetcher::with_ok(200, feed2);
    let release_svc2 = Arc::new(StubReleaseService::succeeding());
    let db_arc2 = Arc::new(db2);

    let workflow2 =
        RssSyncWorkflowImpl::new(db_arc2.clone(), Arc::new(http2), release_svc2.clone());

    let report2 = workflow2.run_sync().await.unwrap();

    assert_eq!(report2.grabs_succeeded, 1);

    let notifs2 = db_arc2.list_notifications(user_id2, false).await.unwrap();
    assert!(!notifs2.is_empty());
    assert_eq!(notifs2[0].notification_type, NotificationType::RssGrabbed);
}
