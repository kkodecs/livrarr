use livrarr_domain::services::*;
use livrarr_domain::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// =============================================================================
// TagwriteChapterExtractor — the real extraction delegate for import tests
// (mirrors the server's ChapterExtractorImpl so import behavior is identical;
// livrarr-library itself no longer carries a tagwrite edge — REQ-005)
// =============================================================================

pub struct TagwriteChapterExtractor;

impl ChapterExtractor for TagwriteChapterExtractor {
    fn extract_m4b_chapters(
        &self,
        path: &std::path::Path,
    ) -> Result<ChapterExtractionResult, ChapterExtractionError> {
        match livrarr_tagwrite::extract_m4b_chapters(path) {
            Ok(r) => Ok(ChapterExtractionResult {
                chapters: r
                    .chapters
                    .into_iter()
                    .map(|c| ExtractedChapter {
                        title: c.title,
                        start_time_secs: c.start_time_secs,
                    })
                    .collect(),
                duration_secs: r.duration_secs,
            }),
            Err(livrarr_tagwrite::ChapterExtractionError::IoError(e)) => {
                Err(ChapterExtractionError::IoError(e))
            }
            Err(livrarr_tagwrite::ChapterExtractionError::ParseError(e)) => {
                Err(ChapterExtractionError::ParseError(e))
            }
        }
    }
}

// =============================================================================
// StubHttpFetcher — returns canned responses
// =============================================================================

#[derive(Clone)]
pub struct StubHttpFetcher {
    responses: Arc<Mutex<Vec<Result<FetchResponse, FetchError>>>>,
    call_count: Arc<AtomicUsize>,
}

impl Default for StubHttpFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl StubHttpFetcher {
    pub fn new() -> Self {
        Self {
            responses: Arc::new(Mutex::new(vec![])),
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn with_response(response: Result<FetchResponse, FetchError>) -> Self {
        let s = Self::new();
        s.responses.lock().unwrap().push(response);
        s
    }

    pub fn with_ok(status: u16, body: Vec<u8>) -> Self {
        Self::with_response(Ok(FetchResponse {
            status,
            headers: vec![],
            body,
        }))
    }

    pub fn with_error(err: FetchError) -> Self {
        Self::with_response(Err(err))
    }

    /// Push an additional canned response to the queue.
    pub fn push_response(&self, response: Result<FetchResponse, FetchError>) {
        self.responses.lock().unwrap().push(response);
    }

    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }

    fn next_response(&self) -> Result<FetchResponse, FetchError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            Ok(FetchResponse {
                status: 200,
                headers: vec![],
                body: vec![],
            })
        } else if responses.len() == 1 {
            match &responses[0] {
                Ok(r) => Ok(FetchResponse {
                    status: r.status,
                    headers: r.headers.clone(),
                    body: r.body.clone(),
                }),
                Err(e) => Err(match e {
                    FetchError::Connection(s) => FetchError::Connection(s.clone()),
                    FetchError::Timeout(d) => FetchError::Timeout(*d),
                    FetchError::BodyTooLarge { max_bytes } => FetchError::BodyTooLarge {
                        max_bytes: *max_bytes,
                    },
                    FetchError::AntiBotDetected => FetchError::AntiBotDetected,
                    FetchError::Ssrf(s) => FetchError::Ssrf(s.clone()),
                    FetchError::HttpError {
                        status,
                        classification,
                    } => FetchError::HttpError {
                        status: *status,
                        classification: classification.clone(),
                    },
                    FetchError::RateLimited => FetchError::RateLimited,
                    FetchError::CircuitOpen { retry_after } => FetchError::CircuitOpen {
                        retry_after: *retry_after,
                    },
                }),
            }
        } else {
            responses.remove(0)
        }
    }
}

impl HttpFetcher for StubHttpFetcher {
    async fn fetch(&self, _req: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.next_response()
    }

    async fn fetch_ssrf_safe(&self, _req: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.next_response()
    }
}

// =============================================================================
// StubLlmCaller — validates fields, returns canned content
// =============================================================================

pub struct StubLlmCaller {
    configured: bool,
    response_content: String,
    should_fail: bool,
}

impl StubLlmCaller {
    pub fn configured(content: &str) -> Self {
        Self {
            configured: true,
            response_content: content.into(),
            should_fail: false,
        }
    }

    pub fn not_configured() -> Self {
        Self {
            configured: false,
            response_content: String::new(),
            should_fail: false,
        }
    }

    pub fn failing() -> Self {
        Self {
            configured: true,
            response_content: String::new(),
            should_fail: true,
        }
    }
}

