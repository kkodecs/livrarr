//! `HttpFetcher` trait implementation backed by `reqwest`.
//!
//! Provides rate-limited HTTP fetching with SSRF protection, anti-bot detection,
//! and streaming body-size enforcement.

use std::time::Duration;

use livrarr_domain::services::{
    FetchError, FetchRequest, FetchResponse, HttpFetcher, HttpMethod, RateBucket, UserAgentProfile,
};

use crate::breaker::BreakerSignal;
use crate::outbound_queue;
use crate::ssrf;

/// TCP-connect budget for `fetch_ssrf_safe_fast_connect`. A dead/unreachable
/// host fails here in well under a second instead of riding out whatever the
/// caller's overall `req.timeout` (or an outer deadline wrapping the call,
/// like phase1 cover download's 3s budget) happens to be. Chosen to comfortably
/// cover a slow-but-live handshake while cutting off a black-holed host fast.
const FAST_CONNECT_TIMEOUT: Duration = Duration::from_millis(600);

/// Default cooldown for an indexer's per-indexer rate-limit breaker when a 429
/// trips it and no usable `Retry-After` is present — a definitive "stop", not a
/// maybe (same reasoning as the GR anti-bot immediate trip). Provider buckets
/// are unaffected: their 429 handling stays at the client layer and keeps
/// reporting plain `Failure`.
const INDEXER_RATE_LIMITED_COOLDOWN: Duration = Duration::from_secs(30 * 60);

/// Translate a 429's `Retry-After` header into a rate-limit-breaker open
/// window. Delta-seconds ONLY: an integer string → that many seconds, clamped
/// to `[10s, 6h]` (a 1–9s value floors to 10s; a value over 6h caps to 6h).
/// Anything else — absent, `≤ 0`, non-integer, or an HTTP-date form — falls
/// back to [`INDEXER_RATE_LIMITED_COOLDOWN`] (30 min). Delta-seconds is the
/// only honored form because it is unambiguous under clock drift; no claim is
/// made about what any indexer emits — the fallback IS the contract.
fn indexer_rate_limit_open_for(retry_after: Option<&str>) -> Duration {
    const MIN: Duration = Duration::from_secs(10);
    const MAX: Duration = Duration::from_secs(6 * 60 * 60);
    match retry_after.and_then(|s| s.trim().parse::<i64>().ok()) {
        Some(secs) if secs > 0 => Duration::from_secs(secs as u64).clamp(MIN, MAX),
        _ => INDEXER_RATE_LIMITED_COOLDOWN,
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
        UserAgentProfile::Server => crate::livrarr_user_agent(),
        UserAgentProfile::Custom(s) => s.clone(),
    }
}

// ---------------------------------------------------------------------------
// HttpFetcherImpl
// ---------------------------------------------------------------------------

#[cfg(feature = "test-helpers")]
type ScriptedTransport =
    std::sync::Arc<dyn Fn(&FetchRequest) -> ScriptedTransportOutcome + Send + Sync + 'static>;

/// Production implementation of [`HttpFetcher`].
#[derive(Clone)]
pub struct HttpFetcherImpl {
    client: reqwest::Client,
    ssrf_client: reqwest::Client,
    /// Same SSRF-safe configuration as `ssrf_client`, plus `FAST_CONNECT_TIMEOUT`.
    /// A separate, persistent client (not built per-request) so repeated calls
    /// against the same live host within one process still get connection
    /// reuse — only `fetch_ssrf_safe_fast_connect` uses it.
    fast_connect_ssrf_client: reqwest::Client,
    /// Same trust posture as `client` (unrestricted — admin-configured
    /// infrastructure per the SSRF trusted-infrastructure pattern) but never
    /// auto-follows redirects, so `fetch_no_redirect` can hand the caller a
    /// raw 3xx response and its `Location` header instead of erroring (or
    /// silently chasing it) — e.g. to recover a `magnet:` redirect target
    /// reqwest's redirect-following client rejects.
    no_redirect_client: reqwest::Client,
    /// Same SSRF-safe configuration as `ssrf_client`, but never auto-follows
    /// redirects (Unit B3 #3) — for the Readarr client's verify-then-restrict,
    /// non-admin (untrusted) origins, which must be protected against
    /// DNS-rebinding on every connection. Deliberately a SEPARATE client from
    /// `no_redirect_client`, which stays unprotected on purpose: it is shared
    /// with admin-approved TRUSTED-infrastructure callers (insight #37), and
    /// adding the SSRF resolver there would regress those trusted setups.
    readarr_safe_client: reqwest::Client,
    #[cfg(feature = "test-helpers")]
    scripted_transport: Option<ScriptedTransport>,
    /// Hermetic DNS answers for the explicit SSRF preflight that precedes a
    /// scripted transport. The production resolver remains unchanged, and
    /// supplied addresses still pass through the normal private-IP filter.
    #[cfg(feature = "test-helpers")]
    ssrf_preflight_overrides: std::collections::HashMap<String, Vec<std::net::SocketAddr>>,
}

