use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RateBucket {
    OpenLibrary,
    Hardcover,
    Audnexus,
    Goodreads,
    Indexer(String),
    None,
}

#[derive(Debug, Clone)]
pub enum UserAgentProfile {
    Browser,
    Server,
    Custom(String),
}

#[derive(Debug)]
pub struct FetchRequest {
    pub url: String,
    pub method: HttpMethod,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
    pub rate_bucket: RateBucket,
    pub max_body_bytes: usize,
    pub anti_bot_check: bool,
    pub user_agent: UserAgentProfile,
}

#[derive(Debug)]
pub struct FetchResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("connection error: {0}")]
    Connection(String),
    #[error("timeout after {0:?}")]
    Timeout(Duration),
    #[error("response body exceeds {max_bytes} byte limit")]
    BodyTooLarge { max_bytes: usize },
    #[error("anti-bot page detected")]
    AntiBotDetected,
    #[error("SSRF: {0}")]
    Ssrf(String),
    #[error("HTTP {status}: {classification}")]
    HttpError { status: u16, classification: String },
    #[error("rate limited")]
    RateLimited,
}

#[trait_variant::make(Send)]
pub trait HttpFetcher: Send + Sync {
    async fn fetch(&self, req: FetchRequest) -> Result<FetchResponse, FetchError>;
    async fn fetch_ssrf_safe(&self, req: FetchRequest) -> Result<FetchResponse, FetchError>;
}
