#![allow(dead_code, unused_imports)]

//! Behavioral tests for HttpFetcher trait (PRIM-HTTP-001..007).
//! Covers: fn.http_fetcher.fetch, fn.http_fetcher.fetch_ssrf_safe
//! Test obligations: test.fetch.body_cap, test.fetch.anti_bot_gate,
//!   test.ssrf.private_ip, test.ssrf.cross_domain_redirect, test.ssrf.credentials_in_url

use std::time::Duration;

use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use livrarr_domain::services::*;
use livrarr_http::fetcher::HttpFetcherImpl;
use tokio::net::TcpListener;

/// Spin up an axum server on a random port, returning `http://127.0.0.1:{port}`.
async fn spawn_server(app: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{}", addr.port())
}

fn default_req(url: &str) -> FetchRequest {
    FetchRequest {
        url: url.to_string(),
        method: HttpMethod::Get,
        headers: vec![],
        body: None,
        timeout: Duration::from_secs(5),
        rate_bucket: RateBucket::None,
        max_body_bytes: 1024 * 1024, // 1MB default
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
    }
}

// =============================================================================
// fetch — standard HTTP fetching
// =============================================================================

#[tokio::test]
async fn test_fetch_200_within_body_cap_returns_full_body() {
    // PRIM-HTTP-001, PRIM-HTTP-005: Given a 200 response with 5KB body and max_body_bytes=10KB, returns Ok with full body
    let body_5kb = vec![b'A'; 5 * 1024];
    let body_clone = body_5kb.clone();
    let app = Router::new().route(
        "/ok",
        get(move || {
            let b = body_clone.clone();
            async move { b }
        }),
    );
    let base = spawn_server(app).await;
    let fetcher = HttpFetcherImpl::new().unwrap();

    let mut req = default_req(&format!("{base}/ok"));
    req.max_body_bytes = 10 * 1024;

    let resp = fetcher.fetch(req).await.unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body.len(), 5 * 1024);
    assert_eq!(resp.body, body_5kb);
}

#[tokio::test]
async fn test_fetch_body_exceeding_cap_returns_body_too_large() {
    // PRIM-HTTP-005, test.fetch.body_cap: Given a response exceeding max_body_bytes, returns BodyTooLarge
    let body_20kb = vec![b'B'; 20 * 1024];
    let app = Router::new().route(
        "/big",
        get(move || {
            let b = body_20kb.clone();
            async move { b }
        }),
    );
    let base = spawn_server(app).await;
    let fetcher = HttpFetcherImpl::new().unwrap();

    let mut req = default_req(&format!("{base}/big"));
    req.max_body_bytes = 5 * 1024;

    let err = fetcher.fetch(req).await.unwrap_err();
    assert!(
        matches!(err, FetchError::BodyTooLarge { max_bytes: 5120 }),
        "expected BodyTooLarge, got: {err:?}"
    );
}

#[tokio::test]
async fn test_fetch_429_returns_rate_limited() {
    // PRIM-HTTP-004: Given a 429 response, returns RateLimited
    let app = Router::new().route("/throttle", get(|| async { StatusCode::TOO_MANY_REQUESTS }));
    let base = spawn_server(app).await;
    let fetcher = HttpFetcherImpl::new().unwrap();

    let req = default_req(&format!("{base}/throttle"));
    let err = fetcher.fetch(req).await.unwrap_err();
    assert!(
        matches!(err, FetchError::RateLimited),
        "expected RateLimited, got: {err:?}"
    );
}

