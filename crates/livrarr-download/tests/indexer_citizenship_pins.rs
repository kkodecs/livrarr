use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use livrarr_db::*;
use livrarr_domain::services::*;
use livrarr_domain::*;
use livrarr_download::release_service::ReleaseServiceImpl;

#[derive(Clone)]
struct RecordingHttpFetcher {
    requests: Arc<Mutex<Vec<FetchRequest>>>,
    responses: Arc<Mutex<VecDeque<Result<FetchResponse, FetchError>>>>,
}

impl RecordingHttpFetcher {
    fn new(responses: Vec<Result<FetchResponse, FetchError>>) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            responses: Arc::new(Mutex::new(responses.into())),
        }
    }

    fn ok(status: u16, body: impl Into<Vec<u8>>) -> Result<FetchResponse, FetchError> {
        Ok(FetchResponse {
            status,
            headers: Vec::new(),
            body: body.into(),
        })
    }

    fn qbit_auth_ok() -> Result<FetchResponse, FetchError> {
        Ok(FetchResponse {
            status: 200,
            headers: vec![(
                "set-cookie".to_string(),
                "SID=pin-cookie; Path=/".to_string(),
            )],
            body: Vec::new(),
        })
    }

    fn requests(&self) -> std::sync::MutexGuard<'_, Vec<FetchRequest>> {
        self.requests.lock().unwrap()
    }

    fn call_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    fn next_response(&self) -> Result<FetchResponse, FetchError> {
        let mut responses = self.responses.lock().unwrap();
        let response = responses
            .pop_front()
            .unwrap_or_else(|| Self::ok(200, Vec::new()));
        if responses.is_empty() {
            responses.push_back(clone_fetch_result(&response));
        }
        response
    }
}

impl HttpFetcher for RecordingHttpFetcher {
    async fn fetch(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.requests.lock().unwrap().push(req);
        self.next_response()
    }

    async fn fetch_ssrf_safe(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.requests.lock().unwrap().push(req);
        self.next_response()
    }
}

fn clone_fetch_result(r: &Result<FetchResponse, FetchError>) -> Result<FetchResponse, FetchError> {
    match r {
        Ok(resp) => Ok(FetchResponse {
            status: resp.status,
            headers: resp.headers.clone(),
            body: resp.body.clone(),
        }),
        Err(err) => Err(clone_fetch_error(err)),
    }
}

fn clone_fetch_error(err: &FetchError) -> FetchError {
    match err {
        FetchError::Connection(s) => FetchError::Connection(s.clone()),
        FetchError::Timeout(d) => FetchError::Timeout(*d),
        FetchError::BodyTooLarge { max_bytes } => FetchError::BodyTooLarge {
            max_bytes: *max_bytes,
        },
        FetchError::AntiBotDetected => FetchError::AntiBotDetected,
        FetchError::Ssrf(s) => FetchError::Ssrf(s.clone()),
        FetchError::HttpError {
            status,
            classification,
        } => FetchError::HttpError {
            status: *status,
            classification: classification.clone(),
        },
        FetchError::RateLimited => FetchError::RateLimited,
        FetchError::CircuitOpen { retry_after } => FetchError::CircuitOpen {
            retry_after: *retry_after,
        },
    }
}

#[derive(Clone)]
struct StubDb {
    work: Work,
    indexer: Indexer,
    client: DownloadClient,
    grabs: Arc<Mutex<Vec<CreateGrabDbRequest>>>,
}

