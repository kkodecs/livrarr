//! `DefaultProviderQueue` — the centralized scatter-gather request queue (R-22).
//!
//! Responsibilities (covered by behavioral contract tests):
//!   - Parallel dispatch across applicable providers (`tokio::task::JoinSet`).
//!   - Per-provider circuit breaker (Closed / Open / HalfOpen).
//!   - Panic isolation — a provider task panic becomes a per-provider
//!     `PermanentFailure { ProviderPanic }` outcome. Other providers complete normally.
//!   - Durable phase-1 outcome persistence in `provider_retry_state` ([I-11]).
//!   - Retry budget — `attempts == max_attempts - 1` plus a fresh `WillRetry`
//!     dispatch converts to `PermanentFailure { RetryBudgetExhausted }`.
//!   - Suppression budget — same idea for `Suppressed` against
//!     `max_suppressed_passes` and `max_suppression_window_secs`.
//!   - Restart safety — providers with an existing phase-2 terminal retry-state
//!     row are skipped without being called.
//!   - Mode coercion — `Manual` and `HardRefresh` flip `WillRetry` and `Suppressed`
//!     to merge-eligible (`Conflict` always blocks).
//!   - Applicability — non-applicable providers are absent from outcomes entirely.
//!
//! Out of scope this session (deferred, no behavioral test):
//!   - `requests_per_second` pacing
//!   - Priority-class background slot reservation
//!   - Fair scheduling for concurrency=1 providers
//!   - Real-network provider client variants (only `StubProviderClient` exists)
//!
//! These are surfaced in `build/plan-metadata-overhaul.md` and land alongside
//! real-provider cutover.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use livrarr_db::{DbError, ProviderRetryStateDb};
use livrarr_domain::services::{CallOperation, CallOutcomeClass, ProviderCallRecord};
use livrarr_domain::{
    AnchorQuery, MetadataProvider, OutcomeClass, PermanentFailureReason, Work, WorkId,
};
use tokio::sync::{Mutex as TokioMutex, Semaphore};
use tokio::task::JoinSet;
use tracing::warn;

use crate::{
    CircuitBreakerConfig, CircuitState, EnrichmentContext, EnrichmentMode, NormalizedWorkDetail,
    ProviderOutcome, ProviderQueue, ProviderQueueConfig, ProviderQueueError, ScatterGatherResult,
    WillRetryReason,
};
use livrarr_external_data::provider_client::ProviderClient;

/// Initial circuit state for a provider. Used by `DefaultProviderQueueBuilder` to
/// inject a known state for behavioral tests (`CircuitStateSnapshot`,
/// `CircuitOpenSuppressedSkip`). Production startup defaults every provider to
/// `Closed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitialCircuitState {
    Closed,
    Open,
    HalfOpen,
}

impl From<InitialCircuitState> for CircuitState {
    fn from(s: InitialCircuitState) -> Self {
        match s {
            InitialCircuitState::Closed => Self::Closed,
            InitialCircuitState::Open => Self::Open,
            InitialCircuitState::HalfOpen => Self::HalfOpen,
        }
    }
}

/// REQ-006 anchor derivation: the anchor query each provider's enrichment
/// fetch uses, from the work's stored anchors. Empty/whitespace values count
/// as absent. Hardcover prefers ISBN (the working by-key path — see the HcKey
/// gap note in provider_client.rs); OpenLibrary prefers its own key.
fn derive_anchor_query(provider: MetadataProvider, work: &Work) -> Option<AnchorQuery> {
    fn present(v: &Option<String>) -> Option<String> {
        v.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }
    match provider {
        MetadataProvider::GoogleBooks => present(&work.isbn_13).map(AnchorQuery::Isbn13),
        MetadataProvider::Goodreads => present(&work.gr_key).map(AnchorQuery::GrKey),
        MetadataProvider::Hardcover => present(&work.isbn_13)
            .map(AnchorQuery::Isbn13)
            .or_else(|| present(&work.hc_key).map(AnchorQuery::HcKey)),
        MetadataProvider::OpenLibrary => present(&work.ol_key)
            .map(AnchorQuery::OlKey)
            .or_else(|| present(&work.isbn_13).map(AnchorQuery::Isbn13)),
        MetadataProvider::Audnexus | MetadataProvider::Audible => {
            present(&work.asin).map(AnchorQuery::Asin)
        }
        // Never scatter providers; no anchor surface exists for them.
        MetadataProvider::Llm | MetadataProvider::Readarr => None,
    }
}

