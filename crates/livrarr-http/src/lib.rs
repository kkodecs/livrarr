use std::time::Duration;

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
}

impl HttpClientBuilder {
    pub fn timeout(mut self, duration: Duration) -> Self {
        self.timeout = Some(duration);
        self
    }

    pub fn retry(self, _max_attempts: u32, _backoff: Duration) -> Self {
        // Retry logic is handled at the call site for now —
        // reqwest doesn't have built-in retry middleware.
        self
    }

    pub fn rate_limit(self, _rps: u32) -> Self {
        // Rate limiting handled at call site per provider.
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

pub mod rate_limit;

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

const STANDARD_BACKOFF: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];

fn livrarr_user_agent() -> String {
    format!("Livrarr/{}", env!("CARGO_PKG_VERSION"))
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
        &STANDARD_BACKOFF
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
        &STANDARD_BACKOFF
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