/// Hermetic transport result used by cross-crate real-router tests. The
/// production fetch path still owns queue admission and request-only timing;
/// only the actual socket exchange is replaced.
#[cfg(feature = "test-helpers")]
pub enum ScriptedTransportOutcome {
    Response {
        delay: Duration,
        response: FetchResponse,
    },
    Error {
        delay: Duration,
        error: FetchError,
    },
}

impl HttpFetcherImpl {
    /// Create a new fetcher with default reqwest clients.
    pub fn new() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(|e| e.to_string())?;

        // `.no_proxy()` on every SSRF-safe client (Unit B4 #4): reqwest
        // auto-uses `HTTP(S)_PROXY` from the environment unless told
        // otherwise, and a configured proxy resolves+connects the target
        // itself — meaning `SsrfSafeResolver` below would never even be
        // consulted. `client`/`no_redirect_client` deliberately keep proxy
        // support: they're the TRUSTED-infrastructure clients (admin-entered
        // URLs), which may legitimately sit behind a corporate proxy
        // (insight #37; security-model-policy.md:111 scopes this
        // requirement to untrusted requests only).
        let ssrf_client = reqwest::Client::builder()
            .dns_resolver(ssrf::SsrfSafeResolver::new())
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|e| e.to_string())?;

        let fast_connect_ssrf_client = reqwest::Client::builder()
            .dns_resolver(ssrf::SsrfSafeResolver::new())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(FAST_CONNECT_TIMEOUT)
            .no_proxy()
            .build()
            .map_err(|e| e.to_string())?;

        let no_redirect_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?;

        let readarr_safe_client = reqwest::Client::builder()
            .dns_resolver(ssrf::SsrfSafeResolver::new())
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|e| e.to_string())?;

        Ok(Self {
            client,
            ssrf_client,
            fast_connect_ssrf_client,
            no_redirect_client,
            readarr_safe_client,
            #[cfg(feature = "test-helpers")]
            scripted_transport: None,
            #[cfg(feature = "test-helpers")]
            ssrf_preflight_overrides: std::collections::HashMap::new(),
        })
    }

    #[cfg(feature = "test-helpers")]
    pub fn with_scripted_transport<F>(mut self, transport: F) -> Self
    where
        F: Fn(&FetchRequest) -> ScriptedTransportOutcome + Send + Sync + 'static,
    {
        self.scripted_transport = Some(std::sync::Arc::new(transport));
        self
    }

    /// Supply a public DNS answer to the SSRF preflight in hermetic scripted-
    /// transport tests. This never bypasses URL or address validation and does
    /// not alter non-test builds.
    #[cfg(feature = "test-helpers")]
    pub fn with_ssrf_preflight_test_dns(
        mut self,
        host: &str,
        address: std::net::SocketAddr,
    ) -> Self {
        self.ssrf_preflight_overrides
            .insert(host.to_ascii_lowercase(), vec![address]);
        self
    }

    /// Hermetic DNS seam for the Readarr trust-boundary regression. Both
    /// Readarr transports resolve the same hostname to the same address: the
    /// safe client still rejects private results, while the trusted,
    /// no-redirect client may connect to admin-approved infrastructure.
    #[cfg(feature = "test-helpers")]
    pub fn with_readarr_test_dns(
        mut self,
        host: &str,
        address: std::net::SocketAddr,
    ) -> Result<Self, String> {
        self.no_redirect_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .resolve(host, address)
            .no_proxy()
            .build()
            .map_err(|e| e.to_string())?;
        self.readarr_safe_client = reqwest::Client::builder()
            .dns_resolver(ssrf::SsrfSafeResolver::with_override(host, address))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|e| e.to_string())?;
        Ok(self)
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
            Err(outbound_queue::AdmissionError::CircuitOpen { retry_after }) => {
                return Err(FetchError::CircuitOpen { retry_after })
            }
            Err(outbound_queue::AdmissionError::QueueFull { retry_after }) => {
                return Err(FetchError::QueueFull { retry_after })
            }
        };

        #[cfg(feature = "test-helpers")]
        if let Some(transport) = &self.scripted_transport {
            let timeout = req.timeout;
            let rate_bucket = req.rate_bucket.clone();
            let outcome = transport(&req);
            let scripted = async move {
                match outcome {
                    ScriptedTransportOutcome::Response { delay, response } => {
                        tokio::time::sleep(delay).await;
                        Ok(response)
                    }
                    ScriptedTransportOutcome::Error { delay, error } => {
                        tokio::time::sleep(delay).await;
                        Err(error)
                    }
                }
            };
            return match tokio::time::timeout(timeout, scripted).await {
                Ok(result) => result,
                Err(_) => {
                    outbound_queue::shared().report_outcome(rate_bucket, BreakerSignal::Failure);
                    Err(FetchError::Timeout(timeout))
                }
            };
        }

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
                    FetchError::Connection(format!(
                        "{}: {}",
                        livrarr_domain::redact_secrets(&req.url),
                        e.without_url()
                    ))
                };
                outbound_queue::shared()
                    .report_outcome(req.rate_bucket.clone(), BreakerSignal::Failure);
                return Err(err);
            }
        };

        let status = response.status().as_u16();

        // 429 → RateLimited, routed by bucket (issue #130):
        //   * `Indexer { indexer: Some(_) }`: this ONE indexer is rate-limited.
        //     Trip its per-indexer rate-limit breaker (honoring `Retry-After`,
        //     else a 30-minute cooldown) AND report SUCCESS to the shared
        //     transport breaker — the host answered, so it must not look dead to
        //     its neighbours on the same origin.
        //   * `Indexer { indexer: None }` (unresolved-identity fallback): no
        //     per-indexer breaker to trip; still report transport success.
        //   * Provider buckets: unchanged — plain `Failure` toward the
        //     transport (provider) threshold, exactly as before.
        if status == 429 {
            let queue = outbound_queue::shared();
            match &req.rate_bucket {
                RateBucket::Indexer {
                    indexer: Some(_), ..
                } => {
                    let open_for = indexer_rate_limit_open_for(
                        response
                            .headers()
                            .get(reqwest::header::RETRY_AFTER)
                            .and_then(|v| v.to_str().ok()),
                    );
                    queue.report_rate_limit_outcome(
                        req.rate_bucket.clone(),
                        BreakerSignal::TripImmediately {
                            open_for: Some(open_for),
                        },
                    );
                    queue.report_outcome(req.rate_bucket.clone(), BreakerSignal::Success);
                }
                RateBucket::Indexer { indexer: None, .. } => {
                    queue.report_outcome(req.rate_bucket.clone(), BreakerSignal::Success);
                }
                _ => {
                    queue.report_outcome(req.rate_bucket.clone(), BreakerSignal::Failure);
                }
            }
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
                        FetchError::Connection(format!(
                            "{}: {}",
                            livrarr_domain::redact_secrets(&req.url),
                            e.without_url()
                        ))
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

        // A completed response (any status) on an Indexer bucket: the host is
        // alive (transport success) and — past the 429 early-return above — is
        // not rate-limiting this indexer, so report rate-limit success too,
        // which cleanly closes a half-open probe once a cooldown has elapsed.
        // Provider buckets self-report at their client layer; `do_fetch` stays
        // silent for them on success.
        if matches!(req.rate_bucket, RateBucket::Indexer { .. }) {
            let queue = outbound_queue::shared();
            queue.report_outcome(req.rate_bucket.clone(), BreakerSignal::Success);
            queue.report_rate_limit_outcome(req.rate_bucket.clone(), BreakerSignal::Success);
        }

        Ok(FetchResponse {
            status,
            headers,
            body,
        })
    }

    /// Shared body for `fetch_ssrf_safe` and `fetch_ssrf_safe_fast_connect` —
    /// identical SSRF preflight and manual redirect loop, differing only in
    /// which pre-built client (and therefore which connect-phase budget)
    /// carries the actual sends.
    async fn fetch_ssrf_safe_impl(
        &self,
        req: FetchRequest,
        client: &reqwest::Client,
    ) -> Result<FetchResponse, FetchError> {
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
            #[cfg(feature = "test-helpers")]
            let addrs: Vec<_> = if let Some(addresses) = self
                .ssrf_preflight_overrides
                .get(&host.to_ascii_lowercase())
            {
                addresses.clone()
            } else {
                tokio::net::lookup_host(format!("{host}:{port}"))
                    .await
                    .map_err(|e| FetchError::Ssrf(format!("DNS resolution failed: {e}")))?
                    .collect()
            };
            #[cfg(not(feature = "test-helpers"))]
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

            let result = self.do_fetch(client, follow_req).await?;

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

    /// Fetch via the SSRF-safe, no-redirect client (Unit B3 #3) — for the
    /// Readarr client's untrusted, verify-then-restrict origins. An inherent
    /// method (not part of the `HttpFetcher` trait) since `ReadarrClient`
    /// holds a concrete `HttpFetcherImpl`, not a `dyn HttpFetcher`.
    pub async fn fetch_readarr(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.do_fetch(&self.readarr_safe_client, req).await
    }
}