/// Pipeline-level skip/pacing record (REQ-001): emitted by the queue when it
/// decides not to call (or not to pace through) a provider, since no client
/// call happens that could record itself.
fn record_queue_skip(
    sink: &Option<Arc<dyn livrarr_domain::services::ProviderCallSink>>,
    provider: MetadataProvider,
    work_id: WorkId,
    outcome: CallOutcomeClass,
    detail: Option<&str>,
) {
    if let Some(sink) = sink {
        sink.record(ProviderCallRecord {
            provider: provider.record_key().to_string(),
            operation: CallOperation::Enrich,
            work_id: Some(work_id),
            started_at: Utc::now(),
            duration_ms: 0,
            outcome,
            detail: detail.map(str::to_string),
        });
    }
}

/// Pluggable applicability check. The queue calls this once per (provider, work)
/// at dispatch time; non-applicable providers are absent from `ScatterGatherResult.outcomes`
/// and never invoked.
pub type ApplicabilityRule = Arc<dyn Fn(MetadataProvider, &Work) -> bool + Send + Sync>;

/// In-memory circuit breaker state for a single provider.
#[derive(Debug)]
struct BreakerState {
    state: CircuitState,
    config: CircuitBreakerConfig,
    /// Recent failure timestamps inside `evaluation_window_secs`.
    recent_failures: Vec<DateTime<Utc>>,
    /// When the breaker last transitioned to Open. Used to gate Open → HalfOpen.
    opened_at: Option<DateTime<Utc>>,
    /// Number of probe attempts taken in the current HalfOpen window.
    half_open_probes_taken: u32,
    /// Number of probe successes in the current HalfOpen window.
    half_open_probe_successes: u32,
}

impl BreakerState {
    fn new(state: CircuitState, config: CircuitBreakerConfig) -> Self {
        let opened_at = match state {
            CircuitState::Open => Some(Utc::now()),
            _ => None,
        };
        Self {
            state,
            config,
            recent_failures: Vec::new(),
            opened_at,
            half_open_probes_taken: 0,
            half_open_probe_successes: 0,
        }
    }

    /// Snapshot the current state, transitioning Open → HalfOpen if the open
    /// duration has elapsed.
    fn current(&mut self) -> CircuitState {
        if self.state == CircuitState::Open {
            if let Some(opened) = self.opened_at {
                if Utc::now() - opened
                    >= chrono::Duration::seconds(self.config.open_duration_secs as i64)
                {
                    self.state = CircuitState::HalfOpen;
                    self.half_open_probes_taken = 0;
                    self.half_open_probe_successes = 0;
                }
            }
        }
        self.state
    }

    fn record_success(&mut self) {
        self.recent_failures.clear();
        match self.state {
            CircuitState::Closed => {}
            CircuitState::Open => {
                // A success while Open shouldn't normally happen (we don't dispatch),
                // but if it does, reset to Closed.
                self.state = CircuitState::Closed;
                self.opened_at = None;
            }
            CircuitState::HalfOpen => {
                self.half_open_probe_successes = self.half_open_probe_successes.saturating_add(1);
                self.half_open_probes_taken = self.half_open_probes_taken.saturating_add(1);
                if self.half_open_probes_taken >= self.config.half_open_probe_count
                    && self.half_open_probe_successes == self.half_open_probes_taken
                {
                    self.state = CircuitState::Closed;
                    self.opened_at = None;
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
                    self.state = CircuitState::Open;
                    self.opened_at = Some(now);
                }
            }
            CircuitState::HalfOpen => {
                self.state = CircuitState::Open;
                self.opened_at = Some(now);
                self.half_open_probes_taken = 0;
                self.half_open_probe_successes = 0;
            }
            CircuitState::Open => {}
        }
    }
}

/// GCRA-based rate limiter — used per-provider to enforce `requests_per_second`.
///
/// Pure scheduling: each caller gets a unique send time. Supports burst capacity
/// and rejects requests that would queue beyond max_queue_time (prevents sleep bombs).
#[derive(Debug)]
struct TokenBucket {
    inner: TokioMutex<GcraInner>,
    interval: Duration,
    max_burst_time: Duration,
    max_queue_time: Duration,
}

