pub use livrarr_db::{
    ApplyEnrichmentMergeRequest, SetFieldProvenanceRequest, UpdateWorkEnrichmentDbRequest,
    UpsertExternalIdRequest,
};
pub use livrarr_domain::{
    ApplyMergeOutcome, DbError, EnrichmentStatus, FieldProvenance, LlmRole, MergeResolved,
    NarrationType, OutcomeClass, PermanentFailureReason, RequestPriority, UserId, WillRetryReason,
    Work, WorkField, WorkId,
};

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};

pub mod audnexus;
pub mod author_service;
pub mod cover;
pub mod enrichment_workflow_service;
pub mod goodreads;
pub mod hardcover;
pub mod http_llm;
pub mod language;
pub mod list_service;
pub mod live_config;
pub mod llm_caller_service;
pub mod llm_scraper;
pub mod llm_validator;
pub mod normalize;
pub mod openlibrary;
pub mod parsers;
pub mod provider_client;
pub mod provider_queue;
pub mod series_query_service;
pub mod series_service;
pub mod title_cleanup;
pub mod work_service;

pub mod author_monitor_workflow;
pub mod provenance;
pub mod rss_sync_workflow;

pub use provider_client::{
    AudnexusClient, GoodreadsClient, HardcoverClient, OpenLibraryClient, ProviderClient,
    StubProviderClient,
};
pub use provider_queue::{
    ApplicabilityRule, DefaultProviderQueue, DefaultProviderQueueBuilder, InitialCircuitState,
};

// =============================================================================
// Metadata Provider Trait
// =============================================================================

#[trait_variant::make(Send)]
pub trait MetadataProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn search_works(&self, query: &str) -> Result<Vec<ProviderSearchResult>, MetadataError>;
    async fn search_authors(&self, query: &str)
        -> Result<Vec<ProviderAuthorResult>, MetadataError>;
    async fn fetch_work_detail(
        &self,
        provider_key: &str,
    ) -> Result<ProviderWorkDetail, MetadataError>;
}

#[derive(Debug, Clone)]
pub struct ProviderSearchResult {
    pub provider_key: String,
    pub title: String,
    pub author_name: Option<String>,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    pub isbn: Option<String>,
    pub publisher: Option<String>,
    pub source: String,
    pub source_type: String,
    pub language: String,
    /// Detail page URL for enrichment (e.g., Goodreads book page). Server-side only.
    pub detail_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProviderAuthorResult {
    pub provider_key: String,
    pub name: String,
    pub work_count: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct ProviderWorkDetail {
    pub title: String,
    pub subtitle: Option<String>,
    pub original_title: Option<String>,
    pub author_name: String,
    pub description: Option<String>,
    pub year: Option<i32>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
    pub genres: Option<Vec<String>>,
    pub language: Option<String>,
    pub page_count: Option<i32>,
    pub publisher: Option<String>,
    pub publish_date: Option<String>,
    pub isbn_13: Option<String>,
    pub cover_url: Option<String>,
    pub hc_key: Option<String>,
    pub asin: Option<String>,
    pub narrator: Option<Vec<String>>,
    pub narration_type: Option<NarrationType>,
    pub abridged: Option<bool>,
    pub duration_seconds: Option<i32>,
    pub rating: Option<f64>,
    pub rating_count: Option<i32>,
}

#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error("provider not configured")]
    NotConfigured,
    #[error("provider request failed: {0}")]
    RequestFailed(String),
    #[error("provider timeout after {0:?}")]
    Timeout(Duration),
    #[error("provider rate limited")]
    RateLimited,
    #[error("provider returned invalid data: {0}")]
    InvalidResponse(String),
    #[error("no match found")]
    NoMatch,
    #[error("authentication failed (check token)")]
    AuthFailed,
    #[error("operation not supported by this provider")]
    UnsupportedOperation,
    #[error("anti-bot challenge detected")]
    AntiBotChallenge,
}

// =============================================================================
// Enrichment Service
// =============================================================================

#[trait_variant::make(Send)]
pub trait EnrichmentService: Send + Sync {
    /// TEMP(pk-tdd): compile-only scaffold — signature updated for metadata-overhaul.
    async fn enrich_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
        mode: EnrichmentMode,
    ) -> Result<EnrichmentResult, EnrichmentError>;

