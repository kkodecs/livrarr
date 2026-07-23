//! Readarr API client for library import (Unit B3 — origin trust boundary).
//!
//! Every request routes through the shared `HttpFetcher` — paced and capped
//! by the process-global outbound queue (`RateBucket::Readarr`), body-size
//! and timeout bounded — and it NEVER follows redirects: any 3xx collapses
//! to the same generic rejection as every other failure. Every rejection
//! this client can produce is the SAME opaque [`ReadarrConnectError`] — the
//! response (and any caller log line built from it) must never surface
//! anything the probed target returned.

use std::time::Duration;

use livrarr_domain::services::{
    FetchError, FetchRequest, HttpMethod, RateBucket, UserAgentProfile,
};
use livrarr_domain::RequestPriority;
use livrarr_http::breaker::BreakerSignal;
use livrarr_http::fetcher::HttpFetcherImpl;
use livrarr_http::outbound_queue;
use serde::Deserialize;

/// Every failure establishing or using a Readarr connection collapses to
/// this ONE opaque error — SSRF/approval rejection, protocol mismatch,
/// network failure, timeout, oversized body, or any non-2xx response.
/// Display is fixed; callers must not interpolate any other detail (status,
/// body, underlying error text) into a user-facing message OR a log line
/// built from it — that is exactly what the probed target could use to
/// fingerprint what's behind it.
#[derive(Debug, Clone, Copy)]
pub struct ReadarrConnectError;

impl std::fmt::Display for ReadarrConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unable to connect to the Readarr instance")
    }
}

impl std::error::Error for ReadarrConnectError {}

impl From<FetchError> for ReadarrConnectError {
    fn from(_: FetchError) -> Self {
        ReadarrConnectError
    }
}

/// Fixed per-request timeout — unchanged from the client's prior behavior.
const READARR_TIMEOUT: Duration = Duration::from_secs(30);

/// Parse and normalize a raw admin/user-supplied Readarr base URL: reject
/// non-http(s) schemes, embedded credentials, a query string, and a
/// fragment. Returns the normalized base (scheme + host[:port] + path, no
/// trailing slash — an explicitly-configured base path, e.g. a reverse-proxy
/// subpath, is preserved so only fixed API suffixes are appended after it)
/// and its bare origin (scheme + host[:port], no path — the
/// `RateBucket::Readarr` key and the approved-origins lookup key,
/// `livrarr_http::normalized_origin`).
pub fn normalize_readarr_base(raw: &str) -> Result<(String, String), ReadarrConnectError> {
    let parsed = reqwest::Url::parse(raw).map_err(|_| ReadarrConnectError)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ReadarrConnectError);
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ReadarrConnectError);
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(ReadarrConnectError);
    }
    let origin = livrarr_http::normalized_origin(raw).ok_or(ReadarrConnectError)?;
    let path = parsed.path().trim_end_matches('/');
    Ok((format!("{origin}{path}"), origin))
}

/// Readarr API client. Construct only from an ALREADY-normalized base+origin
/// (see [`normalize_readarr_base`]) — the trust decision (admin-approved
/// private origin, or SSRF-safe-classified public) is made by the caller
/// (`LiveReadarrImportWorkflow`) before this type exists.
pub struct ReadarrClient {
    base: String,
    origin: String,
    api_key: String,
    fetcher: HttpFetcherImpl,
    max_body_bytes: usize,
    timeout: Duration,
}

impl ReadarrClient {
    pub fn new(base: String, origin: String, api_key: String, fetcher: HttpFetcherImpl) -> Self {
        Self {
            base,
            origin,
            api_key,
            fetcher,
            max_body_bytes: livrarr_http::MAX_RESPONSE_BODY_BYTES,
            timeout: READARR_TIMEOUT,
        }
    }

    /// Test-only: shrink the response-body cap so the cap is cheap to
    /// exercise (mirrors the outbound queue's own test-only seams, e.g.
    /// `set_breaker_config_for_tests`).
    #[cfg(test)]
    pub fn with_max_body_bytes(mut self, max: usize) -> Self {
        self.max_body_bytes = max;
        self
    }

