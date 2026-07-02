//! `HttpFetcher` trait implementation backed by `reqwest`.
//!
//! Provides rate-limited HTTP fetching with SSRF protection, anti-bot detection,
//! and streaming body-size enforcement.

use livrarr_domain::services::{
    FetchError, FetchRequest, FetchResponse, HttpFetcher, HttpMethod, UserAgentProfile,
};

use crate::breaker::BreakerSignal;
use crate::outbound_queue;
use crate::ssrf;

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
        UserAgentProfile::Server => crate::livrarr_user_agent(),
        UserAgentProfile::Custom(s) => s.clone(),
    }
}

// ---------------------------------------------------------------------------
// HttpFetcherImpl
// ---------------------------------------------------------------------------

/// Production implementation of [`HttpFetcher`].
#[derive(Clone)]
pub struct HttpFetcherImpl {
    client: reqwest::Client,
    ssrf_client: reqwest::Client,
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
        })
    }

    /// Internal fetch logic shared between `fetch` and `fetch_ssrf_safe`.
    async fn do_fetch(
        &self,
        client: &reqwest::Client,
        req: FetchRequest,
    ) -> Result<FetchResponse, FetchError> {
        // Wait our turn in the process-global outbound queue: paced per bucket, with a
        // bounded number of in-flight sends. Hold the permit across the send and body
        // read — it releases the in-flight slot on drop. An Open breaker resolves to
        // an error instead of a permit (R-3): no HTTP happens, and this call does not
        // report a breaker outcome — no attempt was made to report on.
        let _permit = match outbound_queue::shared()
            .acquire(req.rate_bucket.clone(), req.priority)
            .await
        {
            Ok(permit) => permit,
            Err(retry_after) => return Err(FetchError::CircuitOpen { retry_after }),
        };

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

        // Send. do_fetch is the single choke point for every FetchError it
        // generates/intercepts itself (Timeout, Connection, RateLimited,
        // BodyTooLarge) — each reports exactly one breaker Failure so a
        // transport failure is never double-counted against the caller's own
        // response-derived reporting.
        let response = match builder.send().await {
            Ok(r) => r,
            Err(e) => {
                let err = if e.is_timeout() {
                    FetchError::Timeout(req.timeout)
                } else {
                    FetchError::Connection(e.to_string())
                };
                outbound_queue::shared()
                    .report_outcome(req.rate_bucket.clone(), BreakerSignal::Failure);
                return Err(err);
            }
        };

        let status = response.status().as_u16();

        // 429 → RateLimited
        if status == 429 {
            outbound_queue::shared()
                .report_outcome(req.rate_bucket.clone(), BreakerSignal::Failure);
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
            let chunk = match stream.chunk().await {
                Ok(c) => c,
                Err(e) => {
                    let err = if e.is_timeout() {
                        FetchError::Timeout(req.timeout)
                    } else {
                        FetchError::Connection(e.to_string())
                    };
                    outbound_queue::shared()
                        .report_outcome(req.rate_bucket.clone(), BreakerSignal::Failure);
                    return Err(err);
                }
            };

            match chunk {
                Some(bytes) => {
                    if body.len() + bytes.len() > max_bytes {
                        outbound_queue::shared()
                            .report_outcome(req.rate_bucket.clone(), BreakerSignal::Failure);
                        return Err(FetchError::BodyTooLarge { max_bytes });
                    }
                    body.extend_from_slice(&bytes);
                }
                None => break,
            }
        }

        // Anti-bot check (only on HTML content types): an interstitial is a hard
        // block on any breaker bucket, not a threshold-counted failure.
        if req.anti_bot_check && is_anti_bot_content_type(&content_type) && scan_for_anti_bot(&body)
        {
            outbound_queue::shared().report_outcome(
                req.rate_bucket.clone(),
                BreakerSignal::TripImmediately { open_for: None },
            );
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

        // Manual redirect loop with full SSRF validation per hop.
        let mut current_url = req.url.clone();
        let mut current_method = req.method;
        let mut current_headers = req.headers.clone();
        let mut current_body = req.body.clone();
        let max_redirects = 5;

        for _ in 0..=max_redirects {
            let follow_req = FetchRequest {
                url: current_url.clone(),
                method: current_method,
                headers: current_headers.clone(),
                body: current_body.clone(),
                timeout: req.timeout,
                rate_bucket: req.rate_bucket.clone(),
                max_body_bytes: req.max_body_bytes,
                anti_bot_check: req.anti_bot_check,
                user_agent: req.user_agent.clone(),
                priority: req.priority,
            };

            let result = self.do_fetch(&self.ssrf_client, follow_req).await?;

            if !(300..400).contains(&result.status) {
                return Ok(result);
            }

            let location = result
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("location"))
                .map(|(_, v)| v.clone());

            let location = match location {
                Some(loc) => loc,
                None => return Ok(result),
            };

            // Resolve relative redirects against current hop (not original URL).
            let current_parsed = url::Url::parse(&current_url)
                .map_err(|e| FetchError::Ssrf(format!("invalid current URL: {e}")))?;
            let redirect_url = url::Url::parse(&location)
                .or_else(|_| current_parsed.join(&location))
                .map_err(|e| FetchError::Ssrf(format!("invalid redirect URL: {e}")))?;

            // Validate redirect target: scheme, credentials, IP.
            match redirect_url.scheme() {
                "http" | "https" => {}
                scheme => {
                    return Err(FetchError::Ssrf(format!(
                        "redirect to scheme '{scheme}' not allowed"
                    )));
                }
            }
            if !redirect_url.username().is_empty() || redirect_url.password().is_some() {
                return Err(FetchError::Ssrf(
                    "redirect to URL with embedded credentials".to_string(),
                ));
            }
            let redirect_host = redirect_url
                .host_str()
                .ok_or_else(|| FetchError::Ssrf("redirect URL has no host".to_string()))?;
            if let Ok(ip) = redirect_host.parse::<std::net::IpAddr>() {
                if ssrf::is_private_ip(ip) {
                    return Err(FetchError::Ssrf(format!(
                        "redirect to private address {ip}"
                    )));
                }
            }

            // Cross-origin detection.
            let is_cross_origin = current_parsed.origin() != redirect_url.origin();

            // Method downgrade: 301/302/303 → GET (per RFC 7231).
            // Block cross-origin 307/308 (preserves method+body — exfil risk).
            match result.status {
                301..=303 => {
                    current_method = HttpMethod::Get;
                    current_body = None;
                    // Strip body-related headers on method downgrade.
                    current_headers.retain(|(k, _)| {
                        let lower = k.to_lowercase();
                        lower != "content-length"
                            && lower != "content-type"
                            && lower != "transfer-encoding"
                    });
                }
                307 | 308 if is_cross_origin => {
                    return Err(FetchError::Ssrf(
                        "cross-origin 307/308 redirect blocked".to_string(),
                    ));
                }
                _ => {}
            }

            // Strip credentials on cross-origin redirect.
            if is_cross_origin {
                current_headers.retain(|(k, _)| {
                    let lower = k.to_lowercase();
                    lower != "authorization"
                        && lower != "cookie"
                        && lower != "proxy-authorization"
                        && lower != "x-api-key"
                        && lower != "host"
                });
            }

            current_url = redirect_url.to_string();
        }

        Err(FetchError::Connection("too many redirects".to_string()))
    }
}
