//! livrarr-enrichment — the metadata merge/enrich engine.
//!
//! Extracted from livrarr-metadata (4a): the provider queue, cross-provider
//! validation, the field-merge engine, cover selection/gating, and the
//! `EnrichmentServiceImpl` spine. Depends only on domain/db/external-data/http;
//! it must not depend on livrarr-metadata or livrarr-identity.

use std::collections::HashMap;
use std::sync::Arc;

use livrarr_db::{
    ApplyEnrichmentMergeRequest, SetFieldProvenanceRequest, UpdateWorkEnrichmentDbRequest,
    UpsertExternalIdRequest,
};
use livrarr_domain::{
    ApplyMergeOutcome, DbError, EnrichmentStatus, FieldProvenance, MergeResolved, NarrationType,
    RequestPriority, UserId, WillRetryReason, Work, WorkField, WorkId,
};
use livrarr_external_data::{NormalizedWorkDetail, ProviderOutcome};

pub mod cover_gate;
pub mod cover_resolution;
pub mod llm_ewl;
pub mod llm_validator;
pub mod pacing_queue;
pub mod provider_queue;

#[cfg(test)]
mod provider_queue_tracer_tests;

pub use pacing_queue::{LivePacingQueue, PacingQueue};
pub use provider_queue::{
    ApplicabilityRule, DefaultProviderQueue, DefaultProviderQueueBuilder, InitialCircuitState,
};

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
    async fn enrich_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
        mode: EnrichmentMode,
        candidate_id: Option<livrarr_domain::identity::CandidateId>,
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
    /// Seam-2 signal: the LLM rejected every provider payload as not-this-book.
    /// Propagated to `domain::services::EnrichmentResult.identity_not_found`; the
    /// caller writes `IdentityStatus::NotFound`. Enrichment never writes identity.
    pub identity_not_found: bool,
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
}

/// Circuit breaker state for a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

/// Per-provider circuit breaker configuration.
///
/// R-22
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures within `evaluation_window_secs` that trips Closed → Open.
    pub failure_threshold: u32,
    /// Rolling window over which failures are counted.
    pub evaluation_window_secs: u64,
    /// How long the breaker stays Open before transitioning to HalfOpen.
    pub open_duration_secs: u64,
    /// In HalfOpen, allow this many probe attempts before deciding Open vs Closed.
    pub half_open_probe_count: u32,
}

/// Per-provider queue configuration.
///
/// R-22
#[derive(Debug, Clone)]
pub struct ProviderQueueConfig {
    pub provider: livrarr_domain::MetadataProvider,
    /// Max in-flight requests against this provider. Reserved 1 slot for Background
    /// when concurrency >= 2 (priority class semantics — not exercised by tests yet).
    pub concurrency: u32,
    /// Pacing limit. Not enforced by the queue runtime in this phase — see deferred
    /// notes in the plan. Field kept on the contract so adapters can query it.
    pub requests_per_second: f64,
    pub circuit_breaker: CircuitBreakerConfig,
    pub max_attempts: u32,
    pub max_suppressed_passes: u32,
    pub max_suppression_window_secs: u64,
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

/// Shared per-provider request queue. Scatter-gather dispatch with per-provider
/// circuit breakers and durable phase-1 outcome persistence.
///
/// R-22
#[trait_variant::make(Send)]
pub trait ProviderQueue: Send + Sync {
    async fn dispatch_enrichment(
        &self,
        work: &Work,
        context: EnrichmentContext,
    ) -> Result<ScatterGatherResult, ProviderQueueError>;

    fn circuit_state(&self, provider: livrarr_domain::MetadataProvider) -> CircuitState;
}

/// TEMP(pk-tdd): reconstructed per-provider outcome for merge input.
#[derive(Debug, Clone)]
pub struct ReconstructedOutcome {
    pub class: livrarr_domain::OutcomeClass,
    pub payload: Option<NormalizedWorkDetail>,
}

/// TEMP(pk-tdd): priority order per field group for merge resolution.
#[derive(Debug, Clone)]
pub struct PriorityModel {
    pub content: Vec<livrarr_domain::MetadataProvider>,
    pub description: Vec<livrarr_domain::MetadataProvider>,
    pub cover: Vec<livrarr_domain::MetadataProvider>,
    pub audio: Vec<livrarr_domain::MetadataProvider>,
}

impl PriorityModel {
    /// English: HC → GR → Readarr → OL → Audible, Audio: Audible → Audnexus → HC.
    pub fn english() -> Self {
        use livrarr_domain::MetadataProvider as P;
        Self {
            content: vec![
                P::Hardcover,
                P::Goodreads,
                P::Readarr,
                P::OpenLibrary,
                P::Audible,
            ],
            description: vec![
                P::Hardcover,
                P::Goodreads,
                P::Readarr,
                P::OpenLibrary,
                P::Audible,
            ],
            cover: vec![
                P::Hardcover,
                P::Goodreads,
                P::Readarr,
                P::OpenLibrary,
                P::Audible,
            ],
            audio: vec![P::Audible, P::Audnexus, P::Hardcover],
        }
    }

