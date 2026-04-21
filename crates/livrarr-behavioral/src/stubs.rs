use livrarr_domain::services::*;
use livrarr_domain::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

pub struct StubEnrichmentWorkflow {
    should_fail: bool,
}

impl StubEnrichmentWorkflow {
    pub fn succeeding() -> Self {
        Self { should_fail: false }
    }

    pub fn failing() -> Self {
        Self { should_fail: true }
    }
}

impl EnrichmentWorkflow for StubEnrichmentWorkflow {
    async fn enrich_work(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _mode: EnrichmentMode,
    ) -> Result<EnrichmentResult, EnrichmentWorkflowError> {
        if self.should_fail {
            return Err(EnrichmentWorkflowError::Queue("stub failure".into()));
        }

        Ok(EnrichmentResult {
            enrichment_status: EnrichmentStatus::Enriched,
            enrichment_source: Some("stub".into()),
            work: Work::default(),
            merge_deferred: false,
            provider_outcomes: HashMap::new(),
        })
    }

    async fn reset_for_manual_refresh(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
    ) -> Result<(), EnrichmentWorkflowError> {
        Ok(())
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
    ) -> Result<AuthorSeriesListView, SeriesServiceError> {
        Ok(AuthorSeriesListView {
            series: vec![],
            fetched_at: None,
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

    async fn retry_import(
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

    async fn confirm_scan(
        &self,
        _user_id: UserId,
        _scan_id: &str,
        _selections: Vec<ScanConfirmation>,
    ) -> Result<ImportResult, ImportWorkflowError> {
        Ok(ImportResult {
            grab_id: 0,
            final_status: GrabStatus::Imported,
            imported_files: vec![],
            failed_files: vec![],
            skipped_files: vec![],
            warnings: vec![],
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