#[derive(Debug)]
struct GcraInner {
    tat: Instant,
}

impl TokenBucket {
    fn new(rate_per_sec: f64, burst: f64) -> Self {
        let interval = if rate_per_sec > 0.0 {
            Duration::from_secs_f64(1.0 / rate_per_sec)
        } else {
            Duration::ZERO
        };
        Self {
            inner: TokioMutex::new(GcraInner {
                tat: Instant::now(),
            }),
            interval,
            max_burst_time: interval.saturating_mul(burst as u32),
            max_queue_time: Duration::from_secs(30),
        }
    }

    async fn acquire(&self) -> Result<(), ()> {
        if self.interval.is_zero() {
            return Ok(());
        }
        let wait = {
            let mut inner = self.inner.lock().await;
            let now = Instant::now();

            // Decay TAT: allow burst by clamping how far in the past TAT can be.
            let tat = if inner.tat < now - self.max_burst_time {
                now - self.max_burst_time
            } else {
                inner.tat
            };

            let send_at = tat + self.interval;
            let wait_time = send_at.saturating_duration_since(now);

            if wait_time > self.max_queue_time {
                tracing::debug!(
                    "rate limiter: rejecting request (queue time {wait_time:?} exceeds max)"
                );
                inner.tat = tat;
                return Err(());
            }

            inner.tat = send_at;
            wait_time
        };
        if wait > Duration::ZERO {
            tokio::time::sleep(wait).await;
        }
        Ok(())
    }
}

/// Per-provider configuration registered with the queue.
struct ProviderEntry {
    client: ProviderClient,
    config: ProviderQueueConfig,
    breaker: Arc<std::sync::RwLock<BreakerState>>,
    /// Token bucket throttle. Allows `config.requests_per_second` calls per second
    /// with a burst of one second's worth (minimum 1). When `requests_per_second`
    /// is 0 or negative, acquire is a no-op (no throttling).
    rate_limiter: Arc<TokenBucket>,
    /// Per-provider concurrency cap. `acquire().await` blocks new dispatches when
    /// `config.concurrency` calls are already in flight. Permits are released
    /// when the spawned task completes (or panics — JoinSet drops the future,
    /// which drops the OwnedSemaphorePermit).
    concurrency: Arc<Semaphore>,
}

/// Builder for `DefaultProviderQueue`. The behavioral test harness uses this to
/// register one stub client per scenario; production wiring uses the same builder
/// to register real-network clients (in a follow-on session).
pub struct DefaultProviderQueueBuilder {
    providers: HashMap<MetadataProvider, ProviderEntry>,
    applicability: Option<ApplicabilityRule>,
    call_sink: Option<Arc<dyn livrarr_domain::services::ProviderCallSink>>,
}

impl Default for DefaultProviderQueueBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultProviderQueueBuilder {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            applicability: None,
            call_sink: None,
        }
    }

    /// Inject the call-record sink (REQ-001): the queue records pipeline-level
    /// skips (no anchor, policy) through it — no client call happens for those.
    pub fn with_call_sink(
        mut self,
        sink: Arc<dyn livrarr_domain::services::ProviderCallSink>,
    ) -> Self {
        self.call_sink = Some(sink);
        self
    }

    pub fn add_provider(
        mut self,
        provider: MetadataProvider,
        client: ProviderClient,
        config: ProviderQueueConfig,
    ) -> Self {
        let breaker = BreakerState::new(CircuitState::Closed, config.circuit_breaker.clone());
        // Burst of one second's worth of requests, minimum 1, so a queue
        // configured at 0.5 req/s still accepts a single request immediately.
        let burst = config.requests_per_second.max(1.0);
        let rate_limiter = Arc::new(TokenBucket::new(config.requests_per_second, burst));
        let concurrency_cap = config.concurrency.max(1) as usize;
        let concurrency = Arc::new(Semaphore::new(concurrency_cap));
        self.providers.insert(
            provider,
            ProviderEntry {
                client,
                config,
                breaker: Arc::new(std::sync::RwLock::new(breaker)),
                rate_limiter,
                concurrency,
            },
        );
        self
    }

    /// Inject an initial circuit state for a registered provider. Must be called
    /// after `add_provider` for that provider. Preserves the entry's existing
    /// rate limiter + concurrency semaphore.
    pub fn with_initial_circuit_state(
        mut self,
        provider: MetadataProvider,
        state: InitialCircuitState,
    ) -> Self {
        if let Some(entry) = self.providers.get_mut(&provider) {
            let breaker = BreakerState::new(state.into(), entry.config.circuit_breaker.clone());
            entry.breaker = Arc::new(std::sync::RwLock::new(breaker));
        }
        self
    }

    pub fn with_applicability_rule(mut self, rule: ApplicabilityRule) -> Self {
        self.applicability = Some(rule);
        self
    }

    pub fn build<DB>(self, retry_db: Arc<DB>) -> DefaultProviderQueue<DB>
    where
        DB: ProviderRetryStateDb + Send + Sync + 'static,
    {
        let applicability = self
            .applicability
            .unwrap_or_else(|| Arc::new(|_provider, _work| true));
        DefaultProviderQueue {
            providers: Arc::new(self.providers),
            applicability,
            retry_db,
            call_sink: self.call_sink,
        }
    }
}