    /// Foreign: GB → GR → HC → Readarr → OL → Audible, Audio: Audible → Audnexus → HC.
    pub fn foreign() -> Self {
        use livrarr_domain::MetadataProvider as P;
        Self {
            content: vec![
                P::GoogleBooks,
                P::Goodreads,
                P::Hardcover,
                P::Readarr,
                P::OpenLibrary,
                P::Audible,
            ],
            description: vec![
                P::GoogleBooks,
                P::Goodreads,
                P::Hardcover,
                P::Readarr,
                P::OpenLibrary,
                P::Audible,
            ],
            cover: vec![
                P::GoogleBooks,
                P::Goodreads,
                P::Hardcover,
                P::Readarr,
                P::OpenLibrary,
                P::Audible,
            ],
            audio: vec![P::Audible, P::Audnexus, P::Hardcover],
        }
    }

    /// Select model based on work language.
    pub fn for_language(language: Option<&str>) -> Self {
        match livrarr_external_data::language::provider_priority(language) {
            livrarr_external_data::language::ProviderPriority::English => Self::english(),
            livrarr_external_data::language::ProviderPriority::Foreign => Self::foreign(),
        }
    }
}

/// TEMP(pk-tdd): inputs to MergeEngine::merge.
#[derive(Debug, Clone)]
pub struct MergeInput {
    pub current_work: Work,
    pub current_provenance: Vec<FieldProvenance>,
    pub provider_results: HashMap<livrarr_domain::MetadataProvider, ReconstructedOutcome>,
    pub mode: EnrichmentMode,
    pub priority_model: PriorityModel,
}

/// TEMP(pk-tdd): output of MergeEngine::merge.
#[derive(Debug, Clone)]
pub struct MergeOutput {
    pub conflict_detected: bool,
    pub work_update: Option<MergeResolved<UpdateWorkEnrichmentDbRequest>>,
    pub provenance_upserts: Vec<SetFieldProvenanceRequest>,
    pub provenance_deletes: Vec<WorkField>,
    pub external_id_updates: Vec<UpsertExternalIdRequest>,
    pub enrichment_status: EnrichmentStatus,
    pub enrichment_source: Option<String>,
    pub cover_resolution: Option<livrarr_domain::CoverResolution>,
    pub audiobook_cover_resolution: Option<livrarr_domain::CoverResolution>,
}

/// TEMP(pk-tdd): error from MergeEngine::merge.
#[derive(Debug, thiserror::Error)]
pub enum MergeError {
    #[error("priority model has no providers for required field groups")]
    EmptyPriorityModel,
}

/// Merge engine — computes field-level merge from provider outcomes.
///
/// Async because the LLM arbitration path makes a network call.
/// The deterministic fallback is purely synchronous — the async overhead
/// is negligible compared to the prior scatter-gather.
#[trait_variant::make(Send)]
pub trait MergeEngine: Send + Sync {
    async fn merge(&self, inputs: MergeInput) -> Result<MergeOutput, MergeError>;

    /// Merge from already-fetched per-provider payloads — zero provider network
    /// calls (REQ-014/015). The add path reuses the payloads the resolver cached
    /// during discovery instead of re-querying. See ir-v2 metadata-merge-reuse.
    async fn merge_from_cached(
        &self,
        work: Work,
        payloads: HashMap<livrarr_domain::MetadataProvider, NormalizedWorkDetail>,
        current_provenance: Vec<FieldProvenance>,
        language: Option<&str>,
    ) -> Result<MergeOutput, MergeError>;
}

/// Deterministic merge engine (REQ-004/REQ-005, P-C): pure and zero-LLM. The
/// per-merge priority model is taken from `MergeInput`; the engine is stateless.
pub struct DefaultMergeEngine;

/// Build the DB apply-request from a computed merge output, rewriting the
/// per-row ids to the target (user_id, work_id). Shared by the network
/// enrichment path (`enrich_work`) and the cached-payload reuse path in
/// `WorkService::add` (REQ-014/015) so both produce byte-identical writes.
pub fn build_apply_request(
    merge_output: &MergeOutput,
    user_id: livrarr_domain::UserId,
    work_id: livrarr_domain::WorkId,
    expected_merge_generation: i64,
) -> ApplyEnrichmentMergeRequest {
    let provenance_upserts = merge_output
        .provenance_upserts
        .iter()
        .map(|p| SetFieldProvenanceRequest {
            user_id,
            work_id,
            ..p.clone()
        })
        .collect();
    let external_id_updates = merge_output
        .external_id_updates
        .iter()
        .map(|e| UpsertExternalIdRequest {
            work_id,
            ..e.clone()
        })
        .collect();
    ApplyEnrichmentMergeRequest {
        user_id,
        work_id,
        expected_merge_generation,
        work_update: merge_output.work_update.clone(),
        new_enrichment_status: merge_output.enrichment_status,
        provenance_upserts,
        provenance_deletes: merge_output.provenance_deletes.clone(),
        external_id_updates,
    }
}

impl DefaultMergeEngine {
    /// Construct the deterministic merge engine. `priority_model` is accepted for
    /// call-site compatibility; the per-merge model comes from `MergeInput`.
    pub fn new(_priority_model: PriorityModel) -> Self {
        Self
    }
}

impl DefaultMergeEngine {
    /// Compatibility constructor for call sites that previously supplied an LLM
    /// caller. The merge is purely deterministic now (REQ-005/D-010), so the
    /// caller and its configured flag are accepted and discarded.
    pub fn new_with_llm<L>(_priority_model: PriorityModel, _llm: L, _llm_configured: bool) -> Self
    where
        L: livrarr_domain::services::LlmCaller + Send + Sync,
    {
        Self
    }
}

impl MergeEngine for DefaultMergeEngine {
    async fn merge(&self, inputs: MergeInput) -> Result<MergeOutput, MergeError> {
        // REQ-005/D-010: the merge is purely deterministic — ZERO LLM, even when a
        // caller is configured. Language routing (REQ-014/#133) is enforced here at
        // the single chokepoint both the cached and network entry paths funnel through,
        // so a foreign work can never take English OpenLibrary/Hardcover metadata.
        let inputs = drop_language_incompatible_providers(inputs);
        merge_impl(inputs)
    }