    /// TEMP(pk-tdd): compile-only scaffold — reset work for manual refresh.
    async fn reset_for_manual_refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<(), EnrichmentError>;
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

/// TEMP(pk-tdd): normalized provider output — common schema for all metadata providers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NormalizedWorkDetail {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub original_title: Option<String>,
    pub author_name: Option<String>,
    pub description: Option<String>,
    pub year: Option<i32>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
    pub genres: Option<Vec<String>>,
    pub language: Option<String>,
    pub page_count: Option<i32>,
    pub duration_seconds: Option<i32>,
    pub publisher: Option<String>,
    pub publish_date: Option<String>,
    pub hc_key: Option<String>,
    pub gr_key: Option<String>,
    pub ol_key: Option<String>,
    pub isbn_13: Option<String>,
    pub asin: Option<String>,
    pub narrator: Option<Vec<String>>,
    pub narration_type: Option<NarrationType>,
    pub abridged: Option<bool>,
    pub rating: Option<f64>,
    pub rating_count: Option<i32>,
    pub cover_url: Option<String>,
    pub additional_isbns: Vec<String>,
    pub additional_asins: Vec<String>,
}

/// TEMP(pk-tdd): per-provider outcome with typed payload for Success.
#[derive(Debug, Clone)]
pub enum ProviderOutcome<T> {
    Success(Box<T>),
    NotFound,
    NotConfigured,
    WillRetry {
        reason: WillRetryReason,
        next_attempt_at: DateTime<Utc>,
    },
    PermanentFailure {
        reason: PermanentFailureReason,
    },
    Conflict {
        detail: String,
    },
    Suppressed {
        until: DateTime<Utc>,
    },
}

impl<T> ProviderOutcome<T> {
    pub fn class(&self) -> livrarr_domain::OutcomeClass {
        match self {
            Self::Success(_) => livrarr_domain::OutcomeClass::Success,
            Self::NotFound => livrarr_domain::OutcomeClass::NotFound,
            Self::NotConfigured => livrarr_domain::OutcomeClass::NotConfigured,
            Self::WillRetry { .. } => livrarr_domain::OutcomeClass::WillRetry,
            Self::PermanentFailure { .. } => livrarr_domain::OutcomeClass::PermanentFailure,
            Self::Conflict { .. } => livrarr_domain::OutcomeClass::Conflict,
            Self::Suppressed { .. } => livrarr_domain::OutcomeClass::Suppressed,
        }
    }

    /// TEMP(pk-tdd): returns true if this outcome is eligible for merge in background mode.
    pub fn can_merge(&self) -> bool {
        self.class().can_merge()
    }

    /// TEMP(pk-tdd): returns true if this outcome is eligible for merge in manual/hard-refresh mode.
    /// Manual mode coerces WillRetry and Suppressed; only Conflict still blocks.
    pub fn can_merge_manual(&self) -> bool {
        !matches!(self, Self::Conflict { .. })
    }
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
    /// TEMP(pk-tdd): standard English-language priority model.
    /// Content: HC→GR→OL, Description: HC→OL→GR, Cover: GR→HC→OL, Audio: Audnexus→HC.
    pub fn english() -> Self {
        use livrarr_domain::MetadataProvider as P;
        Self {
            content: vec![P::Hardcover, P::Goodreads, P::OpenLibrary],
            description: vec![P::Hardcover, P::OpenLibrary, P::Goodreads],
            cover: vec![P::Goodreads, P::Hardcover, P::OpenLibrary],
            audio: vec![P::Audnexus, P::Hardcover],
        }
    }