impl HttpFetcher for HttpFetcherImpl {
    async fn fetch(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.do_fetch(&self.client, req).await
    }

    async fn fetch_ssrf_safe(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.fetch_ssrf_safe_impl(req, &self.ssrf_client).await
    }

    async fn fetch_ssrf_safe_fast_connect(
        &self,
        req: FetchRequest,
    ) -> Result<FetchResponse, FetchError> {
        self.fetch_ssrf_safe_impl(req, &self.fast_connect_ssrf_client)
            .await
    }

    async fn fetch_no_redirect(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.do_fetch(&self.no_redirect_client, req).await
    }
}

#[cfg(test)]
mod redaction_tests {
    use super::*;
    use livrarr_domain::services::RateBucket;
    use livrarr_domain::RequestPriority;

    /// REQ-004 / AC-002: a failed fetch whose URL carries a secret must not leak
    /// that secret into the `FetchError` string — which is exactly the string the
    /// RSS-sync WARN log emits verbatim (`rss_sync.rs:66`). Drives a real
    /// connection failure (port 1 refuses) so the reqwest error is genuine, then
    /// asserts the sentinel apikey never survives into the error text.
    #[tokio::test]
    async fn failed_fetch_redacts_apikey_in_error() {
        let fetcher = HttpFetcherImpl::new().unwrap();
        let req = FetchRequest {
            url: "http://127.0.0.1:1/2/api?apikey=FAKEKEYSENTINEL".to_string(),
            method: HttpMethod::Get,
            headers: vec![],
            body: None,
            timeout: Duration::from_secs(2),
            rate_bucket: RateBucket::None,
            max_body_bytes: 4096,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
            priority: RequestPriority::Normal,
        };

        let err = fetcher
            .fetch(req)
            .await
            .expect_err("connecting to 127.0.0.1:1 must fail");
        let msg = err.to_string();

        assert!(
            !msg.contains("FAKEKEYSENTINEL"),
            "apikey leaked into fetch error (would reach logs): {msg}"
        );
        assert!(
            msg.contains("[REDACTED]"),
            "expected redacted url in error, got: {msg}"
        );
    }
}