    /// Merge from already-fetched per-provider payloads — zero provider network
    /// calls (REQ-014/015). Wraps each payload as a ReconstructedOutcome and runs
    /// the deterministic merge. The foreign-work OpenLibrary/Hardcover drop
    /// (REQ-027) is enforced centrally in `merge`, so this cached path and the
    /// network path share one language-routing policy (#133).
    async fn merge_from_cached(
        &self,
        work: Work,
        payloads: HashMap<livrarr_domain::MetadataProvider, NormalizedWorkDetail>,
        current_provenance: Vec<FieldProvenance>,
        language: Option<&str>,
    ) -> Result<MergeOutput, MergeError> {
        let provider_results = payloads
            .into_iter()
            .map(|(provider, detail)| {
                (
                    provider,
                    ReconstructedOutcome {
                        class: livrarr_domain::OutcomeClass::Success,
                        payload: Some(detail),
                    },
                )
            })
            .collect();
        let input = MergeInput {
            current_work: work,
            current_provenance,
            provider_results,
            mode: EnrichmentMode::Manual,
            priority_model: PriorityModel::for_language(language),
        };
        self.merge(input).await
    }
}

// =============================================================================
// Merge implementation helpers
// =============================================================================

/// Enforce P2 (a book's language is sacred): a foreign-language work must never
/// take English-centric OpenLibrary/Hardcover metadata (#133 / REQ-027). Called
/// once at the `MergeEngine::merge` chokepoint, so the rule is caller-independent —
/// `PriorityModel::foreign()` still lists OL/HC as fallbacks, so reordering alone
/// is insufficient; the providers must be removed from the inputs. OL/HC anchors
/// are captured upstream at the identity resolver (language-agnostic), so only
/// metadata contribution is affected, not identity.
fn drop_language_incompatible_providers(mut inputs: MergeInput) -> MergeInput {
    use livrarr_domain::MetadataProvider as P;
    let is_foreign = matches!(
        livrarr_external_data::language::provider_priority(inputs.current_work.language.as_deref()),
        livrarr_external_data::language::ProviderPriority::Foreign
    );
    if is_foreign {
        inputs
            .provider_results
            .retain(|provider, _| !matches!(provider, P::OpenLibrary | P::Hardcover));
    }
    inputs
}

/// Field category for priority model lookup.
enum FieldCategory {
    Content,
    Description,
    Cover,
    Audio,
}

/// Map a WorkField to its priority model category.
fn field_category(field: WorkField) -> FieldCategory {
    match field {
        WorkField::Description => FieldCategory::Description,
        WorkField::CoverUrl => FieldCategory::Cover,
        WorkField::DurationSeconds
        | WorkField::Narrator
        | WorkField::NarrationType
        | WorkField::Abridged
        | WorkField::Asin => FieldCategory::Audio,
        // Everything else is content
        _ => FieldCategory::Content,
    }
}

/// Get the priority list for a field from the priority model.
fn priority_list_for(field: WorkField, pm: &PriorityModel) -> &[livrarr_domain::MetadataProvider] {
    match field_category(field) {
        FieldCategory::Content => &pm.content,
        FieldCategory::Description => &pm.description,
        FieldCategory::Cover => &pm.cover,
        FieldCategory::Audio => &pm.audio,
    }
}

/// Represents a resolved field value — either a string-like option, or typed data.
/// We use an enum to handle the different field value types uniformly.
#[derive(Debug, Clone)]
enum FieldValue {
    Str(Option<String>),
    Int(Option<i32>),
    Float(Option<f64>),
    Bool(Option<bool>),
    Strings(Option<Vec<String>>),
    NarrationType(Option<NarrationType>),
}

impl FieldValue {
    fn is_some(&self) -> bool {
        match self {
            Self::Str(v) => v.is_some(),
            Self::Int(v) => v.is_some(),
            Self::Float(v) => v.is_some(),
            Self::Bool(v) => v.is_some(),
            Self::Strings(v) => v.is_some(),
            Self::NarrationType(v) => v.is_some(),
        }
    }
}

/// Extract a field value from NormalizedWorkDetail.
fn extract_provider_field(field: WorkField, detail: &NormalizedWorkDetail) -> FieldValue {
    match field {
        WorkField::Title => FieldValue::Str(non_blank(&detail.title)),
        WorkField::SortTitle => FieldValue::Str(None), // not in NormalizedWorkDetail
        WorkField::Subtitle => FieldValue::Str(non_blank(&detail.subtitle)),
        WorkField::OriginalTitle => FieldValue::Str(non_blank(&detail.original_title)),
        WorkField::AuthorName => FieldValue::Str(non_blank(&detail.author_name)),
        WorkField::Description => FieldValue::Str(non_blank(&detail.description)),
        WorkField::Year => FieldValue::Int(detail.year),
        WorkField::SeriesName => FieldValue::Str(non_blank(&detail.series_name)),
        WorkField::SeriesPosition => FieldValue::Float(detail.series_position),
        WorkField::Genres => FieldValue::Strings(detail.genres.clone()),
        WorkField::Language => FieldValue::Str(non_blank(&detail.language)),
        WorkField::PageCount => FieldValue::Int(detail.page_count),
        WorkField::DurationSeconds => FieldValue::Int(detail.duration_seconds),
        WorkField::Publisher => FieldValue::Str(non_blank(&detail.publisher)),
        WorkField::PublishDate => FieldValue::Str(non_blank(&detail.publish_date)),
        WorkField::OlKey => FieldValue::Str(non_blank(&detail.ol_key)),
        WorkField::HcKey => FieldValue::Str(non_blank(&detail.hc_key)),
        WorkField::GrKey => FieldValue::Str(non_blank(&detail.gr_key)),
        WorkField::Isbn13 => FieldValue::Str(non_blank(&detail.isbn_13)),
        WorkField::Asin => FieldValue::Str(non_blank(&detail.asin)),
        WorkField::Narrator => FieldValue::Strings(detail.narrator.clone()),
        WorkField::NarrationType => FieldValue::NarrationType(detail.narration_type),
        WorkField::Abridged => FieldValue::Bool(detail.abridged),
        WorkField::Rating => FieldValue::Float(detail.rating),
        WorkField::RatingCount => FieldValue::Int(detail.rating_count),
        WorkField::CoverUrl => FieldValue::Str(non_blank(&detail.cover_url)),
    }
}

/// Extract current field value from the Work struct.
fn extract_current_field(field: WorkField, work: &Work) -> FieldValue {
    match field {
        WorkField::Title => FieldValue::Str(non_blank_owned(&work.title)),
        WorkField::SortTitle => FieldValue::Str(work.sort_title.clone()),
        WorkField::Subtitle => FieldValue::Str(work.subtitle.clone()),
        WorkField::OriginalTitle => FieldValue::Str(work.original_title.clone()),
        WorkField::AuthorName => FieldValue::Str(non_blank_owned(&work.author_name)),
        WorkField::Description => FieldValue::Str(work.description.clone()),
        WorkField::Year => FieldValue::Int(work.year),
        WorkField::SeriesName => FieldValue::Str(work.series_name.clone()),
        WorkField::SeriesPosition => FieldValue::Float(work.series_position),
        WorkField::Genres => FieldValue::Strings(work.genres.clone()),
        WorkField::Language => FieldValue::Str(non_blank(&work.language)),
        WorkField::PageCount => FieldValue::Int(work.page_count),
        WorkField::DurationSeconds => FieldValue::Int(work.duration_seconds),
        WorkField::Publisher => FieldValue::Str(work.publisher.clone()),
        WorkField::PublishDate => FieldValue::Str(work.publish_date.clone()),
        WorkField::OlKey => FieldValue::Str(work.ol_key.clone()),
        WorkField::HcKey => FieldValue::Str(work.hc_key.clone()),
        WorkField::GrKey => FieldValue::Str(work.gr_key.clone()),
        WorkField::Isbn13 => FieldValue::Str(work.isbn_13.clone()),
        WorkField::Asin => FieldValue::Str(work.asin.clone()),
        WorkField::Narrator => FieldValue::Strings(work.narrator.clone()),
        WorkField::NarrationType => FieldValue::NarrationType(work.narration_type),
        WorkField::Abridged => FieldValue::Bool(Some(work.abridged)),
        WorkField::Rating => FieldValue::Float(work.rating),
        WorkField::RatingCount => FieldValue::Int(work.rating_count),
        WorkField::CoverUrl => FieldValue::Str(work.cover_url.clone()),
    }
}

/// Returns None if the string is None or whitespace-only after trimming.
fn non_blank(s: &Option<String>) -> Option<String> {
    s.as_ref().filter(|v| !v.trim().is_empty()).cloned()
}

/// Returns None if the owned string is empty or whitespace-only.
fn non_blank_owned(s: &str) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s.to_owned())
    }
}