    /// Test-only: shrink the per-request timeout so a hung connection is
    /// cheap to exercise.
    #[cfg(test)]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, ReadarrConnectError> {
        let req = FetchRequest {
            url: format!("{}{}", self.base, path),
            method: HttpMethod::Get,
            headers: vec![("X-Api-Key".to_string(), self.api_key.clone())],
            body: None,
            timeout: self.timeout,
            rate_bucket: RateBucket::Readarr {
                origin: self.origin.clone(),
            },
            max_body_bytes: self.max_body_bytes,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
            priority: RequestPriority::Interactive,
        };
        // No redirects, ever: avoids cross-origin X-Api-Key forwarding and a
        // trusted-initial redirect loop. Any 3xx falls through to the same
        // generic rejection as every other non-2xx status below. Routed via
        // `fetch_readarr` (Unit B3 #3): the SSRF-safe client, so a
        // DNS-rebind between admission and this connection is still caught.
        let resp = self.fetcher.fetch_readarr(req).await?;

        // Unit B3 #17: Readarr IS breaker-tracked (`breaker::breaker_tracked`),
        // but `do_fetch`'s own auto-reporting only covers transport-level
        // failures and, for a completed response, is Indexer-bucket-only —
        // so report the outcome here, mirroring a sibling provider client
        // (e.g. `hardcover.rs`'s `hc_post`). Without this, a HalfOpen breaker
        // never closes: a probe's 200 is never reported as a success.
        let signal = if resp.status == 200 {
            BreakerSignal::Success
        } else {
            BreakerSignal::Failure
        };
        outbound_queue::shared().report_outcome(
            RateBucket::Readarr {
                origin: self.origin.clone(),
            },
            signal,
        );

        if resp.status != 200 {
            return Err(ReadarrConnectError);
        }
        serde_json::from_slice(&resp.body).map_err(|_| ReadarrConnectError)
    }

    /// Protocol check — proves the origin actually speaks Readarr's API.
    /// This is NOT the SSRF trust authority (that decision — admin-approved
    /// private, or SSRF-safe-classified public — already happened in the
    /// caller before this client was constructed); it only proves the
    /// connection is worth using.
    pub async fn verify_protocol(&self) -> Result<(), ReadarrConnectError> {
        let status: RdSystemStatus = self.get("/api/v1/system/status").await?;
        if status.app_name.as_deref() == Some("Readarr") {
            Ok(())
        } else {
            Err(ReadarrConnectError)
        }
    }

    pub async fn root_folders(&self) -> Result<Vec<RdRootFolder>, ReadarrConnectError> {
        self.get("/api/v1/rootfolder").await
    }

    pub async fn authors(&self) -> Result<Vec<RdAuthor>, ReadarrConnectError> {
        self.get("/api/v1/author").await
    }

    pub async fn books(&self) -> Result<Vec<RdBook>, ReadarrConnectError> {
        self.get("/api/v1/book").await
    }

    pub async fn book_files_by_author(
        &self,
        author_id: i64,
    ) -> Result<Vec<RdBookFile>, ReadarrConnectError> {
        self.get(&format!("/api/v1/bookfile?authorId={author_id}"))
            .await
    }
}

