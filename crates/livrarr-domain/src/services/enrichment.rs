use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{DbError, EnrichmentStatus, MetadataProvider, OutcomeClass, Work, WorkId};

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
    async fn enrich_work(
        &self,
        user_id: crate::UserId,
        work_id: WorkId,
        mode: EnrichmentMode,
    ) -> Result<EnrichmentResult, EnrichmentWorkflowError>;
    async fn reset_for_manual_refresh(
        &self,
        user_id: crate::UserId,
        work_id: WorkId,
    ) -> Result<(), EnrichmentWorkflowError>;
}