/// Centralized scatter-gather provider request queue. See module-level docs.
pub struct DefaultProviderQueue<DB>
where
    DB: ProviderRetryStateDb + Send + Sync + 'static,
{
    providers: Arc<HashMap<MetadataProvider, ProviderEntry>>,
    applicability: ApplicabilityRule,
    retry_db: Arc<DB>,
    #[allow(dead_code)] // read at green: REQ-006 skip records via REQ-001 sink
    call_sink: Option<Arc<dyn livrarr_domain::services::ProviderCallSink>>,
}

/// Outcome of one provider's phase-1 dispatch, before terminal-budget conversion
/// and durable persistence.
enum DispatchedOutcome {
    /// Provider client returned an outcome normally.
    Returned(ProviderOutcome<NormalizedWorkDetail>),
    /// Provider client task panicked.
    Panicked,
}

/// Read existing terminal state for restart safety. None = no row, or row is non-terminal.
async fn existing_terminal_outcome<DB: ProviderRetryStateDb + Send + Sync>(
    db: &DB,
    user_id: livrarr_domain::UserId,
    work_id: WorkId,
    provider: MetadataProvider,
) -> Result<Option<OutcomeClass>, DbError> {
    let state = db.get_retry_state(user_id, work_id, provider).await?;
    Ok(state
        .and_then(|s| s.last_outcome)
        .filter(|o| o.is_phase2_terminal()))
}

