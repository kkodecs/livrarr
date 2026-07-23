//! Goodreads HTTP transport: fetching search/autocomplete/detail pages
//! through the shared outbound queue, plus the SSRF URL-allowlist guards
//! that gate what the client is allowed to fetch or accept.

use livrarr_domain::services::{
    FetchError, FetchRequest, HttpFetcher, HttpMethod, RateBucket, UserAgentProfile,
};
use livrarr_domain::RequestPriority;
use livrarr_http::breaker::BreakerSignal;
use livrarr_http::outbound_queue;
use std::time::Duration;

use super::parsers::{
    parse_autocomplete_json, parse_detail_html, GoodreadsDetailResult, GoodreadsSearchResult,
};

// =============================================================================
// HTTP fetcher
// =============================================================================

/// Production base URL for Goodreads. Tests pass a local TcpListener URL instead.
pub const GOODREADS_BASE_URL: &str = "https://www.goodreads.com";

/// Browser-like UA — Goodreads serves a stripped page (or anti-bot challenge)
/// without it.
pub const GOODREADS_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// Failure modes from a Goodreads HTTP fetch. Callers map these onto
/// `ProviderOutcome` (queue path) or per-call retry / fallback decisions
/// (identity and series surfaces).
#[derive(Debug, Clone)]
pub enum GoodreadsFetchError {
    /// Response body matched the anti-bot indicator heuristic.
    AntiBot,
    /// HTTP status was non-success. Caller can discriminate 429/5xx vs 4xx.
    HttpStatus(u16),
    /// Transport / DNS / body-read error from `reqwest`.
    Network(String),
    /// Detail page returned 200 OK but no JSON-LD or regex fields parsed out.
    Parse,
    /// The outbound queue's breaker was Open for the Goodreads bucket — no
    /// HTTP was attempted (R-11: the caller must map this to
    /// `WillRetryReason::CircuitOpen`, never burn retry budget on it).
    CircuitOpen(Duration),
    /// The outbound queue's local admission cap rejected the request — no
    /// HTTP was attempted (D3: the caller must map this to
    /// `WillRetryReason::QueueFull`, never burn retry budget on it either).
    QueueFull(Duration),
}

/// Build the canonical detail URL for a `gr_key` against the configured base.
///
/// `gr_key` is the bare identifier (e.g. `"123"` or `"123.Title_Slug"`) — the
/// part after `/book/show/`.
pub fn detail_url_for_gr_key(base_url: &str, gr_key: &str) -> String {
    format!(
        "{}/book/show/{}",
        base_url.trim_end_matches('/'),
        gr_key.trim_start_matches('/'),
    )
}

/// Resolve a (possibly relative) detail URL from `parse_search_html` against
/// the configured base. Production: `base = https://www.goodreads.com`.
/// Tests: `base = http://127.0.0.1:NNNN` (the TcpListener URL).
pub fn resolve_detail_url(base_url: &str, detail_url: &str) -> String {
    if detail_url.starts_with("http://") || detail_url.starts_with("https://") {
        detail_url.to_string()
    } else {
        format!(
            "{}/{}",
            base_url.trim_end_matches('/'),
            detail_url.trim_start_matches('/'),
        )
    }
}

/// Extract the `gr_key` (the `123.Title_Slug` segment) from a detail URL.
/// Returns None if the URL doesn't follow the `/book/show/{key}` shape.
pub fn extract_gr_key(detail_url: &str) -> Option<String> {
    let after = detail_url.split("/book/show/").nth(1)?;
    let key = after.split(['?', '#', '/']).next()?;
    if key.is_empty() {
        None
    } else {
        Some(key.to_string())
    }
}