impl LlmCaller for StubLlmCaller {
    async fn call(&self, req: LlmCallRequest) -> Result<LlmCallResponse, LlmError> {
        for field in req.context.keys() {
            if !req.allowed_fields.contains(field) {
                return Err(LlmError::DisallowedField { field: *field });
            }
        }

        if !self.configured {
            return Err(LlmError::NotConfigured);
        }

        if self.should_fail {
            return Err(LlmError::Provider("stub failure".into()));
        }

        Ok(LlmCallResponse {
            content: self.response_content.clone(),
            model_used: "stub-model".into(),
            elapsed: Duration::from_millis(1),
        })
    }
}

// =============================================================================
// StubEnrichmentWorkflow — returns canned enrichment result
// =============================================================================

#[derive(Clone)]
pub struct StubEnrichmentWorkflow {
    should_fail: bool,
    call_count: Arc<AtomicUsize>,
    work_ids: Arc<Mutex<Vec<WorkId>>>,
    candidate_ids: Arc<Mutex<Vec<Option<livrarr_domain::identity::CandidateId>>>>,
    /// (mode, priority) of every enrich_work call, in order — lets tests assert
    /// the dispatch settings a caller threaded through the pipeline.
    enrich_contexts: Arc<Mutex<Vec<(EnrichmentMode, livrarr_domain::RequestPriority)>>>,
    /// Freshness of every enrich_work call, in order — lets tests pin the
    /// per-door cache policy (REQ-009/D-004: refresh Bypass, background
    /// PreferCache).
    freshness_calls: Arc<Mutex<Vec<livrarr_domain::Freshness>>>,
    reset_call_count: Arc<AtomicUsize>,
    reset_work_ids: Arc<Mutex<Vec<WorkId>>>,
}