impl<DB> ProviderQueue for DefaultProviderQueue<DB>
where
    DB: ProviderRetryStateDb + Send + Sync + 'static,
{
    async fn dispatch_enrichment(
        &self,
        work: &Work,
        context: EnrichmentContext,
    ) -> Result<ScatterGatherResult, ProviderQueueError> {
        let mut outcomes: HashMap<MetadataProvider, ProviderOutcome<NormalizedWorkDetail>> =
            HashMap::new();

        // Partition providers into: skip (not applicable / anchor-less /
        // restart-resumed), suppress-due-to-open-circuit, and dispatch. The
        // dispatch tuple carries clones of the per-provider rate limiter +
        // concurrency semaphore so the spawned task can acquire them
        // independently, plus the derived anchor query (REQ-006).
        struct DispatchEntry {
            provider: MetadataProvider,
            client: ProviderClient,
            config: ProviderQueueConfig,
            rate_limiter: Arc<TokenBucket>,
            concurrency: Arc<Semaphore>,
            anchor: AnchorQuery,
        }
        let mut to_dispatch: Vec<DispatchEntry> = Vec::new();
        let mut suppressed_open: Vec<(MetadataProvider, ProviderQueueConfig)> = Vec::new();

        for (provider, entry) in self.providers.iter() {
            let provider = *provider;

            if !(self.applicability)(provider, work) {
                // Policy skip (e.g. the language applicability rule): recorded
                // by this layer since no client call happens (REQ-001).
                record_queue_skip(
                    &self.call_sink,
                    provider,
                    work.id,
                    CallOutcomeClass::SkippedPolicy,
                    Some("not_applicable"),
                );
                continue;
            }

            // REQ-006: enrichment fetches only by stored anchor. No anchor for
            // this provider → no fetch, a SkippedNoAnchor record, a NotFound
            // outcome (anchor acquisition is the identity track's job).
            let Some(anchor) = derive_anchor_query(provider, work) else {
                record_queue_skip(
                    &self.call_sink,
                    provider,
                    work.id,
                    CallOutcomeClass::SkippedNoAnchor,
                    None,
                );
                outcomes.insert(provider, ProviderOutcome::NotFound);
                continue;
            };

            // Restart safety: skip if the row is already terminal.
            if existing_terminal_outcome(self.retry_db.as_ref(), work.user_id, work.id, provider)
                .await?
                .is_some()
            {
                continue;
            }

            let breaker_state = entry.breaker.write().unwrap().current();
            if breaker_state == CircuitState::Open {
                suppressed_open.push((provider, entry.config.clone()));
                continue;
            }

            to_dispatch.push(DispatchEntry {
                provider,
                client: entry.client.clone(),
                config: entry.config.clone(),
                rate_limiter: entry.rate_limiter.clone(),
                concurrency: entry.concurrency.clone(),
                anchor,
            });
        }

        // Phase 1: scatter — spawn each provider call. Panic isolation via JoinSet.
        // Each spawned task waits on the per-provider concurrency semaphore + token
        // bucket BEFORE invoking the anchor-grounded fetch. Permit drops on task
        // return / panic.
        let mut set: JoinSet<(MetadataProvider, DispatchedOutcome)> = JoinSet::new();
        let language = work.language.clone();
        let dispatch_work_id = work.id;
        for d in &to_dispatch {
            let provider = d.provider;
            let client = d.client.clone();
            let rate_limiter = d.rate_limiter.clone();
            let concurrency = d.concurrency.clone();
            let anchor = d.anchor.clone();
            let language = language.clone();
            let sink = self.call_sink.clone();
            set.spawn(async move {
                // Concurrency permit first (held for the full call duration).
                let _permit = concurrency.acquire_owned().await;
                // Then rate-limit token (token bucket pacing).
                if rate_limiter.acquire().await.is_err() {
                    // The fetch never happened — the queue records the pacing
                    // rejection (clients record only real calls).
                    record_queue_skip(
                        &sink,
                        provider,
                        dispatch_work_id,
                        CallOutcomeClass::RateLimited,
                        Some("queue_pacing_rejected"),
                    );
                    return (
                        provider,
                        DispatchedOutcome::Returned(ProviderOutcome::WillRetry {
                            reason: WillRetryReason::RateLimit,
                            next_attempt_at: chrono::Utc::now() + chrono::Duration::seconds(60),
                        }),
                    );
                }
                let outcome = client.fetch_by_anchor(anchor, language.as_deref()).await;
                (provider, DispatchedOutcome::Returned(outcome))
            });
        }

        // Phase 1: gather — collect outcomes, mapping panics to ProviderPanic.
        let mut dispatched: HashMap<MetadataProvider, DispatchedOutcome> = HashMap::new();
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((provider, outcome)) => {
                    dispatched.insert(provider, outcome);
                }
                Err(join_err) if join_err.is_panic() => {
                    // Recover the provider id by id() — we can't, JoinError doesn't
                    // expose the provider tag. Use the task id we wrapped earlier.
                    // Workaround: panicked tasks need a separate path. Spawn with
                    // metadata wasn't possible above, so we rebuild using JoinHandle
                    // tracking. Since we can't recover the provider here, we mark
                    // any missing providers as panicked at the end of the gather phase.
                    warn!("provider task panicked (id mapping resolved post-gather)");
                }
                Err(join_err) => {
                    warn!("provider task join error (non-panic): {join_err}");
                }
            }
        }

        // Reconcile: any to_dispatch provider that didn't show up in `dispatched`
        // panicked or was canceled — treat as ProviderPanic per IR.
        for d in &to_dispatch {
            dispatched
                .entry(d.provider)
                .or_insert(DispatchedOutcome::Panicked);
        }

        // For each dispatched outcome, apply budget rules and persist phase-1
        // state durably ([I-11]). Then build the in-memory result outcome.
        for d in &to_dispatch {
            let provider = d.provider;
            let raw = dispatched
                .remove(&provider)
                .expect("dispatched entry must exist after reconciliation");

            let final_outcome = match raw {
                DispatchedOutcome::Panicked => ProviderOutcome::PermanentFailure {
                    reason: PermanentFailureReason::ProviderPanic,
                },
                DispatchedOutcome::Returned(outcome) => {
                    self.apply_budget_rules(work, provider, &d.config, outcome)
                        .await?
                }
            };

            // Durable persistence.
            self.persist_phase1_outcome(work, provider, &final_outcome)
                .await?;

            // Update circuit breaker.
            let breaker = self.providers.get(&provider).unwrap().breaker.clone();
            let mut bs = breaker.write().unwrap();
            match &final_outcome {
                ProviderOutcome::Success(_) | ProviderOutcome::NotFound => bs.record_success(),
                ProviderOutcome::WillRetry { .. } | ProviderOutcome::PermanentFailure { .. } => {
                    bs.record_failure()
                }
                // NotConfigured, Conflict, and Suppressed neither count as a clean
                // success nor a straightforward provider failure for breaker arithmetic.
                ProviderOutcome::NotConfigured
                | ProviderOutcome::Conflict { .. }
                | ProviderOutcome::Suppressed { .. } => {}
            }

            outcomes.insert(provider, final_outcome);
        }

        // Open-circuit suppression: producer skipped the call entirely. Record
        // Suppressed in DB and outcomes map.
        for (provider, config) in suppressed_open {
            let until = Utc::now()
                + chrono::Duration::seconds(config.circuit_breaker.open_duration_secs as i64);
            self.retry_db
                .record_suppressed(work.user_id, work.id, provider, until)
                .await?;
            outcomes.insert(provider, ProviderOutcome::Suppressed { until });
        }

        let conflict_present = outcomes
            .values()
            .any(|o| matches!(o, ProviderOutcome::Conflict { .. }));
        let merge_eligible = !conflict_present;
        let deferred = if conflict_present {
            false
        } else {
            match context.mode {
                EnrichmentMode::Background => outcomes.values().any(|o| !o.can_merge()),
                EnrichmentMode::Manual | EnrichmentMode::HardRefresh => false,
            }
        };

        Ok(ScatterGatherResult {
            work_id: work.id,
            outcomes,
            merge_eligible,
            deferred,
        })
    }

    fn circuit_state(&self, provider: MetadataProvider) -> CircuitState {
        let entry = match self.providers.get(&provider) {
            Some(e) => e,
            None => return CircuitState::Closed,
        };
        entry.breaker.read().unwrap().state
    }
}

