//! livrarr-enrichment — the metadata merge/enrich engine.
//!
//! Extracted from livrarr-metadata (4a): the provider queue, cross-provider
//! validation, the field-merge engine, cover selection/gating, and the
//! `EnrichmentServiceImpl` spine. Depends only on domain/db/external-data/http;
//! it must not depend on livrarr-metadata or livrarr-identity.

use std::collections::HashMap;
use std::sync::Arc;

use livrarr_domain::{
    ApplyMergeOutcome, DbError, EnrichmentStatus, RequestPriority, UserId, WillRetryReason, Work,
    WorkId,
};
use livrarr_external_data::{NormalizedWorkDetail, ProviderOutcome};

pub mod cover_gate;
pub mod cover_rank;
pub mod cover_resolution;
mod merge_engine;
pub mod provider_queue;

#[cfg(test)]
mod provider_queue_tracer_tests;

pub use merge_engine::{
    build_apply_request, DefaultMergeEngine, MergeEngine, MergeError, MergeInput, MergeOutput,
    PriorityModel,
};
pub use provider_queue::{ApplicabilityRule, DefaultProviderQueue, DefaultProviderQueueBuilder};

/// No-op `LlmCaller` used as the default `L` type parameter for
/// `EnrichmentServiceImpl` when no LLM is configured. Relocated here from
/// `livrarr-metadata::work_service` to keep the default in-crate (an
/// enrichment->metadata reference would be a forbidden back-edge).
pub struct StubNoLlm;

impl livrarr_domain::services::LlmCaller for StubNoLlm {
    async fn call(
        &self,
        _req: livrarr_domain::services::LlmCallRequest,
    ) -> Result<livrarr_domain::services::LlmCallResponse, livrarr_domain::services::LlmError> {
        Err(livrarr_domain::services::LlmError::NotConfigured)
    }
}

#[trait_variant::make(Send)]
pub trait EnrichmentService: Send + Sync {
    /// The one enrichment road (REQ-001). `candidate_id` (metadata-refactor R-001):
    /// when `Some`, the pipeline consumes that picked candidate's cached discovery
    /// payloads (zero network) before gateway-fetching the rest; `None` = enrich
    /// from the network.
    /// `priority` (B4) drives the `EnrichmentContext` handed to
    /// `ProviderQueue::dispatch_enrichment` — independent of `mode`.
    /// `freshness` (REQ-009) decides whether provider fetches may be served
    /// from the persistent provider-response cache — orthogonal to `priority`.
    async fn enrich_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
        mode: EnrichmentMode,
        candidate_id: Option<livrarr_domain::identity::CandidateId>,
        priority: RequestPriority,
        freshness: livrarr_domain::Freshness,
    ) -> Result<EnrichmentResult, EnrichmentError>;

    /// TEMP(pk-tdd): compile-only scaffold — reset work for manual refresh.
    async fn reset_for_manual_refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<(), EnrichmentError>;

    /// Pre-inject source provider data before calling `enrich_work`.
    /// The data is consumed during scatter-gather as a Readarr provider outcome.
    ///
    /// `EnrichmentServiceImpl` overrides this; stubs provide a no-op.
    async fn inject_source_data(
        &self,
        user_id: UserId,
        work_id: WorkId,
        data: livrarr_domain::services::SourceProviderData,
    );

    /// Thread one identity-edit preview fetch down to the queue's client
    /// registry (identity-edit r4 §Preview seam). Desugared stub default:
    /// `NotConfigured` for doubles without a queue.
    fn preview_fetch(
        &self,
        provider: livrarr_domain::MetadataProvider,
        query: livrarr_domain::AnchorQuery,
        language: Option<String>,
        priority: RequestPriority,
    ) -> impl std::future::Future<Output = PreviewFetchOutcome> + Send {
        let _ = (provider, query, language, priority);
        async move { PreviewFetchOutcome::NotConfigured }
    }
}

