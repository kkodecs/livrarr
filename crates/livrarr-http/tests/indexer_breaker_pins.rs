use std::time::Duration;

use livrarr_domain::services::{
    FetchError, FetchRequest, HttpFetcher, HttpMethod, RateBucket, UserAgentProfile,
};
use livrarr_domain::RequestPriority;
use livrarr_http::fetcher::HttpFetcherImpl;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn spawn_status_server(statuses: Vec<u16>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}");
    tokio::spawn(async move {
        for status in statuses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf).await.unwrap();
            let reason = if status == 429 {
                "Too Many Requests"
            } else {
                "OK"
            };
            let body = if status == 429 { "limited" } else { "ok" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            let _ = socket.shutdown().await;
        }
    });
    url
}

fn request(url: &str, bucket: RateBucket) -> FetchRequest {
    FetchRequest {
        url: url.to_string(),
        method: HttpMethod::Get,
        headers: Vec::new(),
        body: None,
        timeout: Duration::from_secs(5),
        rate_bucket: bucket,
        max_body_bytes: 4096,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
        priority: RequestPriority::Normal,
    }
}

/// #130 regression, end-to-end through `HttpFetcherImpl`: two indexers behind
/// ONE Prowlarr origin. A 429 for indexer A trips only A's per-indexer
/// rate-limit breaker — NOT the shared transport breaker (the host answered, so
/// it stays alive) and NOT sibling indexer B on the same origin.
#[tokio::test]
async fn indexer_429_opens_only_that_indexer_not_its_origin_siblings() {
    // Connection 1 = A's 429; connection 2 = B's 200. A's second fetch is
    // CircuitOpen (no HTTP), so it never reaches the server.
    let server_url = spawn_status_server(vec![429, 200]).await;
    let fetcher = HttpFetcherImpl::new().unwrap();
    let origin = "pin-shared-prowlarr-origin-20260721".to_string();
    let indexer_a = RateBucket::Indexer {
        origin: origin.clone(),
        indexer: Some("pin-indexer-a-20260721".to_string()),
    };
    let indexer_b = RateBucket::Indexer {
        origin,
        indexer: Some("pin-indexer-b-20260721".to_string()),
    };

    let first = fetcher.fetch(request(&server_url, indexer_a.clone())).await;
    assert!(
        matches!(first, Err(FetchError::RateLimited)),
        "first indexer response (429) should surface as RateLimited, got {first:?}"
    );

    let second_same_indexer = fetcher.fetch(request(&server_url, indexer_a)).await;
    assert!(
        matches!(second_same_indexer, Err(FetchError::CircuitOpen { .. })),
        "the SAME indexer must be breaker-blocked after its own 429, got {second_same_indexer:?}"
    );

    let sibling_same_origin = fetcher.fetch(request(&server_url, indexer_b)).await;
    assert!(
        sibling_same_origin.is_ok(),
        "a sibling indexer on the SAME Prowlarr origin must still proceed after A's 429 (issue #130), got {sibling_same_origin:?}"
    );
}