/// Map an `HttpFetcher` transport failure onto `GoodreadsFetchError`. The
/// fetcher intercepts HTTP 429 at the transport level (`FetchError::RateLimited`)
/// rather than surfacing it as a normal response status, so it is translated
/// back to `HttpStatus(429)` here — preserving the existing 429-vs-other-error
/// discrimination (`map_fetch_err` treats `HttpStatus(429)` as `RateLimit`, any
/// other `Network` failure as `ServerError`).
fn map_transport_err(context: &str, err: FetchError) -> GoodreadsFetchError {
    match err {
        FetchError::RateLimited => GoodreadsFetchError::HttpStatus(429),
        FetchError::CircuitOpen { retry_after } => GoodreadsFetchError::CircuitOpen(retry_after),
        FetchError::QueueFull { retry_after } => GoodreadsFetchError::QueueFull(retry_after),
        other => GoodreadsFetchError::Network(format!("{context}: {other}")),
    }
}

/// Which underlying `HttpFetcher` method carries a `fetch_goodreads_html*`
/// request. See the two public entry points below for when each applies.
enum GrFetchMode {
    /// The URL was built entirely by trusted code (a fixed base + an
    /// already-normalized, persisted anchor) — e.g. the established-gr_key
    /// detail fetch. Not runtime-derived from scraped content.
    Unrestricted,
    /// The URL was built from Goodreads' own scraped page/JSON content
    /// (untrusted input, Unit B4) — e.g. a search-hit's `bookUrl` field.
    /// `fetch_ssrf_safe` validates every redirect hop, not just the initial
    /// URL, so a valid-looking GR URL whose *response* redirects to a
    /// loopback/private address is still rejected.
    SsrfSafe,
}

/// Fetch a Goodreads HTML page. Adds the Chrome UA and treats non-success
/// status and anti-bot challenge pages as errors.
///
/// Used by the queue path (`GoodreadsClient` in `provider_client`). Pacing is
/// the outbound queue's responsibility, per the per-provider `RateBucket`.
pub async fn fetch_goodreads_html<F: HttpFetcher>(
    fetcher: &F,
    url: &str,
    priority: RequestPriority,
) -> Result<String, GoodreadsFetchError> {
    fetch_goodreads_html_via(fetcher, url, priority, GrFetchMode::Unrestricted).await
}

/// Same as [`fetch_goodreads_html`], but routes through `fetch_ssrf_safe`
/// (Unit B4) — for the search-tier resolution path, where `url` was built
/// from Goodreads' own scraped `bookUrl` field. `validate_detail_url` guards
/// the raw value before it ever reaches here; this guards the fetch itself
/// against a valid-looking URL whose response redirects somewhere unsafe.
pub async fn fetch_goodreads_html_ssrf_safe<F: HttpFetcher>(
    fetcher: &F,
    url: &str,
    priority: RequestPriority,
) -> Result<String, GoodreadsFetchError> {
    fetch_goodreads_html_via(fetcher, url, priority, GrFetchMode::SsrfSafe).await
}

async fn fetch_goodreads_html_via<F: HttpFetcher>(
    fetcher: &F,
    url: &str,
    priority: RequestPriority,
    mode: GrFetchMode,
) -> Result<String, GoodreadsFetchError> {
    let req = FetchRequest {
        url: url.to_string(),
        method: HttpMethod::Get,
        headers: vec![("Accept-Language".to_string(), "en-US,en;q=0.9".to_string())],
        body: None,
        timeout: Duration::from_secs(30),
        rate_bucket: RateBucket::Goodreads,
        max_body_bytes: 5 * 1024 * 1024,
        // The fetcher's marker scan is a different, Cloudflare-flavored check —
        // the GR-specific `is_anti_bot_page` body check below owns this instead.
        anti_bot_check: false,
        user_agent: UserAgentProfile::Custom(GOODREADS_USER_AGENT.to_string()),
        priority,
    };
    let resp = match mode {
        GrFetchMode::Unrestricted => fetcher.fetch(req).await,
        GrFetchMode::SsrfSafe => fetcher.fetch_ssrf_safe(req).await,
    }
    .map_err(|e| map_transport_err("GR request", e))?;
    if !(200..300).contains(&resp.status) {
        if (500..600).contains(&resp.status) {
            outbound_queue::shared().report_outcome(RateBucket::Goodreads, BreakerSignal::Failure);
        }
        return Err(GoodreadsFetchError::HttpStatus(resp.status));
    }
    let html = String::from_utf8_lossy(&resp.body).into_owned();
    if crate::provider_util::is_anti_bot_page(&html) {
        // A 200-but-soft-blocked interstitial is a hard block on the
        // breaker, not a threshold-counted failure (R-8).
        outbound_queue::shared().report_outcome(
            RateBucket::Goodreads,
            BreakerSignal::TripImmediately { open_for: None },
        );
        return Err(GoodreadsFetchError::AntiBot);
    }
    outbound_queue::shared().report_outcome(RateBucket::Goodreads, BreakerSignal::Success);
    Ok(html)
}

