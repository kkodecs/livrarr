use livrarr_db::{EnrichmentRetryDb, WorkDb};
use livrarr_domain::services::{
    EnrichmentMode as DomainEnrichmentMode, EnrichmentResult as DomainEnrichmentResult,
    EnrichmentWorkflow, EnrichmentWorkflowError, SourceProviderData,
};
use livrarr_domain::*;

use crate::{EnrichmentError, EnrichmentMode, EnrichmentService};
use std::sync::Arc;

/// Adapter that implements the domain's EnrichmentWorkflow trait by delegating
/// to the existing EnrichmentServiceImpl. Converts between metadata-crate types
/// and domain-crate types.
pub struct EnrichmentWorkflowImpl<S> {
    inner: Arc<S>,
}

impl<S> EnrichmentWorkflowImpl<S> {
    pub fn new(inner: Arc<S>) -> Self {
        Self { inner }
    }
}

fn convert_mode(mode: DomainEnrichmentMode) -> EnrichmentMode {
    match mode {
        DomainEnrichmentMode::Background => EnrichmentMode::Background,
        DomainEnrichmentMode::Manual => EnrichmentMode::Manual,
        DomainEnrichmentMode::HardRefresh => EnrichmentMode::HardRefresh,
    }
}

fn convert_error(e: EnrichmentError) -> EnrichmentWorkflowError {
    match e {
        EnrichmentError::WorkNotFound => EnrichmentWorkflowError::WorkNotFound,
        EnrichmentError::MergeSuperseded => EnrichmentWorkflowError::MergeSuperseded,
        EnrichmentError::CorruptRetryPayload { work_id, provider } => {
            EnrichmentWorkflowError::CorruptRetryPayload { work_id, provider }
        }
        EnrichmentError::Queue(e) => EnrichmentWorkflowError::Queue(e.to_string()),
        EnrichmentError::Merge(e) => EnrichmentWorkflowError::Merge(e.to_string()),
        EnrichmentError::Db(e) => EnrichmentWorkflowError::Db(e),
        EnrichmentError::AllProvidersFailed => {
            EnrichmentWorkflowError::Queue("all providers failed".into())
        }
    }
}

impl<S> EnrichmentWorkflow for EnrichmentWorkflowImpl<S>
where
    S: EnrichmentService + Send + Sync,
{
    async fn enrich_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
        mode: DomainEnrichmentMode,
        candidate_id: Option<livrarr_domain::identity::CandidateId>,
        priority: RequestPriority,
        freshness: livrarr_domain::Freshness,
    ) -> Result<DomainEnrichmentResult, EnrichmentWorkflowError> {
        let metadata_mode = convert_mode(mode);

        let result = self
            .inner
            .enrich_work(
                user_id,
                work_id,
                metadata_mode,
                candidate_id,
                priority,
                freshness,
            )
            .await
            .map_err(convert_error)?;

        Ok(DomainEnrichmentResult {
            enrichment_status: result.enrichment_status,
            enrichment_source: result.enrichment_source,
            work: result.work,
            merge_deferred: result.merge_deferred,
            provider_outcomes: result.provider_outcomes,
            cover_resolution: result.cover_resolution,
            audiobook_cover_resolution: result.audiobook_cover_resolution,
            identity_not_found: result.identity_not_found,
            changed: result.changed,
            attempted: result.attempted,
        })
    }

    async fn reset_for_manual_refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<(), EnrichmentWorkflowError> {
        self.inner
            .reset_for_manual_refresh(user_id, work_id)
            .await
            .map_err(convert_error)
    }

    async fn inject_source_data(&self, user_id: UserId, work_id: WorkId, data: SourceProviderData) {
        self.inner.inject_source_data(user_id, work_id, data).await;
    }

    async fn fetch_anchor_preview(
        &self,
        provider: MetadataProvider,
        query: AnchorQuery,
        language: Option<String>,
        priority: RequestPriority,
    ) -> Result<livrarr_domain::services::IdentityPreviewOutcome, EnrichmentWorkflowError> {
        use livrarr_domain::services::{IdentityPreviewOutcome, IdentityPreviewRecord};
        Ok(
            match self
                .inner
                .preview_fetch(provider, query, language, priority)
                .await
            {
                livrarr_enrichment::PreviewFetchOutcome::Resolved(detail) => {
                    // The adapter is where enrichment-layer payloads become
                    // domain results — livrarr-domain never names
                    // NormalizedWorkDetail (AC-25).
                    IdentityPreviewOutcome::Resolved(Box::new(IdentityPreviewRecord {
                        title: detail.title.clone(),
                        author: detail.author_name.clone(),
                        year: detail.year,
                        language: detail.language.clone(),
                        cover_url: detail.cover_url.clone(),
                        ol_key: detail.ol_key.clone(),
                        gr_key: detail.gr_key.clone(),
                        hc_key: detail.hc_key.clone(),
                        isbn_13: detail.isbn_13.clone(),
                        asin: detail.asin.clone(),
                    }))
                }
                livrarr_enrichment::PreviewFetchOutcome::NotFound => {
                    IdentityPreviewOutcome::NotFound
                }
                livrarr_enrichment::PreviewFetchOutcome::NotConfigured => {
                    IdentityPreviewOutcome::NotConfigured
                }
                livrarr_enrichment::PreviewFetchOutcome::Unavailable => {
                    IdentityPreviewOutcome::Unavailable
                }
            },
        )
    }
}

/// Standalone impl for tests that only need reset, not the full enrichment pipeline.
pub struct ResetOnlyEnrichmentWorkflow<D> {
    db: D,
}

impl<D> ResetOnlyEnrichmentWorkflow<D> {
    pub fn new(db: D) -> Self {
        Self { db }
    }
}

impl<D> EnrichmentWorkflow for ResetOnlyEnrichmentWorkflow<D>
where
    D: WorkDb + EnrichmentRetryDb + Send + Sync,
{
    async fn enrich_work(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _mode: DomainEnrichmentMode,
        _candidate_id: Option<livrarr_domain::identity::CandidateId>,
        _priority: RequestPriority,
        _freshness: livrarr_domain::Freshness,
    ) -> Result<DomainEnrichmentResult, EnrichmentWorkflowError> {
        Err(EnrichmentWorkflowError::Queue(
            "enrichment not available in reset-only mode".into(),
        ))
    }

    async fn reset_for_manual_refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<(), EnrichmentWorkflowError> {
        self.db
            .get_work(user_id, work_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => EnrichmentWorkflowError::WorkNotFound,
                other => EnrichmentWorkflowError::Db(other),
            })?;

        self.db
            .reset_for_manual_refresh(user_id, work_id)
            .await
            .map_err(EnrichmentWorkflowError::Db)?;

        Ok(())
    }

    async fn inject_source_data(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _data: SourceProviderData,
    ) {
        // no-op — reset-only workflow does not run enrichment
    }
}