#[cfg(test)]
mod retry_after_tests {
    use super::*;

    /// Design §6 matrix: a 429's `Retry-After` → the rate-limit breaker's open
    /// window. Delta-seconds only, clamped to `[10s, 6h]`; everything else
    /// (absent, `≤ 0`, non-integer, HTTP-date) falls back to the 30-min default.
    #[test]
    fn indexer_retry_after_open_for_matrix() {
        let default = Duration::from_secs(30 * 60);
        let d = indexer_rate_limit_open_for;
        assert_eq!(d(None), default, "absent → 30-min default");
        assert_eq!(
            d(Some("120")),
            Duration::from_secs(120),
            "plain delta-seconds"
        );
        assert_eq!(d(Some("3")), Duration::from_secs(10), "1-9s floors to 10s");
        assert_eq!(
            d(Some("999999")),
            Duration::from_secs(6 * 60 * 60),
            ">6h caps to 6h"
        );
        assert_eq!(d(Some("0")), default, "zero → default");
        assert_eq!(d(Some("-5")), default, "negative → default");
        assert_eq!(
            d(Some("Wed, 21 Oct 2026 07:28:00 GMT")),
            default,
            "HTTP-date form is not honored → default"
        );
        assert_eq!(d(Some("garbage")), default, "non-integer → default");
        assert_eq!(
            d(Some("  60  ")),
            Duration::from_secs(60),
            "surrounding whitespace trimmed"
        );
    }
}