    /// Foreign-language priority model. GR-only for content/description/cover,
    /// Audnexus-only for audio. OL and HC excluded — poor foreign-language data quality.
    pub fn foreign() -> Self {
        use livrarr_domain::MetadataProvider as P;
        Self {
            content: vec![P::Goodreads],
            description: vec![P::Goodreads],
            cover: vec![P::Goodreads],
            audio: vec![P::Audnexus],
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
}

/// TEMP(pk-tdd): error from MergeEngine::merge.
#[derive(Debug, thiserror::Error)]
pub enum MergeError {
    #[error("priority model has no providers for required field groups")]
    EmptyPriorityModel,
}

/// TEMP(pk-tdd): merge engine — computes field-level merge from provider outcomes.
pub trait MergeEngine: Send + Sync {
    fn merge(&self, inputs: MergeInput) -> Result<MergeOutput, MergeError>;
}

/// TEMP(pk-tdd): default merge engine implementation (stub — implement in metadata-overhaul).
pub struct DefaultMergeEngine {
    _priority_model: PriorityModel,
}

impl DefaultMergeEngine {
    pub fn new(priority_model: PriorityModel) -> Self {
        Self {
            _priority_model: priority_model,
        }
    }
}

impl MergeEngine for DefaultMergeEngine {
    fn merge(&self, inputs: MergeInput) -> Result<MergeOutput, MergeError> {
        merge_impl(inputs)
    }
}

// =============================================================================
// Merge implementation helpers
// =============================================================================

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
        WorkField::Language => FieldValue::Str(work.language.clone()),
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
    WorkField::CoverUrl,
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
            enrichment_status: EnrichmentStatus::Conflict,
            enrichment_source: None,
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
        // 4a. cover_manual bypass
        if field == WorkField::CoverUrl && inputs.current_work.cover_manual {
            let current = extract_current_field(field, &inputs.current_work);
            resolved_values.insert(field, current);
            continue;
        }

        // 4b. User-owned skip
        if let Some(fp) = prov_map.get(&field) {
            if fp.setter == livrarr_domain::ProvenanceSetter::User {
                // Use current work value (or None if user-cleared)
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
    let merged_cover_url = get_str(WorkField::CoverUrl);

    // 6. Status classification (R-14).
    let enrichment_status = match (merged_description.is_some(), merged_cover_url.is_some()) {
        (true, true) => EnrichmentStatus::Enriched,
        (true, false) | (false, true) => EnrichmentStatus::Partial,
        (false, false) => EnrichmentStatus::Failed,
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
        cover_url: get_str(WorkField::CoverUrl),
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
    })
}

/// Per-work lock map type [I-12].
type PerWorkLocks = tokio::sync::Mutex<HashMap<(UserId, WorkId), Arc<tokio::sync::Mutex<()>>>>;

/// Enrichment service implementation.
/// Generic over DB, Q (ProviderQueue), ME (MergeEngine), and V (LlmValidator)
/// to avoid dyn-compatibility issues.
pub struct EnrichmentServiceImpl<DB, Q, ME, V> {
    db: Arc<DB>,
    queue: Arc<Q>,
    merge_engine: Arc<ME>,
    /// Cross-provider semantic validator. Inserts an identity-check +
    /// per-provider accept/reject step between scatter-gather and merge.
    /// Use `NoOpLlmValidator` to disable when LLM is not configured.
    validator: Arc<V>,
    /// Per-work lock map [I-12]: serializes concurrent enrichment calls for the same (user_id, work_id).
    locks: Arc<PerWorkLocks>,
}

impl<DB, Q, ME, V> EnrichmentServiceImpl<DB, Q, ME, V>
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
{
    pub fn new(db: Arc<DB>, queue: Arc<Q>, merge_engine: Arc<ME>, validator: Arc<V>) -> Self {
        Self {
            db,
            queue,
            merge_engine,
            validator,
            locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }
}

impl<DB, Q, ME, V> EnrichmentService for EnrichmentServiceImpl<DB, Q, ME, V>
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
{
    async fn enrich_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
        mode: EnrichmentMode,
    ) -> Result<EnrichmentResult, EnrichmentError> {
        // Step 1: Acquire per-work lock [I-12]
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
        let scatter_result = self.queue.dispatch_enrichment(&work, context).await?;

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
                "all Success providers rejected by LLM identity check — escalating to Conflict"
            );
            let apply_req = ApplyEnrichmentMergeRequest {
                user_id,
                work_id,
                expected_merge_generation: generation,
                work_update: None,
                new_enrichment_status: livrarr_domain::EnrichmentStatus::Conflict,
                provenance_upserts: Vec::new(),
                provenance_deletes: Vec::new(),
                external_id_updates: Vec::new(),
            };
            let _ = self.db.apply_enrichment_merge(apply_req).await?;
            let result_work = self.db.get_work(user_id, work_id).await?;
            return Ok(EnrichmentResult {
                enrichment_status: livrarr_domain::EnrichmentStatus::Conflict,
                enrichment_source: result_work.enrichment_source.clone(),
                llm_task_spawned: false,
                work: result_work,
                merge_deferred,
                provider_outcomes,
            });
        }

        // Determine priority model based on work language
        let priority_model = match current_work.language.as_deref() {
            Some(lang) if !matches!(lang.to_lowercase().as_str(), "en" | "eng" | "english") => {
                PriorityModel::foreign()
            }
            _ => PriorityModel::english(),
        };

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

            let me = self.merge_engine.clone();
            let merge_output = tokio::task::spawn_blocking(move || me.merge(merge_input))
                .await
                .expect("merge task panicked")?;

            // Rewrite IDs in sub-requests to match the actual user_id/work_id
            let provenance_upserts: Vec<_> = merge_output
                .provenance_upserts
                .iter()
                .map(|p| SetFieldProvenanceRequest {
                    user_id,
                    work_id,
                    ..p.clone()
                })
                .collect();
            let external_id_updates: Vec<_> = merge_output
                .external_id_updates
                .iter()
                .map(|e| UpsertExternalIdRequest {
                    work_id,
                    ..e.clone()
                })
                .collect();

            let apply_req = ApplyEnrichmentMergeRequest {
                user_id,
                work_id,
                expected_merge_generation: generation,
                work_update: merge_output.work_update.clone(),
                new_enrichment_status: merge_output.enrichment_status,
                provenance_upserts,
                provenance_deletes: merge_output.provenance_deletes.clone(),
                external_id_updates,
            };

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
}

// =============================================================================
// Hardcover Matching
// =============================================================================

#[trait_variant::make(Send)]
pub trait HardcoverMatcher: Send + Sync {
    async fn match_deterministic(
        &self,
        title: &str,
        author: &str,
        candidates: &[HardcoverCandidate],
    ) -> Option<HardcoverCandidate>;

