//! Per-bucket circuit breaker for the outbound queue.
//!
//! The queue is the single transport-level gate: a bucket whose breaker is
//! Open dispatches nothing and callers get `FetchError::CircuitOpen` instead
//! of HTTP. The enrichment queue currently holds a separate, provider-keyed
//! breaker of the same shape; both run until the enrichment queue's transport
//! duties are retired.
//!
//! Design of record: `docs/metadata-remediation-phase3-queue-design.md`
//! (LOCKED v4).

use std::time::Duration;

use chrono::{DateTime, Utc};

use livrarr_domain::services::RateBucket;

/// Circuit breaker state for a provider/bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

/// Per-bucket circuit breaker configuration.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of failures within `evaluation_window_secs` that trips Closed → Open.
    pub failure_threshold: u32,
    /// Rolling window over which failures are counted.
    pub evaluation_window_secs: u64,
    /// How long the breaker stays Open before transitioning to HalfOpen.
    pub open_duration_secs: u64,
    /// In HalfOpen, allow this many probe attempts before deciding Open vs Closed.
    pub half_open_probe_count: u32,
}

/// Outcome reported to a bucket's breaker after a dispatched call completes
/// (R-8/R-12/R-14). `TripImmediately` bypasses the failure-threshold count for
/// a hard block (anti-bot interstitial, GB quota) — `open_for` overrides the
/// bucket's configured open duration when the caller computed a
/// provider-specific window (e.g. GB's Pacific-midnight reset); `None` uses
/// the bucket's configured `open_duration_secs`. The queue itself never
/// computes a provider-specific window — it stays timezone/provider-rule-free.
#[derive(Debug, Clone, Copy)]
pub enum BreakerSignal {
    Success,
    Failure,
    TripImmediately { open_for: Option<Duration> },
}

/// In-memory circuit breaker state machine for one breaker-tracked bucket.
/// Same window-prune / threshold-open / half-open-probe accounting as the
/// enrichment queue's breaker — the queue is now the single source of truth
/// for this decision.
#[derive(Debug)]
pub struct BreakerState {
    state: CircuitState,
    config: CircuitBreakerConfig,
    /// Recent failure timestamps inside `evaluation_window_secs`.
    recent_failures: Vec<DateTime<Utc>>,
    /// When the current Open window ends. Set at trip time from either the
    /// configured `open_duration_secs` or a `TripImmediately { open_for }`
    /// override.
    open_until: Option<DateTime<Utc>>,
    half_open_probes_taken: u32,
    half_open_probe_successes: u32,
}

impl BreakerState {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            state: CircuitState::Closed,
            config,
            recent_failures: Vec::new(),
            open_until: None,
            half_open_probes_taken: 0,
            half_open_probe_successes: 0,
        }
    }

    /// Snapshot the current state, transitioning Open → HalfOpen if the open
    /// window has elapsed.
    pub fn current(&mut self) -> CircuitState {
        if self.state == CircuitState::Open {
            if let Some(until) = self.open_until {
                if Utc::now() >= until {
                    self.state = CircuitState::HalfOpen;
                    self.half_open_probes_taken = 0;
                    self.half_open_probe_successes = 0;
                }
            }
        }
        self.state
    }

    /// Time remaining in the current Open window. Meaningful only right after
    /// `current()` returned `Open` — zero once the window has elapsed.
    pub fn retry_after(&self) -> Duration {
        self.open_until
            .map(|until| (until - Utc::now()).to_std().unwrap_or(Duration::ZERO))
            .unwrap_or(Duration::ZERO)
    }

    fn open(&mut self, open_for: Duration) {
        self.state = CircuitState::Open;
        self.open_until = Some(
            Utc::now()
                + chrono::Duration::from_std(open_for).unwrap_or_else(|_| {
                    chrono::Duration::seconds(self.config.open_duration_secs as i64)
                }),
        );
        self.recent_failures.clear();
        self.half_open_probes_taken = 0;
        self.half_open_probe_successes = 0;
    }

    fn record_success(&mut self) {
        self.recent_failures.clear();
        match self.state {
            CircuitState::Closed => {}
            CircuitState::Open => {
                // A success while Open shouldn't normally happen — the queue
                // doesn't dispatch during Open — but a stale report landing
                // late is treated as recovery rather than ignored.
                self.state = CircuitState::Closed;
                self.open_until = None;
            }
            CircuitState::HalfOpen => {
                self.half_open_probe_successes = self.half_open_probe_successes.saturating_add(1);
                self.half_open_probes_taken = self.half_open_probes_taken.saturating_add(1);
                if self.half_open_probes_taken >= self.config.half_open_probe_count
                    && self.half_open_probe_successes == self.half_open_probes_taken
                {
                    self.state = CircuitState::Closed;
                    self.open_until = None;
                }
            }
        }
    }

    fn record_failure(&mut self) {
        let now = Utc::now();
        self.recent_failures.push(now);
        let window = chrono::Duration::seconds(self.config.evaluation_window_secs as i64);
        self.recent_failures.retain(|t| now - *t <= window);

        match self.state {
            CircuitState::Closed => {
                if self.recent_failures.len() as u32 >= self.config.failure_threshold {
                    self.open(Duration::from_secs(self.config.open_duration_secs));
                }
            }
            CircuitState::HalfOpen => {
                self.open(Duration::from_secs(self.config.open_duration_secs));
            }
            CircuitState::Open => {}
        }
    }

    fn trip_immediately(&mut self, open_for: Option<Duration>) {
        self.open(open_for.unwrap_or_else(|| Duration::from_secs(self.config.open_duration_secs)));
    }

    /// Apply a reported outcome to the state machine.
    pub fn apply(&mut self, signal: BreakerSignal) {
        match signal {
            BreakerSignal::Success => self.record_success(),
            BreakerSignal::Failure => self.record_failure(),
            BreakerSignal::TripImmediately { open_for } => self.trip_immediately(open_for),
        }
    }
}

/// Per-bucket breaker configuration (R-1/R-9): default 5 failures / 60s
/// window / 60s open / 1 probe. Goodreads overrides `open_duration_secs` to
/// 3600s — an anti-bot-hostile scrape target where a tripped breaker should
/// stay quiet far longer than the API-backed providers. (Google Books'
/// Pacific-midnight quota reset arrives via `BreakerSignal::TripImmediately`'s
/// `open_for` override, computed by the client, not here.)
pub fn config_for(bucket: &RateBucket) -> CircuitBreakerConfig {
    let open_duration_secs = if matches!(bucket, RateBucket::Goodreads) {
        3600
    } else {
        60
    };
    CircuitBreakerConfig {
        failure_threshold: 5,
        evaluation_window_secs: 60,
        open_duration_secs,
        half_open_probe_count: 1,
    }
}

/// Breaker-tracked buckets: exactly the six single-host book-provider APIs.
/// Every other bucket — `None`, `Indexer(_)`, and any future bucket (e.g. a
/// cover-image bucket aggregating many hosts) — is pace-only: no breaker
/// state, never trips. The allowlist is deliberate: a new bucket must opt IN
/// to breaking, because a shared breaker over a multi-host bucket lets one
/// bad host suppress the rest.
pub fn breaker_tracked(bucket: &RateBucket) -> bool {
    matches!(
        bucket,
        RateBucket::OpenLibrary
            | RateBucket::Goodreads
            | RateBucket::Hardcover
            | RateBucket::GoogleBooks
            | RateBucket::Audnexus
            | RateBucket::Audible
    )
}