impl StubEnrichmentWorkflow {
    pub fn succeeding() -> Self {
        Self {
            should_fail: false,
            call_count: Arc::new(AtomicUsize::new(0)),
            work_ids: Arc::new(Mutex::new(Vec::new())),
            candidate_ids: Arc::new(Mutex::new(Vec::new())),
            enrich_contexts: Arc::new(Mutex::new(Vec::new())),
            freshness_calls: Arc::new(Mutex::new(Vec::new())),
            reset_call_count: Arc::new(AtomicUsize::new(0)),
            reset_work_ids: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn failing() -> Self {
        Self {
            should_fail: true,
            call_count: Arc::new(AtomicUsize::new(0)),
            work_ids: Arc::new(Mutex::new(Vec::new())),
            candidate_ids: Arc::new(Mutex::new(Vec::new())),
            enrich_contexts: Arc::new(Mutex::new(Vec::new())),
            freshness_calls: Arc::new(Mutex::new(Vec::new())),
            reset_call_count: Arc::new(AtomicUsize::new(0)),
            reset_work_ids: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn enrich_contexts(&self) -> Vec<(EnrichmentMode, livrarr_domain::RequestPriority)> {
        self.enrich_contexts.lock().unwrap().clone()
    }

    pub fn freshness_calls(&self) -> Vec<livrarr_domain::Freshness> {
        self.freshness_calls.lock().unwrap().clone()
    }

    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }

    pub fn work_ids(&self) -> Vec<WorkId> {
        self.work_ids.lock().unwrap().clone()
    }

    pub fn candidate_ids(&self) -> Vec<Option<livrarr_domain::identity::CandidateId>> {
        self.candidate_ids.lock().unwrap().clone()
    }

    pub fn reset_call_count(&self) -> usize {
        self.reset_call_count.load(Ordering::SeqCst)
    }

    pub fn reset_work_ids(&self) -> Vec<WorkId> {
        self.reset_work_ids.lock().unwrap().clone()
    }
}

impl EnrichmentWorkflow for StubEnrichmentWorkflow {
    async fn enrich_work(
        &self,
        _user_id: UserId,
        work_id: WorkId,
        mode: EnrichmentMode,
        candidate_id: Option<livrarr_domain::identity::CandidateId>,
        priority: livrarr_domain::RequestPriority,
        freshness: livrarr_domain::Freshness,
    ) -> Result<EnrichmentResult, EnrichmentWorkflowError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.freshness_calls.lock().unwrap().push(freshness);
        self.work_ids.lock().unwrap().push(work_id);
        self.candidate_ids.lock().unwrap().push(candidate_id);
        self.enrich_contexts.lock().unwrap().push((mode, priority));

        if self.should_fail {
            return Err(EnrichmentWorkflowError::Queue("stub failure".into()));
        }

        Ok(EnrichmentResult {
            enrichment_status: EnrichmentStatus::Enriched,
            enrichment_source: Some("stub".into()),
            work: Work::default(),
            merge_deferred: false,
            provider_outcomes: HashMap::new(),
            cover_resolution: None,
            audiobook_cover_resolution: None,
            identity_not_found: false,
            changed: true,
            attempted: true,
        })
    }

    async fn reset_for_manual_refresh(
        &self,
        _user_id: UserId,
        work_id: WorkId,
    ) -> Result<(), EnrichmentWorkflowError> {
        self.reset_call_count.fetch_add(1, Ordering::SeqCst);
        self.reset_work_ids.lock().unwrap().push(work_id);
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

// =============================================================================
// StubSeriesQueryService — returns canned series views
// =============================================================================

pub struct StubSeriesQueryService;

impl SeriesQueryService for StubSeriesQueryService {
    async fn list_enriched(
        &self,
        _user_id: UserId,
    ) -> Result<Vec<SeriesListView>, SeriesServiceError> {
        Ok(vec![])
    }

    async fn get_detail(
        &self,
        _user_id: UserId,
        _series_id: i64,
    ) -> Result<SeriesDetailView, SeriesServiceError> {
        Err(SeriesServiceError::NotFound)
    }

    async fn update_flags(
        &self,
        _user_id: UserId,
        _series_id: i64,
        _monitor_ebook: bool,
        _monitor_audiobook: bool,
        _language: Option<String>,
    ) -> Result<UpdateSeriesView, SeriesServiceError> {
        Err(SeriesServiceError::NotFound)
    }

    async fn resolve_gr_candidates(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
    ) -> Result<Vec<GrAuthorCandidateView>, SeriesServiceError> {
        Ok(vec![])
    }

    async fn list_author_series(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _raw: bool,
    ) -> Result<AuthorSeriesListView, SeriesServiceError> {
        Ok(AuthorSeriesListView {
            series: vec![],
            fetched_at: None,
            raw_available: false,
            filtered_count: 0,
            raw_count: 0,
        })
    }

    async fn refresh_author_series(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
    ) -> Result<AuthorSeriesListView, SeriesServiceError> {
        Ok(AuthorSeriesListView {
            series: vec![],
            fetched_at: None,
            raw_available: false,
            filtered_count: 0,
            raw_count: 0,
        })
    }

    async fn monitor_series(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _req: MonitorSeriesServiceRequest,
    ) -> Result<MonitorSeriesView, SeriesServiceError> {
        Err(SeriesServiceError::NotFound)
    }

    async fn run_series_monitor_worker(
        &self,
        _params: SeriesMonitorWorkerParams,
    ) -> Result<(), SeriesServiceError> {
        Ok(())
    }

    async fn promote_stub(
        &self,
        _user_id: UserId,
        _series_id: i64,
        _explicit_gr_key: Option<String>,
    ) -> Result<PromoteStubOutcome, SeriesServiceError> {
        Err(SeriesServiceError::NotFound)
    }

    async fn series_books(
        &self,
        _user_id: UserId,
        _series_id: i64,
    ) -> Result<SeriesBooksView, SeriesServiceError> {
        Err(SeriesServiceError::NotFound)
    }
}

// =============================================================================
// StubImportWorkflow — returns empty import results
// =============================================================================

pub struct StubImportWorkflow;

impl ImportWorkflow for StubImportWorkflow {
    async fn import_grab(
        &self,
        _user_id: UserId,
        grab_id: GrabId,
    ) -> Result<ImportResult, ImportWorkflowError> {
        Ok(ImportResult {
            grab_id,
            final_status: GrabStatus::Imported,
            imported_files: vec![],
            failed_files: vec![],
            skipped_files: vec![],
            warnings: vec![],
        })
    }

    async fn import_file(
        &self,
        _user_id: UserId,
        req: ImportFileRequest,
    ) -> Result<ImportFileOutcome, ImportWorkflowError> {
        Ok(ImportFileOutcome::Imported {
            item_id: 0,
            path: req.target_relative,
        })
    }
}

// =============================================================================
// StubRssSyncWorkflow — returns empty report
// =============================================================================

pub struct StubRssSyncWorkflow;

impl RssSyncWorkflow for StubRssSyncWorkflow {
    async fn run_sync(&self) -> Result<RssSyncReport, RssSyncError> {
        Ok(RssSyncReport::empty())
    }
}

// =============================================================================
// Test helper: create users
// =============================================================================

pub async fn create_test_user(db: &livrarr_db::sqlite::SqliteDb) -> i64 {
    use livrarr_db::UserDb;
    db.create_user(livrarr_db::CreateUserDbRequest {
        username: "testuser".into(),
        password_hash: "hash".into(),
        role: UserRole::Admin,
        api_key_hash: "testhash".into(),
    })
    .await
    .unwrap()
    .id
}

pub async fn create_second_test_user(db: &livrarr_db::sqlite::SqliteDb) -> i64 {
    use livrarr_db::UserDb;
    db.create_user(livrarr_db::CreateUserDbRequest {
        username: "otheruser".into(),
        password_hash: "hash".into(),
        role: UserRole::User,
        api_key_hash: "testhash2".into(),
    })
    .await
    .unwrap()
    .id
}
