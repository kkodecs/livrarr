pub use livrarr_domain::*;

use std::path::PathBuf;
use std::time::Duration;

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
    pub hardcover_id: Option<String>,
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
}

// =============================================================================
// Enrichment Service
// =============================================================================

#[trait_variant::make(Send)]
pub trait EnrichmentService: Send + Sync {
    async fn enrich_work(&self, work: &Work) -> Result<EnrichmentResult, EnrichmentError>;
    async fn refresh_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<EnrichmentResult, EnrichmentError>;
}

pub struct EnrichmentResult {
    pub enrichment_status: EnrichmentStatus,
    pub enrichment_source: Option<String>,
    pub llm_task_spawned: bool,
    pub work: Work,
}

#[derive(Debug, thiserror::Error)]
pub enum EnrichmentError {
    #[error("work not found")]
    WorkNotFound,
    #[error("database error: {0}")]
    Db(#[from] DbError),
    #[error("all providers failed")]
    AllProvidersFailed,
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
    pub hardcover_id: String,
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

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("LLM not configured")]
    NotConfigured,
    #[error("LLM request failed: {0}")]
    RequestFailed(String),
    #[error("LLM timeout")]
    Timeout,
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

enum OlProviderMode {
    Works(Vec<ProviderSearchResult>),
    Authors(Vec<ProviderAuthorResult>),
    Detail(Box<ProviderWorkDetail>),
    Error(MetadataErrorKind),
}

// MetadataError isn't Clone, so store a reconstructible kind
enum MetadataErrorKind {
    NotConfigured,
    Timeout(Duration),
    RateLimited,
    AuthFailed,
    RequestFailed(String),
}

impl MetadataErrorKind {
    fn to_error(&self) -> MetadataError {
        match self {
            Self::NotConfigured => MetadataError::NotConfigured,
            Self::Timeout(d) => MetadataError::Timeout(*d),
            Self::RateLimited => MetadataError::RateLimited,
            Self::AuthFailed => MetadataError::AuthFailed,
            Self::RequestFailed(s) => MetadataError::RequestFailed(s.clone()),
        }
    }
}

pub struct OpenLibraryProvider {
    mode: OlProviderMode,
}

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
            _ => MetadataErrorKind::RequestFailed("unknown".to_string()),
        };
        Self {
            mode: OlProviderMode::Error(kind),
        }
    }
}

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

enum OlSearchMode {
    Works(Vec<WorkSearchResult>),
    Authors(Vec<AuthorSearchResult>),
}

pub struct OlSearchService {
    mode: OlSearchMode,
}

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
        mode: EnrichmentMode,
    }

    enum EnrichmentMode {
        Success,
        Partial,
        AllFail,
        NotFound,
        ManualCover,
        LlmFallback,
    }

    impl EnrichmentService for StubEnrichment {
        async fn enrich_work(&self, work: &Work) -> Result<EnrichmentResult, EnrichmentError> {
            match self.mode {
                EnrichmentMode::Success => Ok(EnrichmentResult {
                    enrichment_status: EnrichmentStatus::Enriched,
                    enrichment_source: Some("hardcover+audnexus".to_string()),
                    llm_task_spawned: false,
                    work: Work {
                        title: "Enriched Title".to_string(),
                        ..work.clone()
                    },
                }),
                EnrichmentMode::Partial => Ok(EnrichmentResult {
                    enrichment_status: EnrichmentStatus::Partial,
                    enrichment_source: Some("openlibrary".to_string()),
                    llm_task_spawned: false,
                    work: Work {
                        title: "Partial Title".to_string(),
                        ..work.clone()
                    },
                }),
                EnrichmentMode::AllFail => Ok(EnrichmentResult {
                    enrichment_status: EnrichmentStatus::Failed,
                    enrichment_source: None,
                    llm_task_spawned: false,
                    work: Work {
                        title: if work.title.is_empty() {
                            "Retained".to_string()
                        } else {
                            work.title.clone()
                        },
                        ..work.clone()
                    },
                }),
                EnrichmentMode::NotFound => Err(EnrichmentError::WorkNotFound),
                EnrichmentMode::ManualCover => Ok(EnrichmentResult {
                    enrichment_status: EnrichmentStatus::Enriched,
                    enrichment_source: Some("hardcover".to_string()),
                    llm_task_spawned: false,
                    work: Work {
                        cover_manual: true,
                        ..work.clone()
                    },
                }),
                EnrichmentMode::LlmFallback => Ok(EnrichmentResult {
                    enrichment_status: EnrichmentStatus::Enriched,
                    enrichment_source: Some("hardcover".to_string()),
                    llm_task_spawned: true,
                    work: work.clone(),
                }),
            }
        }

        async fn refresh_work(
            &self,
            _user_id: UserId,
            _work_id: WorkId,
        ) -> Result<EnrichmentResult, EnrichmentError> {
            self.enrich_work(&Work::default()).await
        }
    }

    pub fn enrichment_stub_success() -> StubEnrichment {
        StubEnrichment {
            mode: EnrichmentMode::Success,
        }
    }
    pub fn enrichment_stub_partial() -> StubEnrichment {
        StubEnrichment {
            mode: EnrichmentMode::Partial,
        }
    }
    pub fn enrichment_stub_all_fail() -> StubEnrichment {
        StubEnrichment {
            mode: EnrichmentMode::AllFail,
        }
    }
    pub fn enrichment_stub_not_found() -> StubEnrichment {
        StubEnrichment {
            mode: EnrichmentMode::NotFound,
        }
    }
    pub fn enrichment_stub_manual_cover() -> StubEnrichment {
        StubEnrichment {
            mode: EnrichmentMode::ManualCover,
        }
    }
    pub fn enrichment_stub_llm_fallback() -> StubEnrichment {
        StubEnrichment {
            mode: EnrichmentMode::LlmFallback,
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
                    if candidates.is_empty() {
                        return None;
                    }
                    None // ambiguous
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
                _ => PathBuf::from("/tmp/livrarr")
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
    }

    impl LlmClient for StubLlmClient {
        async fn chat_completion(&self, _messages: Vec<LlmMessage>) -> Result<String, LlmError> {
            match &self.mode {
                LlmMode::Ok(s) => Ok(s.clone()),
                LlmMode::Err(k) => Err(match k {
                    LlmErrorKind::NotConfigured => LlmError::NotConfigured,
                    LlmErrorKind::Timeout => LlmError::Timeout,
                    LlmErrorKind::RateLimited => LlmError::RateLimited,
                    LlmErrorKind::RequestFailed(s) => LlmError::RequestFailed(s.clone()),
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
            LlmError::Timeout => LlmErrorKind::Timeout,
            LlmError::RateLimited => LlmErrorKind::RateLimited,
            LlmError::RequestFailed(s) => LlmErrorKind::RequestFailed(s),
            LlmError::InvalidResponse(s) => LlmErrorKind::RequestFailed(s),
        };
        StubLlmClient {
            mode: LlmMode::Err(kind),
        }
    }
}