#[cfg(test)]
mod proxy_bypass_tests {
    //! Unit B4 #4: reqwest auto-uses `HTTP(S)_PROXY` from the environment
    //! unless a client is built with `.no_proxy()` — and when a proxy IS in
    //! play, the proxy resolves+connects the target itself, so our custom
    //! `SsrfSafeResolver` (attached via `.dns_resolver()`) is never consulted.
    //! These tests stand a bare `TcpListener` in for a forward proxy: it
    //! never speaks HTTP CONNECT, so it can't complete a real tunnel, but the
    //! one fact that matters — did a connection attempt land on it at all —
    //! is fully observable without one.
    use super::*;
    use livrarr_domain::services::{FetchRequest, HttpMethod, RateBucket, UserAgentProfile};
    use livrarr_domain::RequestPriority;
    use std::sync::Mutex;

    /// `std::env::set_var` mutates the whole process — serializes every test
    /// in this module against each other so two of them can't stomp on the
    /// same `HTTPS_PROXY` value mid-flight (the default `cargo test` runs
    /// tests concurrently within one binary).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// The lock only orders test execution — it guards no state that a panic
    /// could leave corrupted — so a poisoned lock (one test panicked while
    /// holding it) must not cascade into every test queued behind it.
    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Snapshots `HTTPS_PROXY` and restores it on drop (including on a
    /// mid-test panic), so a failure here can never leak a stray proxy
    /// setting into an unrelated test that runs afterward in this binary.
    struct EnvVarGuard {
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set_https_proxy(value: &str) -> Self {
            let previous = std::env::var("HTTPS_PROXY").ok();
            std::env::set_var("HTTPS_PROXY", value);
            Self { previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(v) => std::env::set_var("HTTPS_PROXY", v),
                None => std::env::remove_var("HTTPS_PROXY"),
            }
        }
    }

    /// TEST-NET-1 (RFC 5737) — reserved for documentation, never routed on
    /// the real internet. `is_private_ip` does not (and must not) flag it, so
    /// it clears the SSRF preflight exactly like a genuine public host, but a
    /// *direct* connection attempt to it fails fast or blackholes instead of
    /// ever reaching a real server — so watching whether the fake proxy
    /// below gets contacted doesn't depend on live network access.
    const TEST_NET_TARGET: &str = "https://192.0.2.1:1/";

    fn probe_req(url: &str) -> FetchRequest {
        FetchRequest {
            url: url.to_string(),
            method: HttpMethod::Get,
            headers: vec![],
            body: None,
            timeout: Duration::from_millis(800),
            rate_bucket: RateBucket::None,
            max_body_bytes: 1024,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
            priority: RequestPriority::Normal,
        }
    }

    /// A bare TCP listener standing in for a forward proxy. Returns its
    /// `http://host:port` URL alongside the listener itself.
    async fn fake_proxy() -> (tokio::net::TcpListener, String) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        (listener, format!("http://{addr}"))
    }