/// Search Goodreads by TITLE ONLY via the WAF-free autocomplete JSON endpoint.
///
/// The author deliberately stays OUT of the query string: autocomplete
/// prefix-matches the raw string, so "Ender's Game Orson Scott Card" ranks
/// study guides whose TITLES contain "by Orson Scott Card" above the real
/// record — famous books drown in such parasites and the real book drops out
/// of the hit list entirely (38/135 imports, live 2026-07-03). Author
/// agreement is the PICKER's job (`gr_best_match` requires author-token
/// overlap against each hit's own author field), which is both stricter and
/// immune to that ranking poison.
pub async fn search_goodreads<F: HttpFetcher>(
    fetcher: &F,
    base_url: &str,
    title: &str,
    priority: RequestPriority,
) -> Result<Vec<GoodreadsSearchResult>, GoodreadsFetchError> {
    let base = base_url.trim_end_matches('/');
    let query = urlencoding::encode(title);
    let url = format!("{base}/book/auto_complete?format=json&q={query}");
    let req = FetchRequest {
        url,
        method: HttpMethod::Get,
        headers: vec![("Accept-Language".to_string(), "en-US,en;q=0.9".to_string())],
        body: None,
        timeout: Duration::from_secs(30),
        rate_bucket: RateBucket::Goodreads,
        max_body_bytes: 2 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Custom(GOODREADS_USER_AGENT.to_string()),
        priority,
    };
    let resp = fetcher
        .fetch(req)
        .await
        .map_err(|e| map_transport_err("GR autocomplete", e))?;
    if !(200..300).contains(&resp.status) {
        return Err(GoodreadsFetchError::HttpStatus(resp.status));
    }
    let body = String::from_utf8_lossy(&resp.body).into_owned();
    Ok(parse_autocomplete_json(&body))
}

/// Fetch and parse a Goodreads detail page. Returns `Err(Parse)` if the page
/// loads but yields no useful fields.
pub async fn fetch_goodreads_detail<F: HttpFetcher>(
    fetcher: &F,
    detail_url: &str,
    priority: RequestPriority,
) -> Result<GoodreadsDetailResult, GoodreadsFetchError> {
    let html = fetch_goodreads_html(fetcher, detail_url, priority).await?;
    parse_detail_html(&html).ok_or(GoodreadsFetchError::Parse)
}

// =============================================================================
// URL validation
// =============================================================================

/// Validate that a detail URL points to Goodreads (SSRF protection).
/// Accepts relative paths (`/book/show/...`) and absolute Goodreads URLs.
pub fn validate_detail_url(url: &str) -> bool {
    // Allow relative paths for internal use
    if url.starts_with("/book/show/") {
        return true;
    }

    let parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return false,
    };

    if parsed.scheme() != "https" {
        return false;
    }

    match parsed.host_str() {
        Some(host) => host == "www.goodreads.com" || host == "goodreads.com",
        None => false,
    }
}

