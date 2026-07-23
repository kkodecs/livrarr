use std::time::Duration;

pub mod breaker;
pub mod fetcher;
pub mod outbound_queue;
pub mod ssrf;

/// Composable HTTP client.
#[derive(Clone)]
pub struct HttpClient {
    inner: reqwest::Client,
}

impl HttpClient {
    pub fn builder() -> HttpClientBuilder {
        HttpClientBuilder::default()
    }

    pub fn inner(&self) -> &reqwest::Client {
        &self.inner
    }

    pub fn get(&self, url: &str) -> reqwest::RequestBuilder {
        self.inner.get(url)
    }

    pub fn post(&self, url: &str) -> reqwest::RequestBuilder {
        self.inner.post(url)
    }
}

/// Builder for configuring an HTTP client.
#[derive(Default)]
pub struct HttpClientBuilder {
    timeout: Option<Duration>,
    user_agent: Option<String>,
    danger_accept_invalid_certs: bool,
    ssrf_safe: bool,
}

impl HttpClientBuilder {
    pub fn timeout(mut self, duration: Duration) -> Self {
        self.timeout = Some(duration);
        self
    }

    pub fn user_agent(mut self, agent: &str) -> Self {
        self.user_agent = Some(agent.to_string());
        self
    }

    pub fn danger_accept_invalid_certs(mut self, accept: bool) -> Self {
        self.danger_accept_invalid_certs = accept;
        self
    }

    /// Enable SSRF protection via a custom DNS resolver that rejects private IPs
    /// at connection time. Prevents redirect-based and DNS-rebinding SSRF.
    pub fn ssrf_safe(mut self, enable: bool) -> Self {
        self.ssrf_safe = enable;
        self
    }

    pub fn build(self) -> Result<HttpClient, HttpClientError> {
        const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

        let mut builder = reqwest::Client::builder();

        builder = builder.timeout(self.timeout.unwrap_or(DEFAULT_TIMEOUT));

        if let Some(ua) = self.user_agent {
            builder = builder.user_agent(ua);
        }

        if self.danger_accept_invalid_certs {
            builder = builder.danger_accept_invalid_certs(true);
        }

        if self.ssrf_safe {
            // Unit B4 #4: without `.no_proxy()`, an env-inherited proxy
            // would resolve+connect the target itself, bypassing
            // `SsrfSafeResolver` entirely — see `fetcher.rs`'s matching fix
            // for the full writeup.
            builder = builder
                .dns_resolver(ssrf::SsrfSafeResolver::new())
                .no_proxy();
        }

        let inner = builder
            .build()
            .map_err(|e| HttpClientError::Build(e.to_string()))?;

        Ok(HttpClient { inner })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HttpClientError {
    #[error("request failed: {source}")]
    Request {
        #[source]
        source: reqwest::Error,
    },
    #[error("request build failed: {0}")]
    Build(String),
    #[error("timeout after {0:?}")]
    Timeout(Duration),
    #[error("TLS error: {0}")]
    Tls(String),
}

// ---------------------------------------------------------------------------
// v2.1 — HTTP Client Contracts
// ---------------------------------------------------------------------------

/// Max response body size for downloads and covers.
///
/// Satisfies: IMPL-META-006, IMPL-DLC-003
pub const MAX_RESPONSE_BODY_BYTES: usize = 10 * 1024 * 1024; // 10 MB

/// HTTP client preset kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Foreground,
    Background,
    HealthCheck,
    Download,
}

/// Error classification for retry decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpErrorKind {
    Status4xx,
    Status429,
    Status5xx,
    Connection,
    Timeout,
    Tls,
}

/// Retry disposition for a given error kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDisposition {
    Retryable,
    NoRetry,
}

/// Contract for HTTP client presets.
///
/// Satisfies: IMPL-HTTP-001 through IMPL-HTTP-006
pub trait HttpClientContract {
    fn kind(&self) -> ClientKind;
    fn timeout(&self) -> Duration;
    fn retry_enabled(&self) -> bool;
    fn max_attempts(&self) -> usize;
    fn backoff_schedule(&self) -> &[Duration];
    fn retry_disposition(&self, error_kind: HttpErrorKind) -> RetryDisposition;
    fn user_agent(&self) -> String;
    fn skip_ssl_validation(&self) -> bool;
}

/// BackgroundClient retry backoff: two delays before final attempt.
const BACKGROUND_BACKOFF: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];

/// ForegroundClient retry backoff: one delay before final attempt.
const FOREGROUND_BACKOFF: [Duration; 1] = [Duration::from_secs(2)];

pub fn livrarr_user_agent() -> String {
    format!(
        "KkodecsBookBot/{} (Livrarr; kkodecs@proton.me; https://github.com/kkodecs/livrarr)",
        env!("CARGO_PKG_VERSION")
    )
}

/// Normalize a URL down to its origin — lowercased `scheme://host[:port]` —
/// so different fetches against the same indexer host (search, grab, RSS)
/// share one `RateBucket::Indexer` pace lane and cooldown, regardless of
/// path, query string, or the indexer's configured display name. Returns
/// `None` if the URL fails to parse or carries no host.
///
/// Lives here rather than in livrarr-domain because livrarr-domain does not
/// depend on the `url` crate; livrarr-download and livrarr-metadata both
/// already depend on livrarr-http, so this is the one shared crate that
/// needs no new dependency edge for either caller.
pub fn normalized_origin(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let scheme = parsed.scheme();
    let host = parsed.host_str()?.to_lowercase();
    match parsed.port() {
        Some(port) => Some(format!("{scheme}://{host}:{port}")),
        None => Some(format!("{scheme}://{host}")),
    }
}

