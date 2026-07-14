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

#[tokio::test]
async fn indexer_429_opens_only_that_origin_bucket() {
    let server_url = spawn_status_server(vec![429, 200]).await;
    let fetcher = HttpFetcherImpl::new().unwrap();
    let bucket_a = RateBucket::Indexer("pin-origin-a-20260713".to_string());
    let bucket_b = RateBucket::Indexer("pin-origin-b-20260713".to_string());

    let first = fetcher.fetch(request(&server_url, bucket_a.clone())).await;
    assert!(
        matches!(first, Err(FetchError::RateLimited)),
        "first indexer response should surface as RateLimited, got {first:?}"
    );

    let second_same_bucket = fetcher.fetch(request(&server_url, bucket_a)).await;
    assert!(
        matches!(second_same_bucket, Err(FetchError::CircuitOpen { .. })),
        "same indexer bucket should be breaker-blocked after 429, got {second_same_bucket:?}"
    );

    let other_bucket = fetcher.fetch(request(&server_url, bucket_b)).await;
    assert!(
        other_bucket.is_ok(),
        "different indexer origin bucket must still proceed, got {other_bucket:?}"
    );
}