/// Lowercase name for a MetadataProvider (for enrichment_source).
fn provider_name(p: livrarr_domain::MetadataProvider) -> &'static str {
    match p {
        livrarr_domain::MetadataProvider::Hardcover => "hardcover",
        livrarr_domain::MetadataProvider::OpenLibrary => "openlibrary",
        livrarr_domain::MetadataProvider::Goodreads => "goodreads",
        livrarr_domain::MetadataProvider::Audnexus => "audnexus",
        livrarr_domain::MetadataProvider::Llm => "llm",
        livrarr_domain::MetadataProvider::Readarr => "readarr",
        livrarr_domain::MetadataProvider::GoogleBooks => "google_books",
        livrarr_domain::MetadataProvider::Audible => "audible",
    }
}

/// The ordered list of fields that we merge. SortTitle is excluded because
/// NormalizedWorkDetail and UpdateWorkEnrichmentDbRequest don't carry it.
const MERGE_FIELDS: &[WorkField] = &[
    WorkField::Title,
    WorkField::Subtitle,
    WorkField::OriginalTitle,
    WorkField::AuthorName,
    WorkField::Description,
    WorkField::Year,
    WorkField::SeriesName,
    WorkField::SeriesPosition,
    WorkField::Genres,
    WorkField::Language,
    WorkField::PageCount,
    WorkField::DurationSeconds,
    WorkField::Publisher,
    WorkField::PublishDate,
    WorkField::OlKey,
    WorkField::HcKey,
    WorkField::GrKey,
    WorkField::Isbn13,
    WorkField::Asin,
    WorkField::Narrator,
    WorkField::NarrationType,
    WorkField::Abridged,
    WorkField::Rating,
    WorkField::RatingCount,
];

