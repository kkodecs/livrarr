use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    AnchorQuery, CoverResolution, DbError, EnrichmentStatus, MetadataProvider, OutcomeClass,
    RequestPriority, Work, WorkId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrichmentMode {
    Background,
    Manual,
    HardRefresh,
}

/// Live availability of the three REQ-027 title+author route-search legs.
///
/// The provider queue authors these facts from its registered clients and
/// current credential configuration. Persistence consumes them as query bind
/// parameters; SQL must never infer runtime provider fireability from schema
/// shape alone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdentitySearchAvailability {
    pub open_library: bool,
    pub goodreads: bool,
    pub hardcover: bool,
}

impl IdentitySearchAvailability {
    /// Compatibility value for callers that predate live availability binding.
    pub const fn all() -> Self {
        Self {
            open_library: true,
            goodreads: true,
            hardcover: true,
        }
    }
}

/// Evidence-only result of a connected Work's REQ-027 search entrance.
/// Anchored enrichment payloads and metadata merge state are deliberately
/// absent: this result can only feed the identity-road handoff and chase ledger.
#[derive(Debug, Clone, Default)]
pub struct IdentityRouteSearchResult {
    pub captured_provider_identity: Vec<crate::identity_layer::ProviderIdentityEvidence>,
    pub captured_route_proposals: Vec<crate::identity_layer::RouteKey>,
    pub provider_chase_attempted: bool,
    pub search_leg_fired: bool,
    pub search_ledger_burnable: bool,
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
    /// True when the pass ran to completion — a provider dispatch or a
    /// cached-candidate merge concluded (including a no-op merge). False only
    /// when the pass ended with no provider dispatch and no merge application.
    /// The history writer records no metadata event for an unattempted pass.
    pub attempted: bool,
    /// Work-level provider identities observed in successful payloads. This
    /// layer reports evidence only; the identity road is the sole writer.
    pub captured_provider_identity: Vec<crate::identity_layer::ProviderIdentityEvidence>,
    pub captured_route_proposals: Vec<crate::identity_layer::RouteKey>,
    /// Honest provider chase marker (skips and source-only merges are false).
    pub provider_chase_attempted: bool,
    /// REQ-027 v11 ledger discriminator. True only when at least one
    /// title+author route-search leg was actually spawned in this pass.
    pub search_leg_fired: bool,
    /// True only when every spawned route-search leg concluded with an honest
    /// miss or proposal card, making the shared generation ledger burnable.
    pub search_ledger_burnable: bool,
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

    /// Run only REQ-027 route-search legs for an enrichment-complete connected
    /// Work. The production workflow overrides this evidence-only default;
    /// doubles without a provider registry truthfully report that no leg fired.
    fn search_work_routes(
        &self,
        user_id: crate::UserId,
        work_id: WorkId,
        priority: RequestPriority,
    ) -> impl std::future::Future<Output = Result<IdentityRouteSearchResult, EnrichmentWorkflowError>>
           + Send {
        let _ = (user_id, work_id, priority);
        async move { Ok(IdentityRouteSearchResult::default()) }
    }

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

    /// One preview fetch against the named provider by anchor query (design
    /// identity-edit r4 §Preview seam). Returns the domain preview record —
    /// this trait never names the enrichment-layer payload type. No provider
    /// response cache, no retry-state writes, no budget bookkeeping; call
    /// records emit at the client wrapper as usual.
    ///
    /// Stub default (desugared — `trait_variant` cannot expand a provided
    /// `async fn`): reports `NotConfigured`, the truthful answer for a
    /// workflow with no provider registry.
    fn fetch_anchor_preview(
        &self,
        provider: MetadataProvider,
        query: AnchorQuery,
        language: Option<String>,
        priority: RequestPriority,
    ) -> impl std::future::Future<
        Output = Result<super::IdentityPreviewOutcome, EnrichmentWorkflowError>,
    > + Send {
        let _ = (provider, query, language, priority);
        async move { Ok(super::IdentityPreviewOutcome::NotConfigured) }
    }
}
