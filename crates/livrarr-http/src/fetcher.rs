//! `HttpFetcher` trait implementation backed by `reqwest`.
//!
//! Provides rate-limited HTTP fetching with SSRF protection, anti-bot detection,
//! and streaming body-size enforcement.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use livrarr_domain::services::{
    FetchError, FetchRequest, FetchResponse, HttpFetcher, HttpMethod, RateBucket, UserAgentProfile,
};
use tokio::time::Instant;

use crate::ssrf;

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

struct BucketLimiter {
    min_interval: Duration,
    last_request: Instant,
}

/// Simple per-bucket rate limiter. Blocks (sleeps) until the minimum interval
/// since the last request in the same bucket has elapsed.
struct RateLimiterMap {
    buckets: Mutex<HashMap<RateBucket, BucketLimiter>>,
}

impl RateLimiterMap {
    fn new() -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
        }
    }

    async fn acquire(&self, bucket: &RateBucket) {
        let interval = Self::interval_for(bucket);
        if interval.is_zero() {
            return;
        }

        let sleep_until = {
            let mut map = self.buckets.lock().unwrap();
            let now = Instant::now();
            let entry = map.entry(bucket.clone()).or_insert_with(|| BucketLimiter {
                min_interval: interval,
                last_request: now - interval, // allow first request immediately
            });
            entry.min_interval = interval;

            let earliest = entry.last_request + entry.min_interval;
            if earliest > now {
                entry.last_request = earliest;
                Some(earliest)
            } else {
                entry.last_request = now;
                None
            }
        };

        if let Some(deadline) = sleep_until {
            tokio::time::sleep_until(deadline).await;
        }
    }

    fn interval_for(bucket: &RateBucket) -> Duration {
        match bucket {
            RateBucket::OpenLibrary => Duration::from_secs(1),
            RateBucket::Goodreads => Duration::from_secs(1),
            RateBucket::Hardcover => Duration::from_secs(1),
            RateBucket::Audnexus => Duration::from_secs(2),
            RateBucket::Indexer(_) => Duration::from_millis(500),
            RateBucket::None => Duration::ZERO,
        }
    }
}

// ---------------------------------------------------------------------------
// Anti-bot detection
// ---------------------------------------------------------------------------

const ANTI_BOT_MARKERS: &[&str] = &[
    "cf-browser-verification",
    "challenge-platform",
    "cf-challenge",
    "jschl-answer",
    "turnstile",
];

fn is_anti_bot_content_type(ct: &str) -> bool {
    let lower = ct.to_ascii_lowercase();
    lower.contains("text/html") || lower.contains("application/xhtml+xml")
}

fn scan_for_anti_bot(body_prefix: &[u8]) -> bool {
    // Only scan first 8KB for markers
    let scan_len = body_prefix.len().min(8192);
    let text = String::from_utf8_lossy(&body_prefix[..scan_len]);
    let lower = text.to_ascii_lowercase();
    ANTI_BOT_MARKERS.iter().any(|m| lower.contains(m))
}

// ---------------------------------------------------------------------------
// User agent strings
// ---------------------------------------------------------------------------

fn user_agent_string(profile: &UserAgentProfile) -> String {
    match profile {
        UserAgentProfile::Browser => {
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
                .to_string()
        }
        UserAgentProfile::Server => format!("Livrarr/{}", env!("CARGO_PKG_VERSION")),
        UserAgentProfile::Custom(s) => s.clone(),
    }
}

// ---------------------------------------------------------------------------
// HttpFetcherImpl
// ---------------------------------------------------------------------------

/// Production implementation of [`HttpFetcher`].
pub struct HttpFetcherImpl {
    client: reqwest::Client,
    ssrf_client: reqwest::Client,
    rate_limiters: RateLimiterMap,
}

impl HttpFetcherImpl {
    /// Create a new fetcher with default reqwest clients.
    pub fn new() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(|e| e.to_string())?;

        let ssrf_client = reqwest::Client::builder()
            .dns_resolver(ssrf::SsrfSafeResolver::new())
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?;