    async fn match_llm(
        &self,
        work_id: WorkId,
        title: &str,
        author: &str,
        candidates: &[HardcoverCandidate],
    ) -> Result<HardcoverCandidate, MetadataError>;
}

#[derive(Debug, Clone)]
pub struct HardcoverCandidate {
    pub hc_key: String,
    pub title: String,
    pub author_name: Option<String>,
    pub users_read_count: i64,
    pub detail: ProviderWorkDetail,
}

// =============================================================================
// LLM Client
// =============================================================================

#[trait_variant::make(Send)]
pub trait LlmClient: Send + Sync {
    async fn chat_completion(&self, messages: Vec<LlmMessage>) -> Result<String, LlmError>;
}

#[derive(Debug, Clone)]
pub struct LlmMessage {
    pub role: LlmRole,
    pub content: String,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum LlmError {
    #[error("LLM not configured")]
    NotConfigured,
    #[error("LLM request failed: {0}")]
    RequestFailed(String),
    #[error("LLM timeout after {0:?}")]
    Timeout(Duration),
    #[error("LLM rate limited")]
    RateLimited,
    #[error("LLM returned invalid response: {0}")]
    InvalidResponse(String),
}

// =============================================================================
// Cover Cache
// =============================================================================

#[trait_variant::make(Send)]
pub trait CoverCache: Send + Sync {
    async fn cache_cover(&self, work_id: WorkId, cover_url: &str) -> Result<(), CoverError>;
    async fn save_manual_cover(
        &self,
        work_id: WorkId,
        image_data: &[u8],
        content_type: &str,
    ) -> Result<(), CoverError>;
    fn expected_cover_path(&self, work_id: WorkId) -> PathBuf;
    fn delete_cover(&self, work_id: WorkId) -> Result<(), CoverError>;
}

#[derive(Debug, thiserror::Error)]
pub enum CoverError {
    #[error("cover download failed: {0}")]
    DownloadFailed(String),
    #[error("image conversion failed: {0}")]
    ConversionFailed(String),
    #[error("I/O error: {0}")]
    Io(String),
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
}

// =============================================================================
// Search Service
// =============================================================================

#[trait_variant::make(Send)]
pub trait SearchService: Send + Sync {
    async fn search_works(&self, query: &str) -> Result<Vec<WorkSearchResult>, MetadataError>;
    async fn search_authors(&self, query: &str) -> Result<Vec<AuthorSearchResult>, MetadataError>;
}

#[derive(Debug, Clone)]
pub struct WorkSearchResult {
    pub ol_key: String,
    pub title: String,
    pub author_name: Option<String>,
    pub author_ol_key: Option<String>,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AuthorSearchResult {
    pub ol_key: String,
    pub name: String,
    pub work_count: Option<i32>,
}

// =============================================================================
// OpenLibraryProvider — configurable test double
// =============================================================================

#[cfg(test)]
enum OlProviderMode {
    Works(Vec<ProviderSearchResult>),
    Authors(Vec<ProviderAuthorResult>),
    Detail(Box<ProviderWorkDetail>),
    Error(MetadataErrorKind),
}

// MetadataError isn't Clone, so store a reconstructible kind
#[cfg(test)]
enum MetadataErrorKind {
    NotConfigured,
    Timeout(Duration),
    RateLimited,
    AuthFailed,
    RequestFailed(String),
    InvalidResponse(String),
    NoMatch,
    UnsupportedOperation,
    AntiBotChallenge,
}

#[cfg(test)]
impl MetadataErrorKind {
    fn to_error(&self) -> MetadataError {
        match self {
            Self::NotConfigured => MetadataError::NotConfigured,
            Self::Timeout(d) => MetadataError::Timeout(*d),
            Self::RateLimited => MetadataError::RateLimited,
            Self::AuthFailed => MetadataError::AuthFailed,
            Self::RequestFailed(s) => MetadataError::RequestFailed(s.clone()),
            Self::InvalidResponse(s) => MetadataError::InvalidResponse(s.clone()),
            Self::NoMatch => MetadataError::NoMatch,
            Self::UnsupportedOperation => MetadataError::UnsupportedOperation,
            Self::AntiBotChallenge => MetadataError::AntiBotChallenge,
        }
    }
}

#[cfg(test)]
pub struct OpenLibraryProvider {
    mode: OlProviderMode,
}

#[cfg(test)]
impl OpenLibraryProvider {
    pub fn new_test(results: Vec<ProviderSearchResult>) -> Self {
        Self {
            mode: OlProviderMode::Works(results),
        }
    }

