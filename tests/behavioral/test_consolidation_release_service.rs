#![allow(dead_code, unused_imports)]

//! Behavioral tests for ReleaseService trait (SVC-RELEASE-001..004).
//! Covers: fn.release_service.{search, grab}
//! Test obligations: test.release.search.*, test.release.grab.*

use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    CreateDownloadClientDbRequest, CreateIndexerDbRequest, CreateUserDbRequest,
    CreateWorkDbRequest, DownloadClientDb, GrabDb, IndexerDb, UserDb, WorkDb,
};
use livrarr_domain::services::*;
use livrarr_domain::*;
use livrarr_download::release_service::ReleaseServiceImpl;

fn test_trusted_origins() -> std::sync::Arc<livrarr_http::ssrf::TrustedOrigins> {
    let origins = std::sync::Arc::new(livrarr_http::ssrf::TrustedOrigins::new());
    origins.rebuild(&[
        "http://indexer.test".to_string(),
        "http://tracker.example.com".to_string(),
        "http://usenet.example.com".to_string(),
    ]);
    origins
}

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

async fn setup_work(db: &SqliteDb, user_id: i64) -> i64 {
    let work = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "The Great Book".into(),
            author_name: "Jane Author".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    work.id
}

async fn setup_indexer(db: &SqliteDb, name: &str, enabled: bool) -> i64 {
    let indexer = db
        .create_indexer(CreateIndexerDbRequest {
            name: name.into(),
            protocol: "torznab".into(),
            url: format!("http://indexer-{name}.example.com"),
            api_path: "/api".into(),
            api_key: Some("testkey123".into()),
            categories: vec![7020, 3030],
            priority: 1,
            enable_automatic_search: true,
            enable_interactive_search: enabled,
            enable_rss: true,
            enabled,
        })
        .await
        .unwrap();
    indexer.id
}