fn background_retry_disposition(error_kind: HttpErrorKind) -> RetryDisposition {
    match error_kind {
        HttpErrorKind::Status5xx | HttpErrorKind::Connection => RetryDisposition::Retryable,
        _ => RetryDisposition::NoRetry,
    }
}

// ---------------------------------------------------------------------------
// ForegroundClient — 3s timeout, no retry
// ---------------------------------------------------------------------------

pub struct ForegroundClient;

impl HttpClientContract for ForegroundClient {
    fn kind(&self) -> ClientKind {
        ClientKind::Foreground
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(3)
    }
    fn retry_enabled(&self) -> bool {
        true
    }
    fn max_attempts(&self) -> usize {
        2
    }
    fn backoff_schedule(&self) -> &[Duration] {
        &FOREGROUND_BACKOFF
    }
    fn retry_disposition(&self, error_kind: HttpErrorKind) -> RetryDisposition {
        background_retry_disposition(error_kind)
    }
    fn user_agent(&self) -> String {
        livrarr_user_agent()
    }
    fn skip_ssl_validation(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// BackgroundClient — 30s timeout, retry enabled
// ---------------------------------------------------------------------------

pub struct BackgroundClient;

impl HttpClientContract for BackgroundClient {
    fn kind(&self) -> ClientKind {
        ClientKind::Background
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
    fn retry_enabled(&self) -> bool {
        true
    }
    fn max_attempts(&self) -> usize {
        3
    }
    fn backoff_schedule(&self) -> &[Duration] {
        &BACKGROUND_BACKOFF
    }
    fn retry_disposition(&self, e: HttpErrorKind) -> RetryDisposition {
        background_retry_disposition(e)
    }
    fn user_agent(&self) -> String {
        livrarr_user_agent()
    }
    fn skip_ssl_validation(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// HealthCheckClient — 5s timeout, no retry
// ---------------------------------------------------------------------------

pub struct HealthCheckClient;

impl HttpClientContract for HealthCheckClient {
    fn kind(&self) -> ClientKind {
        ClientKind::HealthCheck
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(5)
    }
    fn retry_enabled(&self) -> bool {
        false
    }
    fn max_attempts(&self) -> usize {
        1
    }
    fn backoff_schedule(&self) -> &[Duration] {
        &[]
    }
    fn retry_disposition(&self, _: HttpErrorKind) -> RetryDisposition {
        RetryDisposition::NoRetry
    }
    fn user_agent(&self) -> String {
        livrarr_user_agent()
    }
    fn skip_ssl_validation(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// DownloadClient (HTTP preset) — 30s timeout, retry enabled, configurable SSL
// ---------------------------------------------------------------------------

pub struct DownloadClient {
    skip_ssl: bool,
}

impl DownloadClient {
    pub fn new(skip_ssl_validation: bool) -> Self {
        Self {
            skip_ssl: skip_ssl_validation,
        }
    }
}

impl HttpClientContract for DownloadClient {
    fn kind(&self) -> ClientKind {
        ClientKind::Download
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
    fn retry_enabled(&self) -> bool {
        true
    }
    fn max_attempts(&self) -> usize {
        3
    }
    fn backoff_schedule(&self) -> &[Duration] {
        &BACKGROUND_BACKOFF
    }
    fn retry_disposition(&self, e: HttpErrorKind) -> RetryDisposition {
        background_retry_disposition(e)
    }
    fn user_agent(&self) -> String {
        livrarr_user_agent()
    }
    fn skip_ssl_validation(&self) -> bool {
        self.skip_ssl
    }
}

#[cfg(test)]
mod ssrf_safe_proxy_tests {
    //! Unit B4 #4: `HttpClientBuilder::build()`'s `ssrf_safe(true)` branch
    //! attaches `SsrfSafeResolver`, but without `.no_proxy()` an env-inherited
    //! proxy bypasses it the same way as the SSRF-safe clients in
    //! `fetcher.rs` — see that file's `proxy_bypass_tests` module for the
    //! full mechanism writeup.
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// This lock only orders test execution — it guards no state a panic
    /// could leave corrupted — so a poisoned lock must not cascade.
    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

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
    /// the real internet. See `fetcher::proxy_bypass_tests` for why this
    /// makes the test deterministic without live network access.
    const TEST_NET_TARGET: &str = "https://192.0.2.1:1/";

    async fn fake_proxy() -> (tokio::net::TcpListener, String) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        (listener, format!("http://{addr}"))
    }

    async fn proxy_was_contacted(listener: tokio::net::TcpListener) -> bool {
        tokio::time::timeout(Duration::from_millis(1500), listener.accept())
            .await
            .is_ok()
    }

    #[tokio::test]
    async fn ssrf_safe_true_client_does_not_route_through_https_proxy_env() {
        let (proxy, proxy_url) = fake_proxy().await;

        // Scoped so the lock and env var are released before the `.await`
        // below: env proxy vars are read once, at `build()` time, so the
        // client's proxy config is already fixed by the time this block
        // ends — holding the guard any longer buys nothing (and a std
        // `MutexGuard` held across an await point is its own clippy lint).
        let client = {
            let _serialize = lock_env();
            let _env = EnvVarGuard::set_https_proxy(&proxy_url);
            HttpClient::builder().ssrf_safe(true).build().unwrap()
        };

        let (contacted, _) = tokio::join!(
            proxy_was_contacted(proxy),
            client
                .get(TEST_NET_TARGET)
                .timeout(Duration::from_millis(800))
                .send()
        );

        assert!(
            !contacted,
            "an ssrf_safe(true) HttpClient must not route through HTTPS_PROXY — \
             a proxy would resolve+connect the target itself, bypassing SsrfSafeResolver"
        );
    }
}