        Ok(Self {
            client,
            ssrf_client,
            rate_limiters: RateLimiterMap::new(),
        })
    }

    /// Internal fetch logic shared between `fetch` and `fetch_ssrf_safe`.
    async fn do_fetch(
        &self,
        client: &reqwest::Client,
        req: FetchRequest,
    ) -> Result<FetchResponse, FetchError> {
        // Rate limit
        self.rate_limiters.acquire(&req.rate_bucket).await;

        // Build request
        let method = match req.method {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
            HttpMethod::Put => reqwest::Method::PUT,
            HttpMethod::Delete => reqwest::Method::DELETE,
        };

        let ua = user_agent_string(&req.user_agent);

        let mut builder = client
            .request(method, &req.url)
            .timeout(req.timeout)
            .header(reqwest::header::USER_AGENT, &ua);

        for (k, v) in &req.headers {
            builder = builder.header(k, v);
        }

        if let Some(body) = req.body {
            builder = builder.body(body);
        }

        // Send
        let response = builder.send().await.map_err(|e| {
            if e.is_timeout() {
                FetchError::Timeout(req.timeout)
            } else {
                FetchError::Connection(e.to_string())
            }
        })?;

        let status = response.status().as_u16();

        // 429 → RateLimited
        if status == 429 {
            return Err(FetchError::RateLimited);
        }

        // Collect response headers
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        // Get content-type for anti-bot check
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        // Stream body with size enforcement
        let max_bytes = req.max_body_bytes;
        let mut body = Vec::new();
        let mut stream = response;

        loop {
            let chunk = stream.chunk().await.map_err(|e| {
                if e.is_timeout() {
                    FetchError::Timeout(req.timeout)
                } else {
                    FetchError::Connection(e.to_string())
                }
            })?;

            match chunk {
                Some(bytes) => {
                    if body.len() + bytes.len() > max_bytes {
                        return Err(FetchError::BodyTooLarge { max_bytes });
                    }
                    body.extend_from_slice(&bytes);
                }
                None => break,
            }
        }

        // Anti-bot check (only on HTML content types)
        if req.anti_bot_check && is_anti_bot_content_type(&content_type) && scan_for_anti_bot(&body)
        {
            return Err(FetchError::AntiBotDetected);
        }

        Ok(FetchResponse {
            status,
            headers,
            body,
        })
    }
}

impl HttpFetcher for HttpFetcherImpl {
    async fn fetch(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.do_fetch(&self.client, req).await
    }

    async fn fetch_ssrf_safe(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
        // Pre-flight validation
        let parsed =
            url::Url::parse(&req.url).map_err(|e| FetchError::Ssrf(format!("invalid URL: {e}")))?;

        // Reject non-http(s) schemes
        match parsed.scheme() {
            "http" | "https" => {}
            scheme => {
                return Err(FetchError::Ssrf(format!(
                    "scheme '{scheme}' not allowed; only http/https"
                )));
            }
        }

        // Reject embedded credentials
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(FetchError::Ssrf(
                "URLs with embedded credentials are not allowed".to_string(),
            ));
        }

        // Resolve hostname and validate IPs
        let host = parsed
            .host_str()
            .ok_or_else(|| FetchError::Ssrf("URL has no host".to_string()))?;
        let port = parsed.port_or_known_default().unwrap_or(80);

        // Check literal IP
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            if ssrf::is_private_ip(ip) {
                return Err(FetchError::Ssrf(format!(
                    "address {ip} is private/reserved"
                )));
            }
        } else {
            // DNS resolution check
            let addrs: Vec<_> = tokio::net::lookup_host(format!("{host}:{port}"))
                .await
                .map_err(|e| FetchError::Ssrf(format!("DNS resolution failed: {e}")))?
                .collect();

            if addrs.is_empty() {
                return Err(FetchError::Ssrf(
                    "DNS resolution returned no addresses".to_string(),
                ));
            }

            for addr in &addrs {
                if ssrf::is_private_ip(addr.ip()) {
                    return Err(FetchError::Ssrf(format!(
                        "resolved address {} is private/reserved",
                        addr.ip()
                    )));
                }
            }
        }

        // Use the SSRF-safe client (has SsrfSafeResolver for redirect protection)
        // Redirects are disabled (Policy::none) — we handle them manually
        let mut current_url = req.url.clone();
        let mut current_host = host.to_string();
        let max_redirects = 5;

        for _ in 0..=max_redirects {
            let follow_req = FetchRequest {
                url: current_url.clone(),
                method: req.method,
                headers: req.headers.clone(),
                body: req.body.clone(),
                timeout: req.timeout,
                rate_bucket: req.rate_bucket.clone(),
                max_body_bytes: req.max_body_bytes,
                anti_bot_check: req.anti_bot_check,
                user_agent: req.user_agent.clone(),
            };

            let result = self.do_fetch(&self.ssrf_client, follow_req).await?;

            if !(300..400).contains(&result.status) {
                return Ok(result);
            }

            // Handle redirect
            let location = result
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("location"))
                .map(|(_, v)| v.clone());

            let location = match location {
                Some(loc) => loc,
                None => return Ok(result),
            };

            let redirect_url = url::Url::parse(&location)
                .or_else(|_| parsed.join(&location))
                .map_err(|e| FetchError::Ssrf(format!("invalid redirect URL: {e}")))?;

            let redirect_host = redirect_url.host_str().unwrap_or("");
            if redirect_host != current_host {
                return Err(FetchError::Ssrf(format!(
                    "cross-domain redirect from {current_host} to {redirect_host}"
                )));
            }

            current_url = redirect_url.to_string();
            current_host = redirect_host.to_string();
        }

        Err(FetchError::Connection("too many redirects".to_string()))
    }
}