fn torznab_xml_with_items(items: &[(&str, &str, &str, i32, i64)]) -> Vec<u8> {
    let mut xml = String::from(r#"<?xml version="1.0" encoding="UTF-8"?><rss><channel>"#);
    for (title, guid, download_url, seeders, size) in items {
        xml.push_str(&format!(
            r#"<item><title>{title}</title><guid>{guid}</guid><link>{download_url}</link><size>{size}</size><newznab:attr name="seeders" value="{seeders}"/><newznab:attr name="category" value="7020"/></item>"#,
        ));
    }
    xml.push_str("</channel></rss>");
    xml.into_bytes()
}

fn torznab_xml_single(title: &str, guid: &str, seeders: i32) -> Vec<u8> {
    torznab_xml_with_items(&[(
        title,
        guid,
        "http://dl.example.com/file.torrent",
        seeders,
        1024,
    )])
}

// =============================================================================
// search
// =============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_release_search_partial_indexer_failure_returns_results_plus_warning() {
    // SVC-RELEASE-002: Given 2 indexers with 1 failing, returns results from successful indexer plus warning
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work_id = setup_work(&db, user_id).await;
    setup_indexer(&db, "good", true).await;
    setup_indexer(&db, "bad", true).await;

    // First indexer succeeds, second fails
    let xml = torznab_xml_single("Book Title", "guid-1", 10);
    let http = StubHttpFetcher::with_ok(200, xml);
    http.push_response(Err(FetchError::Connection("connection refused".into())));

    let svc = ReleaseServiceImpl::new(db, http, test_trusted_origins());
    let result = svc
        .search(
            user_id,
            SearchReleasesRequest {
                work_id,
                refresh: false,
                cache_only: false,
            },
        )
        .await;

    let resp = result.expect("search should succeed with partial failure");
    assert!(
        !resp.results.is_empty(),
        "should have results from successful indexer"
    );
    assert!(
        !resp.warnings.is_empty(),
        "should have warning about failed indexer"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_release_search_all_indexers_fail_returns_error() {
    // SVC-RELEASE-002: Given all indexers failing, returns AllIndexersFailed
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work_id = setup_work(&db, user_id).await;
    setup_indexer(&db, "idx1", true).await;
    setup_indexer(&db, "idx2", true).await;

    let http = StubHttpFetcher::with_error(FetchError::Connection("refused".into()));

    let svc = ReleaseServiceImpl::new(db, http, test_trusted_origins());
    let result = svc
        .search(
            user_id,
            SearchReleasesRequest {
                work_id,
                refresh: false,
                cache_only: false,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ReleaseServiceError::AllIndexersFailed)),
        "expected AllIndexersFailed, got {result:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_release_search_dedup_same_indexer() {
    // SVC-RELEASE-002: Given duplicate guid from same indexer, dedupes
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work_id = setup_work(&db, user_id).await;
    setup_indexer(&db, "idx1", true).await;

    // XML with duplicate guid from the same indexer
    let xml = torznab_xml_with_items(&[
        (
            "Book A",
            "same-guid",
            "http://dl.example.com/a.torrent",
            10,
            1024,
        ),
        (
            "Book A copy",
            "same-guid",
            "http://dl.example.com/b.torrent",
            5,
            512,
        ),
        (
            "Book B",
            "different-guid",
            "http://dl.example.com/c.torrent",
            3,
            2048,
        ),
    ]);
    let http = StubHttpFetcher::with_ok(200, xml);

    let svc = ReleaseServiceImpl::new(db, http, test_trusted_origins());
    let result = svc
        .search(
            user_id,
            SearchReleasesRequest {
                work_id,
                refresh: false,
                cache_only: false,
            },
        )
        .await;

    let resp = result.expect("search should succeed");
    // Should have 2 results: one for same-guid (deduped) and one for different-guid
    assert_eq!(resp.results.len(), 2, "duplicate guid should be deduped");
    let guids: Vec<&str> = resp.results.iter().map(|r| r.guid.as_str()).collect();
    assert!(guids.contains(&"same-guid"));
    assert!(guids.contains(&"different-guid"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_release_search_keeps_cross_indexer_duplicates() {
    // SVC-RELEASE-002: Given same guid from different indexers, keeps both
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work_id = setup_work(&db, user_id).await;
    setup_indexer(&db, "indexer-a", true).await;
    setup_indexer(&db, "indexer-b", true).await;

    // Both indexers return an item with the same guid
    let xml_a = torznab_xml_single("Book From A", "shared-guid", 10);
    let xml_b = torznab_xml_single("Book From B", "shared-guid", 5);
    let http = StubHttpFetcher::with_ok(200, xml_a);
    http.push_response(Ok(FetchResponse {
        status: 200,
        headers: vec![],
        body: xml_b,
    }));

    let svc = ReleaseServiceImpl::new(db, http, test_trusted_origins());
    let result = svc
        .search(
            user_id,
            SearchReleasesRequest {
                work_id,
                refresh: false,
                cache_only: false,
            },
        )
        .await;

    let resp = result.expect("search should succeed");
    // Same guid but different indexers — both should be kept
    let shared_guid_count = resp
        .results
        .iter()
        .filter(|r| r.guid == "shared-guid")
        .count();
    assert_eq!(
        shared_guid_count, 2,
        "same guid from different indexers should both be kept"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_release_search_sort_seeders_desc() {
    // SVC-RELEASE-002: Results sorted by seeders desc for torrents
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work_id = setup_work(&db, user_id).await;
    setup_indexer(&db, "idx1", true).await;

    let xml = torznab_xml_with_items(&[
        (
            "Low Seeds",
            "guid-low",
            "http://dl.example.com/low.torrent",
            2,
            1024,
        ),
        (
            "High Seeds",
            "guid-high",
            "http://dl.example.com/high.torrent",
            50,
            1024,
        ),
        (
            "Mid Seeds",
            "guid-mid",
            "http://dl.example.com/mid.torrent",
            10,
            1024,
        ),
        (
            "Tied Seeds Big",
            "guid-tied-big",
            "http://dl.example.com/tied-big.torrent",
            10,
            4096,
        ),
        (
            "Tied Seeds Small",
            "guid-tied-small",
            "http://dl.example.com/tied-small.torrent",
            10,
            512,
        ),
    ]);
    let http = StubHttpFetcher::with_ok(200, xml);

    let svc = ReleaseServiceImpl::new(db, http, test_trusted_origins());
    let result = svc
        .search(
            user_id,
            SearchReleasesRequest {
                work_id,
                refresh: false,
                cache_only: false,
            },
        )
        .await;

    let resp = result.expect("search should succeed");
    let seeders: Vec<i32> = resp
        .results
        .iter()
        .map(|r| r.seeders.unwrap_or(0))
        .collect();
    // Should be: 50, 10, 10, 10, 2
    assert_eq!(seeders[0], 50, "highest seeders first");
    assert_eq!(*seeders.last().unwrap(), 2, "lowest seeders last");

    // Within the tied seeders (10), sort by size desc
    let tied: Vec<i64> = resp
        .results
        .iter()
        .filter(|r| r.seeders == Some(10))
        .map(|r| r.size)
        .collect();
    assert!(
        tied.windows(2).all(|w| w[0] >= w[1]),
        "tied seeders should be sorted by size desc: {tied:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_release_search_skips_items_missing_guid() {
    // SVC-RELEASE-002: Items missing guid are skipped with warning
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work_id = setup_work(&db, user_id).await;
    setup_indexer(&db, "idx1", true).await;

    // XML with one valid item and one missing guid
    let xml = br#"<?xml version="1.0" encoding="UTF-8"?><rss><channel>
        <item><title>Good Book</title><guid>valid-guid</guid><link>http://dl.example.com/good.torrent</link><size>1024</size><newznab:attr name="seeders" value="5"/><newznab:attr name="category" value="7020"/></item>
        <item><title>Bad Book</title><link>http://dl.example.com/bad.torrent</link><size>512</size></item>
    </channel></rss>"#
        .to_vec();
    let http = StubHttpFetcher::with_ok(200, xml);

    let svc = ReleaseServiceImpl::new(db, http, test_trusted_origins());
    let result = svc
        .search(
            user_id,
            SearchReleasesRequest {
                work_id,
                refresh: false,
                cache_only: false,
            },
        )
        .await;

    let resp = result.expect("search should succeed");
    assert_eq!(resp.results.len(), 1, "should only have valid item");
    assert_eq!(resp.results[0].guid, "valid-guid");
    assert!(
        resp.warnings.iter().any(|w| w.contains("missing guid")),
        "should have warning about missing guid: {:?}",
        resp.warnings
    );
}

// =============================================================================
// grab
// =============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_release_grab_happy_path_creates_sent_grab() {
    // SVC-RELEASE-003: Given valid request with available client, creates grab with Sent status
    // This test requires a stub HTTP fetcher that simulates qBit auth + add torrent responses.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work_id = setup_work(&db, user_id).await;

    // Create a qBit download client
    let client = db
        .create_download_client(CreateDownloadClientDbRequest {
            name: "test-qbit".into(),
            implementation: DownloadClientImplementation::QBittorrent,
            host: "qbit.example.com".into(),
            port: 8080,
            use_ssl: false,
            skip_ssl_validation: false,
            url_base: None,
            username: Some("admin".into()),
            password: Some("password".into()),
            category: "livrarr".into(),
            enabled: true,
            api_key: None,
        })
        .await
        .unwrap();

    // Stub: first call = auth OK with SID cookie, second call = add torrent OK
    let http = StubHttpFetcher::with_response(Ok(FetchResponse {
        status: 200,
        headers: vec![("Set-Cookie".into(), "SID=abc123; path=/".into())],
        body: b"Ok.".to_vec(),
    }));
    http.push_response(Ok(FetchResponse {
        status: 200,
        headers: vec![],
        body: b"Ok.".to_vec(),
    }));

    let svc = ReleaseServiceImpl::new(db.clone(), http, test_trusted_origins());
    let result = svc
        .grab(
            user_id,
            GrabRequest {
                work_id,
                download_url: "http://tracker.example.com/file.torrent".into(),
                title: "The Great Book".into(),
                indexer: "test-indexer".into(),
                guid: "grab-guid-1".into(),
                size: 1024,
                protocol: DownloadProtocol::Torrent,
                categories: vec![7020],
                download_client_id: Some(client.id),
                source: GrabSource::Manual,
            },
        )
        .await;

    let grab = result.expect("grab should succeed");
    assert_eq!(grab.status, GrabStatus::Sent);
    assert_eq!(grab.work_id, work_id);
    assert_eq!(grab.user_id, user_id);
    assert_eq!(grab.media_type, Some(MediaType::Ebook));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_release_grab_ssrf_invalid_url_rejected() {
    // SVC-RELEASE-003: Given SSRF-invalid URL, returns Ssrf without contacting client
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work_id = setup_work(&db, user_id).await;

    // No download client needed — SSRF check should reject before client lookup
    let http = StubHttpFetcher::new();

    let svc = ReleaseServiceImpl::new(db, http.clone(), test_trusted_origins());
    let result = svc
        .grab(
            user_id,
            GrabRequest {
                work_id,
                download_url: "http://127.0.0.1:8080/secret".into(),
                title: "SSRF Test".into(),
                indexer: "test-indexer".into(),
                guid: "ssrf-guid".into(),
                size: 1024,
                protocol: DownloadProtocol::Torrent,
                categories: vec![7020],
                download_client_id: None,
                source: GrabSource::Manual,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ReleaseServiceError::Ssrf(_))),
        "expected Ssrf error, got {result:?}"
    );
    assert_eq!(http.call_count(), 0, "HTTP should not have been called");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_release_grab_no_client_for_protocol() {
    // SVC-RELEASE-003: Given no client for protocol, returns NoClient
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work_id = setup_work(&db, user_id).await;

    // No download clients configured at all
    let http = StubHttpFetcher::new();

    let svc = ReleaseServiceImpl::new(db, http, test_trusted_origins());
    let result = svc
        .grab(
            user_id,
            GrabRequest {
                work_id,
                download_url: "http://tracker.example.com/file.torrent".into(),
                title: "No Client Test".into(),
                indexer: "test-indexer".into(),
                guid: "noclient-guid".into(),
                size: 1024,
                protocol: DownloadProtocol::Torrent,
                categories: vec![7020],
                download_client_id: None,
                source: GrabSource::Manual,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ReleaseServiceError::NoClient { .. })),
        "expected NoClient error, got {result:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_release_grab_client_unreachable() {
    // SVC-RELEASE-003: Given client unreachable, returns ClientUnreachable
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work_id = setup_work(&db, user_id).await;

    let _client = db
        .create_download_client(CreateDownloadClientDbRequest {
            name: "unreachable-qbit".into(),
            implementation: DownloadClientImplementation::QBittorrent,
            host: "dead.example.com".into(),
            port: 8080,
            use_ssl: false,
            skip_ssl_validation: false,
            url_base: None,
            username: Some("admin".into()),
            password: Some("password".into()),
            category: "livrarr".into(),
            enabled: true,
            api_key: None,
        })
        .await
        .unwrap();

    // Stub: auth call fails with connection error
    let http = StubHttpFetcher::with_error(FetchError::Connection("connection refused".into()));

    let svc = ReleaseServiceImpl::new(db.clone(), http, test_trusted_origins());
    let result = svc
        .grab(
            user_id,
            GrabRequest {
                work_id,
                download_url: "http://tracker.example.com/file.torrent".into(),
                title: "Unreachable Test".into(),
                indexer: "test-indexer".into(),
                guid: "unreachable-guid".into(),
                size: 1024,
                protocol: DownloadProtocol::Torrent,
                categories: vec![7020],
                download_client_id: Some(_client.id),
                source: GrabSource::Manual,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ReleaseServiceError::ClientUnreachable(_))),
        "expected ClientUnreachable, got {result:?}"
    );

    // Verify no grab record was created
    let grabs = db.list_active_grabs().await.unwrap();
    assert!(
        grabs.is_empty(),
        "no grab should exist after client failure"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_release_grab_client_rejection_leaves_no_db_record() {
    // SVC-RELEASE-003: If download client rejects the add, no grab record exists in DB
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work_id = setup_work(&db, user_id).await;

    let client = db
        .create_download_client(CreateDownloadClientDbRequest {
            name: "reject-qbit".into(),
            implementation: DownloadClientImplementation::QBittorrent,
            host: "reject.example.com".into(),
            port: 8080,
            use_ssl: false,
            skip_ssl_validation: false,
            url_base: None,
            username: Some("admin".into()),
            password: Some("password".into()),
            category: "livrarr".into(),
            enabled: true,
            api_key: None,
        })
        .await
        .unwrap();

    // Stub: auth succeeds, add returns 400 (rejected)
    let http = StubHttpFetcher::with_response(Ok(FetchResponse {
        status: 200,
        headers: vec![("Set-Cookie".into(), "SID=abc123; path=/".into())],
        body: b"Ok.".to_vec(),
    }));
    http.push_response(Ok(FetchResponse {
        status: 400,
        headers: vec![],
        body: b"rejected".to_vec(),
    }));

    let svc = ReleaseServiceImpl::new(db.clone(), http, test_trusted_origins());
    let result = svc
        .grab(
            user_id,
            GrabRequest {
                work_id,
                download_url: "http://tracker.example.com/file.torrent".into(),
                title: "Rejected Test".into(),
                indexer: "test-indexer".into(),
                guid: "rejected-guid".into(),
                size: 1024,
                protocol: DownloadProtocol::Torrent,
                categories: vec![7020],
                download_client_id: Some(client.id),
                source: GrabSource::Manual,
            },
        )
        .await;

    assert!(result.is_err(), "grab should fail on rejection");

    // Verify no grab record was created
    let grabs = db.list_active_grabs().await.unwrap();
    assert!(
        grabs.is_empty(),
        "no grab should exist after client rejection"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_release_grab_category_mapping() {
    // SVC-RELEASE-003: Categories 7020 maps to ebook, 3030 maps to audiobook
    // End-to-end test: grab with audiobook categories, verify media_type on Grab record
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work_id = setup_work(&db, user_id).await;

    let client = db
        .create_download_client(CreateDownloadClientDbRequest {
            name: "test-qbit".into(),
            implementation: DownloadClientImplementation::QBittorrent,
            host: "qbit.example.com".into(),
            port: 8080,
            use_ssl: false,
            skip_ssl_validation: false,
            url_base: None,
            username: Some("admin".into()),
            password: Some("password".into()),
            category: "livrarr".into(),
            enabled: true,
            api_key: None,
        })
        .await
        .unwrap();

    // Stub: auth OK + add OK
    let http = StubHttpFetcher::with_response(Ok(FetchResponse {
        status: 200,
        headers: vec![("Set-Cookie".into(), "SID=abc123; path=/".into())],
        body: b"Ok.".to_vec(),
    }));
    http.push_response(Ok(FetchResponse {
        status: 200,
        headers: vec![],
        body: b"Ok.".to_vec(),
    }));

    let svc = ReleaseServiceImpl::new(db, http, test_trusted_origins());

    // Grab with audiobook category 3030
    let grab = svc
        .grab(
            user_id,
            GrabRequest {
                work_id,
                download_url: "http://tracker.example.com/audiobook.torrent".into(),
                title: "Audiobook Title".into(),
                indexer: "test-indexer".into(),
                guid: "audiobook-guid".into(),
                size: 2048,
                protocol: DownloadProtocol::Torrent,
                categories: vec![3030],
                download_client_id: Some(client.id),
                source: GrabSource::Manual,
            },
        )
        .await
        .expect("grab should succeed");

    assert_eq!(
        grab.media_type,
        Some(MediaType::Audiobook),
        "3030 should map to audiobook via grab()"
    );
}