#[tokio::test]
async fn test_fetch_html_with_anti_bot_markers_returns_anti_bot_detected() {
    // PRIM-HTTP-006, test.fetch.anti_bot_gate: text/html with anti-bot markers → AntiBotDetected
    let app = Router::new().route(
        "/bot",
        get(|| async {
            (
                [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
                r#"<html><body><div class="cf-browser-verification">checking...</div></body></html>"#,
            )
        }),
    );
    let base = spawn_server(app).await;
    let fetcher = HttpFetcherImpl::new().unwrap();

    let mut req = default_req(&format!("{base}/bot"));
    req.anti_bot_check = true;

    let err = fetcher.fetch(req).await.unwrap_err();
    assert!(
        matches!(err, FetchError::AntiBotDetected),
        "expected AntiBotDetected, got: {err:?}"
    );
}

#[tokio::test]
async fn test_fetch_json_with_anti_bot_markers_returns_ok() {
    // PRIM-HTTP-006, test.fetch.anti_bot_gate: application/json with anti-bot markers → Ok (content-type gate)
    let app = Router::new().route(
        "/json",
        get(|| async {
            (
                [(
                    axum::http::header::CONTENT_TYPE,
                    "application/json; charset=utf-8",
                )],
                r#"{"data": "cf-browser-verification", "challenge-platform": true}"#,
            )
        }),
    );
    let base = spawn_server(app).await;
    let fetcher = HttpFetcherImpl::new().unwrap();

    let mut req = default_req(&format!("{base}/json"));
    req.anti_bot_check = true;

    let resp = fetcher.fetch(req).await.unwrap();
    assert_eq!(resp.status, 200);
}

#[tokio::test]
async fn test_fetch_rate_limiter_blocks_second_request() {
    // PRIM-HTTP-004: Rate limiter blocks second request within rate window
    let app = Router::new().route("/rl", get(|| async { "ok" }));
    let base = spawn_server(app).await;
    let fetcher = HttpFetcherImpl::new().unwrap();

    // Use Goodreads bucket (1s interval)
    let mut req1 = default_req(&format!("{base}/rl"));
    req1.rate_bucket = RateBucket::Goodreads;

    let start = tokio::time::Instant::now();
    let _ = fetcher.fetch(req1).await.unwrap();

    let mut req2 = default_req(&format!("{base}/rl"));
    req2.rate_bucket = RateBucket::Goodreads;
    let _ = fetcher.fetch(req2).await.unwrap();

    let elapsed = start.elapsed();
    // Second request should have been delayed by ~1s
    assert!(
        elapsed >= Duration::from_millis(900),
        "expected >= 900ms gap for rate-limited bucket, got {elapsed:?}"
    );
}

// =============================================================================
// fetch_ssrf_safe — SSRF-protected fetching
// =============================================================================

#[tokio::test]
async fn test_ssrf_rejects_loopback_127() {
    // PRIM-HTTP-003, test.ssrf.private_ip: 127.0.0.1 → Ssrf
    let fetcher = HttpFetcherImpl::new().unwrap();
    let req = default_req("http://127.0.0.1/test");
    let err = fetcher.fetch_ssrf_safe(req).await.unwrap_err();
    assert!(
        matches!(err, FetchError::Ssrf(_)),
        "expected Ssrf, got: {err:?}"
    );
}

#[tokio::test]
async fn test_ssrf_rejects_private_192_168() {
    // PRIM-HTTP-003, test.ssrf.private_ip: 192.168.1.1 → Ssrf
    let fetcher = HttpFetcherImpl::new().unwrap();
    let req = default_req("http://192.168.1.1/test");
    let err = fetcher.fetch_ssrf_safe(req).await.unwrap_err();
    assert!(
        matches!(err, FetchError::Ssrf(_)),
        "expected Ssrf, got: {err:?}"
    );
}

#[tokio::test]
async fn test_ssrf_rejects_ipv6_loopback() {
    // PRIM-HTTP-003, test.ssrf.private_ip: ::1 → Ssrf
    let fetcher = HttpFetcherImpl::new().unwrap();
    let req = default_req("http://[::1]/test");
    let err = fetcher.fetch_ssrf_safe(req).await.unwrap_err();
    assert!(
        matches!(err, FetchError::Ssrf(_)),
        "expected Ssrf, got: {err:?}"
    );
}

#[tokio::test]
async fn test_ssrf_rejects_embedded_credentials() {
    // PRIM-HTTP-003, test.ssrf.credentials_in_url: user:pass@host → Ssrf
    let fetcher = HttpFetcherImpl::new().unwrap();
    let req = default_req("http://user:pass@example.com/path");
    let err = fetcher.fetch_ssrf_safe(req).await.unwrap_err();
    assert!(
        matches!(err, FetchError::Ssrf(_)),
        "expected Ssrf, got: {err:?}"
    );
}

#[tokio::test]
async fn test_ssrf_rejects_non_http_schemes() {
    // PRIM-HTTP-003: file://, ftp://, gopher:// schemes must be rejected
    let fetcher = HttpFetcherImpl::new().unwrap();

    for url in &[
        "file:///etc/passwd",
        "ftp://ftp.example.com/file",
        "gopher://gopher.example.com/",
    ] {
        let req = default_req(url);
        let err = fetcher.fetch_ssrf_safe(req).await.unwrap_err();
        assert!(
            matches!(err, FetchError::Ssrf(_)),
            "expected Ssrf for {url}, got: {err:?}"
        );
    }
}

#[tokio::test]
#[ignore = "requires VPS tunnel: ssh -L 19876:127.0.0.1:19876 ktown"]
async fn test_ssrf_rejects_cross_domain_redirect() {
    // PRIM-HTTP-003, test.ssrf.cross_domain_redirect: redirect to different domain → Ssrf
    // VPS test server at ktown:19876 returns 302 to example.com
    let fetcher = HttpFetcherImpl::new().unwrap();
    let req = default_req("http://45.32.72.229:19876/redirect-cross");
    let err = fetcher.fetch_ssrf_safe(req).await.unwrap_err();
    assert!(
        matches!(err, FetchError::Ssrf(_)),
        "expected Ssrf for cross-domain redirect, got: {err:?}"
    );
}

#[tokio::test]
#[ignore = "requires VPS tunnel: ssh -L 19876:127.0.0.1:19876 ktown"]
async fn test_ssrf_allows_same_domain_redirect() {
    // PRIM-HTTP-003: same-domain redirect should be followed successfully
    // VPS test server at ktown:19876 returns 302 to /destination on same host
    let fetcher = HttpFetcherImpl::new().unwrap();
    let req = default_req("http://45.32.72.229:19876/redirect-same");
    let resp = fetcher.fetch_ssrf_safe(req).await.unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, b"arrived");
}

#[tokio::test]
#[ignore = "requires VPS tunnel: ssh -L 19876:127.0.0.1:19876 ktown"]
async fn test_ssrf_allows_valid_public_url() {
    // PRIM-HTTP-003: valid public URL → Ok
    // VPS test server at ktown:19876 returns 200 "arrived"
    let fetcher = HttpFetcherImpl::new().unwrap();
    let req = default_req("http://45.32.72.229:19876/destination");
    let resp = fetcher.fetch_ssrf_safe(req).await.unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, b"arrived");
}

#[tokio::test]
#[ignore = "requires tracing test subscriber setup"]
async fn test_fetch_redacts_auth_headers_in_logs() {
    // PRIM-HTTP-003: must not log Authorization or X-Api-Key header values
    // Would need tracing-test or tracing-subscriber with in-memory layer.
}
