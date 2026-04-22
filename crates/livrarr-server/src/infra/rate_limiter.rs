use std::time::Duration;

// =============================================================================
// Token bucket shared inner state
// =============================================================================

pub(super) struct RateLimiterInner {
    pub tokens: f64,
    pub last_refill: std::time::Instant,
}

// =============================================================================
// OpenLibrary Rate Limiter — 3 req/sec, burst of 10
// =============================================================================

pub const OL_RATE: f64 = 3.0;
pub const OL_BURST: f64 = 10.0;

pub struct OlRateLimiter {
    state: tokio::sync::Mutex<RateLimiterInner>,
}

impl Default for OlRateLimiter {
    fn default() -> Self {
        Self {
            state: tokio::sync::Mutex::new(RateLimiterInner {
                tokens: OL_BURST,
                last_refill: std::time::Instant::now(),
            }),
        }
    }
}

impl OlRateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn acquire(&self) {
        loop {
            let mut inner = self.state.lock().await;
            let now = std::time::Instant::now();
            let elapsed = now.duration_since(inner.last_refill).as_secs_f64();
            inner.tokens = (inner.tokens + elapsed * OL_RATE).min(OL_BURST);
            inner.last_refill = now;

            if inner.tokens >= 1.0 {
                inner.tokens -= 1.0;
                return;
            }

            let wait = (1.0 - inner.tokens) / OL_RATE;
            drop(inner);
            tokio::time::sleep(Duration::from_secs_f64(wait)).await;
        }
    }
}

// =============================================================================
// Goodreads Rate Limiter — async-safe token bucket for outbound requests
// =============================================================================

/// Outbound rate limiter for Goodreads requests.
/// Token bucket: 1 token/second, burst of 5.
pub struct GoodreadsRateLimiter {
    state: tokio::sync::Mutex<RateLimiterInner>,
}

pub const GR_RATE: f64 = 1.0;
pub const GR_BURST: f64 = 5.0;

impl Default for GoodreadsRateLimiter {
    fn default() -> Self {
        Self {
            state: tokio::sync::Mutex::new(RateLimiterInner {
                tokens: GR_BURST,
                last_refill: std::time::Instant::now(),
            }),
        }
    }
}

impl GoodreadsRateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire a token, waiting if necessary. Never blocks the tokio runtime.
    pub async fn acquire(&self) {
        loop {
            let mut inner = self.state.lock().await;
            let now = std::time::Instant::now();
            let elapsed = now.duration_since(inner.last_refill).as_secs_f64();
            inner.tokens = (inner.tokens + elapsed * GR_RATE).min(GR_BURST);
            inner.last_refill = now;

            if inner.tokens >= 1.0 {
                inner.tokens -= 1.0;
                return;
            }

            let wait = (1.0 - inner.tokens) / GR_RATE;
            drop(inner);
            tracing::debug!(wait_secs = %format!("{wait:.2}"), "Goodreads rate limiter: waiting");
            tokio::time::sleep(Duration::from_secs_f64(wait)).await;
        }
    }
}