    pub fn new_test_authors(results: Vec<ProviderAuthorResult>) -> Self {
        Self {
            mode: OlProviderMode::Authors(results),
        }
    }

    pub fn new_test_detail(detail: ProviderWorkDetail) -> Self {
        Self {
            mode: OlProviderMode::Detail(Box::new(detail)),
        }
    }

    pub fn new_test_err(err: MetadataError) -> Self {
        let kind = match err {
            MetadataError::NotConfigured => MetadataErrorKind::NotConfigured,
            MetadataError::Timeout(d) => MetadataErrorKind::Timeout(d),
            MetadataError::RateLimited => MetadataErrorKind::RateLimited,
            MetadataError::AuthFailed => MetadataErrorKind::AuthFailed,
            MetadataError::RequestFailed(s) => MetadataErrorKind::RequestFailed(s),
            MetadataError::InvalidResponse(s) => MetadataErrorKind::InvalidResponse(s),
            MetadataError::NoMatch => MetadataErrorKind::NoMatch,
            MetadataError::UnsupportedOperation => MetadataErrorKind::UnsupportedOperation,
            MetadataError::AntiBotChallenge => MetadataErrorKind::AntiBotChallenge,
        };
        Self {
            mode: OlProviderMode::Error(kind),
        }
    }
}

#[cfg(test)]
impl MetadataProvider for OpenLibraryProvider {
    fn name(&self) -> &str {
        "openlibrary"
    }

    async fn search_works(&self, _query: &str) -> Result<Vec<ProviderSearchResult>, MetadataError> {
        match &self.mode {
            OlProviderMode::Works(r) => Ok(r.clone()),
            OlProviderMode::Error(k) => Err(k.to_error()),
            _ => Ok(vec![]),
        }
    }

