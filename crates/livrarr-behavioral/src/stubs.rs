use chrono::{DateTime, Utc};
use livrarr_db::{
    AuthorLinkClaim, AuthorLinkDb, AuthorNameVariantDb, AuthorProviderCall,
    AuthorRouteBackfillReport, DbError, GuardedRouteWrite,
};
use livrarr_domain::identity_layer;
use livrarr_domain::services::*;
use livrarr_domain::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

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
                    FetchError::QueueFull { retry_after } => FetchError::QueueFull {
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
            captured_provider_identity: Vec::new(),
            captured_route_proposals: Vec::new(),
            provider_chase_attempted: false,
            search_leg_fired: false,
            search_ledger_burnable: false,
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
// Author-provider linking stubs
// =============================================================================

pub struct StubAuthorProviderGateway {
    pub keyed_results: HashMap<(AuthorProvider, String), Vec<ProviderAuthorRef>>,
    pub ol_search_results: Vec<OpenLibraryAuthorCandidate>,
    pub ol_catalog_pages: Vec<OpenLibraryCatalogPage>,
    pub calls: Mutex<Vec<AuthorProviderCall>>,
}

impl StubAuthorProviderGateway {
    pub fn calls(&self) -> Vec<AuthorProviderCall> {
        self.calls
            .lock()
            .expect("author-provider call log mutex poisoned")
            .clone()
    }

    fn record_call(&self, provider: AuthorProvider, work_route: String, priority: RequestPriority) {
        self.calls
            .lock()
            .expect("author-provider call log mutex poisoned")
            .push(AuthorProviderCall {
                provider,
                work_route,
                priority,
            });
    }
}

impl AuthorProviderGateway for StubAuthorProviderGateway {
    async fn fetch_work_authors(
        &self,
        provider: AuthorProvider,
        work_route: String,
        priority: RequestPriority,
    ) -> Result<Vec<ProviderAuthorRef>, AuthorProviderError> {
        self.record_call(provider, work_route.clone(), priority);
        Ok(self
            .keyed_results
            .get(&(provider, work_route))
            .cloned()
            .unwrap_or_default())
    }

    async fn search_open_library_authors(
        &self,
        query: String,
        limit: u32,
        priority: RequestPriority,
    ) -> Result<Vec<OpenLibraryAuthorCandidate>, AuthorProviderError> {
        self.record_call(
            AuthorProvider::OpenLibrary,
            format!("ol_search:{query}:limit={limit}"),
            priority,
        );
        Ok(self.ol_search_results.clone())
    }

    async fn fetch_open_library_catalog_page(
        &self,
        author_route: OpenLibraryAuthorKey,
        cursor: Option<String>,
        priority: RequestPriority,
    ) -> Result<OpenLibraryCatalogPage, AuthorProviderError> {
        self.record_call(
            AuthorProvider::OpenLibrary,
            format!("ol_catalog:{author_route:?}:cursor={cursor:?}"),
            priority,
        );

        let page = match cursor.as_deref() {
            None => self.ol_catalog_pages.first(),
            Some(requested_cursor) => self
                .ol_catalog_pages
                .windows(2)
                .find(|pages| pages[0].next_cursor.as_deref() == Some(requested_cursor))
                .map(|pages| &pages[1]),
        };

        page.cloned().ok_or(AuthorProviderError::NotConfigured)
    }
}

pub struct StubAuthorLinkService;

impl AuthorLinkService for StubAuthorLinkService {
    async fn list_review(
        &self,
        _user_id: UserId,
    ) -> Result<Vec<AuthorLinkReview>, AuthorLinkError> {
        todo!()
    }

    async fn pick_candidate(
        &self,
        _user_id: UserId,
        _candidate_id: i64,
    ) -> Result<AuthorRoute, AuthorLinkError> {
        todo!()
    }

    async fn attach_selected_route(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _key: AuthorRouteKey,
    ) -> Result<AuthorRoute, AuthorLinkError> {
        todo!()
    }

    async fn dismiss_candidate(
        &self,
        _user_id: UserId,
        _candidate_id: i64,
    ) -> Result<(), AuthorLinkError> {
        todo!()
    }

    async fn remove_route(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _route_id: i64,
    ) -> Result<(), AuthorLinkError> {
        todo!()
    }

    async fn re_resolve(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
    ) -> Result<AuthorLinkProgress, AuthorLinkError> {
        todo!()
    }

    async fn progress(&self, _user_id: UserId) -> Result<AuthorSweepProgress, AuthorLinkError> {
        todo!()
    }
}

pub struct StubAuthorLinkWorkflow;

impl AuthorLinkWorkflow for StubAuthorLinkWorkflow {
    async fn enqueue(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _trigger: AuthorLinkTrigger,
    ) -> Result<(), AuthorLinkError> {
        todo!()
    }

    async fn submit_evidence(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _evidence: AgreedAuthorRouteEvidence,
    ) -> Result<RouteWriteOutcome, AuthorLinkError> {
        todo!()
    }

    async fn record_readarr_rejection(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _rejected: RejectedAuthorRouteEvidence,
    ) -> Result<AuthorLinkCandidate, AuthorLinkError> {
        todo!()
    }

    async fn run_due(
        &self,
        _batch_size: u32,
        _cancel: CancellationToken,
    ) -> Result<AuthorSweepTickSummary, AuthorLinkError> {
        todo!()
    }
}

pub struct StubAuthorLinkDb;

impl AuthorLinkDb for StubAuthorLinkDb {
    async fn ensure_enqueued(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _trigger: AuthorLinkTrigger,
    ) -> Result<(), DbError> {
        todo!()
    }

    async fn create_or_adopt_author(
        &self,
        _request: livrarr_db::CreateAuthorGateRequest,
    ) -> Result<(livrarr_domain::Author, bool), DbError> {
        Err(DbError::NotFound { entity: "author" })
    }

    async fn ensure_missing_progress_rows(&self, _limit: u32) -> Result<u32, DbError> {
        todo!()
    }

    async fn claim_due(
        &self,
        _now: DateTime<Utc>,
        _lease_until: DateTime<Utc>,
        _limit: u32,
    ) -> Result<Vec<AuthorLinkClaim>, DbError> {
        todo!()
    }

    async fn load_road_input(&self, _claim: AuthorLinkClaim) -> Result<AuthorRoadInput, DbError> {
        todo!()
    }

    async fn load_progress(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
    ) -> Result<livrarr_domain::AuthorLinkProgress, DbError> {
        Err(DbError::NotFound {
            entity: "author link progress",
        })
    }

    async fn begin_evidence_generation(
        &self,
        _claim: AuthorLinkClaim,
        _evidence_generation: i64,
    ) -> Result<(), DbError> {
        Err(DbError::ClaimLost)
    }

    async fn compute_evidence_fingerprint(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
    ) -> Result<AuthorEvidenceFingerprint, DbError> {
        todo!()
    }

    async fn prepare_key_attempts(
        &self,
        _claim: AuthorLinkClaim,
        _evidence_generation: i64,
        _keys: Vec<SettledWorkProviderKey>,
    ) -> Result<Vec<AuthorKeyAttempt>, DbError> {
        todo!()
    }

    async fn complete_key_attempt(
        &self,
        _claim: AuthorLinkClaim,
        _key_attempt_id: i64,
        _outcome: AuthorKeyAttemptOutcome,
        _authorial_credits_seen: u32,
    ) -> Result<(), DbError> {
        todo!()
    }

    async fn generation_authorial_credit_count(
        &self,
        _claim: AuthorLinkClaim,
        _evidence_generation: i64,
    ) -> Result<u64, DbError> {
        todo!()
    }

    async fn generation_outstanding_retries(
        &self,
        _claim: AuthorLinkClaim,
        _evidence_generation: i64,
    ) -> Result<Vec<livrarr_domain::OutstandingKeyRetry>, DbError> {
        todo!()
    }

    async fn generation_pending_candidate_count(
        &self,
        _claim: AuthorLinkClaim,
        _evidence_generation: i64,
    ) -> Result<u32, DbError> {
        todo!()
    }

    async fn revoke_dismissals_and_replay(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
    ) -> Result<(), DbError> {
        todo!()
    }

    async fn apply_guarded_route(
        &self,
        _write: GuardedRouteWrite,
    ) -> Result<RouteWriteOutcome, DbError> {
        todo!()
    }

    async fn record_candidates(
        &self,
        _claim: AuthorLinkClaim,
        _candidates: Vec<AuthorLinkCandidate>,
    ) -> Result<(), DbError> {
        todo!()
    }

    async fn record_readarr_rejection(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _rejected: RejectedAuthorRouteEvidence,
    ) -> Result<AuthorLinkCandidate, DbError> {
        todo!()
    }

    async fn advance_progress(
        &self,
        _claim: AuthorLinkClaim,
        _update: AuthorLinkProgressUpdate,
    ) -> Result<(), DbError> {
        todo!()
    }

    async fn pick_candidate_as_user(
        &self,
        _user_id: UserId,
        _candidate_id: i64,
    ) -> Result<AuthorRoute, DbError> {
        todo!()
    }

    async fn attach_route_as_user(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _key: AuthorRouteKey,
    ) -> Result<AuthorRoute, DbError> {
        todo!()
    }

    async fn dismiss_candidate_as_user(
        &self,
        _user_id: UserId,
        _candidate_id: i64,
    ) -> Result<(), DbError> {
        todo!()
    }

    async fn list_review(&self, _user_id: UserId) -> Result<Vec<AuthorLinkReview>, DbError> {
        todo!()
    }

    async fn sweep_progress(&self, _user_id: UserId) -> Result<AuthorSweepProgress, DbError> {
        todo!()
    }

    async fn remove_route_as_user(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _route_id: i64,
    ) -> Result<(), DbError> {
        todo!()
    }

    async fn list_active_routes(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _provider: Option<AuthorProvider>,
    ) -> Result<Vec<AuthorRoute>, DbError> {
        todo!()
    }

    async fn has_active_route(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _provider: AuthorProvider,
    ) -> Result<bool, DbError> {
        todo!()
    }

    async fn list_routes_for_view(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
    ) -> Result<Vec<AuthorRoute>, DbError> {
        todo!()
    }

    async fn compatibility_projection(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
    ) -> Result<AuthorCompatibilityProjection, DbError> {
        todo!()
    }

    async fn ingest_legacy_routes(&self) -> Result<AuthorRouteBackfillReport, DbError> {
        todo!()
    }

    async fn verify_cutover_ready(&self) -> Result<AuthorRouteBackfillReport, DbError> {
        todo!()
    }
}

impl AuthorNameVariantDb for StubAuthorLinkDb {
    async fn record_observed_names(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _observations: &[ProviderAuthorNameObservation],
    ) -> Result<u32, DbError> {
        todo!()
    }

    async fn record_author_observed_names(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _observations: &[ProviderAuthorNameObservation],
    ) -> Result<u32, DbError> {
        todo!()
    }

    async fn list_name_variants(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
    ) -> Result<Vec<livrarr_domain::AuthorNameVariant>, DbError> {
        todo!()
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

// =============================================================================
// Identity-layer-rewrite (F2) — StubIdentityRoadService. IR v1
// `livrarr-behavioral` module (ir-v1-identity-layer-rewrite.yaml:1401-1407).
// Trait+impl+stub pattern (insight 7): trait in domain, impl in
// livrarr-metadata, this stub here. No behavior yet (stub-writing scope) —
// `todo!()` bodies, matching this file's existing stub convention.
// =============================================================================

pub struct StubIdentityRoadService {
    pub requests: Arc<Mutex<Vec<identity_layer::IdentityRoadRequest>>>,
    /// Verbatim IR v1 shape (no interior-mutability wrapper) — unused by any
    /// `todo!()` body yet, so `&self`-only trait methods do not need one
    /// at this stub stage.
    pub outcomes: std::collections::VecDeque<identity_layer::IdentityRoadOutcome>,
}

impl identity_layer::IdentityRoadService for StubIdentityRoadService {
    async fn settle(
        &self,
        _request: identity_layer::IdentityRoadRequest,
    ) -> Result<identity_layer::IdentityRoadOutcome, identity_layer::IdentityRoadError> {
        todo!()
    }

    async fn resolve_review(
        &self,
        _actor: identity_layer::ReviewActor,
        _command: identity_layer::ReviewResolutionCommand,
    ) -> Result<identity_layer::IdentityRoadOutcome, identity_layer::IdentityRoadError> {
        todo!()
    }
}

/// Repository-backed road used by handler regression fixtures that exercise
/// the pending-route affirm continuation without composing network gateways.
#[derive(Clone)]
pub struct SqlitePendingRouteRoad {
    db: livrarr_db::sqlite::SqliteDb,
}

impl SqlitePendingRouteRoad {
    pub fn new(db: livrarr_db::sqlite::SqliteDb) -> Self {
        Self { db }
    }
}

impl identity_layer::IdentityRoadService for SqlitePendingRouteRoad {
    async fn settle(
        &self,
        request: identity_layer::IdentityRoadRequest,
    ) -> Result<identity_layer::IdentityRoadOutcome, identity_layer::IdentityRoadError> {
        use identity_layer::WorkIdentityRepository as _;
        if request.origin != identity_layer::IdentityRoadOrigin::AffirmPendingRoute {
            return Err(identity_layer::IdentityRoadError::InvalidDoorEvidence);
        }
        let work_id = request
            .existing_work_id
            .ok_or(identity_layer::IdentityRoadError::InvalidDoorEvidence)?;
        let route = request
            .evidence
            .provider_identity
            .first()
            .ok_or(identity_layer::IdentityRoadError::InvalidDoorEvidence)?;
        let captured = self
            .db
            .read_captured_identity(request.user_id, work_id)
            .await
            .map_err(map_pending_route_repo_error)?;
        let text_distinction =
            (captured.text_distinction != "common").then(|| captured.text_distinction.clone());
        let committed = self
            .db
            .commit_settlement(identity_layer::SettlementCommit {
                user_id: request.user_id,
                existing_work_id: Some(work_id),
                add_source: None,
                identity_title: captured.identity_title,
                text_distinction,
                contributors: vec![identity_layer::WorkContributor {
                    user_id: request.user_id,
                    work_id,
                    author_id: captured.primary_author_id,
                    ordinal: 0,
                    roles: Vec::new(),
                }],
                routes: captured.active_routes,
                absorbed_work_ids: Vec::new(),
                expected_generation: captured.identity_generation,
                review_cards: vec![identity_layer::SettlementReviewCard::PendingRoute {
                    work_id,
                    candidate: identity_layer::ParkedRouteCandidate {
                        route: route.route.clone(),
                        proposed_owner: identity_layer::RouteOwner::Work(work_id),
                    },
                }],
            })
            .await
            .map_err(map_pending_route_repo_error)?;
        let card =
            committed.review_cards.first().copied().ok_or_else(|| {
                identity_layer::IdentityRoadError::Database("missing card".into())
            })?;
        Ok(identity_layer::IdentityRoadOutcome::ReviewPending {
            review_id: card.id,
            kind: card.kind,
            unattached: false,
            expected_generation: card.generation,
            provenance: identity_layer::EvidenceProvenance::User,
        })
    }

    async fn resolve_review(
        &self,
        actor: identity_layer::ReviewActor,
        command: identity_layer::ReviewResolutionCommand,
    ) -> Result<identity_layer::IdentityRoadOutcome, identity_layer::IdentityRoadError> {
        use identity_layer::WorkIdentityRepository as _;
        let committed = self
            .db
            .commit_review_continuation(actor, command, tokio_util::sync::CancellationToken::new())
            .await
            .map_err(map_pending_route_repo_error)?;
        let identity = committed
            .identity
            .ok_or(identity_layer::IdentityRoadError::InvalidResolution)?;
        Ok(identity_layer::IdentityRoadOutcome::Settled {
            work_id: identity.own_work_id,
            created: false,
            routes: identity.active_routes,
            status: identity.status,
            library_items_moved: committed.library_items_moved,
            grabs_moved: committed.grabs_moved,
        })
    }
}

fn map_pending_route_repo_error(
    error: identity_layer::IdentityRepositoryError,
) -> identity_layer::IdentityRoadError {
    match error {
        identity_layer::IdentityRepositoryError::NotFound => {
            identity_layer::IdentityRoadError::NotFound
        }
        identity_layer::IdentityRepositoryError::StaleGeneration => {
            identity_layer::IdentityRoadError::StaleGeneration
        }
        identity_layer::IdentityRepositoryError::UnauthorizedScope => {
            identity_layer::IdentityRoadError::UnauthorizedScope
        }
        identity_layer::IdentityRepositoryError::ReviewKindMismatch => {
            identity_layer::IdentityRoadError::ReviewKindMismatch
        }
        identity_layer::IdentityRepositoryError::InvalidResolution => {
            identity_layer::IdentityRoadError::InvalidResolution
        }
        identity_layer::IdentityRepositoryError::ReviewProposalInvalidated(reason) => {
            identity_layer::IdentityRoadError::ReviewProposalInvalidated(reason)
        }
        identity_layer::IdentityRepositoryError::Cancelled => {
            identity_layer::IdentityRoadError::Cancelled
        }
        other => identity_layer::IdentityRoadError::Database(other.to_string()),
    }
}
