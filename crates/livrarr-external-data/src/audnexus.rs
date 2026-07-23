//! Audnexus REST client, consumed via `ProviderClient::Audnexus` (queue
//! dispatch and the identity-resolution fan-out).

use livrarr_domain::services::{
    FetchError, FetchRequest, HttpFetcher, HttpMethod, RateBucket, UserAgentProfile,
};
use livrarr_domain::RequestPriority;
use livrarr_http::breaker::BreakerSignal;
use livrarr_http::outbound_queue;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use crate::types::ProviderFetchError;

/// Classify a non-2xx, non-404/410 Audnexus response status (Unit A). The
/// single place this decision is made, mirroring
/// `openlibrary::classify_ol_error`, so `cached_fetch`'s one HTTP call site
/// stays the sole authority both `query_audnexus`/`query_audnexus_by_asin`
/// (and, one layer up, the anchor and seeded `AudnexusClient` surfaces) build
/// on.
fn classify_audnexus_error(status: u16) -> ProviderFetchError {
    match status {
        429 => ProviderFetchError::RateLimited,
        500..=599 => ProviderFetchError::Transient,
        _ => ProviderFetchError::Other(format!("HTTP {status}")),
    }
}

const CACHE_CAP: usize = 512;

struct CachedResponse {
    last_modified: String,
    body: serde_json::Value,
}

#[derive(Clone)]
pub struct AudnexusCache(Arc<Mutex<lru::LruCache<String, CachedResponse>>>);

impl Default for AudnexusCache {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(lru::LruCache::new(
            NonZeroUsize::new(CACHE_CAP).unwrap(),
        ))))
    }
}

impl AudnexusCache {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Parsed subset of the Audnexus book detail response — narrators, runtime,
/// ASIN, and cover URL.
#[derive(Debug, Clone)]
pub struct AudnexusResult {
    pub narrators: Vec<String>,
    pub narrators_empty: bool,
    pub duration_seconds: Option<i32>,
    pub asin: Option<String>,
    pub cover_url: Option<String>,
}

/// Query Audnexus, preferring lookup by ASIN and falling back to title+author search.
///
/// Returns `Ok(Some(_))` on a parseable hit, `Ok(None)` if no match, `Err(_)` on
/// transport or parse errors. The error string is opaque — callers that need
/// failure-class discrimination (timeout vs 5xx vs DNS) should inspect the
/// underlying `reqwest::Error` themselves.
pub async fn query_audnexus<F: HttpFetcher>(
    fetcher: &F,
    base_url: &str,
    asin: Option<&str>,
    title: &str,
    author: &str,
    cache: &AudnexusCache,
    priority: RequestPriority,
) -> Result<Option<AudnexusResult>, ProviderFetchError> {
    let base = base_url.trim_end_matches('/');

    // Try by ASIN first.
    if let Some(asin) = asin {
        if let Some(result) =
            query_audnexus_by_asin(fetcher, base_url, asin, cache, priority).await?
        {
            return Ok(Some(result));
        }
    }

    // Fallback: search by title + author.
    let url = format!(
        "{base}/books?title={}&author={}",
        urlencoding(title),
        urlencoding(author),
    );
    if let Some(result) = cached_fetch(fetcher, &url, cache, priority).await? {
        let book = if result.is_array() {
            result.as_array().and_then(|a| a.first()).cloned()
        } else {
            Some(result)
        };
        return match book {
            Some(b) => Ok(Some(parse_audnexus(&b, None))),
            None => Ok(None),
        };
    }

    Ok(None)
}

/// ASIN-only Audnexus lookup — the anchor tier of [`query_audnexus`], exposed
/// separately for the anchor-grounded enrichment surface (REQ-006): no
/// title/author fallback exists on this path.
pub async fn query_audnexus_by_asin<F: HttpFetcher>(
    fetcher: &F,
    base_url: &str,
    asin: &str,
    cache: &AudnexusCache,
    priority: RequestPriority,
) -> Result<Option<AudnexusResult>, ProviderFetchError> {
    let base = base_url.trim_end_matches('/');
    let url = format!("{base}/books/{asin}");
    match cached_fetch(fetcher, &url, cache, priority).await? {
        Some(result) => Ok(Some(parse_audnexus(&result, Some(asin)))),
        None => Ok(None),
    }
}

/// Fetch `url` with `If-Modified-Since` conditional-request caching (R-13):
/// a prior response's `Last-Modified` header is replayed on the next request
/// for the same URL; a `304` response is served from the cache without
/// re-parsing the (empty) body. A genuine "not found" (HTTP 404/410) is a
/// soft `Ok(None)` miss. A live 429 or 5xx is retryable, not a permanent miss
/// (Unit A): both surface as a typed `Err` — `RateLimited` / `Transient` —
/// including a fetcher-intercepted HTTP 429 (`FetchError::RateLimited`),
/// which the pre-Unit-A code folded into the same "no result" `Ok(None)`.
async fn cached_fetch<F: HttpFetcher>(
    fetcher: &F,
    url: &str,
    cache: &AudnexusCache,
    priority: RequestPriority,
) -> Result<Option<serde_json::Value>, ProviderFetchError> {
    let cached_last_modified = {
        let mut guard = cache.0.lock().await;
        guard.get(url).map(|c| c.last_modified.clone())
    };

    let mut headers = Vec::new();
    if let Some(ref lm) = cached_last_modified {
        headers.push(("If-Modified-Since".to_string(), lm.clone()));
    }

    let req = FetchRequest {
        url: url.to_string(),
        method: HttpMethod::Get,
        headers,
        body: None,
        timeout: Duration::from_secs(30),
        rate_bucket: RateBucket::Audnexus,
        max_body_bytes: 2 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
        priority,
    };

    let resp = match fetcher.fetch(req).await {
        Ok(r) => r,
        Err(FetchError::RateLimited) => return Err(ProviderFetchError::RateLimited),
        Err(FetchError::CircuitOpen { retry_after }) => {
            return Err(ProviderFetchError::CircuitOpen(retry_after));
        }
        Err(FetchError::QueueFull { retry_after }) => {
            return Err(ProviderFetchError::QueueFull(retry_after));
        }
        // A transport layer that represents an HTTP status as a distinct
        // error (rather than a normal response) still carries a real status
        // — classify it exactly like one.
        Err(FetchError::HttpError { status, .. }) => return Err(classify_audnexus_error(status)),
        Err(e) => {
            tracing::debug!(%url, error = %e, "audnexus fetch: transport failure");
            return Err(ProviderFetchError::Transient);
        }
    };

    if resp.status == 304 {
        let mut guard = cache.0.lock().await;
        if let Some(cached) = guard.get(url) {
            tracing::debug!(%url, "audnexus 304 — reusing cached response");
            outbound_queue::shared().report_outcome(RateBucket::Audnexus, BreakerSignal::Success);
            return Ok(Some(cached.body.clone()));
        }
    }

    if !(200..300).contains(&resp.status) {
        if resp.status == 404 || resp.status == 410 {
            return Ok(None);
        }
        if (500..600).contains(&resp.status) {
            outbound_queue::shared().report_outcome(RateBucket::Audnexus, BreakerSignal::Failure);
        }
        return Err(classify_audnexus_error(resp.status));
    }

    // HTTP header names are case-insensitive; the fetcher preserves whatever
    // casing the server sent, so match case-insensitively (reqwest's
    // `HeaderMap::get` — used by the pre-fetcher code — does the same).
    let last_modified = resp
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("last-modified"))
        .map(|(_, v)| v.clone());