    async fn search_authors(
        &self,
        _query: &str,
    ) -> Result<Vec<ProviderAuthorResult>, MetadataError> {
        match &self.mode {
            OlProviderMode::Authors(r) => Ok(r.clone()),
            OlProviderMode::Error(k) => Err(k.to_error()),
            _ => Ok(vec![]),
        }
    }

    async fn fetch_work_detail(
        &self,
        _provider_key: &str,
    ) -> Result<ProviderWorkDetail, MetadataError> {
        match &self.mode {
            OlProviderMode::Detail(d) => Ok(*d.clone()),
            OlProviderMode::Error(k) => Err(k.to_error()),
            _ => Err(MetadataError::NoMatch),
        }
    }
}

// =============================================================================
// OlSearchService — configurable test double
// =============================================================================

#[cfg(test)]
enum OlSearchMode {
    Works(Vec<WorkSearchResult>),
    Authors(Vec<AuthorSearchResult>),
}

#[cfg(test)]
pub struct OlSearchService {
    mode: OlSearchMode,
}

#[cfg(test)]
impl OlSearchService {
    pub fn new_test(results: Vec<WorkSearchResult>) -> Self {
        Self {
            mode: OlSearchMode::Works(results),
        }
    }

    pub fn new_test_authors(results: Vec<AuthorSearchResult>) -> Self {
        Self {
            mode: OlSearchMode::Authors(results),
        }
    }
}

#[cfg(test)]
impl SearchService for OlSearchService {
    async fn search_works(&self, _query: &str) -> Result<Vec<WorkSearchResult>, MetadataError> {
        match &self.mode {
            OlSearchMode::Works(r) => Ok(r.clone()),
            _ => Ok(vec![]),
        }
    }

    async fn search_authors(&self, _query: &str) -> Result<Vec<AuthorSearchResult>, MetadataError> {
        match &self.mode {
            OlSearchMode::Authors(r) => Ok(r.clone()),
            _ => Ok(vec![]),
        }
    }
}

// =============================================================================
// Test doubles module
// =============================================================================

#[cfg(test)]
pub mod tests {
    use super::*;

    // --- Enrichment stubs ---

    pub struct StubEnrichment {
        mode: StubEnrichmentMode,
    }

    // Renamed to avoid collision with the public EnrichmentMode added for metadata-overhaul.
    enum StubEnrichmentMode {
        Success,
        Partial,
        AllFail,
        NotFound,
        ManualCover,
        LlmFallback,
    }

    impl EnrichmentService for StubEnrichment {
        async fn enrich_work(
            &self,
            _user_id: UserId,
            _work_id: WorkId,
            _mode: EnrichmentMode,
        ) -> Result<EnrichmentResult, EnrichmentError> {
            // TEMP(pk-tdd): stub uses internal scenario mode; real work_id lookup not needed here.
            let work = Work::default();
            match self.mode {
                StubEnrichmentMode::Success => Ok(EnrichmentResult {
                    enrichment_status: EnrichmentStatus::Enriched,
                    enrichment_source: Some("hardcover+audnexus".to_string()),
                    llm_task_spawned: false,
                    work: Work {
                        title: "Enriched Title".to_string(),
                        ..work
                    },
                    merge_deferred: false,
                    provider_outcomes: std::collections::HashMap::new(),
                }),
                StubEnrichmentMode::Partial => Ok(EnrichmentResult {
                    enrichment_status: EnrichmentStatus::Partial,
                    enrichment_source: Some("openlibrary".to_string()),
                    llm_task_spawned: false,
                    work: Work {
                        title: "Partial Title".to_string(),
                        ..work
                    },
                    merge_deferred: false,
                    provider_outcomes: std::collections::HashMap::new(),
                }),
                StubEnrichmentMode::AllFail => Ok(EnrichmentResult {
                    enrichment_status: EnrichmentStatus::Failed,
                    enrichment_source: None,
                    llm_task_spawned: false,
                    work: Work {
                        title: if work.title.is_empty() {
                            "Retained".to_string()
                        } else {
                            work.title.clone()
                        },
                        ..work
                    },
                    merge_deferred: false,
                    provider_outcomes: std::collections::HashMap::new(),
                }),
                StubEnrichmentMode::NotFound => Err(EnrichmentError::WorkNotFound),
                StubEnrichmentMode::ManualCover => Ok(EnrichmentResult {
                    enrichment_status: EnrichmentStatus::Enriched,
                    enrichment_source: Some("hardcover".to_string()),
                    llm_task_spawned: false,
                    work: Work {
                        cover_manual: true,
                        ..work
                    },
                    merge_deferred: false,
                    provider_outcomes: std::collections::HashMap::new(),
                }),
                StubEnrichmentMode::LlmFallback => Ok(EnrichmentResult {
                    enrichment_status: EnrichmentStatus::Enriched,
                    enrichment_source: Some("hardcover".to_string()),
                    llm_task_spawned: true,
                    work,
                    merge_deferred: false,
                    provider_outcomes: std::collections::HashMap::new(),
                }),
            }
        }