#[derive(Debug, Clone)]
pub struct EnrichmentResult {
    pub enrichment_status: EnrichmentStatus,
    pub enrichment_source: Option<String>,
    pub llm_task_spawned: bool,
    pub work: Work,
    /// TEMP(pk-tdd): compile-only scaffold — deferred when not all outcomes are phase-2 terminal.
    pub merge_deferred: bool,
    /// TEMP(pk-tdd): compile-only scaffold — per-provider outcome classes.
    pub provider_outcomes: HashMap<livrarr_domain::MetadataProvider, livrarr_domain::OutcomeClass>,
    pub cover_resolution: Option<livrarr_domain::CoverResolution>,
    pub audiobook_cover_resolution: Option<livrarr_domain::CoverResolution>,
    /// Seam-2 signal: the merge detected a per-provider Conflict — identity
    /// could not be confirmed. Propagated to
    /// `domain::services::EnrichmentResult.identity_not_found`; the caller
    /// writes `IdentityStatus::NotFound`. Enrichment never writes identity.
    pub identity_not_found: bool,
    /// True when the merge actually changed any work field, external ID, or
    /// cover resolution. Drives the materialize gate in the wrapper
    /// (REQ-012): cover download + retag runs only when changed=true.
    pub changed: bool,
    /// Per-field/per-provider dissents recorded by this merge (REQ-014):
    /// excluded contributions, persisted queryably; never block the merge.
    pub dissents: Vec<livrarr_domain::FieldDissent>,
    /// True when the pass ran to completion — a provider dispatch or a
    /// cached-candidate merge concluded (including a no-op merge). False only
    /// when the pass ended with no provider dispatch and no merge application.
    /// Authored by `enrich_work`'s own control flow; the cached-reuse arm
    /// returns an empty `provider_outcomes` map on a completed attempt, so
    /// emptiness is NOT a usable no-attempt signal.
    pub attempted: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum EnrichmentError {
    #[error("work not found")]
    WorkNotFound,
    #[error("database error: {0}")]
    Db(#[from] DbError),
    #[error("all providers failed")]
    AllProvidersFailed,
    /// TEMP(pk-tdd): compile-only scaffold — queue dispatch failed.
    #[error("provider queue error: {0}")]
    Queue(#[from] ProviderQueueError),
    /// TEMP(pk-tdd): compile-only scaffold — merge engine error.
    #[error("merge error: {0}")]
    Merge(#[from] MergeError),
    /// TEMP(pk-tdd): compile-only scaffold — CAS exhausted after max retries.
    #[error("merge superseded after max retries")]
    MergeSuperseded,
    /// TEMP(pk-tdd): compile-only scaffold — persisted retry payload is corrupt.
    #[error("corrupt retry payload for work {work_id} provider {provider:?}")]
    CorruptRetryPayload {
        work_id: WorkId,
        provider: livrarr_domain::MetadataProvider,
    },
}

// =============================================================================
// TEMP(pk-tdd): compile-only scaffolding for metadata-overhaul behavioral tests.
// All types below are stubs — implement when metadata-overhaul is coded.
// =============================================================================

/// TEMP(pk-tdd): enrichment mode — background, manual, or hard-refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrichmentMode {
    Background,
    Manual,
    HardRefresh,
}

/// TEMP(pk-tdd): output of scatter-gather provider dispatch.
#[derive(Debug, Clone)]
pub struct ScatterGatherResult {
    pub work_id: WorkId,
    pub outcomes: HashMap<livrarr_domain::MetadataProvider, ProviderOutcome<NormalizedWorkDetail>>,
    pub merge_eligible: bool,
    pub deferred: bool,
}

/// TEMP(pk-tdd): context passed to ProviderQueue::dispatch_enrichment.
#[derive(Debug, Clone)]
pub struct EnrichmentContext {
    pub priority: RequestPriority,
    pub mode: EnrichmentMode,
    /// REQ-009: whether this dispatch may satisfy provider fetches from the
    /// persistent provider-response cache (D-004 — orthogonal to `priority`).
    pub freshness: livrarr_domain::Freshness,
}

/// Per-provider queue configuration.
///
/// R-22
#[derive(Debug, Clone)]
pub struct ProviderQueueConfig {
    pub provider: livrarr_domain::MetadataProvider,
    pub max_attempts: u32,
}

/// Queue infrastructure error. Provider-level failures and panics become per-provider
/// `ProviderOutcome` variants in `ScatterGatherResult` rather than queue-level errors.
///
/// R-22
#[derive(Debug, thiserror::Error)]
pub enum ProviderQueueError {
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

/// Outcome of one identity-edit preview fetch (identity-edit r4 §Preview
/// seam). Lives at the queue layer because both the provider payload and the
/// client registry do; the domain leaf sees only the mapped preview record.
#[derive(Debug, Clone)]
pub enum PreviewFetchOutcome {
    Resolved(Box<NormalizedWorkDetail>),
    NotFound,
    NotConfigured,
    /// Retryable outage or permanent fetch failure — nothing certifiable.
    Unavailable,
}

/// Shared per-provider request queue. Scatter-gather dispatch with durable
/// phase-1 outcome persistence.
///
/// R-22
#[trait_variant::make(Send)]
pub trait ProviderQueue: Send + Sync {
    async fn dispatch_enrichment(
        &self,
        work: &Work,
        context: EnrichmentContext,
    ) -> Result<ScatterGatherResult, ProviderQueueError>;

    /// One preview fetch against the named provider's client by anchor query
    /// (identity-edit r4): a direct `fetch_by_anchor` — NO provider-response
    /// cache read or write (that cache's single seam stays
    /// `dispatch_enrichment`), NO `provider_retry_state` writes, no budget
    /// bookkeeping. Call records emit at the client wrapper (truthful HTTP).
    ///
    /// Desugared stub default (`trait_variant` cannot expand a provided
    /// `async fn`): a queue double with no registry reports `NotConfigured`.
    fn preview_fetch(
        &self,
        provider: livrarr_domain::MetadataProvider,
        query: livrarr_domain::AnchorQuery,
        language: Option<String>,
        priority: RequestPriority,
    ) -> impl std::future::Future<Output = PreviewFetchOutcome> + Send {
        let _ = (provider, query, language, priority);
        async move { PreviewFetchOutcome::NotConfigured }
    }
}

/// TEMP(pk-tdd): reconstructed per-provider outcome for merge input.
#[derive(Debug, Clone)]
pub struct ReconstructedOutcome {
    pub class: livrarr_domain::OutcomeClass,
    pub payload: Option<NormalizedWorkDetail>,
}

// =============================================================================
// REQ-011: resolve_status — determine the correct EnrichmentStatus after merge
// =============================================================================

/// Classify the final enrichment status after a merge (REQ-011):
/// - `Enriched` when the merge produced ≥1 usable field (the merge engine
///   already sets this via `has_meaningful_text`, so trust its verdict when
///   Enriched or Thin).
/// - `Thin` when ≥1 provider responded with a Success outcome (including empty
///   payloads) but the merge produced no usable text.
/// - `Failed` when NO provider returned a Success outcome — all were
///   NotConfigured, WillRetry, PermanentFailure, or NotFound. This
///   is the transient "try later" state; the background job will retry.
///
/// The merge engine's own `enrichment_status` already handles Enriched/Thin
/// correctly; this function only overrides to `Failed` when appropriate.
/// (REQ-014: the whole-work Conflict outcome is retired — status is computed
/// from surviving contributions; conflicted providers are dissent-isolated.)
fn resolve_status(
    merge_status: EnrichmentStatus,
    provider_results: &HashMap<livrarr_domain::MetadataProvider, ReconstructedOutcome>,
) -> EnrichmentStatus {
    if merge_status == EnrichmentStatus::Enriched {
        return EnrichmentStatus::Enriched;
    }
    // If ANY provider had a Success outcome (even an empty one), the work is at
    // most Thin — we know the book, we just found no useful metadata.
    let any_success = provider_results
        .values()
        .any(|o| o.class == livrarr_domain::OutcomeClass::Success);
    if any_success {
        EnrichmentStatus::Thin
    } else {
        EnrichmentStatus::Failed
    }
}

/// REQ-002 (work-history): "did this pass change the work" — content fields
/// only. The apply's bookkeeping writes (status/source/enriched_at) don't
/// count: the enriched event carries status separately, and a pass that only
/// re-stamped bookkeeping changed nothing the user can see. Compared on the
/// persisted rows because `MergeOutput.work_update` is a last-known-good echo
/// (`Some` on every eligible merge) — presence is not a change signal.
fn content_changed(before: &Work, after: &Work) -> bool {
    let mut before = before.clone();
    before.enrichment_status = after.enrichment_status;
    before.enriched_at = after.enriched_at;
    before.enrichment_source = after.enrichment_source.clone();
    before != *after
}

// =============================================================================
// Candidate-reuse anchor matching (relocated from livrarr-metadata::work_service)
// =============================================================================

/// Revalidate cached per-provider payloads against the work's confirmed anchors
/// (D-005): require no contradiction AND at least one positive anchor overlap,
/// so a stale or colliding `candidate_id` falls back to network enrichment
/// instead of applying unrelated payloads.
fn cached_payloads_match_work(
    work: &Work,
    payloads: &HashMap<
        livrarr_domain::MetadataProvider,
        livrarr_external_data::NormalizedWorkDetail,
    >,
) -> bool {
    // No payload may contradict a confirmed anchor on the work.
    let no_contradiction = payloads.values().all(|p| {
        payload_anchor_compatible(work.ol_key.as_deref(), p.ol_key.as_deref())
            && payload_anchor_compatible(work.gr_key.as_deref(), p.gr_key.as_deref())
            && payload_anchor_compatible(work.hc_key.as_deref(), p.hc_key.as_deref())
            && payload_anchor_compatible(work.isbn_13.as_deref(), p.isbn_13.as_deref())
            && payload_anchor_compatible(work.asin.as_deref(), p.asin.as_deref())
    });
    // At least one payload must positively share a matching anchor, so an
    // anchorless work or vacuously empty payload set cannot pass.
    let positive_match = payloads.values().any(|p| {
        payload_anchors_match(work.ol_key.as_deref(), p.ol_key.as_deref())
            || payload_anchors_match(work.gr_key.as_deref(), p.gr_key.as_deref())
            || payload_anchors_match(work.hc_key.as_deref(), p.hc_key.as_deref())
            || payload_anchors_match(work.isbn_13.as_deref(), p.isbn_13.as_deref())
            || payload_anchors_match(work.asin.as_deref(), p.asin.as_deref())
    });
    no_contradiction && positive_match
}

/// True when both anchors are present AND equal (positive overlap).
fn payload_anchors_match(work_anchor: Option<&str>, payload_anchor: Option<&str>) -> bool {
    matches!((work_anchor, payload_anchor), (Some(a), Some(b)) if a == b)
}

/// Compatible = either anchor is absent, OR both are present and equal.
/// A work anchor of "OL123" vs payload anchor of "OL456" is a contradiction.
fn payload_anchor_compatible(work_anchor: Option<&str>, payload_anchor: Option<&str>) -> bool {
    match (work_anchor, payload_anchor) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    }
}

// =============================================================================
// Per-work lock infrastructure
// =============================================================================

/// Per-work lock map type [I-12].
type PerWorkLocks = tokio::sync::Mutex<HashMap<(UserId, WorkId), Arc<tokio::sync::Mutex<()>>>>;

/// On drop, schedules a sweep of the PerWorkLocks map that removes entries
/// whose only remaining reference is the map itself (orphaned per-work mutexes).
/// Without this, the map grows unboundedly across enrichment calls.
struct SweepLocksOnDrop {
    locks: Arc<PerWorkLocks>,
}

impl Drop for SweepLocksOnDrop {
    fn drop(&mut self) {
        let locks = self.locks.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut lock_map = locks.lock().await;
                lock_map.retain(|_, arc| Arc::strong_count(arc) > 1);
            });
        }
    }
}