impl StubDb {
    fn new() -> Self {
        Self {
            work: Work {
                id: 10,
                user_id: 7,
                title: "Pinned Book".to_string(),
                author_name: "Pinned Author".to_string(),
                added_at: Utc::now(),
                ..Default::default()
            },
            indexer: indexer(),
            client: qbit_client(),
            grabs: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

fn indexer() -> Indexer {
    Indexer {
        id: 42,
        name: "Display Name".to_string(),
        protocol: "torznab".to_string(),
        url: "HTTPS://Indexer.Example:8443/base".to_string(),
        api_path: "api".to_string(),
        api_key: Some("pin-api-key".to_string()),
        categories: vec![7000],
        priority: 1,
        enable_automatic_search: true,
        enable_interactive_search: true,
        supports_book_search: true,
        enable_rss: true,
        enabled: true,
        added_at: Utc::now(),
    }
}

fn qbit_client() -> DownloadClient {
    DownloadClient {
        id: 5,
        name: "qBit".to_string(),
        implementation: DownloadClientImplementation::QBittorrent,
        host: "download-client.local".to_string(),
        port: 8080,
        use_ssl: false,
        skip_ssl_validation: false,
        url_base: None,
        username: Some("u".to_string()),
        password: Some("p".to_string()),
        category: "books".to_string(),
        download_dir: None,
        enabled: true,
        api_key: None,
        is_default_for_protocol: true,
    }
}

fn trusted_origins_for(urls: &[&str]) -> Arc<livrarr_http::ssrf::TrustedOrigins> {
    let trusted = Arc::new(livrarr_http::ssrf::TrustedOrigins::new());
    trusted.rebuild(&urls.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    trusted
}

fn service(
    db: StubDb,
    http: RecordingHttpFetcher,
) -> ReleaseServiceImpl<StubDb, RecordingHttpFetcher> {
    ReleaseServiceImpl::new(
        db,
        http,
        trusted_origins_for(&["https://indexer.example:8443/base", "http://127.0.0.1:1"]),
    )
}

fn search_request(cache_only: bool, refresh: bool) -> SearchReleasesRequest {
    SearchReleasesRequest {
        work_id: 10,
        refresh,
        cache_only,
    }
}

fn grab_request(download_url: &str) -> GrabRequest {
    GrabRequest {
        work_id: 10,
        download_url: download_url.to_string(),
        title: "Pinned Release".to_string(),
        indexer: "Display Name".to_string(),
        guid: "pin-guid".to_string(),
        size: 1234,
        protocol: DownloadProtocol::Torrent,
        categories: vec![7000],
        download_client_id: None,
        source: GrabSource::Manual,
    }
}

fn torznab_one_item() -> Vec<u8> {
    br#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:torznab="http://torznab.com/schemas/2015/feed">
  <channel>
    <item>
      <title>Pinned Release</title>
      <guid>pin-guid</guid>
      <link>https://indexer.example:8443/download/pin-guid</link>
      <enclosure url="https://indexer.example:8443/download/pin-guid" length="1234" type="application/x-bittorrent"/>
      <torznab:attr name="seeders" value="9"/>
      <torznab:attr name="category" value="7000"/>
    </item>
  </channel>
</rss>"#
        .to_vec()
}

#[tokio::test]
async fn a1_grab_torrent_file_download_uses_origin_keyed_indexer_bucket() {
    let http = RecordingHttpFetcher::new(vec![
        RecordingHttpFetcher::ok(200, b"d4:infod4:name4:pinee"),
        RecordingHttpFetcher::qbit_auth_ok(),
        RecordingHttpFetcher::ok(200, b"Ok."),
    ]);
    let service = service(StubDb::new(), http.clone());

    let _ = service
        .grab(
            7,
            grab_request("https://indexer.example:8443/download/pin-guid"),
        )
        .await
        .expect("grab should reach qBit with scripted responses");

    let requests = http.requests();
    let indexer_fetch = requests
        .iter()
        .find(|req| req.url == "https://indexer.example:8443/download/pin-guid")
        .expect("grab must fetch the indexer download URL");
    assert_eq!(
        indexer_fetch.rate_bucket,
        RateBucket::Indexer {
            origin: "https://indexer.example:8443".to_string(),
            indexer: Some("42".to_string()),
        }
    );
}

#[tokio::test]
async fn grab_bucket_keys_on_configured_indexer_origin_not_download_host() {
    let http = RecordingHttpFetcher::new(vec![
        RecordingHttpFetcher::ok(200, b"d4:infod4:name4:pinee"),
        RecordingHttpFetcher::qbit_auth_ok(),
        RecordingHttpFetcher::ok(200, b"Ok."),
    ]);
    let mut db = StubDb::new();
    db.indexer.url = "https://tracker.example".to_string();
    let service = service(db, http.clone());

    let _ = service
        .grab(7, grab_request("https://example.com/file.torrent"))
        .await
        .expect("grab should reach qBit with scripted responses");

    let requests = http.requests();
    let torrent_file_fetch = requests
        .iter()
        .find(|req| req.url == "https://example.com/file.torrent")
        .expect("grab must fetch the release download URL");
    assert_eq!(
        torrent_file_fetch.rate_bucket,
        RateBucket::Indexer {
            origin: "https://tracker.example".to_string(),
            indexer: Some("42".to_string()),
        }
    );
}

#[tokio::test]
async fn a2_search_uses_origin_keyed_indexer_bucket_not_display_name() {
    let http = RecordingHttpFetcher::new(vec![RecordingHttpFetcher::ok(200, torznab_one_item())]);
    let service = service(StubDb::new(), http.clone());

    let _ = service
        .search(7, search_request(false, false))
        .await
        .expect("scripted indexer search should parse");

    let requests = http.requests();
    let search_fetch = requests
        .iter()
        .find(|req| req.url.contains("/api?t=search"))
        .expect("search must fetch the indexer API");
    assert_eq!(
        search_fetch.rate_bucket,
        RateBucket::Indexer {
            origin: "https://indexer.example:8443".to_string(),
            indexer: Some("42".to_string()),
        }
    );
}

#[tokio::test]
async fn a3_cache_only_cold_makes_zero_fetches_and_returns_empty_success() {
    let http = RecordingHttpFetcher::new(vec![Err(FetchError::Connection(
        "cache_only must not contact indexers".to_string(),
    ))]);
    let service = service(StubDb::new(), http.clone());

    let response = service
        .search(7, search_request(true, false))
        .await
        .expect("cache_only cold miss should be successful");

    assert_eq!(
        http.call_count(),
        0,
        "cache_only must not perform live HTTP"
    );
    assert!(response.results.is_empty());
    assert_eq!(response.cache_age_seconds, None);
}

#[tokio::test]
async fn a4_default_mode_second_identical_search_uses_warm_cache_and_reports_age() {
    let http = RecordingHttpFetcher::new(vec![RecordingHttpFetcher::ok(200, torznab_one_item())]);
    let service = service(StubDb::new(), http.clone());

    let first = service
        .search(7, search_request(false, false))
        .await
        .expect("first default search should fetch and cache");
    assert_eq!(first.results.len(), 1);
    let calls_after_first = http.call_count();

    let second = service
        .search(7, search_request(false, false))
        .await
        .expect("second default search should be served from cache");

    assert_eq!(
        http.call_count(),
        calls_after_first,
        "second default-mode search should make zero additional fetches"
    );
    assert!(
        second.cache_age_seconds.is_some(),
        "cache hits must report cache_age_seconds"
    );
}

#[tokio::test]
async fn a5_refresh_fetches_even_when_default_mode_cache_is_warm() {
    // Guard pin: this may already be green while every search is live, but protects
    // the post-fix rule that refresh bypasses a warm cache.
    let http = RecordingHttpFetcher::new(vec![RecordingHttpFetcher::ok(200, torznab_one_item())]);
    let service = service(StubDb::new(), http.clone());

    let _ = service
        .search(7, search_request(false, false))
        .await
        .expect("default search should warm cache");
    let calls_after_default = http.call_count();

    let _ = service
        .search(7, search_request(false, true))
        .await
        .expect("refresh search should fetch");

    assert!(
        http.call_count() > calls_after_default,
        "refresh=true must perform live HTTP even with a warm cache"
    );
}

#[tokio::test]
async fn a6_rate_limited_torrent_file_fetch_errors_without_delegating_url_to_qbit() {
    let http = RecordingHttpFetcher::new(vec![
        Err(FetchError::RateLimited),
        RecordingHttpFetcher::qbit_auth_ok(),
        RecordingHttpFetcher::ok(200, b"Ok."),
    ]);
    let service = service(StubDb::new(), http.clone());
    let indexer_url = "http://127.0.0.1:1/download/pin-guid";

    let result = service.grab(7, grab_request(indexer_url)).await;

    assert!(
        result.is_err(),
        "rate-limited indexer fetch must fail the grab instead of falling back"
    );
    let requests = http.requests();
    assert!(
        !requests.iter().any(|req| {
            req.url.ends_with("/api/v2/torrents/add")
                && req
                    .body
                    .as_ref()
                    .is_some_and(|body| String::from_utf8_lossy(body).contains(indexer_url))
        }),
        "qBit add must not receive the indexer download URL after RateLimited"
    );
}

impl IndexerDb for StubDb {
    async fn get_indexer(&self, _id: IndexerId) -> Result<Indexer, DbError> {
        Ok(self.indexer.clone())
    }

    async fn get_indexer_with_credentials(&self, _id: IndexerId) -> Result<Indexer, DbError> {
        Ok(self.indexer.clone())
    }

    async fn list_indexers(&self) -> Result<Vec<Indexer>, DbError> {
        Ok(vec![self.indexer.clone()])
    }

    async fn list_enabled_interactive_indexers(&self) -> Result<Vec<Indexer>, DbError> {
        Ok(vec![self.indexer.clone()])
    }

    async fn create_indexer(&self, _req: CreateIndexerDbRequest) -> Result<Indexer, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn update_indexer(
        &self,
        _id: IndexerId,
        _req: UpdateIndexerDbRequest,
    ) -> Result<Indexer, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn delete_indexer(&self, _id: IndexerId) -> Result<(), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn set_supports_book_search(
        &self,
        _id: IndexerId,
        _supports: bool,
    ) -> Result<(), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn list_enabled_rss_indexers(&self) -> Result<Vec<Indexer>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn get_rss_state(
        &self,
        _indexer_id: IndexerId,
    ) -> Result<Option<IndexerRssState>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn upsert_rss_state(
        &self,
        _indexer_id: IndexerId,
        _last_publish_date: Option<&str>,
        _last_guid: &str,
    ) -> Result<(), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }
}

impl WorkDb for StubDb {
    async fn get_work(&self, _user_id: UserId, _id: WorkId) -> Result<Work, DbError> {
        Ok(self.work.clone())
    }

    async fn list_works(&self, _user_id: UserId) -> Result<Vec<Work>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn list_works_by_author(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
    ) -> Result<Vec<Work>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn list_works_paginated(
        &self,
        _user_id: UserId,
        _page: u32,
        _per_page: u32,
        _sort_by: &str,
        _sort_dir: &str,
        _media_type: Option<MediaType>,
        _language: Option<&str>,
    ) -> Result<(Vec<Work>, i64), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn update_work_enrichment(
        &self,
        _user_id: UserId,
        _id: WorkId,
        _req: UpdateWorkEnrichmentDbRequest,
    ) -> Result<Work, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn update_work_user_fields(
        &self,
        _user_id: UserId,
        _id: WorkId,
        _req: UpdateWorkUserFieldsDbRequest,
    ) -> Result<Work, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn set_cover_manual(
        &self,
        _user_id: UserId,
        _id: WorkId,
        _manual: bool,
    ) -> Result<(), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn set_identity_status(
        &self,
        _user_id: UserId,
        _id: WorkId,
        _status: IdentityStatus,
    ) -> Result<(), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn update_cover_metadata(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _cover_url: Option<&str>,
        _cover_source: &str,
        _cover_trust: CoverTrust,
        _cover_width: i32,
        _cover_height: i32,
    ) -> Result<(), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn update_audiobook_cover_metadata(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _audiobook_cover_url: Option<&str>,
        _audiobook_cover_source: &str,
        _audiobook_cover_trust: CoverTrust,
        _audiobook_cover_width: i32,
        _audiobook_cover_height: i32,
    ) -> Result<(), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn update_cover_dimensions(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _width: i32,
        _height: i32,
    ) -> Result<(), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn update_audiobook_cover_dimensions(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _width: i32,
        _height: i32,
    ) -> Result<(), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn delete_work(&self, _user_id: UserId, _id: WorkId) -> Result<Work, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn merge_works(&self, _req: MergeWorksDbRequest) -> Result<Work, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn set_work_series_id(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _series_id: Option<i64>,
    ) -> Result<(), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn normalize_work_series_fields(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _series_name: &str,
        _series_position: Option<f64>,
    ) -> Result<(), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn list_orphan_series_works_all_users(&self) -> Result<Vec<Work>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn work_exists_by_ol_key(
        &self,
        _user_id: UserId,
        _ol_key: &str,
    ) -> Result<bool, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn list_works_for_enrichment(&self, _user_id: UserId) -> Result<Vec<Work>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn list_works_by_author_ol_keys(
        &self,
        _user_id: UserId,
        _author_ol_key: &str,
    ) -> Result<Vec<String>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn list_work_provider_keys_by_author(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
    ) -> Result<Vec<(Option<String>, Option<String>)>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn find_by_normalized_match(
        &self,
        _user_id: UserId,
        _title: &str,
        _author: &str,
    ) -> Result<Vec<Work>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn find_normalized_match_no_anchor_for_user(
        &self,
        _user_id: UserId,
        _raw_title: &str,
        _raw_author: &str,
    ) -> Result<Option<Work>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn find_works_by_bridge(
        &self,
        _user_id: UserId,
        _isbn_13: Option<&str>,
        _asin: Option<&str>,
    ) -> Result<Vec<Work>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn list_monitored_works_all_users(&self) -> Result<Vec<Work>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn list_work_owners_all_users(&self) -> Result<Vec<(WorkId, UserId)>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn list_identity_pending_works(&self) -> Result<Vec<Work>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn list_stale_unenriched_works(
        &self,
        _older_than: DateTime<Utc>,
    ) -> Result<Vec<Work>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn list_failed_works_without_retry_state(&self) -> Result<Vec<Work>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn apply_enrichment_merge(
        &self,
        _req: ApplyEnrichmentMergeRequest,
    ) -> Result<ApplyMergeOutcome, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn reset_for_manual_refresh(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
    ) -> Result<(), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn list_convergence_due(
        &self,
        _user_id: UserId,
        _now: DateTime<Utc>,
        _threshold: u32,
        _limit: i64,
    ) -> Result<Vec<WorkId>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn set_next_convergence_at(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _at: Option<DateTime<Utc>>,
    ) -> Result<(), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn list_conflict_works(&self, _user_id: UserId) -> Result<Vec<Work>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn get_merge_generation(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
    ) -> Result<i64, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn search_works(
        &self,
        _user_id: UserId,
        _query: &str,
        _page: u32,
        _per_page: u32,
    ) -> Result<(Vec<Work>, i64), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }
}

impl DownloadClientDb for StubDb {
    async fn get_download_client(&self, _id: DownloadClientId) -> Result<DownloadClient, DbError> {
        Ok(self.client.clone())
    }

    async fn get_download_client_with_credentials(
        &self,
        _id: DownloadClientId,
    ) -> Result<DownloadClient, DbError> {
        Ok(self.client.clone())
    }

    async fn list_download_clients(&self) -> Result<Vec<DownloadClient>, DbError> {
        Ok(vec![self.client.clone()])
    }

    async fn create_download_client(
        &self,
        _req: CreateDownloadClientDbRequest,
    ) -> Result<DownloadClient, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn update_download_client(
        &self,
        _id: DownloadClientId,
        _req: UpdateDownloadClientDbRequest,
    ) -> Result<DownloadClient, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn delete_download_client(&self, _id: DownloadClientId) -> Result<(), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn get_default_download_client(
        &self,
        _client_type: &str,
    ) -> Result<Option<DownloadClient>, DbError> {
        Ok(Some(self.client.clone()))
    }
}

impl GrabDb for StubDb {
    async fn get_grab(&self, _user_id: UserId, _id: GrabId) -> Result<Grab, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn list_grabs_by_work(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
    ) -> Result<Vec<Grab>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn list_active_grabs(&self) -> Result<Vec<Grab>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn upsert_grab(&self, req: CreateGrabDbRequest) -> Result<Grab, DbError> {
        self.grabs.lock().unwrap().push(CreateGrabDbRequest {
            user_id: req.user_id,
            work_id: req.work_id,
            download_client_id: req.download_client_id,
            title: req.title.clone(),
            indexer: req.indexer.clone(),
            guid: req.guid.clone(),
            size: req.size,
            download_url: req.download_url.clone(),
            download_id: req.download_id.clone(),
            status: req.status,
            media_type: req.media_type,
        });
        Ok(Grab {
            id: 77,
            user_id: req.user_id,
            work_id: req.work_id,
            download_client_id: req.download_client_id,
            title: req.title,
            indexer: req.indexer,
            guid: req.guid,
            size: req.size,
            download_url: req.download_url,
            download_id: req.download_id,
            status: req.status,
            import_error: None,
            media_type: req.media_type,
            content_path: None,
            grabbed_at: Utc::now(),
            import_retry_count: 0,
            import_failed_at: None,
        })
    }

    async fn update_grab_status(
        &self,
        _user_id: UserId,
        _id: GrabId,
        _status: GrabStatus,
        _import_error: Option<&str>,
    ) -> Result<(), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn update_grab_download_id(
        &self,
        _user_id: UserId,
        _id: GrabId,
        _download_id: &str,
    ) -> Result<(), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn get_grab_by_download_id(&self, _download_id: &str) -> Result<Option<Grab>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn reset_importing_grabs(&self) -> Result<u64, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn set_grab_content_path(
        &self,
        _user_id: UserId,
        _id: GrabId,
        _content_path: &str,
    ) -> Result<(), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn list_grabs_paginated(
        &self,
        _user_id: UserId,
        _page: u32,
        _per_page: u32,
    ) -> Result<(Vec<Grab>, i64), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn try_set_importing(&self, _user_id: UserId, _id: GrabId) -> Result<bool, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn active_grab_exists(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _media_type: MediaType,
    ) -> Result<bool, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn release_already_failed(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _media_type: MediaType,
        _guid: &str,
    ) -> Result<bool, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn recent_failed_grab_count(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _media_type: MediaType,
        _since: DateTime<Utc>,
    ) -> Result<i64, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn list_retriable_grabs(&self, _max_retries: i32) -> Result<Vec<Grab>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn increment_import_retry(&self, _user_id: UserId, _id: GrabId) -> Result<(), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn queue_summary(&self, _user_id: UserId) -> Result<QueueSummary, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }
}

impl HistoryDb for StubDb {
    async fn list_history(
        &self,
        _user_id: UserId,
        _filter: HistoryFilter,
    ) -> Result<Vec<HistoryEvent>, DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn list_history_paginated(
        &self,
        _user_id: UserId,
        _filter: HistoryFilter,
        _page: u32,
        _per_page: u32,
    ) -> Result<(Vec<HistoryEvent>, i64), DbError> {
        unreachable!("not exercised by indexer citizenship pins")
    }

    async fn create_history_event(&self, _req: CreateHistoryEventDbRequest) -> Result<(), DbError> {
        Ok(())
    }
}