        async fn reset_for_manual_refresh(
            &self,
            _user_id: UserId,
            _work_id: WorkId,
        ) -> Result<(), EnrichmentError> {
            Ok(())
        }
    }

    pub fn enrichment_stub_success() -> StubEnrichment {
        StubEnrichment {
            mode: StubEnrichmentMode::Success,
        }
    }
    pub fn enrichment_stub_partial() -> StubEnrichment {
        StubEnrichment {
            mode: StubEnrichmentMode::Partial,
        }
    }
    pub fn enrichment_stub_all_fail() -> StubEnrichment {
        StubEnrichment {
            mode: StubEnrichmentMode::AllFail,
        }
    }
    pub fn enrichment_stub_not_found() -> StubEnrichment {
        StubEnrichment {
            mode: StubEnrichmentMode::NotFound,
        }
    }
    pub fn enrichment_stub_manual_cover() -> StubEnrichment {
        StubEnrichment {
            mode: StubEnrichmentMode::ManualCover,
        }
    }
    pub fn enrichment_stub_llm_fallback() -> StubEnrichment {
        StubEnrichment {
            mode: StubEnrichmentMode::LlmFallback,
        }
    }

    // --- Matcher stubs ---

    pub struct StubMatcher {
        mode: MatcherMode,
    }

    enum MatcherMode {
        Hit,
        Tiebreaker,
        Ambiguous,
        LlmTimeout,
        LlmSuccess,
    }

    impl HardcoverMatcher for StubMatcher {
        async fn match_deterministic(
            &self,
            _title: &str,
            _author: &str,
            candidates: &[HardcoverCandidate],
        ) -> Option<HardcoverCandidate> {
            match self.mode {
                MatcherMode::Hit => candidates.first().cloned(),
                MatcherMode::Tiebreaker => candidates
                    .iter()
                    .max_by_key(|c| c.users_read_count)
                    .cloned(),
                MatcherMode::Ambiguous | MatcherMode::LlmTimeout | MatcherMode::LlmSuccess => {
                    None // ambiguous — defer to LLM
                }
            }
        }

        async fn match_llm(
            &self,
            _work_id: WorkId,
            _title: &str,
            _author: &str,
            candidates: &[HardcoverCandidate],
        ) -> Result<HardcoverCandidate, MetadataError> {
            match self.mode {
                MatcherMode::LlmTimeout => Err(MetadataError::Timeout(Duration::from_secs(30))),
                MatcherMode::LlmSuccess => {
                    candidates.first().cloned().ok_or(MetadataError::NoMatch)
                }
                _ => Err(MetadataError::NoMatch),
            }
        }
    }

    pub fn matcher_deterministic_hit() -> StubMatcher {
        StubMatcher {
            mode: MatcherMode::Hit,
        }
    }
    pub fn matcher_deterministic_tiebreaker() -> StubMatcher {
        StubMatcher {
            mode: MatcherMode::Tiebreaker,
        }
    }
    pub fn matcher_deterministic_ambiguous() -> StubMatcher {
        StubMatcher {
            mode: MatcherMode::Ambiguous,
        }
    }
    pub fn matcher_llm_timeout() -> StubMatcher {
        StubMatcher {
            mode: MatcherMode::LlmTimeout,
        }
    }
    pub fn matcher_llm_success() -> StubMatcher {
        StubMatcher {
            mode: MatcherMode::LlmSuccess,
        }
    }