    /// Races the fake proxy's `accept()` against a deadline well above every
    /// client-side timeout used here, so it reflects "was the proxy ever
    /// contacted" rather than "which future happened to finish first".
    async fn proxy_was_contacted(listener: tokio::net::TcpListener) -> bool {
        tokio::time::timeout(Duration::from_millis(1500), listener.accept())
            .await
            .is_ok()
    }

    /// Builds an `HttpFetcherImpl` with `HTTPS_PROXY` temporarily set to
    /// `proxy_url`, then immediately releases the lock and restores the env
    /// var — the fetcher's proxy config is fixed at `build()` time, so
    /// nothing downstream needs the guard held any longer (and a std
    /// `MutexGuard` held across an `.await` is its own clippy lint).
    fn build_fetcher_with_https_proxy(proxy_url: &str) -> HttpFetcherImpl {
        let _serialize = lock_env();
        let _env = EnvVarGuard::set_https_proxy(proxy_url);
        HttpFetcherImpl::new().unwrap()
    }

    #[tokio::test]
    async fn fetch_ssrf_safe_does_not_route_through_https_proxy_env() {
        let (proxy, proxy_url) = fake_proxy().await;
        let fetcher = build_fetcher_with_https_proxy(&proxy_url);
        let (contacted, _) = tokio::join!(
            proxy_was_contacted(proxy),
            fetcher.fetch_ssrf_safe(probe_req(TEST_NET_TARGET))
        );

        assert!(
            !contacted,
            "fetch_ssrf_safe (ssrf_client) must not route through HTTPS_PROXY — \
             a proxy would resolve+connect the target itself, bypassing SsrfSafeResolver"
        );
    }

    #[tokio::test]
    async fn fetch_ssrf_safe_fast_connect_does_not_route_through_https_proxy_env() {
        let (proxy, proxy_url) = fake_proxy().await;
        let fetcher = build_fetcher_with_https_proxy(&proxy_url);
        let (contacted, _) = tokio::join!(
            proxy_was_contacted(proxy),
            fetcher.fetch_ssrf_safe_fast_connect(probe_req(TEST_NET_TARGET))
        );

        assert!(
            !contacted,
            "fetch_ssrf_safe_fast_connect (fast_connect_ssrf_client) must not route through HTTPS_PROXY"
        );
    }

    #[tokio::test]
    async fn fetch_readarr_does_not_route_through_https_proxy_env() {
        let (proxy, proxy_url) = fake_proxy().await;
        let fetcher = build_fetcher_with_https_proxy(&proxy_url);
        let (contacted, _) = tokio::join!(
            proxy_was_contacted(proxy),
            fetcher.fetch_readarr(probe_req(TEST_NET_TARGET))
        );

        assert!(
            !contacted,
            "fetch_readarr (readarr_safe_client) must not route through HTTPS_PROXY"
        );
    }

    #[tokio::test]
    async fn trusted_fetch_still_honors_https_proxy_env() {
        // Insight #37 / security-model-policy.md:111: only UNTRUSTED
        // outbound requests must disable env-proxy inheritance. The plain
        // trusted `client` (admin-configured infra) must keep honoring it —
        // proven here the same way as the negative cases above, just
        // asserting the opposite outcome.
        let (proxy, proxy_url) = fake_proxy().await;
        let fetcher = build_fetcher_with_https_proxy(&proxy_url);
        let (contacted, _) = tokio::join!(
            proxy_was_contacted(proxy),
            fetcher.fetch(probe_req(TEST_NET_TARGET))
        );

        assert!(
            contacted,
            "trusted `fetch` (unrestricted client) must keep honoring HTTPS_PROXY — \
             admin-configured infra may legitimately sit behind a corporate proxy"
        );
    }
}