// ---------------------------------------------------------------------------
// Response types — lightweight deserialization structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RdSystemStatus {
    app_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdRootFolder {
    pub id: i64,
    pub name: Option<String>,
    pub path: String,
    pub accessible: Option<bool>,
    pub free_space: Option<i64>,
    pub total_space: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdAuthor {
    pub id: i64,
    pub author_name: Option<String>,
    pub sort_name: Option<String>,
    pub foreign_author_id: Option<String>,
    pub overview: Option<String>,
    pub genres: Option<Vec<String>>,
    pub images: Option<Vec<RdImage>>,
    pub monitored: Option<bool>,
    pub added: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdBook {
    pub id: i64,
    pub title: Option<String>,
    pub author_id: i64,
    pub foreign_book_id: Option<String>,
    pub series_title: Option<String>,
    pub overview: Option<String>,
    pub release_date: Option<String>,
    pub page_count: Option<i32>,
    pub genres: Option<Vec<String>>,
    pub ratings: Option<RdRatings>,
    pub images: Option<Vec<RdImage>>,
    pub monitored: Option<bool>,
    pub added: Option<String>,
    pub editions: Option<Vec<RdEdition>>,
}

impl RdBook {
    /// Returns the monitored edition, or the first edition if none is monitored.
    pub fn monitored_edition(&self) -> Option<&RdEdition> {
        let editions = self.editions.as_ref()?;
        editions
            .iter()
            .find(|e| e.monitored.unwrap_or(false))
            .or_else(|| editions.first())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdEdition {
    pub id: i64,
    pub book_id: Option<i64>,
    pub foreign_edition_id: Option<String>,
    pub isbn13: Option<String>,
    pub asin: Option<String>,
    pub title: Option<String>,
    pub language: Option<String>,
    pub overview: Option<String>,
    pub format: Option<String>,
    pub is_ebook: Option<bool>,
    pub publisher: Option<String>,
    pub page_count: Option<i32>,
    pub release_date: Option<String>,
    pub images: Option<Vec<RdImage>>,
    pub monitored: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdBookFile {
    pub id: i64,
    pub author_id: Option<i64>,
    pub book_id: i64,
    pub path: String,
    pub size: i64,
    pub date_added: Option<String>,
    pub quality: Option<RdQuality>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdQuality {
    pub quality: Option<RdQualityInner>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdQualityInner {
    pub id: i32,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdRatings {
    pub votes: Option<i32>,
    pub value: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdImage {
    pub url: Option<String>,
    pub cover_type: Option<String>,
    pub remote_url: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map a Readarr quality ID to a Livrarr MediaType string.
/// Returns None for Unknown(0) — caller should infer from file extension.
pub fn quality_to_media_type(quality_id: i32) -> Option<&'static str> {
    match quality_id {
        1..=4 => Some("ebook"),       // PDF, MOBI, EPUB, AZW3
        10..=13 => Some("audiobook"), // MP3, FLAC, M4B, UnknownAudio
        _ => None,                    // Unknown(0) or unrecognized
    }
}

/// Infer media type from file extension when quality is Unknown.
pub fn media_type_from_extension(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?.to_lowercase();
    match ext.as_str() {
        "epub" | "mobi" | "azw" | "azw3" | "pdf" | "cbz" | "cbr" => Some("ebook"),
        "mp3" | "m4b" | "m4a" | "flac" | "ogg" | "wma" | "aac" => Some("audiobook"),
        _ => None,
    }
}

#[cfg(test)]
mod url_normalization_tests {
    use super::*;

    #[test]
    fn rejects_embedded_credentials() {
        assert!(normalize_readarr_base("http://user:pass@10.0.0.5:8787").is_err());
    }

    #[test]
    fn rejects_query_string() {
        assert!(normalize_readarr_base("http://10.0.0.5:8787/?x=1").is_err());
    }

    #[test]
    fn rejects_fragment() {
        assert!(normalize_readarr_base("http://10.0.0.5:8787/#frag").is_err());
    }

    #[test]
    fn rejects_non_http_scheme() {
        assert!(normalize_readarr_base("ftp://10.0.0.5:8787").is_err());
    }

    #[test]
    fn preserves_explicit_base_path_and_strips_trailing_slash() {
        let (base, origin) = normalize_readarr_base("http://10.0.0.5:8787/readarr/").unwrap();
        assert_eq!(base, "http://10.0.0.5:8787/readarr");
        assert_eq!(origin, "http://10.0.0.5:8787");
    }

    #[test]
    fn bare_origin_has_empty_path() {
        let (base, origin) = normalize_readarr_base("http://10.0.0.5:8787").unwrap();
        assert_eq!(base, "http://10.0.0.5:8787");
        assert_eq!(origin, "http://10.0.0.5:8787");
    }

    #[test]
    fn default_port_is_omitted_from_origin() {
        let (_, origin) = normalize_readarr_base("http://readarr.example.com/x").unwrap();
        assert_eq!(origin, "http://readarr.example.com");
    }
}

/// `ReadarrClient` behavior against a real local HTTP server: body cap,
/// timeout, bucket pacing (never `RateBucket::None`), and end-to-end base-
/// path joining (Part 1 points 2, 3).
#[cfg(test)]
mod client_behavior_tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;
    use tokio::net::TcpListener;

    async fn spawn_server(app: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://127.0.0.1:{}", addr.port())
    }

    fn client_for(base: String) -> ReadarrClient {
        let origin = livrarr_http::normalized_origin(&base).unwrap();
        ReadarrClient::new(
            base,
            origin,
            "key".to_string(),
            HttpFetcherImpl::new().unwrap(),
        )
    }

    #[tokio::test]
    async fn oversized_body_is_rejected_by_the_configured_cap() {
        let big_body = vec![b'A'; 2048];
        let app = Router::new().route(
            "/api/v1/rootfolder",
            get(move || {
                let b = big_body.clone();
                async move { b }
            }),
        );
        let base = spawn_server(app).await;
        let client = client_for(base).with_max_body_bytes(1024);

        assert!(
            client.root_folders().await.is_err(),
            "a body exceeding the configured cap must be rejected"
        );
    }

    #[tokio::test]
    async fn body_within_the_cap_is_accepted() {
        let app = Router::new().route(
            "/api/v1/rootfolder",
            get(|| async { axum::Json(serde_json::json!([])) }),
        );
        let base = spawn_server(app).await;
        let client = client_for(base).with_max_body_bytes(1024);

        assert!(
            client.root_folders().await.is_ok(),
            "a small body under the cap must succeed"
        );
    }

    #[tokio::test]
    async fn a_hung_connection_times_out_and_is_rejected_generically() {
        // Accepts the TCP connection but never writes an HTTP response —
        // the client's short test-injected timeout must still fire.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (_socket, _peer) = listener.accept().await.unwrap();
            // Hold the connection open forever without responding.
            std::future::pending::<()>().await;
        });
        let base = format!("http://127.0.0.1:{}", addr.port());
        let client = client_for(base).with_timeout(Duration::from_millis(150));

        let err = client.root_folders().await.err().unwrap();
        assert_eq!(
            err.to_string(),
            ReadarrConnectError.to_string(),
            "a timeout must render the same generic message as every other rejection"
        );
    }

    #[tokio::test]
    async fn requests_through_the_readarr_bucket_are_paced_not_bypassed() {
        // RateBucket::None would let both dispatch immediately; the Readarr
        // bucket paces same-origin calls (Unit B3 point 3) — proves the
        // client is NOT on RateBucket::None.
        let app = Router::new()
            .route(
                "/api/v1/rootfolder",
                get(|| async { axum::Json(serde_json::json!([])) }),
            )
            .route(
                "/api/v1/author",
                get(|| async { axum::Json(serde_json::json!([])) }),
            );
        let base = spawn_server(app).await;
        let client = client_for(base);

        let start = std::time::Instant::now();
        client.root_folders().await.unwrap();
        client.authors().await.unwrap();
        let elapsed = start.elapsed();

        assert!(
            elapsed >= Duration::from_millis(400),
            "the second call through the same Readarr origin must be paced (~500ms), got {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn base_path_is_preserved_end_to_end_not_just_the_bare_origin() {
        // The Readarr instance lives under a reverse-proxy subpath — every
        // fixed API suffix must land under that SAME preserved base path,
        // not at the bare origin root.
        let app = Router::new().nest(
            "/readarr-base",
            Router::new().route(
                "/api/v1/rootfolder",
                get(|| async {
                    axum::Json(serde_json::json!([{
                        "id": 7, "name": "Books", "path": "/books",
                        "accessible": true, "freeSpace": 1, "totalSpace": 1
                    }]))
                }),
            ),
        );
        let raw_base = spawn_server(app).await;
        let (base, origin) = normalize_readarr_base(&format!("{raw_base}/readarr-base")).unwrap();
        let client = ReadarrClient::new(
            base,
            origin,
            "key".to_string(),
            HttpFetcherImpl::new().unwrap(),
        );

        let folders = client
            .root_folders()
            .await
            .expect("the request must be routed under the preserved base path");
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].id, 7);
    }

    /// Unit B3 #3 (DNS-rebinding): approval only ever inspects the origin
    /// STRING (an admin-approved entry is a straight string match — see
    /// `origin_is_permitted` in `readarr_import_workflow.rs`; a public
    /// origin is resolved once, at admission time). The data-fetching
    /// client itself is the last line of defense against a hostname that
    /// resolves privately by the time of the REAL connection (a rebind
    /// between admission and use). `localhost` is a real, deterministic
    /// stand-in for that rebound hostname — this environment's
    /// `/etc/hosts` maps it only to `127.0.0.1`, exactly where the test
    /// server listens, so a client with no DNS-rebinding protection
    /// connects right through.
    #[tokio::test]
    async fn get_refuses_a_host_that_resolves_to_a_private_address() {
        let app = Router::new().route(
            "/api/v1/rootfolder",
            get(|| async { axum::Json(serde_json::json!([])) }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let base = format!("http://localhost:{port}");
        let client = client_for(base);

        let result = client.root_folders().await;
        assert!(
            result.is_err(),
            "a Readarr origin resolving to a private/loopback address must be refused, not connected"
        );
    }
}

/// Unit B3 #17: `ReadarrClient::get` must report outcomes to the outbound
/// queue's breaker (Readarr IS breaker-tracked — `breaker::breaker_tracked`)
/// so a HalfOpen probe can actually close it, instead of leaving it stuck
/// HalfOpen forever (where any subsequent transport hiccup — already
/// auto-reported as a `Failure` by `do_fetch` for every bucket — re-opens it
/// immediately, bypassing the normal 5-failure threshold).
#[cfg(test)]
mod breaker_report_tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;
    use livrarr_http::breaker::BreakerSignal;
    use livrarr_http::outbound_queue;
    use tokio::net::TcpListener;

    async fn spawn_server(app: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://127.0.0.1:{}", addr.port())
    }

    /// There is no direct circuit-state accessor exposed outside
    /// `livrarr-http` (by design — `OutboundQueue`'s registry is private),
    /// so this proves the close BEHAVIORALLY: trip the breaker Open with an
    /// already-elapsed window so the very next request is admitted as a
    /// HalfOpen probe; feed it a real 200 through `ReadarrClient::get`; then
    /// report one manual `Failure` directly to the SAME bucket, standing in
    /// for a subsequent transport hiccup. A properly-closed breaker absorbs
    /// one failure (threshold 5) and keeps admitting; a breaker stuck at
    /// HalfOpen (the bug) re-opens on ANY single failure, which the next
    /// `acquire` call surfaces as `CircuitOpen`.
    #[tokio::test]
    async fn successful_get_closes_a_half_open_breaker() {
        let app = Router::new().route(
            "/api/v1/rootfolder",
            get(|| async { axum::Json(serde_json::json!([])) }),
        );
        let base = spawn_server(app).await;
        let origin = livrarr_http::normalized_origin(&base).unwrap();
        let bucket = RateBucket::Readarr {
            origin: origin.clone(),
        };
        let client = ReadarrClient::new(
            base,
            origin,
            "key".to_string(),
            HttpFetcherImpl::new().unwrap(),
        );

        // Trip Open with an already-elapsed window: the next admission
        // check transitions it straight to HalfOpen and lets the probe
        // through.
        outbound_queue::shared().report_outcome(
            bucket.clone(),
            BreakerSignal::TripImmediately {
                open_for: Some(Duration::from_secs(0)),
            },
        );

        client
            .root_folders()
            .await
            .expect("the HalfOpen probe request must succeed");

        // Simulate a single subsequent hiccup directly against the same
        // bucket. If the probe above closed the breaker, one failure
        // (threshold 5) must not retrip it.
        outbound_queue::shared().report_outcome(bucket.clone(), BreakerSignal::Failure);

        let admitted = outbound_queue::shared()
            .acquire(bucket, RequestPriority::Interactive)
            .await;
        assert!(
            admitted.is_ok(),
            "a single failure after the probe must not reopen a properly-closed breaker \
             (a breaker stuck at HalfOpen reopens on any single failure)"
        );
    }
}