impl<DB> DefaultProviderQueue<DB>
where
    DB: ProviderRetryStateDb + Send + Sync + 'static,
{
    /// Apply retry/suppression budget conversion. Reads existing retry-state row
    /// to know prior `attempts` / `suppressed_passes` / `first_suppressed_at`.
    async fn apply_budget_rules(
        &self,
        work: &Work,
        provider: MetadataProvider,
        config: &ProviderQueueConfig,
        outcome: ProviderOutcome<NormalizedWorkDetail>,
    ) -> Result<ProviderOutcome<NormalizedWorkDetail>, ProviderQueueError> {
        match outcome {
            ProviderOutcome::WillRetry {
                reason,
                next_attempt_at,
            } => {
                // R-11: a breaker-open pass is a PAUSE (the provider is
                // temporarily down), never a step toward a retry-budget
                // dead-end — it must consume neither the attempt nor the
                // suppression budget. Return it unchanged.
                if reason == WillRetryReason::CircuitOpen {
                    return Ok(ProviderOutcome::WillRetry {
                        reason,
                        next_attempt_at,
                    });
                }
                let prior = self
                    .retry_db
                    .get_retry_state(work.user_id, work.id, provider)
                    .await?;
                let prior_attempts = prior.as_ref().map(|s| s.attempts).unwrap_or(0);
                if prior_attempts.saturating_add(1) >= config.max_attempts {
                    Ok(ProviderOutcome::PermanentFailure {
                        reason: PermanentFailureReason::RetryBudgetExhausted,
                    })
                } else {
                    Ok(ProviderOutcome::WillRetry {
                        reason,
                        next_attempt_at,
                    })
                }
            }
            ProviderOutcome::Suppressed { until } => {
                let prior = self
                    .retry_db
                    .get_retry_state(work.user_id, work.id, provider)
                    .await?;
                let prior_suppressed = prior.as_ref().map(|s| s.suppressed_passes).unwrap_or(0);
                let prior_window_start = prior.as_ref().and_then(|s| s.first_suppressed_at);

                let budget_exhausted =
                    prior_suppressed.saturating_add(1) >= config.max_suppressed_passes;
                let window_elapsed = prior_window_start
                    .map(|start| {
                        Utc::now() - start
                            >= chrono::Duration::seconds(config.max_suppression_window_secs as i64)
                    })
                    .unwrap_or(false);

                if budget_exhausted || window_elapsed {
                    Ok(ProviderOutcome::PermanentFailure {
                        reason: PermanentFailureReason::SuppressionExhausted,
                    })
                } else {
                    Ok(ProviderOutcome::Suppressed { until })
                }
            }
            other => Ok(other),
        }
    }

    /// Persist the per-provider phase-1 outcome to `provider_retry_state` ([I-11]).
    /// Success outcomes carry `normalized_payload_json`; non-Success terminal
    /// outcomes clear it.
    async fn persist_phase1_outcome(
        &self,
        work: &Work,
        provider: MetadataProvider,
        outcome: &ProviderOutcome<NormalizedWorkDetail>,
    ) -> Result<(), ProviderQueueError> {
        match outcome {
            ProviderOutcome::Success(payload) => {
                let json = serde_json::to_string(payload.as_ref())
                    .expect("NormalizedWorkDetail is always JSON-serializable");
                self.retry_db
                    .record_terminal_outcome(
                        work.user_id,
                        work.id,
                        provider,
                        OutcomeClass::Success,
                        Some(json),
                    )
                    .await?;
            }
            ProviderOutcome::NotFound => {
                self.retry_db
                    .record_terminal_outcome(
                        work.user_id,
                        work.id,
                        provider,
                        OutcomeClass::NotFound,
                        None,
                    )
                    .await?;
            }
            ProviderOutcome::NotConfigured => {
                self.retry_db
                    .record_terminal_outcome(
                        work.user_id,
                        work.id,
                        provider,
                        OutcomeClass::NotConfigured,
                        None,
                    )
                    .await?;
            }
            ProviderOutcome::PermanentFailure { .. } => {
                self.retry_db
                    .record_terminal_outcome(
                        work.user_id,
                        work.id,
                        provider,
                        OutcomeClass::PermanentFailure,
                        None,
                    )
                    .await?;
            }
            ProviderOutcome::Conflict { .. } => {
                self.retry_db
                    .record_terminal_outcome(
                        work.user_id,
                        work.id,
                        provider,
                        OutcomeClass::Conflict,
                        None,
                    )
                    .await?;
            }
            ProviderOutcome::WillRetry {
                reason,
                next_attempt_at,
            } => {
                // R-11: a breaker-open pass persists via `record_will_retry_paused`
                // (same row shape, `attempts` NOT incremented) — a paused provider
                // must not spend retry budget while its breaker is open.
                if *reason == WillRetryReason::CircuitOpen {
                    self.retry_db
                        .record_will_retry_paused(work.user_id, work.id, provider, *next_attempt_at)
                        .await?;
                } else {
                    self.retry_db
                        .record_will_retry(work.user_id, work.id, provider, *next_attempt_at)
                        .await?;
                }
            }
            ProviderOutcome::Suppressed { until } => {
                self.retry_db
                    .record_suppressed(work.user_id, work.id, provider, *until)
                    .await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod circuit_open_budget_tests {
    //! R-11: a breaker-open `WillRetry { CircuitOpen }` must never convert to
    //! `PermanentFailure` at the retry-attempt-budget boundary, and its
    //! persistence must never bump `attempts` (see the `record_will_retry_paused`
    //! db-level tests in `livrarr-db`). This is the one spot budget conversion
    //! happens (`apply_budget_rules`), driven end-to-end through
    //! `dispatch_enrichment` with a scripted `StubProviderClient`.

    use std::sync::Arc;

    use livrarr_db::{
        CreateUserDbRequest, CreateWorkDbRequest, ProviderRetryStateDb, UserDb, WorkDbCreate,
    };
    use livrarr_domain::{MetadataProvider, RequestPriority, UserRole, WillRetryReason};
    use livrarr_external_data::{ProviderClient, ProviderOutcome, StubProviderClient};

    use crate::provider_queue::DefaultProviderQueueBuilder;
    use crate::{
        CircuitBreakerConfig, EnrichmentContext, EnrichmentMode, ProviderQueue, ProviderQueueConfig,
    };

    fn config(max_attempts: u32) -> ProviderQueueConfig {
        ProviderQueueConfig {
            provider: MetadataProvider::OpenLibrary,
            concurrency: 1,
            requests_per_second: 0.0,
            circuit_breaker: CircuitBreakerConfig {
                failure_threshold: 5,
                evaluation_window_secs: 60,
                open_duration_secs: 60,
                half_open_probe_count: 1,
            },
            max_attempts,
            max_suppressed_passes: 3,
            max_suppression_window_secs: 3600,
        }
    }

    async fn seed_db_and_work() -> (livrarr_db::sqlite::SqliteDb, livrarr_domain::Work) {
        let db = livrarr_db::create_test_db().await;
        let user_id = db
            .create_user(CreateUserDbRequest {
                username: "circuit_open_budget_user".to_string(),
                password_hash: "hash".to_string(),
                role: UserRole::Admin,
                api_key_hash: "apikey".to_string(),
            })
            .await
            .unwrap()
            .id;
        let (work, _) = db
            .create_work(CreateWorkDbRequest {
                user_id,
                title: "Budget Book".to_string(),
                author_name: "Budget Author".to_string(),
                // OpenLibrary's REQ-006 anchor gate requires ol_key or isbn_13
                // before the queue will dispatch to the client at all.
                ol_key: Some("OL1W".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        (db, work)
    }

    /// Prior REAL retries (non-CircuitOpen) parked one short of the
    /// `max_attempts` boundary — the next WillRetry{ServerError} pass would
    /// normally convert to PermanentFailure{RetryBudgetExhausted}. A
    /// WillRetry{CircuitOpen} pass at the exact same boundary must NOT.
    #[tokio::test]
    async fn will_retry_circuit_open_survives_the_max_attempts_boundary() {
        let (db, work) = seed_db_and_work().await;
        let max_attempts = 3;
        for _ in 0..(max_attempts - 1) {
            db.record_will_retry(
                work.user_id,
                work.id,
                MetadataProvider::OpenLibrary,
                chrono::Utc::now() + chrono::Duration::seconds(60),
            )
            .await
            .unwrap();
        }
        let prior = db
            .get_retry_state(work.user_id, work.id, MetadataProvider::OpenLibrary)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(prior.attempts, max_attempts - 1);

        let client = ProviderClient::Stub(StubProviderClient::new(
            MetadataProvider::OpenLibrary,
            ProviderOutcome::WillRetry {
                reason: WillRetryReason::CircuitOpen,
                next_attempt_at: chrono::Utc::now() + chrono::Duration::seconds(60),
            },
        ));
        let db = Arc::new(db);
        let queue = DefaultProviderQueueBuilder::new()
            .add_provider(MetadataProvider::OpenLibrary, client, config(max_attempts))
            .build(db.clone());

        let ctx = EnrichmentContext {
            priority: RequestPriority::Normal,
            mode: EnrichmentMode::Background,
        };
        let result = queue.dispatch_enrichment(&work, ctx).await.unwrap();
        let outcome = result.outcomes.get(&MetadataProvider::OpenLibrary).unwrap();

        assert!(
            matches!(
                outcome,
                ProviderOutcome::WillRetry {
                    reason: WillRetryReason::CircuitOpen,
                    ..
                }
            ),
            "a breaker-open pass at the max_attempts boundary must stay WillRetry{{CircuitOpen}}, \
             not convert to PermanentFailure — got {outcome:?}"
        );

        // record_will_retry_paused must have been used, not record_will_retry:
        // the prior attempts count is untouched by the CircuitOpen pass.
        let after = db
            .get_retry_state(work.user_id, work.id, MetadataProvider::OpenLibrary)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            after.attempts,
            max_attempts - 1,
            "a breaker-open pass must not increment attempts"
        );
    }
}