/// Validate that a cover URL is from an allowed host (SSRF protection).
/// HTTPS only — all Goodreads/Amazon CDNs serve HTTPS.
pub fn validate_cover_url(url: &str) -> bool {
    const ALLOWED_HOSTS: &[&str] = &[
        "i.gr-assets.com",
        "s.gr-assets.com",
        "m.media-amazon.com",
        "images-na.ssl-images-amazon.com",
        "images.gr-assets.com",
        "compressed.photo.goodreads.com",
    ];

    if let Ok(parsed) = url::Url::parse(url) {
        if parsed.scheme() != "https" {
            return false;
        }
        if let Some(host) = parsed.host_str() {
            return ALLOWED_HOSTS.contains(&host);
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------
    // Door-routing: fetch_goodreads_html / search_goodreads go through the
    // HttpFetcher trait with the Goodreads rate bucket, GET, the exact
    // Chrome UA string via UserAgentProfile::Custom (not a header — the
    // fetcher sets UA from `user_agent`), and the Accept-Language header.
    // anti_bot_check stays false: the app-level `is_anti_bot_page` body scan
    // owns anti-bot detection for Goodreads, not the fetcher's generic scan.
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn fetch_goodreads_html_sends_goodreads_bucket_get_custom_ua_and_accept_language() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            b"<html><body>ok</body></html>".to_vec(),
        );

        let html = fetch_goodreads_html(
            &fetcher,
            "https://www.goodreads.com/book/show/1",
            RequestPriority::Normal,
        )
        .await
        .unwrap();

        assert_eq!(html, "<html><body>ok</body></html>");
        let reqs = fetcher.requests();
        assert_eq!(reqs.len(), 1);
        let req = &reqs[0];
        assert_eq!(req.url, "https://www.goodreads.com/book/show/1");
        assert_eq!(req.rate_bucket, RateBucket::Goodreads);
        assert_eq!(req.method, HttpMethod::Get);
        match &req.user_agent {
            UserAgentProfile::Custom(ua) => assert_eq!(ua.as_str(), GOODREADS_USER_AGENT),
            other => panic!("expected UserAgentProfile::Custom, got {other:?}"),
        }
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Accept-Language" && v == "en-US,en;q=0.9"));
        assert!(!req.headers.iter().any(|(k, _)| k == "User-Agent"));
        assert!(!req.anti_bot_check);
        assert_eq!(req.timeout, Duration::from_secs(30));
    }

    #[tokio::test]
    async fn fetch_goodreads_html_maps_anti_bot_body_to_antibot_error() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            br#"<html><div class="cf-browser-verification">Checking...</div></html>"#.to_vec(),
        );

        let err = fetch_goodreads_html(
            &fetcher,
            "https://www.goodreads.com/book/show/1",
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, GoodreadsFetchError::AntiBot));
    }

    #[tokio::test]
    async fn fetch_goodreads_html_maps_fetcher_rate_limited_to_http_status_429() {
        // The fetcher intercepts HTTP 429 as `FetchError::RateLimited` rather
        // than a normal response — this must still surface as
        // `HttpStatus(429)` so `map_fetch_err` (provider_client.rs) keeps
        // classifying it as `WillRetry { RateLimit }`, not `ServerError`.
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_error(FetchError::RateLimited);

        let err = fetch_goodreads_html(
            &fetcher,
            "https://www.goodreads.com/book/show/1",
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, GoodreadsFetchError::HttpStatus(429)));
    }

    #[tokio::test]
    async fn fetch_goodreads_html_maps_fetcher_queue_full_to_queue_full() {
        // D3/#6: the outbound queue's local admission cap is a transport-
        // level pause — must surface as GoodreadsFetchError::QueueFull, not
        // collapse into the generic Network(_) catch-all (budget-consuming).
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_error(FetchError::QueueFull {
                retry_after: Duration::from_secs(1),
            });

        let err = fetch_goodreads_html(
            &fetcher,
            "https://www.goodreads.com/book/show/1",
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, GoodreadsFetchError::QueueFull(_)));
    }

    #[tokio::test]
    async fn fetch_goodreads_html_maps_http_500_to_http_status() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(500, vec![]);

        let err = fetch_goodreads_html(
            &fetcher,
            "https://www.goodreads.com/book/show/1",
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, GoodreadsFetchError::HttpStatus(500)));
    }

    #[tokio::test]
    async fn search_goodreads_sends_goodreads_bucket_get_autocomplete_url() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(200, b"[]".to_vec());

        let hits = search_goodreads(
            &fetcher,
            "https://www.goodreads.com",
            "Dune",
            RequestPriority::Normal,
        )
        .await
        .unwrap();

        assert!(hits.is_empty());
        let reqs = fetcher.requests();
        assert_eq!(reqs.len(), 1);
        let req = &reqs[0];
        assert_eq!(
            req.url, "https://www.goodreads.com/book/auto_complete?format=json&q=Dune",
            "the query is the TITLE ONLY — an appended author makes autocomplete \
             rank study-guide titles containing the author's name above the real \
             record; author agreement belongs to the picker"
        );
        assert_eq!(req.rate_bucket, RateBucket::Goodreads);
        assert_eq!(req.method, HttpMethod::Get);
        match &req.user_agent {
            UserAgentProfile::Custom(ua) => assert_eq!(ua.as_str(), GOODREADS_USER_AGENT),
            other => panic!("expected UserAgentProfile::Custom, got {other:?}"),
        }
        assert!(!req.anti_bot_check);
        assert_eq!(req.timeout, Duration::from_secs(30));
    }

    #[tokio::test]
    async fn search_goodreads_parses_canned_autocomplete_response() {
        let body = r#"[{"title":"The Hobbit","bookUrl":"/book/show/5907","author":{"name":"J.R.R. Tolkien"}}]"#;
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_ok(200, body.as_bytes().to_vec());

        let hits = search_goodreads(
            &fetcher,
            "https://www.goodreads.com",
            "Hobbit",
            RequestPriority::Normal,
        )
        .await
        .unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "The Hobbit");
    }

    #[tokio::test]
    async fn search_goodreads_maps_fetcher_rate_limited_to_http_status_429() {
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_error(FetchError::RateLimited);

        let err = search_goodreads(
            &fetcher,
            "https://www.goodreads.com",
            "x",
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, GoodreadsFetchError::HttpStatus(429)));
    }

    // =========================================================================
    // URL validation tests
    // =========================================================================

    #[test]
    fn validate_goodreads_detail_urls() {
        assert!(validate_detail_url(
            "https://www.goodreads.com/book/show/2896.Das_Parfum"
        ));
        assert!(validate_detail_url("/book/show/2896.Das_Parfum"));
        assert!(!validate_detail_url("https://evil.com/book/show/123"));
        assert!(!validate_detail_url(
            "https://www.goodreads.com.evil.com/book/show/123"
        ));
        assert!(!validate_detail_url("javascript:alert(1)"));
    }

    #[test]
    fn validate_goodreads_cover_urls() {
        assert!(validate_cover_url(
            "https://i.gr-assets.com/images/S/compressed.photo.goodreads.com/books/123.jpg"
        ));
        assert!(validate_cover_url(
            "https://m.media-amazon.com/images/I/123.jpg"
        ));
        assert!(validate_cover_url(
            "https://images-na.ssl-images-amazon.com/images/I/123.jpg"
        ));
        assert!(!validate_cover_url("https://evil.com/image.jpg"));
        assert!(!validate_cover_url("ftp://i.gr-assets.com/image.jpg"));
        assert!(!validate_cover_url("javascript:alert(1)"));
    }

    #[test]
    fn extract_gr_key_normalizes_to_bare_numeric() {
        // A picked Goodreads result must persist its anchor in the domain canonical
        // form (bare numeric), not the slug — so matching/conflict stay consistent.
        let slug = extract_gr_key("/book/show/5907.The_Hobbit_or_There_and_Back_Again")
            .expect("gr_key from a /book/show url");
        assert_eq!(slug, "5907.The_Hobbit_or_There_and_Back_Again");
        let canonical = livrarr_domain::normalization::normalize_gr_key(&slug).expect("normalizes");
        assert_eq!(canonical, "5907");
    }

    // =========================================================================
    // SSRF validation edge cases
    // =========================================================================

    #[test]
    fn ssrf_detail_url_rejects_non_goodreads() {
        assert!(!validate_detail_url("https://evil.com/book/show/123"));
        assert!(!validate_detail_url("http://localhost/book/show/123"));
        assert!(!validate_detail_url("file:///etc/passwd"));
        assert!(!validate_detail_url("data:text/html,<h1>XSS</h1>"));
    }

    #[test]
    fn ssrf_detail_url_accepts_goodreads() {
        assert!(validate_detail_url(
            "https://www.goodreads.com/book/show/123.Title"
        ));
        assert!(validate_detail_url("/book/show/123.Title"));
    }

    #[test]
    fn ssrf_cover_url_rejects_private_hosts() {
        assert!(!validate_cover_url("https://192.168.1.1/image.jpg"));
        assert!(!validate_cover_url("https://10.0.0.1/image.jpg"));
        assert!(!validate_cover_url("https://localhost/image.jpg"));
        assert!(!validate_cover_url("http://127.0.0.1/image.jpg"));
    }

    #[test]
    fn ssrf_cover_url_rejects_non_http_schemes() {
        assert!(!validate_cover_url("ftp://i.gr-assets.com/image.jpg"));
        assert!(!validate_cover_url("javascript:alert(document.cookie)"));
        assert!(!validate_cover_url("data:image/png;base64,abc"));
    }

    #[test]
    fn ssrf_cover_url_allows_known_cdns() {
        assert!(validate_cover_url(
            "https://i.gr-assets.com/images/S/compressed.photo.goodreads.com/books/123.jpg"
        ));
        assert!(validate_cover_url(
            "https://m.media-amazon.com/images/I/test.jpg"
        ));
        assert!(validate_cover_url(
            "https://images-na.ssl-images-amazon.com/images/I/test.jpg"
        ));
        assert!(validate_cover_url(
            "https://images.gr-assets.com/books/123.jpg"
        ));
    }

    // =========================================================================
    // Unit B4: the search-tier resolution fetch must use fetch_ssrf_safe
    // =========================================================================

    #[tokio::test]
    async fn fetch_goodreads_html_ssrf_safe_routes_through_fetch_ssrf_safe() {
        // The search-tier resolution path's HTML fetch (a URL built from
        // Goodreads' own scraped bookUrl) must go through the per-hop
        // redirect-validated method, not the unrestricted one — this is what
        // catches a valid-looking GR URL whose response redirects to a
        // loopback/private address. Proven with a double that succeeds on
        // `fetch` but rejects `fetch_ssrf_safe`: if this function ever
        // regressed to calling `fetch`, this test would incorrectly observe a
        // successful page fetch instead of the rejection.
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok_but_ssrf_safe_rejects(
            200,
            b"<html><body>ok</body></html>".to_vec(),
        );

        let err = fetch_goodreads_html_ssrf_safe(
            &fetcher,
            "https://www.goodreads.com/book/show/1",
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(
            matches!(err, GoodreadsFetchError::Network(_)),
            "expected the SSRF-safe rejection to surface as a Network fetch error, got {err:?}"
        );
    }

    #[tokio::test]
    async fn fetch_goodreads_html_established_key_path_stays_unrestricted() {
        // The established-gr_key detail fetch (`fetch_goodreads_html`, used
        // by `fetch_detail_by_key`) builds its URL from a fixed base + an
        // already-normalized, persisted anchor — trusted infrastructure, not
        // scraped content — and must keep using the unrestricted method so
        // the existing queue tracer tests (which stand a local TCP listener
        // in for a live GR server) keep working. A double that succeeds on
        // `fetch` but REJECTS `fetch_ssrf_safe` proves this call still
        // reaches the unrestricted path: if it ever regressed to calling
        // `fetch_ssrf_safe`, this test would incorrectly observe a rejection.
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok_but_ssrf_safe_rejects(
            200,
            b"<html><body>ok</body></html>".to_vec(),
        );

        let html = fetch_goodreads_html(
            &fetcher,
            "https://www.goodreads.com/book/show/1",
            RequestPriority::Normal,
        )
        .await
        .expect("the established-key path must keep using the unrestricted fetch method");

        assert_eq!(html, "<html><body>ok</body></html>");
    }
}
