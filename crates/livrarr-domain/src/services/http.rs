use std::time::Duration;

use crate::RequestPriority;

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
    GoogleBooks,
    Audible,
    /// Cover-image downloads from `covers.openlibrary.org` — paced (R-6) but
    /// deliberately NOT breaker-tracked (`breaker_tracked` in
    /// `livrarr-http/src/breaker.rs` is an explicit six-bucket allowlist that
    /// does not include this variant). A separate budget from `OpenLibrary`
    /// (M3): cover fetches must never draw against the OL book-metadata API's
    /// rate budget.
    OpenLibraryCovers,
    Indexer(String),
    None,
}

/// Select the pacing bucket for a cover-image host. Only
/// `covers.openlibrary.org` (case-insensitive) is paced — every other cover
/// host (gr-assets/amazon/google/hardcover/CdL, or a host that failed to
/// parse) deliberately stays on `RateBucket::None` (R-10: a global cover
/// bucket would make a 50-book import 150s+). Takes a host, not a URL —
/// callers that only have a URL do their own minimal host extraction and
/// fall back to `None` if that fails; the worst case of a wrong bucket here
/// is just unpaced, matching today's behavior.
pub fn cover_bucket_for_host(host: &str) -> RateBucket {
    if host.eq_ignore_ascii_case("covers.openlibrary.org") {
        RateBucket::OpenLibraryCovers
    } else {
        RateBucket::None
    }
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
    pub priority: RequestPriority,
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
    /// The outbound queue's per-bucket circuit breaker is Open for this
    /// request's `RateBucket` — no HTTP was attempted (R-3). `retry_after` is
    /// the time remaining until the breaker's open window elapses.
    #[error("circuit open, retry after {retry_after:?}")]
    CircuitOpen { retry_after: Duration },
}

#[trait_variant::make(Send)]
pub trait HttpFetcher: Send + Sync {
    async fn fetch(&self, req: FetchRequest) -> Result<FetchResponse, FetchError>;
    async fn fetch_ssrf_safe(&self, req: FetchRequest) -> Result<FetchResponse, FetchError>;

    /// Same as `fetch_ssrf_safe`, but the TCP-connect phase is bounded far
    /// tighter than `req.timeout` — for a caller that wants to fail fast
    /// against an unreachable host without shrinking the budget a slow-but-
    /// live host needs to finish downloading. The connect budget is an
    /// implementation constant owned by the fetcher, not caller-supplied —
    /// `req.timeout` still governs the rest of the request exactly as with
    /// `fetch_ssrf_safe`.
    ///
    /// Defaulted so every existing implementor keeps today's behavior
    /// unchanged unless it opts in by overriding this method; `HttpFetcherImpl`
    /// is the only override. The body is written pre-desugared (`fn` +
    /// `impl Future`, not `async fn`) because `trait_variant::make` rewrites
    /// an `async fn`'s signature but not its body — a default body written as
    /// `async fn` containing `.await` would no longer compile once its
    /// `asyncness` is stripped.
    fn fetch_ssrf_safe_fast_connect(
        &self,
        req: FetchRequest,
    ) -> impl core::future::Future<Output = Result<FetchResponse, FetchError>> {
        async move { self.fetch_ssrf_safe(req).await }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_bucket_for_host_matches_ol_covers_case_insensitively() {
        assert_eq!(
            cover_bucket_for_host("covers.openlibrary.org"),
            RateBucket::OpenLibraryCovers
        );
        assert_eq!(
            cover_bucket_for_host("COVERS.OPENLIBRARY.ORG"),
            RateBucket::OpenLibraryCovers
        );
        assert_eq!(
            cover_bucket_for_host("Covers.OpenLibrary.Org"),
            RateBucket::OpenLibraryCovers
        );
    }

    #[test]
    fn cover_bucket_for_host_other_and_malformed_hosts_are_none() {
        assert_eq!(cover_bucket_for_host("i.gr-assets.com"), RateBucket::None);
        assert_eq!(
            cover_bucket_for_host("images-na.ssl-images-amazon.com"),
            RateBucket::None
        );
        assert_eq!(
            cover_bucket_for_host("assets.hardcover.app"),
            RateBucket::None
        );
        assert_eq!(cover_bucket_for_host(""), RateBucket::None);
        assert_eq!(
            cover_bucket_for_host("not a valid host!!"),
            RateBucket::None
        );
        assert_eq!(
            cover_bucket_for_host("openlibrary.org"),
            RateBucket::None,
            "the book-metadata host must not accidentally match the cover host"
        );
    }
}