/// Source data pending injection for a specific (user_id, work_id) pair.
type SourceDataStore =
    tokio::sync::Mutex<HashMap<(UserId, WorkId), livrarr_domain::services::SourceProviderData>>;

/// Stamp the applied merge's generation onto the dissent rows and persist
/// them (REQ-014). An empty batch clears stale rows from earlier generations
/// (a clean re-merge resolves prior dissents). Persistence failures are
/// logged, never propagated — dissent bookkeeping must not fail an applied
/// merge. Returns the stamped rows for the caller's result.
async fn persist_dissents<DB: livrarr_db::FieldDissentDb>(
    db: &DB,
    user_id: UserId,
    work_id: WorkId,
    merge_generation: i64,
    mut dissents: Vec<livrarr_domain::FieldDissent>,
) -> Vec<livrarr_domain::FieldDissent> {
    for d in &mut dissents {
        d.merge_generation = merge_generation;
    }
    if let Err(e) = db
        .record_field_dissents(user_id, work_id, dissents.clone())
        .await
    {
        tracing::warn!(work_id, error = %e, "failed to persist merge dissents");
    }
    dissents
}

/// Enrichment service implementation.
/// Generic over DB, Q (ProviderQueue), and ME (MergeEngine).
pub struct EnrichmentServiceImpl<DB, Q, ME> {
    db: Arc<DB>,
    queue: Arc<Q>,
    merge_engine: Arc<ME>,
    /// Per-work lock map [I-12]: serializes concurrent enrichment calls for the same (user_id, work_id).
    locks: Arc<PerWorkLocks>,
    /// Pre-injected source provider data (e.g., from Readarr import).
    /// Set via `pre_inject_source_data` before calling `enrich_work`.
    source_data_store: Arc<SourceDataStore>,
    /// Optional transport cache: holds per-provider payloads the identity
    /// resolver fetched during discovery. When `candidate_id` is `Some` and
    /// the cache has an entry, the merge runs without any network dispatch
    /// (AC-001 / REQ-014/015). Set via `with_transport_cache`; `None` in
    /// contexts where no resolver is composed (tests, CLI tools).
    transport_cache: Option<Arc<livrarr_external_data::transport_cache::TransportCache>>,
    /// Fire-and-forget instrumentation sink (REQ-001). Records emitted here
    /// are pipeline-level (cache-served payloads); per-network-call records
    /// come from the clients and the queue. `None` in compositions that don't
    /// record (tests, CLI tools).
    call_sink: Option<Arc<dyn livrarr_domain::services::ProviderCallSink>>,
}

