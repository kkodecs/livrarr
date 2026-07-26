pub use livrarr_db::{
    ApplyEnrichmentMergeRequest, SetFieldProvenanceRequest, UpdateWorkEnrichmentDbRequest,
    UpsertExternalIdRequest,
};
pub use livrarr_domain::{
    ApplyMergeOutcome, DbError, EnrichmentStatus, FieldProvenance, LlmRole, MergeResolved,
    NarrationType, OutcomeClass, PermanentFailureReason, RequestPriority, UserId, WillRetryReason,
    Work, WorkField, WorkId,
};

use std::path::PathBuf;
use std::time::Duration;

pub mod author_service;
pub mod convergence_service;
pub mod cover;
pub mod cover_alternatives;
pub mod cover_layout_migration;
pub mod cover_provenance_backfill;
pub mod cover_startup;
pub mod cover_write_gate;
pub mod cover_write_gate_recovery;
pub mod discovery_service;
pub mod enrichment_workflow_service;
pub mod http_llm;
pub mod list_service;
pub mod preadd_cover_service;
pub mod series_link;
pub mod series_query_service;
pub mod series_service;
pub mod work_service;

// Re-export of the identity-resolution modules (now in livrarr-identity).
pub use livrarr_identity::{async_resolver, english_identity_resolver, title_cleanup};

pub mod author_monitor_workflow;
pub mod provenance;
pub mod rss_sync_workflow;

// D-014 transitional shim: the merge/enrich engine moved to livrarr-enrichment
// (4a). Re-exported here so existing dependents compile unchanged; AC-021 will
// switch consumers to direct livrarr_enrichment:: imports and delete this shim.
pub use livrarr_enrichment::provider_queue::{
    ApplicabilityRule, DefaultProviderQueue, DefaultProviderQueueBuilder,
};
pub use livrarr_enrichment::{
    cover_rank, cover_resolution, provider_queue, DefaultMergeEngine, EnrichmentContext,
    EnrichmentError, EnrichmentMode, EnrichmentResult, EnrichmentService, EnrichmentServiceImpl,
    MergeEngine, MergeError, MergeInput, MergeOutput, PriorityModel, ProviderQueue,
    ProviderQueueConfig, ProviderQueueError, ReconstructedOutcome, ScatterGatherResult,
};

use livrarr_external_data::{NormalizedWorkDetail, ProviderOutcome};

// =============================================================================
// Provider result types
// =============================================================================

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

pub(crate) fn strip_llm_fences(content: &str) -> &str {
    let trimmed = content.trim();
    trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed)
        .strip_suffix("```")
        .unwrap_or(trimmed)
        .trim()
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
            _candidate_id: Option<livrarr_domain::identity::CandidateId>,
            _priority: livrarr_domain::RequestPriority,
            _freshness: livrarr_domain::Freshness,
        ) -> Result<EnrichmentResult, EnrichmentError> {
            // TEMP(pk-tdd): stub uses internal scenario mode; real work_id lookup not needed here.
            let work = Work::default();
            match self.mode {
                StubEnrichmentMode::Success => Ok(EnrichmentResult {
                    identity_not_found: false,
                    changed: true,
                    enrichment_status: EnrichmentStatus::Enriched,
                    enrichment_source: Some("hardcover+audnexus".to_string()),
                    llm_task_spawned: false,
                    work: Work {
                        title: "Enriched Title".to_string(),
                        ..work
                    },
                    merge_deferred: false,
                    provider_outcomes: std::collections::HashMap::new(),
                    cover_resolution: None,
                    audiobook_cover_resolution: None,
                    dissents: Vec::new(),
                    attempted: true,
                }),
                StubEnrichmentMode::Partial => Ok(EnrichmentResult {
                    identity_not_found: false,
                    changed: false,
                    enrichment_status: EnrichmentStatus::Unenriched,
                    enrichment_source: Some("openlibrary".to_string()),
                    llm_task_spawned: false,
                    work: Work {
                        title: "Partial Title".to_string(),
                        ..work
                    },
                    merge_deferred: false,
                    provider_outcomes: std::collections::HashMap::new(),
                    cover_resolution: None,
                    audiobook_cover_resolution: None,
                    dissents: Vec::new(),
                    attempted: true,
                }),
                StubEnrichmentMode::AllFail => Ok(EnrichmentResult {
                    identity_not_found: false,
                    changed: false,
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
                    cover_resolution: None,
                    audiobook_cover_resolution: None,
                    dissents: Vec::new(),
                    attempted: true,
                }),
                StubEnrichmentMode::NotFound => Err(EnrichmentError::WorkNotFound),
                StubEnrichmentMode::ManualCover => Ok(EnrichmentResult {
                    identity_not_found: false,
                    changed: false,
                    enrichment_status: EnrichmentStatus::Enriched,
                    enrichment_source: Some("hardcover".to_string()),
                    llm_task_spawned: false,
                    work: Work {
                        cover_manual: true,
                        ..work
                    },
                    merge_deferred: false,
                    provider_outcomes: std::collections::HashMap::new(),
                    cover_resolution: None,
                    audiobook_cover_resolution: None,
                    dissents: Vec::new(),
                    attempted: true,
                }),
                StubEnrichmentMode::LlmFallback => Ok(EnrichmentResult {
                    identity_not_found: false,
                    changed: false,
                    enrichment_status: EnrichmentStatus::Enriched,
                    enrichment_source: Some("hardcover".to_string()),
                    llm_task_spawned: true,
                    work,
                    merge_deferred: false,
                    provider_outcomes: std::collections::HashMap::new(),
                    cover_resolution: None,
                    audiobook_cover_resolution: None,
                    dissents: Vec::new(),
                    attempted: true,
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

        async fn inject_source_data(
            &self,
            _user_id: UserId,
            _work_id: WorkId,
            _data: livrarr_domain::services::SourceProviderData,
        ) {
            // no-op stub
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
