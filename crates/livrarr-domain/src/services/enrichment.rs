use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    CoverResolution, DbError, EnrichmentStatus, MetadataProvider, OutcomeClass, RequestPriority,
    Work, WorkId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrichmentMode {
    Background,
    Manual,
    HardRefresh,
}

#[derive(Debug)]
pub struct EnrichmentResult {
    pub enrichment_status: EnrichmentStatus,
    pub enrichment_source: Option<String>,
    pub work: Work,
    pub merge_deferred: bool,
    pub provider_outcomes: HashMap<MetadataProvider, OutcomeClass>,
    pub cover_resolution: Option<CoverResolution>,
    pub audiobook_cover_resolution: Option<CoverResolution>,
    /// Seam-2 signal (REQ-002): enrichment found NO provider payload matching the
    /// locked identity (the LLM rejected them all). Enrichment never writes
    /// identity state — it raises this flag and the caller writes
    /// [`crate::IdentityStatus::NotFound`]. `false` on every normal outcome.
    pub identity_not_found: bool,
    /// True when the merge actually changed any work field, external ID, or cover
    /// resolution. Drives the materialize gate in `run_unified_enrichment` (REQ-012):
    /// cover download + retag runs only when `changed = true`.
    pub changed: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum EnrichmentWorkflowError {
    #[error("work not found")]
    WorkNotFound,
    #[error("merge superseded after CAS retries")]
    MergeSuperseded,
    #[error("merge error: {0}")]
    Merge(String),
    #[error("all providers exhausted for work {work_id}")]
    ProviderExhausted { work_id: WorkId },
    #[error("corrupt retry payload for {provider:?} on work {work_id}")]
    CorruptRetryPayload {
        work_id: WorkId,
        provider: MetadataProvider,
    },
    #[error("provider queue error: {0}")]
    Queue(String),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

#[trait_variant::make(Send)]
pub trait EnrichmentWorkflow: Send + Sync {
    /// `priority` is the queue-ordering hint (B4) for this call's provider
    /// dispatch — independent of `mode`: a door can request Background mode
    /// (suppression/budget semantics) while still wanting its scatter to
    /// queue ahead of a background scan (e.g. the add door: Background mode,
    /// High priority).
    /// `freshness` decides whether provider fetches may be satisfied from the
    /// persistent provider-response cache (REQ-009) — orthogonal to
    /// `priority` (D-004).
    async fn enrich_work(
        &self,
        user_id: crate::UserId,
        work_id: WorkId,
        mode: EnrichmentMode,
        candidate_id: Option<crate::identity::CandidateId>,
        priority: RequestPriority,
        freshness: crate::Freshness,
    ) -> Result<EnrichmentResult, EnrichmentWorkflowError>;
    async fn reset_for_manual_refresh(
        &self,
        user_id: crate::UserId,
        work_id: WorkId,
    ) -> Result<(), EnrichmentWorkflowError>;

    /// Pre-inject source provider data before calling `enrich_work`.
    /// The data is consumed during scatter-gather as a Readarr provider outcome.
    ///
    /// Stubs and test implementations provide a no-op body.
    /// `EnrichmentWorkflowImpl` delegates to `EnrichmentServiceImpl::pre_inject_source_data`.
    async fn inject_source_data(
        &self,
        user_id: crate::UserId,
        work_id: WorkId,
        data: super::SourceProviderData,
    );
}
