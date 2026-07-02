//! `DefaultProviderQueue` — the centralized scatter-gather request queue (R-22).
//!
//! Responsibilities (covered by behavioral contract tests):
//!   - Parallel dispatch across applicable providers (`tokio::task::JoinSet`).
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
//! Pacing, per-provider circuit breaking, and concurrency capping live at the
//! outbound queue (`livrarr_http::outbound_queue`), which paces and caps every
//! HTTP call regardless of caller. This queue does not pace or breaker-gate
//! dispatch itself — a call that needs to wait, waits at the outbound queue.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use livrarr_db::{DbError, ProviderRetryStateDb};
use livrarr_domain::services::{CallOperation, CallOutcomeClass, ProviderCallRecord};
use livrarr_domain::{
    AnchorQuery, MetadataProvider, OutcomeClass, PermanentFailureReason, Work, WorkId,
};
use tokio::task::JoinSet;
use tracing::warn;

use crate::{
    EnrichmentContext, EnrichmentMode, NormalizedWorkDetail, ProviderOutcome, ProviderQueue,
    ProviderQueueConfig, ProviderQueueError, ScatterGatherResult, WillRetryReason,
};
use livrarr_external_data::provider_client::ProviderClient;

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

/// Pipeline-level skip record (REQ-001): emitted by the queue when it decides
/// not to call a provider, since no client call happens that could record
/// itself.
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

/// Per-provider configuration registered with the queue.
struct ProviderEntry {
    client: ProviderClient,
    config: ProviderQueueConfig,
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
        self.providers
            .insert(provider, ProviderEntry { client, config });
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
        // restart-resumed) and dispatch. The dispatch tuple carries the
        // derived anchor query (REQ-006).
        struct DispatchEntry {
            provider: MetadataProvider,
            client: ProviderClient,
            config: ProviderQueueConfig,
            anchor: AnchorQuery,
        }
        let mut to_dispatch: Vec<DispatchEntry> = Vec::new();

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

            to_dispatch.push(DispatchEntry {
                provider,
                client: entry.client.clone(),
                config: entry.config.clone(),
                anchor,
            });
        }

        // Phase 1: scatter — spawn each provider call. Panic isolation via JoinSet.
        // Pacing, concurrency capping, and circuit breaking happen at the outbound
        // queue (every HTTP call routes through it); this layer only dispatches.
        let mut set: JoinSet<(MetadataProvider, DispatchedOutcome)> = JoinSet::new();
        let language = work.language.clone();
        let priority = context.priority;
        for d in &to_dispatch {
            let provider = d.provider;
            let client = d.client.clone();
            let anchor = d.anchor.clone();
            let language = language.clone();
            set.spawn(async move {
                let outcome = client
                    .fetch_by_anchor(anchor, language.as_deref(), priority)
                    .await;
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

            outcomes.insert(provider, final_outcome);
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
    use crate::{EnrichmentContext, EnrichmentMode, ProviderQueue, ProviderQueueConfig};

    fn config(max_attempts: u32) -> ProviderQueueConfig {
        ProviderQueueConfig {
            provider: MetadataProvider::OpenLibrary,
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