/// Core merge implementation.
fn merge_impl(inputs: MergeInput) -> Result<MergeOutput, MergeError> {
    let pm = &inputs.priority_model;

    // 1. Validate priority model: if ANY category is empty, error.
    if pm.content.is_empty()
        || pm.description.is_empty()
        || pm.cover.is_empty()
        || pm.audio.is_empty()
    {
        return Err(MergeError::EmptyPriorityModel);
    }

    // 2. Conflict detection: if ANY provider has Conflict class, block.
    let has_conflict = inputs
        .provider_results
        .values()
        .any(|o| o.class == livrarr_domain::OutcomeClass::Conflict);

    if has_conflict {
        return Ok(MergeOutput {
            conflict_detected: true,
            work_update: None,
            provenance_upserts: Vec::new(),
            provenance_deletes: Vec::new(),
            external_id_updates: Vec::new(),
            enrichment_status: EnrichmentStatus::Unenriched,
            enrichment_source: None,
            cover_resolution: None,
            audiobook_cover_resolution: None,
        });
    }

    // 3. Determine which providers are merge-eligible based on mode.
    let eligible_providers: HashMap<
        livrarr_domain::MetadataProvider,
        Option<&NormalizedWorkDetail>,
    > = inputs
        .provider_results
        .iter()
        .filter(|(_, outcome)| {
            match inputs.mode {
                EnrichmentMode::Background => outcome.class.can_merge(),
                EnrichmentMode::Manual | EnrichmentMode::HardRefresh => {
                    // Only Conflict blocks in manual/hard-refresh, and we've
                    // already handled that above.
                    outcome.class != livrarr_domain::OutcomeClass::Conflict
                }
            }
        })
        .map(|(provider, outcome)| (*provider, outcome.payload.as_ref()))
        .collect();

    // Build a provenance lookup: field → FieldProvenance
    let prov_map: HashMap<WorkField, &FieldProvenance> = inputs
        .current_provenance
        .iter()
        .map(|fp| (fp.field, fp))
        .collect();

    let user_id = inputs.current_work.user_id;
    let work_id = inputs.current_work.id;

    // 4. Resolve each field.
    let mut provenance_upserts = Vec::new();
    let mut provenance_deletes = Vec::new();
    let mut resolved_values: HashMap<WorkField, FieldValue> = HashMap::new();
    let mut contributing_providers: Vec<livrarr_domain::MetadataProvider> = Vec::new();

    for &field in MERGE_FIELDS {
        // 4a. Identity fields are locked at add-time — never overwrite a non-empty
        // title/author/language. Language is identity-sovereign (P2): set once at
        // identity from real data, only a user changes it. A provider may FILL a
        // blank language but never override a set one.
        if field == WorkField::Title
            || field == WorkField::AuthorName
            || field == WorkField::Language
        {
            let current = extract_current_field(field, &inputs.current_work);
            if current.is_some() {
                resolved_values.insert(field, current);
                continue;
            }
        }

        // 4c. User-owned skip
        if let Some(fp) = prov_map.get(&field) {
            if fp.setter == livrarr_domain::ProvenanceSetter::User {
                let current = extract_current_field(field, &inputs.current_work);
                resolved_values.insert(field, current);
                continue;
            }
        }

        // 4c. Find winning provider by priority order
        let priority_list = priority_list_for(field, pm);
        let mut winner: Option<(livrarr_domain::MetadataProvider, FieldValue)> = None;

        for &provider in priority_list {
            if let Some(Some(detail)) = eligible_providers.get(&provider) {
                let val = extract_provider_field(field, detail);
                if val.is_some() {
                    winner = Some((provider, val));
                    break;
                }
            }
        }

        if let Some((provider, val)) = winner {
            // Provider wins — set value and generate provenance upsert
            resolved_values.insert(field, val);
            provenance_upserts.push(SetFieldProvenanceRequest {
                user_id,
                work_id,
                field,
                source: Some(provider),
                setter: livrarr_domain::ProvenanceSetter::Provider,
                cleared: false,
            });
            if !contributing_providers.contains(&provider) {
                contributing_providers.push(provider);
            }
        } else {
            // No winning provider — last-known-good
            let current = extract_current_field(field, &inputs.current_work);

            // If the field was provider-owned and current value exists,
            // generate a provenance delete (old provider no longer claims it).
            if current.is_some() {
                if let Some(fp) = prov_map.get(&field) {
                    if fp.setter == livrarr_domain::ProvenanceSetter::Provider {
                        provenance_deletes.push(field);
                    }
                }
            }

            resolved_values.insert(field, current);
        }
    }

    // 5. Build UpdateWorkEnrichmentDbRequest from resolved values.
    let get_str = |f: WorkField| -> Option<String> {
        match resolved_values.get(&f) {
            Some(FieldValue::Str(v)) => v.clone(),
            _ => None,
        }
    };
    let get_int = |f: WorkField| -> Option<i32> {
        match resolved_values.get(&f) {
            Some(FieldValue::Int(v)) => *v,
            _ => None,
        }
    };
    let get_float = |f: WorkField| -> Option<f64> {
        match resolved_values.get(&f) {
            Some(FieldValue::Float(v)) => *v,
            _ => None,
        }
    };
    let get_bool = |f: WorkField| -> Option<bool> {
        match resolved_values.get(&f) {
            Some(FieldValue::Bool(v)) => *v,
            _ => None,
        }
    };
    let get_strings = |f: WorkField| -> Option<Vec<String>> {
        match resolved_values.get(&f) {
            Some(FieldValue::Strings(v)) => v.clone(),
            _ => None,
        }
    };
    let get_narration_type = |f: WorkField| -> Option<NarrationType> {
        match resolved_values.get(&f) {
            Some(FieldValue::NarrationType(v)) => *v,
            _ => None,
        }
    };

    let merged_description = get_str(WorkField::Description);

    // 5b. Cover resolution (separate from generic field merge). REQ-006: covers
    // are chosen by provider PRIORITY (something-beats-nothing, no size ranking).
    // REQ-008: a user-locked cover (provenance Setter=User) is never resolved
    // over, so materialize neither downloads nor writes a replacement.
    let outcomes_ref: HashMap<livrarr_domain::MetadataProvider, &ReconstructedOutcome> = inputs
        .provider_results
        .iter()
        .map(|(p, o)| (*p, o))
        .collect();
    let cover_user_locked = prov_map
        .get(&WorkField::CoverUrl)
        .is_some_and(|fp| fp.setter == livrarr_domain::ProvenanceSetter::User && !fp.cleared);
    let cover_resolution = if cover_user_locked {
        None
    } else {
        cover_resolution::resolve_cover(
            &inputs.current_work,
            livrarr_domain::CoverMediaType::Ebook,
            &pm.cover,
            &eligible_providers,
            &outcomes_ref,
        )
    };
    let audiobook_cover_resolution = cover_resolution::resolve_cover(
        &inputs.current_work,
        livrarr_domain::CoverMediaType::Audiobook,
        &pm.audio,
        &eligible_providers,
        &outcomes_ref,
    );
    // 6. Status classification (REQ-019): Enriched iff >=1 meaningful text field
    // is present; otherwise Thin ("we know the book, found no info"). The cover
    // is a lazy backfill asset and never gates completion; title/author are
    // identity (present from creation) and are not an enrichment signal.
    let has_meaningful_text = merged_description.is_some()
        || get_str(WorkField::Subtitle).is_some()
        || get_str(WorkField::SeriesName).is_some()
        || get_strings(WorkField::Genres).is_some_and(|g| !g.is_empty())
        || get_str(WorkField::Publisher).is_some();
    let enrichment_status = if has_meaningful_text {
        EnrichmentStatus::Enriched
    } else {
        EnrichmentStatus::Thin
    };

    // 7. enrichment_source: comma-joined lowercased provider names.
    let enrichment_source = if contributing_providers.is_empty() {
        None
    } else {
        let names: Vec<&str> = contributing_providers
            .iter()
            .map(|p| provider_name(*p))
            .collect();
        Some(names.join(","))
    };

    let work_update = UpdateWorkEnrichmentDbRequest {
        title: get_str(WorkField::Title),
        subtitle: get_str(WorkField::Subtitle),
        original_title: get_str(WorkField::OriginalTitle),
        author_name: get_str(WorkField::AuthorName),
        description: merged_description,
        year: get_int(WorkField::Year),
        series_name: get_str(WorkField::SeriesName),
        series_position: get_float(WorkField::SeriesPosition),
        genres: get_strings(WorkField::Genres),
        language: get_str(WorkField::Language).map(|s| livrarr_domain::normalize_language(&s)),
        page_count: get_int(WorkField::PageCount),
        duration_seconds: get_int(WorkField::DurationSeconds),
        publisher: get_str(WorkField::Publisher),
        publish_date: get_str(WorkField::PublishDate),
        ol_key: get_str(WorkField::OlKey),
        gr_key: get_str(WorkField::GrKey),
        hc_key: get_str(WorkField::HcKey),
        isbn_13: get_str(WorkField::Isbn13),
        asin: get_str(WorkField::Asin),
        narrator: get_strings(WorkField::Narrator),
        narration_type: get_narration_type(WorkField::NarrationType),
        abridged: get_bool(WorkField::Abridged),
        rating: get_float(WorkField::Rating),
        rating_count: get_int(WorkField::RatingCount),
        enrichment_status,
        enrichment_source: enrichment_source.clone(),
        // REQ-006: persist the priority-resolved cover URL; fall back to the
        // existing cover when no provider supplied one (non-destructive) or when
        // the user locked it (cover_resolution is None above).
        cover_url: cover_resolution
            .as_ref()
            .map(|c| c.url.clone())
            .or_else(|| inputs.current_work.cover_url.clone()),
    };

    // 8. External ID collection: from all Success providers.
    let mut external_id_updates = Vec::new();
    for (provider, outcome) in &inputs.provider_results {
        if outcome.class == livrarr_domain::OutcomeClass::Success {
            if let Some(ref detail) = outcome.payload {
                for isbn in &detail.additional_isbns {
                    external_id_updates.push(UpsertExternalIdRequest {
                        work_id,
                        id_type: livrarr_domain::ExternalIdType::Isbn13,
                        id_value: isbn.clone(),
                    });
                }
                for asin_val in &detail.additional_asins {
                    external_id_updates.push(UpsertExternalIdRequest {
                        work_id,
                        id_type: livrarr_domain::ExternalIdType::Asin,
                        id_value: asin_val.clone(),
                    });
                }
                let _ = provider; // used above via iteration
            }
        }
    }

    Ok(MergeOutput {
        conflict_detected: false,
        work_update: Some(MergeResolved::new(work_update)),
        provenance_upserts,
        provenance_deletes,
        external_id_updates,
        enrichment_status,
        enrichment_source,
        cover_resolution,
        audiobook_cover_resolution,
    })
}

