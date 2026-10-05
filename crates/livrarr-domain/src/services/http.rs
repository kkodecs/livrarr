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
    /// A configured indexer's outbound traffic, split into two failure domains
    /// (issue #130). `origin` is the normalized upstream host
    /// (`scheme://host[:port]`) — the pacing and transport-breaker domain, so
    /// every indexer proxied through one Prowlarr host shares one pace lane and
    /// one "is the host up?" breaker. `indexer` is the stable DB id of the
    /// configured indexer row — the rate-limit-breaker domain, so a single
    /// indexer's 429 trips only its own cooldown and never blacks out its
    /// neighbours on the same host. `indexer` is `None` only for a release-file
    /// fetch where no indexer row is resolvable (renamed/ad-hoc): pacing and the
    /// transport gate still apply, but there is no per-indexer rate-limit
    /// breaker.
    Indexer {
        origin: String,
        indexer: Option<String>,
    },
    /// A Readarr import origin (Unit B3 Part 1). `origin` is the normalized
    /// upstream host (`livrarr_http::normalized_origin` —
    /// `scheme://host[:port]`) so every request against one Readarr instance
    /// (connect/preview/start, and every fetch inside one import run) shares
    /// one pace lane and one transport breaker. Single-level, unlike
    /// `Indexer` — there is no per-sub-resource rate-limit domain to split
    /// out here.
    Readarr {
        origin: String,
    },
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

/// Shows the URL through the log cleanser, each header by name with its value
/// masked, and the body as its length.
impl std::fmt::Debug for FetchRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        struct Masked;
        impl std::fmt::Debug for Masked {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("[REDACTED]")
            }
        }
        struct Headers<'a>(&'a [(String, String)]);
        impl std::fmt::Debug for Headers<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_list()
                    .entries(self.0.iter().map(|(name, _)| (name, Masked)))
                    .finish()
            }
        }
        struct BodyLength(usize);
        impl std::fmt::Debug for BodyLength {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{} bytes", self.0)
            }
        }
        f.debug_struct("FetchRequest")
            .field("url", &crate::redact::cleanse_log_line(&self.url))
            .field("method", &self.method)
            .field("headers", &Headers(&self.headers))
            .field("body", &self.body.as_ref().map(|b| BodyLength(b.len())))
            .field("timeout", &self.timeout)
            .field("rate_bucket", &self.rate_bucket)
            .field("max_body_bytes", &self.max_body_bytes)
            .field("anti_bot_check", &self.anti_bot_check)
            .field("user_agent", &self.user_agent)
            .field("priority", &self.priority)
            .finish()
    }
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
    /// The outbound queue's admission cap rejected this request — its
    /// bucket's pending queue is at (or past) the reserved threshold for
    /// this request's priority (D3: priority-reserved admission, never
    /// eviction). No HTTP was attempted; nothing already queued was
    /// disturbed. `retry_after` is a hint for when to retry.
    #[error("queue full, retry after {retry_after:?}")]
    QueueFull { retry_after: Duration },
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

    /// Fetch without following redirects — the raw response (status +
    /// headers, including `Location` on a 3xx) is returned so the caller can
    /// read a redirect target the auto-following client would otherwise
    /// chase, or — for a non-HTTP target like a `magnet:` URI — error on.
    /// Goes through the same pacing/breaker/UA path as `fetch`; only the
    /// underlying client's redirect policy differs.
    ///
    /// Defaulted to `fetch` so every existing implementor keeps today's
    /// behavior unless it opts in with a dedicated no-redirect client;
    /// `HttpFetcherImpl` is the only override. Pre-desugared for the same
    /// reason as `fetch_ssrf_safe_fast_connect` above.
    ///
    /// Plainly: the default body below calls `fetch`, so it FOLLOWS
    /// redirects — it does not suppress them. Only an implementor that
    /// overrides this method (like `HttpFetcherImpl`) actually stops at a
    /// redirect. A test double that needs no-redirect behavior (e.g. to
    /// return a 3xx with a `Location` header) must override
    /// `fetch_no_redirect` itself; overriding `fetch` alone is not enough.
    fn fetch_no_redirect(
        &self,
        req: FetchRequest,
    ) -> impl core::future::Future<Output = Result<FetchResponse, FetchError>> {
        async move { self.fetch(req).await }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_request_debug_names_headers_without_their_values_and_counts_the_body() {
        let body = br#"{"query":"BODYSECRETx7k2"}"#.to_vec();
        let length = body.len();
        let request = FetchRequest {
            url: "https://www.googleapis.com/books/v1/volumes?q=dune&api_key=URLSECRETx7k2"
                .to_string(),
            method: HttpMethod::Post,
            headers: vec![
                ("X-Goog-Api-Key".into(), "GBSECRETx7k2".into()),
                ("Authorization".into(), "Bearer HCSECRETx7k2".into()),
                ("X-Fixture-Header".into(), "PLAINVALUEx7k2".into()),
            ],
            body: Some(body.clone()),
            timeout: Duration::from_secs(9),
            rate_bucket: RateBucket::GoogleBooks,
            max_body_bytes: 4096,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
            priority: RequestPriority::Interactive,
        };
        let shown = format!("{request:?}");

        let mut failed = Vec::new();
        for name in ["X-Goog-Api-Key", "Authorization", "X-Fixture-Header"] {
            if !shown.contains(name) {
                failed.push(format!("header name {name} is shown"));
            }
        }
        for value in ["GBSECRETx7k2", "HCSECRETx7k2", "PLAINVALUEx7k2"] {
            if shown.contains(value) {
                failed.push(format!("header value {value} is not shown"));
            }
        }
        if shown.contains("URLSECRETx7k2") {
            failed.push("the URL's api_key value is not shown".into());
        }
        let bytes = format!("{body:?}");
        let hex: String = body.iter().map(|b| format!("{b:02x}")).collect();
        if shown.contains("BODYSECRETx7k2")
            || shown.contains(&bytes[1..bytes.len() - 1])
            || shown.contains(&hex)
        {
            failed.push("the body's bytes are not shown".into());
        }
        if !shown.contains(&length.to_string()) {
            failed.push(format!("the body's length {length} is shown"));
        }
        assert!(
            failed.is_empty(),
            "FetchRequest Debug: expected {failed:?}\nshown: {shown}"
        );
    }

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