    let data: serde_json::Value = serde_json::from_slice(&resp.body)
        .map_err(|e| ProviderFetchError::Other(format!("parse: {e}")))?;

    if let Some(lm) = last_modified {
        let mut guard = cache.0.lock().await;
        guard.put(
            url.to_string(),
            CachedResponse {
                last_modified: lm,
                body: data.clone(),
            },
        );
    }

    outbound_queue::shared().report_outcome(RateBucket::Audnexus, BreakerSignal::Success);
    Ok(Some(data))
}

/// Parse a single Audnexus book JSON object into `AudnexusResult`.
pub fn parse_audnexus(data: &serde_json::Value, asin_hint: Option<&str>) -> AudnexusResult {
    let narrators = data
        .get("narrators")
        .and_then(|n| n.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|n| {
                    n.get("name")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let duration_seconds = data
        .get("runtimeLengthSec")
        .or_else(|| data.get("runtime_length_sec"))
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);

    let asin = data
        .get("asin")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| asin_hint.map(|s| s.to_string()));

    let cover_url = data
        .get("image")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let narrators_empty = narrators.is_empty();

    AudnexusResult {
        narrators,
        narrators_empty,
        duration_seconds,
        asin,
        cover_url,
    }
}

/// Minimal query-string encoder: escapes only the five characters its
/// callers exercise. Switching to the `urlencoding` crate is a deliberate
/// non-change — it would alter encoding semantics.
fn urlencoding(s: &str) -> String {
    s.replace(' ', "+")
        .replace('&', "%26")
        .replace('=', "%3D")
        .replace('?', "%3F")
        .replace('#', "%23")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_audnexus_extracts_narrators_and_runtime() {
        let json = serde_json::json!({
            "asin": "B07ABCDEFG",
            "narrators": [{"name": "Jane Smith"}, {"name": "John Doe"}],
            "runtimeLengthSec": 36000,
        });
        let result = parse_audnexus(&json, None);
        assert_eq!(result.asin.as_deref(), Some("B07ABCDEFG"));
        assert_eq!(result.narrators, vec!["Jane Smith", "John Doe"]);
        assert_eq!(result.duration_seconds, Some(36000));
        assert!(!result.narrators_empty);
    }

    #[test]
    fn parse_audnexus_falls_back_to_asin_hint_when_response_omits_asin() {
        let json = serde_json::json!({"narrators": [], "runtime_length_sec": 1800});
        let result = parse_audnexus(&json, Some("B07HINT123"));
        assert_eq!(result.asin.as_deref(), Some("B07HINT123"));
        assert_eq!(result.duration_seconds, Some(1800));
        assert!(result.narrators_empty);
    }

    // -------------------------------------------------------------------
    // Door-routing / 304-cache / error-mapping: query_audnexus_by_asin goes
    // through the HttpFetcher trait with the Audnexus rate bucket and GET,
    // no auth. A `Last-Modified` response header seeds the cache; the next
    // request for the same URL replays it as `If-Modified-Since`, and a 304
    // response is served from the cache without re-parse.
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn query_audnexus_by_asin_sends_audnexus_bucket_get_no_conditional_header_on_first_call()
    {
        let canned = serde_json::json!({"asin": "B07ABCDEFG", "narrators": []});
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok_headers(
            200,
            vec![("Last-Modified".to_string(), "Tue, 01 Jan 2030".to_string())],
            canned.to_string().into_bytes(),
        );
        let cache = AudnexusCache::new();

        let result = query_audnexus_by_asin(
            &fetcher,
            "https://api.audnex.us",
            "B07ABCDEFG",
            &cache,
            RequestPriority::Normal,
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(result.asin.as_deref(), Some("B07ABCDEFG"));
        let reqs = fetcher.requests();
        assert_eq!(reqs.len(), 1);
        let req = &reqs[0];
        assert_eq!(req.url, "https://api.audnex.us/books/B07ABCDEFG");
        assert_eq!(req.rate_bucket, RateBucket::Audnexus);
        assert_eq!(req.method, HttpMethod::Get);
        assert!(matches!(req.user_agent, UserAgentProfile::Server));
        assert!(!req.anti_bot_check);
        assert!(!req.headers.iter().any(|(k, _)| k == "If-Modified-Since"));
    }

    #[tokio::test]
    async fn query_audnexus_by_asin_replays_last_modified_and_serves_304_from_cache() {
        let canned = serde_json::json!({"asin": "B07ABCDEFG", "narrators": [{"name": "N"}]});
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok_headers(
            200,
            vec![("Last-Modified".to_string(), "Tue, 01 Jan 2030".to_string())],
            canned.to_string().into_bytes(),
        );
        // Second call gets a 304 with no body — must be served from cache.
        fetcher.push_response(Ok(livrarr_domain::services::FetchResponse {
            status: 304,
            headers: vec![],
            body: vec![],
        }));
        let cache = AudnexusCache::new();

        let first = query_audnexus_by_asin(
            &fetcher,
            "https://api.audnex.us",
            "B07ABCDEFG",
            &cache,
            RequestPriority::Normal,
        )
        .await
        .unwrap()
        .unwrap();
        let second = query_audnexus_by_asin(
            &fetcher,
            "https://api.audnex.us",
            "B07ABCDEFG",
            &cache,
            RequestPriority::Normal,
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(first.narrators, second.narrators);
        assert_eq!(second.narrators, vec!["N".to_string()]);

        let reqs = fetcher.requests();
        assert_eq!(reqs.len(), 2);
        // The second request replays the cached Last-Modified value.
        assert!(reqs[1]
            .headers
            .iter()
            .any(|(k, v)| k == "If-Modified-Since" && v == "Tue, 01 Jan 2030"));
    }

    #[tokio::test]
    async fn query_audnexus_by_asin_maps_http_404_to_ok_none() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(404, vec![]);
        let cache = AudnexusCache::new();

        let result = query_audnexus_by_asin(
            &fetcher,
            "https://api.audnex.us",
            "B0MISSING",
            &cache,
            RequestPriority::Normal,
        )
        .await
        .unwrap();

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn query_audnexus_by_asin_maps_http_410_to_ok_none() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(410, vec![]);
        let cache = AudnexusCache::new();

        let result = query_audnexus_by_asin(
            &fetcher,
            "https://api.audnex.us",
            "B0GONE",
            &cache,
            RequestPriority::Normal,
        )
        .await
        .unwrap();

        assert!(result.is_none());
    }

    // -------------------------------------------------------------------
    // Unit A: a live 429/5xx must be retryable, not folded into "no result"
    // (the bug: `cached_fetch` used to fold a 429 into `Ok(None)`, which both
    // `AudnexusClient::fetch` and `fetch_by_asin` read as a genuine miss —
    // permanently dropping the book's metadata on a transient rate-limit).
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn query_audnexus_by_asin_maps_fetcher_rate_limited_to_err_rate_limited() {
        // The fetcher intercepts HTTP 429 as a transport-level
        // `FetchError::RateLimited` before a status is ever seen. This must
        // now surface as `Err(RateLimited)` so the caller can schedule a
        // real retry (`WillRetry { RateLimit }`) instead of treating a live
        // rate-limit as a permanent "no result."
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_error(FetchError::RateLimited);
        let cache = AudnexusCache::new();

        let err = query_audnexus_by_asin(
            &fetcher,
            "https://api.audnex.us",
            "B0RATE",
            &cache,
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ProviderFetchError::RateLimited));
    }

    #[tokio::test]
    async fn query_audnexus_by_asin_maps_fetcher_queue_full_to_queue_full() {
        // D3/#6: the outbound queue's local admission cap (no HTTP attempted)
        // must surface as a typed QueueFull, not fall into the generic
        // Transient catch-all (which would silently consume retry budget).
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_error(FetchError::QueueFull {
                retry_after: Duration::from_secs(1),
            });
        let cache = AudnexusCache::new();

        let err = query_audnexus_by_asin(
            &fetcher,
            "https://api.audnex.us",
            "B0QUEUEFULL",
            &cache,
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ProviderFetchError::QueueFull(_)));
    }

    #[tokio::test]
    async fn query_audnexus_by_asin_maps_http_429_status_to_err_rate_limited() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(429, vec![]);
        let cache = AudnexusCache::new();

        let err = query_audnexus_by_asin(
            &fetcher,
            "https://api.audnex.us",
            "B0RATE",
            &cache,
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ProviderFetchError::RateLimited));
    }

    #[tokio::test]
    async fn query_audnexus_by_asin_maps_http_5xx_to_err_transient() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(503, vec![]);
        let cache = AudnexusCache::new();

        let err = query_audnexus_by_asin(
            &fetcher,
            "https://api.audnex.us",
            "B0SERVERERR",
            &cache,
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ProviderFetchError::Transient));
    }

    #[tokio::test]
    async fn query_audnexus_by_asin_maps_http_403_to_err_other() {
        // Audnexus is keyless: a 403 has no credential to fix. The caller
        // (`provider_client::audnexus_error_outcome`) turns this into an
        // explicit `PermanentFailure`, never `NotConfigured`.
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(403, vec![]);
        let cache = AudnexusCache::new();

        let err = query_audnexus_by_asin(
            &fetcher,
            "https://api.audnex.us",
            "B0FORBIDDEN",
            &cache,
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ProviderFetchError::Other(_)));
    }

    #[tokio::test]
    async fn query_audnexus_by_asin_maps_other_4xx_to_err_other() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(400, vec![]);
        let cache = AudnexusCache::new();

        let err = query_audnexus_by_asin(
            &fetcher,
            "https://api.audnex.us",
            "B0BADREQ",
            &cache,
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ProviderFetchError::Other(_)));
    }

    #[test]
    fn classify_audnexus_error_maps_429_to_rate_limited() {
        assert!(matches!(
            classify_audnexus_error(429),
            ProviderFetchError::RateLimited
        ));
    }

    #[test]
    fn classify_audnexus_error_maps_5xx_to_transient() {
        assert!(matches!(
            classify_audnexus_error(500),
            ProviderFetchError::Transient
        ));
        assert!(matches!(
            classify_audnexus_error(503),
            ProviderFetchError::Transient
        ));
    }

    #[test]
    fn classify_audnexus_error_maps_403_and_other_4xx_to_other() {
        assert!(matches!(
            classify_audnexus_error(403),
            ProviderFetchError::Other(_)
        ));
        assert!(matches!(
            classify_audnexus_error(400),
            ProviderFetchError::Other(_)
        ));
    }

    #[tokio::test]
    async fn query_audnexus_by_asin_maps_network_error_to_transient() {
        // Unit A: a connection failure is retryable (Transient), not an
        // opaque permanent failure.
        let fetcher = crate::test_support::RecordingHttpFetcher::with_error(
            FetchError::Connection("refused".to_string()),
        );
        let cache = AudnexusCache::new();

        let err = query_audnexus_by_asin(
            &fetcher,
            "https://api.audnex.us",
            "B0NET",
            &cache,
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ProviderFetchError::Transient));
    }
}