impl<DB, Q, ME> EnrichmentServiceImpl<DB, Q, ME>
where
    DB: livrarr_db::WorkDb
        + livrarr_db::ProvenanceDb
        + livrarr_db::ProviderRetryStateDb
        + livrarr_db::ExternalIdDb
        + livrarr_db::FieldDissentDb
        + Send
        + Sync
        + 'static,
    Q: ProviderQueue + Send + Sync + 'static,
    ME: MergeEngine + Send + Sync + 'static,
{
    /// `_llm_configured` is retained for call-site compatibility — the merge is
    /// purely deterministic (REQ-005), so the flag has no effect on behavior.
    pub fn new(db: Arc<DB>, queue: Arc<Q>, merge_engine: Arc<ME>, _llm_configured: bool) -> Self {
        Self {
            db,
            queue,
            merge_engine,
            locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            source_data_store: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            transport_cache: None,
            call_sink: None,
        }
    }

    /// Wire the provider-call instrumentation sink (REQ-001). The server
    /// composition root sets this; `None` (the default) records nothing.
    pub fn with_call_sink(
        mut self,
        sink: Arc<dyn livrarr_domain::services::ProviderCallSink>,
    ) -> Self {
        self.call_sink = Some(sink);
        self
    }

    /// Wire the transport cache (produced by the identity resolver at composition
    /// root). When set, `enrich_work` can reuse cached per-provider payloads for
    /// a `candidate_id` hit instead of re-dispatching providers (REQ-014/015).
    pub fn with_transport_cache(
        mut self,
        tc: Arc<livrarr_external_data::transport_cache::TransportCache>,
    ) -> Self {
        self.transport_cache = Some(tc);
        self
    }

    /// Pre-inject source provider data before calling `enrich_work`.
    /// The data is consumed during scatter-gather and appended as a Readarr provider outcome.
    pub async fn pre_inject_source_data(
        &self,
        user_id: UserId,
        work_id: WorkId,
        data: livrarr_domain::services::SourceProviderData,
    ) {
        let mut store = self.source_data_store.lock().await;
        store.insert((user_id, work_id), data);
    }
}

