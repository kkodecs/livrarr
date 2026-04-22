use livrarr_db::{EnrichmentRetryDb, WorkDb};
use livrarr_domain::services::{
    EnrichmentMode as DomainEnrichmentMode, EnrichmentResult as DomainEnrichmentResult,
    EnrichmentWorkflow, EnrichmentWorkflowError,
};
use livrarr_domain::*;

use crate::{EnrichmentError, EnrichmentMode, EnrichmentService};
use std::sync::Arc;

/// Adapter that implements the domain's EnrichmentWorkflow trait by delegating
/// to the existing EnrichmentServiceImpl. Converts between metadata-crate types
/// and domain-crate types.
pub struct EnrichmentWorkflowImpl<S, D> {
    inner: Arc<S>,
    #[allow(dead_code)]
    db: D,
}

impl<S, D> EnrichmentWorkflowImpl<S, D> {
    pub fn new(inner: Arc<S>, db: D) -> Self {
        Self { inner, db }
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

impl<S, D> EnrichmentWorkflow for EnrichmentWorkflowImpl<S, D>
where
    S: EnrichmentService + Send + Sync,
    D: WorkDb + EnrichmentRetryDb + Send + Sync,
{
    async fn enrich_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
        mode: DomainEnrichmentMode,
    ) -> Result<DomainEnrichmentResult, EnrichmentWorkflowError> {
        let metadata_mode = convert_mode(mode);

        let result = self
            .inner
            .enrich_work(user_id, work_id, metadata_mode)
            .await
            .map_err(convert_error)?;

        Ok(DomainEnrichmentResult {
            enrichment_status: result.enrichment_status,
            enrichment_source: result.enrichment_source,
            work: result.work,
            merge_deferred: result.merge_deferred,
            provider_outcomes: result.provider_outcomes,
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
}