    // --- Cover cache stubs ---

    pub struct StubCoverCache {
        mode: CoverCacheMode,
    }

    enum CoverCacheMode {
        Normal(String),
        DownloadFail,
        UnsupportedFormat,
    }

    impl CoverCache for StubCoverCache {
        async fn cache_cover(&self, _work_id: WorkId, _cover_url: &str) -> Result<(), CoverError> {
            match &self.mode {
                CoverCacheMode::DownloadFail => Err(CoverError::DownloadFailed(
                    "test download failure".to_string(),
                )),
                _ => Ok(()),
            }
        }

        async fn save_manual_cover(
            &self,
            _work_id: WorkId,
            _image_data: &[u8],
            content_type: &str,
        ) -> Result<(), CoverError> {
            match &self.mode {
                CoverCacheMode::UnsupportedFormat => {
                    Err(CoverError::UnsupportedFormat(content_type.to_string()))
                }
                _ => Ok(()),
            }
        }

        fn expected_cover_path(&self, work_id: WorkId) -> PathBuf {
            match &self.mode {
                CoverCacheMode::Normal(dir) => PathBuf::from(dir)
                    .join("MediaCover")
                    .join(work_id.to_string())
                    .join("cover.jpg"),
                _ => std::env::temp_dir()
                    .join("livrarr")
                    .join("MediaCover")
                    .join(work_id.to_string())
                    .join("cover.jpg"),
            }
        }

        fn delete_cover(&self, _work_id: WorkId) -> Result<(), CoverError> {
            Ok(())
        }
    }

    pub fn cover_cache_stub(data_dir: &str) -> StubCoverCache {
        StubCoverCache {
            mode: CoverCacheMode::Normal(data_dir.to_string()),
        }
    }

    pub fn cover_cache_download_fail() -> StubCoverCache {
        StubCoverCache {
            mode: CoverCacheMode::DownloadFail,
        }
    }

    pub fn cover_cache_unsupported_format() -> StubCoverCache {
        StubCoverCache {
            mode: CoverCacheMode::UnsupportedFormat,
        }
    }

    // --- LLM stubs ---

    pub struct StubLlmClient {
        mode: LlmMode,
    }

    enum LlmMode {
        Ok(String),
        Err(LlmErrorKind),
    }

    enum LlmErrorKind {
        NotConfigured,
        Timeout,
        RateLimited,
        RequestFailed(String),
        InvalidResponse(String),
    }

    impl LlmClient for StubLlmClient {
        async fn chat_completion(&self, _messages: Vec<LlmMessage>) -> Result<String, LlmError> {
            match &self.mode {
                LlmMode::Ok(s) => Ok(s.clone()),
                LlmMode::Err(k) => Err(match k {
                    LlmErrorKind::NotConfigured => LlmError::NotConfigured,
                    LlmErrorKind::Timeout => LlmError::Timeout(Duration::from_secs(30)),
                    LlmErrorKind::RateLimited => LlmError::RateLimited,
                    LlmErrorKind::RequestFailed(s) => LlmError::RequestFailed(s.clone()),
                    LlmErrorKind::InvalidResponse(s) => LlmError::InvalidResponse(s.clone()),
                }),
            }
        }
    }

    pub fn llm_stub_ok(response: &str) -> StubLlmClient {
        StubLlmClient {
            mode: LlmMode::Ok(response.to_string()),
        }
    }

    pub fn llm_stub_err(err: LlmError) -> StubLlmClient {
        let kind = match err {
            LlmError::NotConfigured => LlmErrorKind::NotConfigured,
            LlmError::Timeout(_) => LlmErrorKind::Timeout,
            LlmError::RateLimited => LlmErrorKind::RateLimited,
            LlmError::RequestFailed(s) => LlmErrorKind::RequestFailed(s),
            LlmError::InvalidResponse(s) => LlmErrorKind::InvalidResponse(s),
        };
        StubLlmClient {
            mode: LlmMode::Err(kind),
        }
    }
}