// =============================================================================
// LLM arbitration merge path
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

/// Enrichment service implementation.
/// Generic over DB, Q (ProviderQueue), ME (MergeEngine), V (LlmValidator),
/// and L (LlmCaller for cover gate disambiguation).
pub struct EnrichmentServiceImpl<DB, Q, ME, V, L = crate::StubNoLlm> {
    db: Arc<DB>,
    queue: Arc<Q>,
    merge_engine: Arc<ME>,
    /// Cross-provider semantic validator. Inserts an identity-check +
    /// per-provider accept/reject step between scatter-gather and merge.
    /// Use `NoOpLlmValidator` to disable when LLM is not configured.
    validator: Arc<V>,
    llm: L,
    llm_configured: bool,
    /// Per-work lock map [I-12]: serializes concurrent enrichment calls for the same (user_id, work_id).
    locks: Arc<PerWorkLocks>,
    /// Pre-injected source provider data (e.g., from Readarr import).
    /// Set via `pre_inject_source_data` before calling `enrich_work`.
    source_data_store: Arc<SourceDataStore>,
}

impl<DB, Q, ME, V, L> EnrichmentServiceImpl<DB, Q, ME, V, L>
where
    DB: livrarr_db::WorkDb
        + livrarr_db::ProvenanceDb
        + livrarr_db::ProviderRetryStateDb
        + livrarr_db::ExternalIdDb
        + Send
        + Sync
        + 'static,
    Q: ProviderQueue + Send + Sync + 'static,
    ME: MergeEngine + Send + Sync + 'static,
    V: crate::llm_validator::LlmValidator + Send + Sync + 'static,
    L: livrarr_domain::services::LlmCaller + Send + Sync + 'static,
{
    pub fn new(
        db: Arc<DB>,
        queue: Arc<Q>,
        merge_engine: Arc<ME>,
        validator: Arc<V>,
        llm: L,
        llm_configured: bool,
    ) -> Self {
        Self {
            db,
            queue,
            merge_engine,
            validator,
            llm,
            llm_configured,
            locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            source_data_store: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
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

impl<DB, Q, ME, V, L> EnrichmentService for EnrichmentServiceImpl<DB, Q, ME, V, L>
where
    DB: livrarr_db::WorkDb
        + livrarr_db::ProvenanceDb
        + livrarr_db::ProviderRetryStateDb
        + livrarr_db::ExternalIdDb
        + Send
        + Sync
        + 'static,
    Q: ProviderQueue + Send + Sync + 'static,
    ME: MergeEngine + Send + Sync + 'static,
    V: crate::llm_validator::LlmValidator + Send + Sync + 'static,
    L: livrarr_domain::services::LlmCaller + Send + Sync + 'static,
{
    async fn enrich_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
        mode: EnrichmentMode,
        candidate_id: Option<livrarr_domain::identity::CandidateId>,
    ) -> Result<EnrichmentResult, EnrichmentError> {
        // metadata-refactor: candidate-reuse + always-materialize relocate here in
        // the green phase (DD-007); the widened signature lands first (stub).
        let _ = candidate_id;
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

        // Step 3: Read merge_generation before dispatch (for CAS baseline)
        let mut generation = self.db.get_merge_generation(user_id, work_id).await?;

        // Step 4: Dispatch to provider queue
        let context = EnrichmentContext {
            priority: RequestPriority::Normal,
            mode,
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
            });
        }

        // Step 8: Build ReconstructedOutcome from ScatterGatherResult
        // For Success outcomes, read back normalized_payload_json from DB retry state
        let mut reconstructed: HashMap<livrarr_domain::MetadataProvider, ReconstructedOutcome> =
            HashMap::new();

        for (provider, outcome) in &scatter_result.outcomes {
            match outcome {
                ProviderOutcome::Success(_) => {
                    // Read back the persisted payload from DB
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
                ProviderOutcome::Suppressed { .. } => {
                    reconstructed.insert(
                        *provider,
                        ReconstructedOutcome {
                            class: livrarr_domain::OutcomeClass::Suppressed,
                            payload: None,
                        },
                    );
                }
            }
        }

        // Step 8.5: LLM cross-provider validation (identity check +
        // per-provider accept/reject + selective field nullification).
        // No-op when LLM is not configured (NoOpLlmValidator) or when the
        // work has no User-set anchor in provenance.
        //
        // On LLM error: log and pass through unchanged — LLM is value-add,
        // never gatekeeps enrichment per project Principle 11.
        let validation = match self
            .validator
            .validate(&current_work, &current_provenance, reconstructed)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    work_id,
                    user_id,
                    "LLM validation failed; passing outcomes through: {e}"
                );
                // Re-build reconstructed unmodified — but we already moved it.
                // Easiest path: re-reconstruct from scatter_result.
                let mut rebuilt: HashMap<livrarr_domain::MetadataProvider, ReconstructedOutcome> =
                    HashMap::new();
                for (provider, outcome) in &scatter_result.outcomes {
                    let class = outcome.class();
                    let payload = if class == livrarr_domain::OutcomeClass::Success {
                        let retry_state =
                            self.db.get_retry_state(user_id, work_id, *provider).await?;
                        retry_state
                            .and_then(|s| s.normalized_payload_json)
                            .and_then(|j| serde_json::from_str::<NormalizedWorkDetail>(&j).ok())
                    } else {
                        None
                    };
                    rebuilt.insert(*provider, ReconstructedOutcome { class, payload });
                }
                crate::llm_validator::ValidationOutcome {
                    reconstructed: rebuilt,
                    rejections: HashMap::new(),
                    all_success_rejected: false,
                }
            }
        };
        let reconstructed = validation.reconstructed;

        // If the LLM rejected EVERY Success payload, escalate the work to
        // Conflict status (terminal, exit only via reset_for_manual_refresh).
        // Skip the merge entirely — there's no usable provider data, and we
        // need the user to manually review which providers are wrong (or
        // edit the locked anchor).
        if validation.all_success_rejected {
            tracing::warn!(
                work_id,
                user_id,
                rejection_count = validation.rejections.len(),
                "all Success providers rejected by LLM identity check — signaling identity-not-found"
            );
            // Enrichment stays Unenriched (nothing merged). The work's identity could
            // not be verified from any source — SIGNAL it via `identity_not_found`; the
            // caller writes `IdentityStatus::NotFound` (one-way seam, REQ-002). Enrichment
            // never writes identity state.
            let apply_req = ApplyEnrichmentMergeRequest {
                user_id,
                work_id,
                expected_merge_generation: generation,
                work_update: None,
                new_enrichment_status: livrarr_domain::EnrichmentStatus::Unenriched,
                provenance_upserts: Vec::new(),
                provenance_deletes: Vec::new(),
                external_id_updates: Vec::new(),
            };
            let _ = self.db.apply_enrichment_merge(apply_req).await?;
            let result_work = self.db.get_work(user_id, work_id).await?;
            return Ok(EnrichmentResult {
                enrichment_status: livrarr_domain::EnrichmentStatus::Unenriched,
                enrichment_source: result_work.enrichment_source.clone(),
                llm_task_spawned: false,
                work: result_work,
                merge_deferred,
                provider_outcomes,
                cover_resolution: None,
                audiobook_cover_resolution: None,
                identity_not_found: true,
            });
        }

        // Cover gate: for English works with an OL key, filter GR cover_urls
        // through the deterministic Jaccard gate before merge (REQ-017).
        let reconstructed = if current_work.language.as_deref() == Some("en")
            && current_work.ol_key.is_some()
        {
            let mut filtered = reconstructed;
            if let Some(gr_outcome) = filtered.get_mut(&livrarr_domain::MetadataProvider::Goodreads)
            {
                if let Some(ref mut payload) = gr_outcome.payload {
                    if payload.cover_url.is_some() {
                        let anchor = crate::cover_gate::OlAnchor {
                            title: &current_work.title,
                            author_name: &current_work.author_name,
                            year: current_work.year,
                            isbn: current_work.isbn_13.as_deref(),
                            ol_key: current_work.ol_key.as_deref().unwrap_or(""),
                        };
                        let candidate = crate::cover_gate::GrCandidate {
                            title: payload.title.as_deref().unwrap_or(""),
                            author_name: payload.author_name.as_deref().unwrap_or(""),
                            year: payload.year,
                            isbn: None,
                            gr_key: payload.gr_key.as_deref().unwrap_or(""),
                        };
                        let outcome = crate::cover_gate::evaluate_gr_cover_gate(
                            &anchor,
                            &candidate,
                            self.llm_configured,
                        );
                        let final_outcome = match outcome {
                            crate::cover_gate::CoverGateOutcome::AskLlm {
                                jaccard,
                                ref prompt_inputs,
                            } => {
                                let decision = crate::llm_ewl::ask_same_book(
                                    &self.llm,
                                    prompt_inputs,
                                    self.llm_configured,
                                )
                                .await;
                                crate::cover_gate::apply_llm_decision(decision, jaccard)
                            }
                            other => other,
                        };
                        match final_outcome {
                            crate::cover_gate::CoverGateOutcome::Apply { .. } => {}
                            _ => {
                                tracing::info!(
                                    work_id,
                                    ?final_outcome,
                                    "cover gate: stripping GR cover_url"
                                );
                                payload.cover_url = None;
                                payload.gr_key = None;
                            }
                        }
                    }
                }
            }
            filtered
        } else {
            reconstructed
        };

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

            let merge_output = self.merge_engine.merge(merge_input).await?;

            let apply_req = build_apply_request(&merge_output, user_id, work_id, generation);

            let apply_outcome = self.db.apply_enrichment_merge(apply_req).await?;

            match apply_outcome {
                ApplyMergeOutcome::Applied
                | ApplyMergeOutcome::NoChange
                | ApplyMergeOutcome::Deferred => {
                    // Success — build result
                    let result_work = self.db.get_work(user_id, work_id).await?;
                    return Ok(EnrichmentResult {
                        enrichment_status: merge_output.enrichment_status,
                        enrichment_source: merge_output.enrichment_source,
                        llm_task_spawned: false,
                        work: result_work,
                        merge_deferred,
                        provider_outcomes,
                        cover_resolution: merge_output.cover_resolution,
                        audiobook_cover_resolution: merge_output.audiobook_cover_resolution,
                        // A merge-detected conflict (per-provider Conflict class or LLM
                        // merge identity_valid=false) signals identity-not-found upward.
                        identity_not_found: merge_output.conflict_detected,
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