impl<DB, Q, ME> EnrichmentService for EnrichmentServiceImpl<DB, Q, ME>
where
    DB: livrarr_db::WorkDb
        + livrarr_db::ProvenanceDb
        + livrarr_db::ProviderRetryStateDb
        + livrarr_db::ExternalIdDb
        + livrarr_db::FieldDissentDb
        + Send
        + Sync
        + 'static,
    Q: ProviderQueue + Send + Sync + 'static,
    ME: MergeEngine + Send + Sync + 'static,
{
    async fn preview_fetch(
        &self,
        provider: livrarr_domain::MetadataProvider,
        query: livrarr_domain::AnchorQuery,
        language: Option<String>,
        priority: RequestPriority,
    ) -> PreviewFetchOutcome {
        self.queue
            .preview_fetch(provider, query, language, priority)
            .await
    }

    async fn enrich_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
        mode: EnrichmentMode,
        candidate_id: Option<livrarr_domain::identity::CandidateId>,
        priority: RequestPriority,
        freshness: livrarr_domain::Freshness,
    ) -> Result<EnrichmentResult, EnrichmentError> {
        let _enrich_span = livrarr_domain::perf::StageTimer::start("enrich", work_id);
        // Step 1: Acquire per-work lock [I-12]
        let _sweep = SweepLocksOnDrop {
            locks: self.locks.clone(),
        };
        let per_work_lock = {
            let mut lock_map = self.locks.lock().await;
            lock_map
                .entry((user_id, work_id))
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _guard = per_work_lock.lock().await;

        // Step 2: Read current work from DB
        let work = self
            .db
            .get_work(user_id, work_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => EnrichmentError::WorkNotFound,
                other => EnrichmentError::Db(other),
            })?;

        // Step 2.5: Candidate reuse (REQ-014/015 / AC-001) — zero-network path.
        // If candidate_id has cached discovery payloads that revalidate against
        // the work's anchors, merge them in-process and return. ANY miss (no
        // candidate, no cache, TTL expiry, anchor mismatch) OR DB error yields
        // `None` and falls cleanly through to the network path below — it never
        // proceeds with empty provenance (which would drop user field-locks) or
        // a fabricated CAS generation.
        let reuse_result: Option<EnrichmentResult> = async {
            let cid = candidate_id.as_ref()?;
            let tc = self.transport_cache.as_ref()?;
            let payloads = tc.cache_take(user_id, cid.clone())?;
            if !cached_payloads_match_work(&work, &payloads) {
                tracing::warn!(
                    work_id,
                    "candidate reuse: payloads do not match work anchors — falling back to network"
                );
                return None;
            }
            // REQ-001: each cache-served payload consumed by this merge is a
            // recorded fetch attempt (outcome Cached).
            if let Some(sink) = &self.call_sink {
                let started_at = chrono::Utc::now();
                for provider in payloads.keys() {
                    sink.record(livrarr_domain::services::ProviderCallRecord {
                        provider: provider.record_key().to_string(),
                        operation: livrarr_domain::services::CallOperation::Enrich,
                        work_id: Some(work_id),
                        started_at,
                        duration_ms: 0,
                        outcome: livrarr_domain::services::CallOutcomeClass::Cached,
                        detail: None,
                    });
                }
            }
            // Snapshot generation + provenance for CAS correctness; a DB read
            // failure here returns None → network fallback (never empty
            // provenance, which would silently drop user field-locks).
            let generation = self.db.get_merge_generation(user_id, work_id).await.ok()?;
            let current_provenance = self.db.list_work_provenance(user_id, work_id).await.ok()?;
            let merge_output = self
                .merge_engine
                .merge_from_cached(
                    work.clone(),
                    payloads,
                    current_provenance,
                    work.language.as_deref(),
                )
                .await
                .ok()?;
            let apply_req = build_apply_request(&merge_output, user_id, work_id, generation);
            match self.db.apply_enrichment_merge(apply_req).await.ok()? {
                ApplyMergeOutcome::Applied
                | ApplyMergeOutcome::NoChange
                | ApplyMergeOutcome::Deferred => {
                    // The merge applied at `generation`; apply_enrichment_merge
                    // bumps by one, so its dissent rows carry generation + 1.
                    let dissents = persist_dissents(
                        self.db.as_ref(),
                        user_id,
                        work_id,
                        generation + 1,
                        merge_output.dissents,
                    )
                    .await;
                    let result_work = self.db.get_work(user_id, work_id).await.ok()?;
                    let changed = content_changed(&work, &result_work)
                        || merge_output.cover_resolution.is_some()
                        || merge_output.audiobook_cover_resolution.is_some();
                    Some(EnrichmentResult {
                        enrichment_status: merge_output.enrichment_status,
                        enrichment_source: merge_output.enrichment_source,
                        llm_task_spawned: false,
                        work: result_work,
                        merge_deferred: false,
                        // No provider_outcomes on the cached-reuse path.
                        provider_outcomes: HashMap::new(),
                        cover_resolution: merge_output.cover_resolution,
                        audiobook_cover_resolution: merge_output.audiobook_cover_resolution,
                        // The merge no longer signals identity (REQ-014);
                        // identity_not_found keeps its identity-track sources only.
                        identity_not_found: false,
                        changed,
                        dissents,
                        attempted: true,
                    })
                }
                ApplyMergeOutcome::Superseded => {
                    tracing::warn!(
                        work_id,
                        "candidate reuse: CAS superseded — falling back to network"
                    );
                    None
                }
            }
        }
        .await;
        if let Some(result) = reuse_result {
            return Ok(result);
        }

        // Step 3: Read merge_generation before dispatch (for CAS baseline)
        let mut generation = self.db.get_merge_generation(user_id, work_id).await?;

        // Step 4: Dispatch to provider queue. `priority` is the caller's
        // explicit queue-ordering hint (B4) — it no longer hardcodes Normal.
        let context = EnrichmentContext {
            priority,
            mode,
            freshness,
        };
        let mut scatter_result = self.queue.dispatch_enrichment(&work, context).await?;

        // Step 4.5: Append source provider data (Readarr import) if pre-injected.
        // Treated as an additional provider outcome — merge engine arbitrates field selection.
        {
            let mut store = self.source_data_store.lock().await;
            if let Some(source_data) = store.remove(&(user_id, work_id)) {
                let normalized: NormalizedWorkDetail = source_data.into();
                scatter_result.outcomes.insert(
                    livrarr_domain::MetadataProvider::Readarr,
                    ProviderOutcome::Success(Box::new(normalized)),
                );
            }
        }

        // Step 5: Re-read current work after dispatch (TOCTOU safety — content freshness)
        let mut current_work = self.db.get_work(user_id, work_id).await?;

        // Step 6: Re-read current provenance after dispatch
        let mut current_provenance = self.db.list_work_provenance(user_id, work_id).await?;

        // Build provider_outcomes for the result (always returned regardless of merge path)
        let provider_outcomes: HashMap<
            livrarr_domain::MetadataProvider,
            livrarr_domain::OutcomeClass,
        > = scatter_result
            .outcomes
            .iter()
            .map(|(p, o)| (*p, o.class()))
            .collect();

        // Step 7: Check if merge should be deferred
        let merge_deferred = scatter_result.deferred && mode == EnrichmentMode::Background;

        // In Background mode with deferred outcomes, skip the merge entirely
        let should_merge = !merge_deferred;

        if !should_merge {
            // Return early with deferred result, no merge
            let result_work = self.db.get_work(user_id, work_id).await?;
            return Ok(EnrichmentResult {
                enrichment_status: result_work.enrichment_status,
                enrichment_source: result_work.enrichment_source.clone(),
                llm_task_spawned: false,
                work: result_work,
                merge_deferred,
                provider_outcomes,
                cover_resolution: None,
                audiobook_cover_resolution: None,
                identity_not_found: false,
                changed: false,
                dissents: Vec::new(),
                attempted: true,
            });
        }

        // Step 8: Build ReconstructedOutcome from ScatterGatherResult
        // For Success outcomes, read back normalized_payload_json from DB retry state
        let mut reconstructed: HashMap<livrarr_domain::MetadataProvider, ReconstructedOutcome> =
            HashMap::new();

        for (provider, outcome) in &scatter_result.outcomes {
            match outcome {
                ProviderOutcome::Success(in_mem) => {
                    // Read back the persisted payload from DB.
                    // Scattered providers always have a retry-state row with
                    // `normalized_payload_json` (written by `persist_phase1_outcome`).
                    // Non-scattered providers (e.g. Readarr import) carry their
                    // payload only in memory — the DB lookup returns None and we
                    // fall back to the in-memory value so they contribute to the merge.
                    let retry_state = self.db.get_retry_state(user_id, work_id, *provider).await?;
                    let payload =
                        if let Some(ref state) = retry_state {
                            if let Some(ref json) = state.normalized_payload_json {
                                let detail: NormalizedWorkDetail = serde_json::from_str(json)
                                    .map_err(|_| EnrichmentError::CorruptRetryPayload {
                                        work_id,
                                        provider: *provider,
                                    })?;
                                Some(detail)
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                    // Fall back to the in-memory payload when the DB has no row.
                    // Preserves restart-safety for scattered providers (their DB row
                    // is always found); lets non-scattered providers (Readarr and any
                    // future additions) reach the merge without special-casing.
                    let payload = payload.or_else(|| Some((**in_mem).clone()));
                    reconstructed.insert(
                        *provider,
                        ReconstructedOutcome {
                            class: livrarr_domain::OutcomeClass::Success,
                            payload,
                        },
                    );
                }
                ProviderOutcome::NotFound => {
                    reconstructed.insert(
                        *provider,
                        ReconstructedOutcome {
                            class: livrarr_domain::OutcomeClass::NotFound,
                            payload: None,
                        },
                    );
                }
                ProviderOutcome::NotConfigured => {
                    reconstructed.insert(
                        *provider,
                        ReconstructedOutcome {
                            class: livrarr_domain::OutcomeClass::NotConfigured,
                            payload: None,
                        },
                    );
                }
                ProviderOutcome::WillRetry { .. } => {
                    reconstructed.insert(
                        *provider,
                        ReconstructedOutcome {
                            class: livrarr_domain::OutcomeClass::WillRetry,
                            payload: None,
                        },
                    );
                }
                ProviderOutcome::PermanentFailure { .. } => {
                    reconstructed.insert(
                        *provider,
                        ReconstructedOutcome {
                            class: livrarr_domain::OutcomeClass::PermanentFailure,
                            payload: None,
                        },
                    );
                }
                ProviderOutcome::Conflict { .. } => {
                    reconstructed.insert(
                        *provider,
                        ReconstructedOutcome {
                            class: livrarr_domain::OutcomeClass::Conflict,
                            payload: None,
                        },
                    );
                }
            }
        }

        // Step 8.5 removed (REQ-005): LLM identity validation no longer runs in
        // the pipeline. Per-provider Conflict outcomes are dissent-isolated by
        // the merge (REQ-014); identity_not_found keeps its identity-track
        // sources only.

        // Determine priority model based on work language
        let priority_model = PriorityModel::for_language(current_work.language.as_deref());

        // Step 9: CAS retry loop — max 3 attempts
        const MAX_CAS_ATTEMPTS: usize = 3;
        for attempt in 0..MAX_CAS_ATTEMPTS {
            let merge_input = MergeInput {
                current_work: current_work.clone(),
                current_provenance: current_provenance.clone(),
                provider_results: reconstructed.clone(),
                mode,
                priority_model: priority_model.clone(),
            };

            let mut merge_output = self.merge_engine.merge(merge_input).await?;

            // REQ-011: apply resolve_status before persisting — the DB must store
            // the same status that is returned to the caller.
            merge_output.enrichment_status =
                resolve_status(merge_output.enrichment_status, &reconstructed);

            let apply_req = build_apply_request(&merge_output, user_id, work_id, generation);

            let apply_outcome = self.db.apply_enrichment_merge(apply_req).await?;

            match apply_outcome {
                ApplyMergeOutcome::Applied
                | ApplyMergeOutcome::NoChange
                | ApplyMergeOutcome::Deferred => {
                    // Success — build result. The merge applied at `generation`;
                    // apply_enrichment_merge bumps by one, so its dissent rows
                    // carry generation + 1.
                    let dissents = persist_dissents(
                        self.db.as_ref(),
                        user_id,
                        work_id,
                        generation + 1,
                        merge_output.dissents,
                    )
                    .await;
                    let result_work = self.db.get_work(user_id, work_id).await?;
                    let changed = content_changed(&current_work, &result_work)
                        || merge_output.cover_resolution.is_some()
                        || merge_output.audiobook_cover_resolution.is_some();
                    return Ok(EnrichmentResult {
                        enrichment_status: merge_output.enrichment_status,
                        enrichment_source: merge_output.enrichment_source,
                        llm_task_spawned: false,
                        work: result_work,
                        merge_deferred,
                        provider_outcomes,
                        cover_resolution: merge_output.cover_resolution,
                        audiobook_cover_resolution: merge_output.audiobook_cover_resolution,
                        // The merge no longer signals identity (REQ-014);
                        // identity_not_found keeps its identity-track sources only.
                        identity_not_found: false,
                        changed,
                        dissents,
                        // A pass whose scatter dispatched nothing (every
                        // provider terminal-skipped, no injected source
                        // payload) and whose merge changed nothing never
                        // attempted anything — REQ-002's "an attempt that
                        // never runs records nothing".
                        attempted: !scatter_result.outcomes.is_empty() || changed,
                    });
                }
                ApplyMergeOutcome::Superseded => {
                    if attempt + 1 >= MAX_CAS_ATTEMPTS {
                        return Err(EnrichmentError::MergeSuperseded);
                    }
                    // Re-read work, generation, and provenance for retry
                    current_work = self.db.get_work(user_id, work_id).await?;
                    generation = self.db.get_merge_generation(user_id, work_id).await?;
                    current_provenance = self.db.list_work_provenance(user_id, work_id).await?;
                }
            }
        }

        Err(EnrichmentError::MergeSuperseded)
    }

    async fn reset_for_manual_refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<(), EnrichmentError> {
        // Acquire per-work lock [I-12] — serializes with enrich_work
        let _sweep = SweepLocksOnDrop {
            locks: self.locks.clone(),
        };
        let per_work_lock = {
            let mut lock_map = self.locks.lock().await;
            lock_map
                .entry((user_id, work_id))
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _guard = per_work_lock.lock().await;

        self.db.reset_for_manual_refresh(user_id, work_id).await?;
        Ok(())
    }

    async fn inject_source_data(
        &self,
        user_id: UserId,
        work_id: WorkId,
        data: livrarr_domain::services::SourceProviderData,
    ) {
        self.pre_inject_source_data(user_id, work_id, data).await;
    }
}
