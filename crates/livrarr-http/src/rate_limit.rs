//! Rate limit policy lookup per metadata provider.
//!
//! Satisfies: IMPL-HTTP-004

/// Provider classification for rate limiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Hardcover,
    Audnexus,
    Other,
}

/// Contract for rate limit configuration.
pub trait RateLimitContract {
    fn requests_per_second(&self, provider: ProviderKind) -> Option<f64>;
}

/// Default rate limiter with per-provider limits.
///
/// Hardcover: 1 req/s, Audnexus: 0.5 req/s, Other: unlimited.
pub struct DefaultRateLimiter;

impl RateLimitContract for DefaultRateLimiter {
    fn requests_per_second(&self, provider: ProviderKind) -> Option<f64> {
        match provider {
            ProviderKind::Hardcover => Some(1.0),
            ProviderKind::Audnexus => Some(0.5),
            ProviderKind::Other => None,
        }
    }
}
