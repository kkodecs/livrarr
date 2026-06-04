use livrarr_db::{
    AuthorDb, ConfigDb, CreateAuthorDbRequest, CreateWorkDbRequest, EnrichmentRetryDb,
    LibraryItemDb, ProvenanceDb, SetFieldProvenanceRequest, UpdateWorkUserFieldsDbRequest, WorkDb,
    WorkDbCreate,
};
use livrarr_domain::keyed_mutex::KeyedMutex;
use livrarr_domain::services::*;
use livrarr_domain::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn iso639_1_to_3(code: &str) -> &str {
    match code {
        "nl" => "dut",
        "fr" => "fre",
        "de" => "ger",
        "it" => "ita",
        "ja" => "jpn",
        "ko" => "kor",
        "pl" => "pol",
        "es" => "spa",
        "en" => "eng",
        other => other,
    }
}

pub struct StubNoLlm;

impl LlmCaller for StubNoLlm {
    async fn call(&self, _req: LlmCallRequest) -> Result<LlmCallResponse, LlmError> {
        Err(LlmError::NotConfigured)
    }
}

struct CachedLookup {
    filtered: Vec<LookupResult>,
    raw: Vec<LookupResult>,
    raw_available: bool,
    created_at: Instant,
}

pub struct WorkServiceImpl<
    D,
    E,
    H,
    L = StubNoLlm,
    M = crate::DefaultMergeEngine,
    T = StubTagService,
> {
    db: D,
    enrichment: E,
    http: H,
    http_client: livrarr_http::HttpClient,
    llm: L,
    data_dir: PathBuf,
    #[allow(dead_code)]
    merge_engine: M,
    tag_service: Arc<T>,
    refresh_locks: KeyedMutex<(UserId, WorkId)>,
    bulk_refresh_users: Arc<std::sync::Mutex<std::collections::HashSet<i64>>>,
    lookup_cache: Arc<std::sync::Mutex<HashMap<(String, String), CachedLookup>>>,
    /// Optional multi-provider identity resolver. When present, `lookup_filtered`
    /// routes discovery through the federated fan-out (the #97 path) instead of
    /// the legacy sequential lookup chain. `None` keeps the legacy chain
    /// (back-compat until the resolver is composed in the server).
    resolver: Option<Arc<crate::english_identity_resolver::LiveEnglishIdentityResolver>>,
}

impl<D, E, H> WorkServiceImpl<D, E, H, StubNoLlm, crate::DefaultMergeEngine, StubTagService> {
    pub fn new(db: D, enrichment: E, http: H, data_dir: PathBuf) -> Self {
        Self {
            db,
            enrichment,
            http,
            http_client: livrarr_http::HttpClient::builder()
                .build()
                .expect("default HttpClient"),
            llm: StubNoLlm,
            data_dir,
            merge_engine: crate::DefaultMergeEngine::new(crate::PriorityModel::english()),
            tag_service: Arc::new(StubTagService),
            refresh_locks: KeyedMutex::new(),
            bulk_refresh_users: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            lookup_cache: Arc::new(std::sync::Mutex::new(HashMap::new())),
            resolver: None,
        }
    }
}

impl<D, E, H, L> WorkServiceImpl<D, E, H, L, crate::DefaultMergeEngine, StubTagService> {
    /// Construct with a custom LLM caller but stub merge engine and tag service.
    /// Use `new_with_all` for production wiring of merge engine and tag service.
    pub fn new_with_llm(db: D, enrichment: E, http: H, llm: L, data_dir: PathBuf) -> Self {
        Self {
            db,
            enrichment,
            http,
            http_client: livrarr_http::HttpClient::builder()
                .build()
                .expect("default HttpClient"),
            llm,
            data_dir,
            merge_engine: crate::DefaultMergeEngine::new(crate::PriorityModel::english()),
            tag_service: Arc::new(StubTagService),
            refresh_locks: KeyedMutex::new(),
            bulk_refresh_users: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            lookup_cache: Arc::new(std::sync::Mutex::new(HashMap::new())),
            resolver: None,
        }
    }
}

impl<D, E, H, L, M, T> WorkServiceImpl<D, E, H, L, M, T> {
    /// Construct with all dependencies explicitly wired.
    /// Used by server AppState for production wiring of merge engine and tag service.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_all(
        db: D,
        enrichment: E,
        http: H,
        http_client: livrarr_http::HttpClient,
        llm: L,
        data_dir: PathBuf,
        merge_engine: M,
        tag_service: Arc<T>,
    ) -> Self {
        Self {
            db,
            enrichment,
            http,
            http_client,
            llm,
            data_dir,
            merge_engine,
            tag_service,
            refresh_locks: KeyedMutex::new(),
            bulk_refresh_users: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            lookup_cache: Arc::new(std::sync::Mutex::new(HashMap::new())),
            resolver: None,
        }
    }

    /// Inject the multi-provider identity resolver so `lookup_filtered` routes
    /// discovery through the federated fan-out (the #97 path) instead of the
    /// legacy sequential lookup chain.
    pub fn with_resolver(
        mut self,
        resolver: Arc<crate::english_identity_resolver::LiveEnglishIdentityResolver>,
    ) -> Self {
        self.resolver = Some(resolver);
        self
    }
}

impl<D, H> WorkServiceImpl<D, (), H> {
    pub fn without_enrichment(
        db: D,
        http: H,
        data_dir: PathBuf,
    ) -> WorkServiceImpl<D, StubNoEnrichment, H, StubNoLlm, crate::DefaultMergeEngine, StubTagService>
    {
        WorkServiceImpl {
            db,
            enrichment: StubNoEnrichment,
            http,
            http_client: livrarr_http::HttpClient::builder()
                .build()
                .expect("default HttpClient"),
            llm: StubNoLlm,
            data_dir,
            merge_engine: crate::DefaultMergeEngine::new(crate::PriorityModel::english()),
            tag_service: Arc::new(StubTagService),
            refresh_locks: KeyedMutex::new(),
            bulk_refresh_users: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            lookup_cache: Arc::new(std::sync::Mutex::new(HashMap::new())),
            resolver: None,
        }
    }
}

pub struct StubNoEnrichment;

impl EnrichmentWorkflow for StubNoEnrichment {
    async fn enrich_work(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _mode: EnrichmentMode,
    ) -> Result<EnrichmentResult, EnrichmentWorkflowError> {
        Ok(EnrichmentResult {
            enrichment_status: EnrichmentStatus::Unenriched,
            enrichment_source: None,
            work: Work::default(),
            merge_deferred: false,
            provider_outcomes: HashMap::new(),
            cover_resolution: None,
            audiobook_cover_resolution: None,
        })
    }

    async fn reset_for_manual_refresh(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
    ) -> Result<(), EnrichmentWorkflowError> {
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

/// No-op TagService stub. Used for `without_enrichment` construction and tests.
pub struct StubTagService;

impl livrarr_domain::services::TagService for StubTagService {
    async fn retag_library_items(
        &self,
        _work: &livrarr_domain::Work,
        _items: &[livrarr_domain::LibraryItem],
    ) -> Vec<livrarr_domain::services::TagSyncItemResult> {
        Vec::new()
    }
}

/// Map a candidate's provenance to the conflict-attribution source so a raised
/// identity conflict reflects the creation path that produced it (REQ-020, D-017).
fn conflict_source_for(setter: ProvenanceSetter) -> livrarr_domain::identity::ConflictSource {
    use livrarr_domain::identity::ConflictSource;
    match setter {
        ProvenanceSetter::User => ConflictSource::ManualAdd,
        ProvenanceSetter::Import => ConflictSource::ReadarrImport,
        ProvenanceSetter::Imported => ConflictSource::ListImport,
        ProvenanceSetter::AutoAdded => ConflictSource::AuthorMonitor,
        ProvenanceSetter::Provider | ProvenanceSetter::System => ConflictSource::ManualAdd,
    }
}

/// Parse a `lookup_filtered` search term into a discovery WorkSeed. An `isbn:`
/// prefix with a valid ISBN seeds the bridge; anything else seeds the title.
fn lookup_term_to_seed(term: &str, lang: &str) -> livrarr_domain::identity::WorkSeed {
    let isbn_13 = term
        .strip_prefix("isbn:")
        .and_then(|rest| livrarr_domain::normalization::normalize_isbn13(rest.trim()));
    let title = if isbn_13.is_some() {
        None
    } else {
        Some(term.to_string())
    };
    livrarr_domain::identity::WorkSeed {
        ol_key: None,
        gr_key: None,
        hc_key: None,
        isbn_13,
        asin: None,
        title,
        author_name: None,
        language: Some(lang.to_string()),
        series_name: None,
        year: None,
        user_confirmed: false,
    }
}

/// True when a search term resolved to a hard identifier (e.g. an `isbn:`
/// lookup) — the signal the identity resolver needs. A bare-title term carries
/// none and is served as a free-text discovery search by the legacy chain.
fn seed_carries_identifier(seed: &livrarr_domain::identity::WorkSeed) -> bool {
    seed.isbn_13.is_some()
        || seed.asin.is_some()
        || seed.ol_key.is_some()
        || seed.gr_key.is_some()
        || seed.hc_key.is_some()
}

/// Map a resolved/confirmable identity into a wire `LookupResult`, carrying the
/// federated anchors + the `candidate_id` payload handle (REQ-014/R-009).
fn lookup_result_from_captured(
    captured: livrarr_domain::identity::CapturedIdentity,
    candidate_id: Option<livrarr_domain::identity::CandidateId>,
    cover_url: Option<String>,
) -> LookupResult {
    LookupResult {
        ol_key: captured.ol_key,
        title: captured.title,
        author_name: captured.author_name,
        author_ol_key: None,
        year: None,
        cover_url,
        description: None,
        series_name: None,
        series_position: None,
        source: None,
        source_type: None,
        language: captured.language,
        detail_url: None,
        rating: None,
        isbn_13: captured.isbn_13,
        candidate_id,
        hc_key: captured.hc_key,
        gr_key: captured.gr_key,
        asin: captured.asin,
    }
}

/// Convert a resolver `Resolution` into wire lookup results: a Resolved identity
/// is a single auto-matched result; NeedsConfirmation becomes the candidate list;
/// Unresolved/Conflict yield no results.
fn lookup_results_from_resolution(
    resolution: livrarr_domain::identity::Resolution,
) -> Vec<LookupResult> {
    use livrarr_domain::identity::Resolution;
    match resolution {
        Resolution::Resolved {
            identity,
            candidate_id,
            ..
        } => vec![lookup_result_from_captured(
            identity,
            Some(candidate_id),
            None,
        )],
        Resolution::NeedsConfirmation { candidates } => candidates
            .into_iter()
            .map(|c| lookup_result_from_captured(c.anchors, Some(c.candidate_id), c.cover_url))
            .collect(),
        Resolution::Unresolved { .. } | Resolution::Conflict { .. } => Vec::new(),
    }
}

impl<D, E, H, L, M, T> WorkServiceImpl<D, E, H, L, M, T>
where
    D: livrarr_domain::services::WorkIdentityRepository + Send + Sync,
{
    /// Conflict preflight + additive anchor merge for a matched/adopted work:
    /// raise an observable conflict for any work-anchor type whose existing
    /// confirmed value differs from the incoming one (REQ-018/020), then fill
    /// the anchor types the existing work lacks (REQ-028, additive only — a
    /// conflicting same-type anchor is never overwritten by the merge).
    async fn preflight_and_merge_anchors(
        &self,
        existing_work_id: livrarr_domain::WorkId,
        incoming: &livrarr_domain::identity::CapturedIdentity,
        source: livrarr_domain::identity::ConflictSource,
    ) -> Result<(), WorkServiceError> {
        let conflicts = self
            .db
            .detect_conflicting_anchors(existing_work_id, incoming, source)
            .await
            .map_err(|e| WorkServiceError::Validation(format!("conflict detection failed: {e}")))?;
        for conflict in conflicts {
            self.db
                .raise_identity_conflict(conflict)
                .await
                .map_err(|e| WorkServiceError::Validation(format!("conflict raise failed: {e}")))?;
        }
        self.db
            .merge_missing_anchors(existing_work_id, incoming)
            .await
            .map_err(|e| WorkServiceError::Validation(format!("anchor merge failed: {e}")))?;
        Ok(())
    }
}

impl<D, E, H, L, M, T> WorkService for WorkServiceImpl<D, E, H, L, M, T>
where
    D: WorkDb
        + WorkDbCreate
        + AuthorDb
        + LibraryItemDb
        + ProvenanceDb
        + EnrichmentRetryDb
        + ConfigDb
        + livrarr_domain::services::WorkIdentityRepository
        + Send
        + Sync,
    E: EnrichmentWorkflow + Send + Sync,
    H: HttpFetcher + Clone + Send + Sync + 'static,
    L: LlmCaller + Send + Sync,
    M: crate::MergeEngine + Send + Sync,
    T: livrarr_domain::services::TagService + Send + Sync,
{
    async fn add(
        &self,
        user_id: UserId,
        candidate: livrarr_domain::identity::WorkCandidate,
    ) -> Result<AddWorkResult, WorkServiceError> {
        use livrarr_domain::identity::{AnchorSetter, AnchorType, IdentityState};

        let cleaned_title = crate::title_cleanup::clean_title(&candidate.fields.title);
        if cleaned_title.is_empty() {
            return Err(WorkServiceError::Validation(
                "title must not be empty".into(),
            ));
        }
        let cleaned_author = crate::title_cleanup::clean_author(&candidate.fields.author_name);
        let normalized_title = livrarr_domain::normalize_for_matching(&cleaned_title);
        let normalized_author = livrarr_domain::normalize_for_matching(&cleaned_author);

        match &candidate.identity {
            IdentityState::Confirmed { anchors, .. } => {
                // Step 1: anchor match over the work-anchor types the candidate
                // carries (ol/gr/hc). Bridges (isbn/asin) are edition-level and
                // are not used to dedup works here (bridge-anchor policy).
                let mut anchor_match: Option<livrarr_domain::WorkId> = None;
                for (anchor_ty, value) in [
                    (AnchorType::OL_WORK, anchors.ol_key.as_deref()),
                    (AnchorType::GR_WORK, anchors.gr_key.as_deref()),
                    (AnchorType::HC_WORK, anchors.hc_key.as_deref()),
                ] {
                    let Some(value) = value.filter(|v| !v.is_empty()) else {
                        continue;
                    };
                    if let Ok(Some(existing_id)) = self
                        .db
                        .find_work_by_anchor(user_id, &AnchorType::new(anchor_ty), value)
                        .await
                    {
                        anchor_match = Some(existing_id);
                        break;
                    }
                }
                if let Some(existing_id) = anchor_match {
                    // Conflict preflight + additive anchor merge BEFORE returning
                    // the matched work (REQ-018/020/028).
                    let setter = candidate
                        .provenance_setter
                        .unwrap_or(ProvenanceSetter::User);
                    self.preflight_and_merge_anchors(
                        existing_id,
                        anchors,
                        conflict_source_for(setter),
                    )
                    .await?;
                    let work = self
                        .db
                        .get_work(user_id, existing_id)
                        .await
                        .map_err(WorkServiceError::Db)?;
                    let (work, enrichment_status) = if candidate.source_provider_data.is_some() {
                        let status = self
                            .run_unified_enrichment(user_id, &work, candidate.source_provider_data)
                            .await;
                        let refreshed = self.db.get_work(user_id, work.id).await.unwrap_or(work);
                        (refreshed, status)
                    } else {
                        let status = work.enrichment_status;
                        (work, status)
                    };
                    return Ok(AddWorkResult {
                        work,
                        created: false,
                        author_created: false,
                        author_id: None,
                        messages: vec![],
                        cover_mtime: None,
                        audiobook_cover_mtime: None,
                        enrichment_status,
                    });
                }

                // Step 3: REQ-005 adopt path — existing ol-key-less work with
                // same normalized identity absorbs the incoming confirmed key.
                if let Some(existing) = self
                    .db
                    .find_normalized_match_no_anchor_for_user(
                        user_id,
                        &candidate.fields.title,
                        &candidate.fields.author_name,
                    )
                    .await
                    .map_err(WorkServiceError::Db)?
                {
                    // Adopt: an anchorless normalized match absorbs the incoming
                    // anchors (REQ-028); the preflight raises a conflict if the
                    // existing work already holds a different confirmed anchor
                    // (REQ-018/020).
                    let setter = candidate
                        .provenance_setter
                        .unwrap_or(ProvenanceSetter::User);
                    self.preflight_and_merge_anchors(
                        existing.id,
                        anchors,
                        conflict_source_for(setter),
                    )
                    .await?;
                    let existing = self
                        .db
                        .get_work(user_id, existing.id)
                        .await
                        .map_err(WorkServiceError::Db)?;
                    let (work, enrichment_status) = if candidate.source_provider_data.is_some() {
                        let status = self
                            .run_unified_enrichment(
                                user_id,
                                &existing,
                                candidate.source_provider_data.clone(),
                            )
                            .await;
                        let refreshed = self
                            .db
                            .get_work(user_id, existing.id)
                            .await
                            .unwrap_or(existing);
                        (refreshed, status)
                    } else {
                        let status = existing.enrichment_status;
                        (existing, status)
                    };
                    return Ok(AddWorkResult {
                        work,
                        created: false,
                        author_created: false,
                        author_id: None,
                        messages: vec![],
                        cover_mtime: None,
                        audiobook_cover_mtime: None,
                        enrichment_status,
                    });
                }

                // Normalized-match dedup with step 3e race-loser detection.
                let existing = self
                    .db
                    .find_by_normalized_match(user_id, &normalized_title, &normalized_author)
                    .await
                    .map_err(WorkServiceError::Db)?;
                if let Some(work) = existing.into_iter().next() {
                    // Step 3e (now wired, REQ-020): conflict preflight + additive
                    // anchor merge on the normalized-identity match — replaces the
                    // former warn-only TODO. A differing confirmed work anchor
                    // (ol/gr/hc) raises an observable conflict (REQ-018).
                    let setter = candidate
                        .provenance_setter
                        .unwrap_or(ProvenanceSetter::User);
                    self.preflight_and_merge_anchors(work.id, anchors, conflict_source_for(setter))
                        .await?;
                    let work = self
                        .db
                        .get_work(user_id, work.id)
                        .await
                        .map_err(WorkServiceError::Db)?;
                    let (work, enrichment_status) = if candidate.source_provider_data.is_some() {
                        let status = self
                            .run_unified_enrichment(
                                user_id,
                                &work,
                                candidate.source_provider_data.clone(),
                            )
                            .await;
                        let refreshed = self.db.get_work(user_id, work.id).await.unwrap_or(work);
                        (refreshed, status)
                    } else {
                        let status = work.enrichment_status;
                        (work, status)
                    };
                    return Ok(AddWorkResult {
                        work,
                        created: false,
                        author_created: false,
                        author_id: None,
                        messages: vec![],
                        cover_mtime: None,
                        audiobook_cover_mtime: None,
                        enrichment_status,
                    });
                }

                let (author_created, author_id) = self
                    .find_or_create_author(
                        user_id,
                        &cleaned_author,
                        candidate.fields.author_ol_key.as_deref(),
                    )
                    .await?;

                let setter = candidate
                    .provenance_setter
                    .unwrap_or(ProvenanceSetter::User);
                let anchor_setter = match setter {
                    ProvenanceSetter::User => AnchorSetter::User,
                    ProvenanceSetter::Import => AnchorSetter::Import,
                    _ => AnchorSetter::AutoSearch,
                };

                let (work, actually_created) = self
                    .db
                    .create_work(CreateWorkDbRequest {
                        user_id,
                        title: cleaned_title,
                        author_name: cleaned_author,
                        normalized_title,
                        normalized_author,
                        author_id,
                        ol_key: None,
                        gr_key: candidate
                            .identity
                            .seed_or_confirmed_anchors()
                            .and_then(|a| a.gr_key.clone()),
                        year: candidate.fields.year,
                        cover_url: candidate.fields.cover_url.clone(),
                        language: Some(livrarr_domain::normalize_language(
                            &candidate.fields.language,
                        )),
                        series_name: candidate.fields.series_name.clone(),
                        series_position: candidate.fields.series_position,
                        monitor_ebook: candidate.monitor_ebook.unwrap_or(true),
                        monitor_audiobook: candidate.monitor_audiobook.unwrap_or(true),
                        import_id: candidate.import_id.clone(),
                        series_id: candidate.series_id,
                        isbn_13: candidate
                            .identity
                            .seed_or_confirmed_anchors()
                            .and_then(|a| a.isbn_13.clone()),
                        asin: candidate
                            .identity
                            .seed_or_confirmed_anchors()
                            .and_then(|a| a.asin.clone()),
                        description: candidate.fields.description.clone(),
                        source_provider_json: None,
                        cover_manual: candidate.cover_manual,
                    })
                    .await
                    .map_err(WorkServiceError::Db)?;

                if !actually_created {
                    return self
                        .handle_race_loser(
                            user_id,
                            work,
                            author_created,
                            author_id,
                            candidate.source_provider_data,
                        )
                        .await;
                }

                // Persist every work-anchor + bridge the candidate carries as a
                // confirmed anchor row (REQ-001/003) — not only the OL key. A
                // resolving-id candidate may have no ol_key (e.g. Readarr gr+isbn).
                for (anchor_ty, value) in [
                    (AnchorType::OL_WORK, anchors.ol_key.as_deref()),
                    (AnchorType::GR_WORK, anchors.gr_key.as_deref()),
                    (AnchorType::HC_WORK, anchors.hc_key.as_deref()),
                    (AnchorType::ISBN_13, anchors.isbn_13.as_deref()),
                    (AnchorType::ASIN, anchors.asin.as_deref()),
                ] {
                    if let Some(value) = value.filter(|v| !v.is_empty()) {
                        self.db
                            .confirm_anchor(
                                work.id,
                                AnchorType::new(anchor_ty),
                                value,
                                anchor_setter,
                            )
                            .await
                            .map_err(|e| {
                                WorkServiceError::Validation(format!("anchor write failed: {e}"))
                            })?;
                    }
                }

                // Reload so add-time provenance sees the anchor-synced identifier
                // columns (ol/gr/hc/isbn/asin), matching the prior
                // create_work_with_anchor reload semantics.
                let work = self
                    .db
                    .get_work(user_id, work.id)
                    .await
                    .map_err(WorkServiceError::Db)?;
                write_addtime_provenance(&self.db, user_id, &work, setter).await;

                self.finish_created_work(
                    user_id,
                    work,
                    author_created,
                    author_id,
                    candidate.source_provider_data,
                    candidate.skip_sync_enrichment,
                )
                .await
            }
            IdentityState::Pending { .. } => {
                if let Some((work, enrichment_status)) = self
                    .try_dedup_by_normalized(
                        user_id,
                        &normalized_title,
                        &normalized_author,
                        &candidate.source_provider_data,
                    )
                    .await?
                {
                    return Ok(AddWorkResult {
                        work,
                        created: false,
                        author_created: false,
                        author_id: None,
                        messages: vec![],
                        cover_mtime: None,
                        audiobook_cover_mtime: None,
                        enrichment_status,
                    });
                }

                let (author_created, author_id) = self
                    .find_or_create_author(
                        user_id,
                        &cleaned_author,
                        candidate.fields.author_ol_key.as_deref(),
                    )
                    .await?;

                let (work, actually_created) = self
                    .db
                    .create_work(CreateWorkDbRequest {
                        user_id,
                        title: cleaned_title,
                        author_name: cleaned_author,
                        normalized_title,
                        normalized_author,
                        author_id,
                        ol_key: None,
                        gr_key: candidate
                            .identity
                            .seed_or_confirmed_anchors()
                            .and_then(|a| a.gr_key.clone()),
                        year: candidate.fields.year,
                        cover_url: candidate.fields.cover_url.clone(),
                        language: Some(livrarr_domain::normalize_language(
                            &candidate.fields.language,
                        )),
                        series_name: candidate.fields.series_name.clone(),
                        series_position: candidate.fields.series_position,
                        monitor_ebook: candidate.monitor_ebook.unwrap_or(true),
                        monitor_audiobook: candidate.monitor_audiobook.unwrap_or(true),
                        import_id: candidate.import_id.clone(),
                        series_id: candidate.series_id,
                        isbn_13: candidate
                            .identity
                            .seed_or_confirmed_anchors()
                            .and_then(|a| a.isbn_13.clone()),
                        asin: candidate
                            .identity
                            .seed_or_confirmed_anchors()
                            .and_then(|a| a.asin.clone()),
                        description: candidate.fields.description.clone(),
                        source_provider_json: None,
                        cover_manual: candidate.cover_manual,
                    })
                    .await
                    .map_err(WorkServiceError::Db)?;

                if !actually_created {
                    return self
                        .handle_race_loser(
                            user_id,
                            work,
                            author_created,
                            author_id,
                            candidate.source_provider_data,
                        )
                        .await;
                }

                let setter = candidate
                    .provenance_setter
                    .unwrap_or(ProvenanceSetter::User);
                write_addtime_provenance(&self.db, user_id, &work, setter).await;

                if let IdentityState::Pending { reason, .. } = &candidate.identity {
                    let anchor_setter = match setter {
                        ProvenanceSetter::User => AnchorSetter::User,
                        ProvenanceSetter::Import => AnchorSetter::Import,
                        _ => AnchorSetter::AutoSearch,
                    };
                    self.db
                        .set_identity_pending(work.id, *reason, anchor_setter)
                        .await
                        .map_err(|e| {
                            WorkServiceError::Validation(format!(
                                "set_identity_pending failed: {e}"
                            ))
                        })?;
                }

                // Enrichment runs even for Pending works — providers search
                // by title/author and don't need a confirmed OL key. Identity
                // resolution is orthogonal; the retry job handles that.
                self.finish_created_work(
                    user_id,
                    work,
                    author_created,
                    author_id,
                    candidate.source_provider_data,
                    candidate.skip_sync_enrichment,
                )
                .await
            }
        }
    }

    async fn get(&self, user_id: UserId, work_id: WorkId) -> Result<Work, WorkServiceError> {
        self.db
            .get_work(user_id, work_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
            })
    }

    async fn get_detail(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<WorkDetailView, WorkServiceError> {
        let work = self.get(user_id, work_id).await?;
        let library_items = self
            .db
            .list_library_items_by_work(user_id, work_id)
            .await
            .map_err(WorkServiceError::Db)?;
        let covers_dir = self.data_dir.join("covers").join(user_id.to_string());
        let cover_mtime = crate::cover::cover_file_mtime(&covers_dir, work_id);
        let audiobook_cover_mtime = crate::cover::audiobook_cover_file_mtime(&covers_dir, work_id);
        Ok(WorkDetailView {
            work,
            library_items,
            cover_mtime,
            audiobook_cover_mtime,
        })
    }

    async fn list(
        &self,
        user_id: UserId,
        filter: WorkFilter,
    ) -> Result<Vec<Work>, WorkServiceError> {
        let mut works = if let Some(author_id) = filter.author_id {
            self.db
                .list_works_by_author(user_id, author_id)
                .await
                .map_err(WorkServiceError::Db)?
        } else {
            self.db
                .list_works(user_id)
                .await
                .map_err(WorkServiceError::Db)?
        };

        if let Some(monitored) = filter.monitored {
            works.retain(|w| (w.monitor_ebook || w.monitor_audiobook) == monitored);
        }
        if let Some(ref status) = filter.enrichment_status {
            works.retain(|w| w.enrichment_status == *status);
        }
        if let Some(media_type) = filter.media_type {
            works.retain(|w| match media_type {
                MediaType::Ebook => w.monitor_ebook,
                MediaType::Audiobook => w.monitor_audiobook,
            });
        }
        if let Some(sort_by) = filter.sort_by {
            let dir = filter.sort_dir.unwrap_or(SortDirection::Asc);
            works.sort_by(|a, b| {
                let cmp = match sort_by {
                    WorkSortField::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
                    WorkSortField::DateAdded | WorkSortField::RecentlyDownloaded => {
                        a.added_at.cmp(&b.added_at)
                    }
                    WorkSortField::Year => a.year.cmp(&b.year),
                    WorkSortField::Author => a.author_name.cmp(&b.author_name),
                };
                match dir {
                    SortDirection::Asc => cmp,
                    SortDirection::Desc => cmp.reverse(),
                }
            });
        }

        Ok(works)
    }

    async fn list_paginated(
        &self,
        user_id: UserId,
        page: u32,
        page_size: u32,
        sort_by: WorkSortField,
        sort_dir: SortDirection,
        media_type: Option<MediaType>,
        language: Option<&str>,
    ) -> Result<PaginatedWorksView, WorkServiceError> {
        let sort_col = match sort_by {
            WorkSortField::Title => "title",
            WorkSortField::DateAdded => "date_added",
            WorkSortField::Year => "year",
            WorkSortField::Author => "author",
            WorkSortField::RecentlyDownloaded => "recently_downloaded",
        };
        let dir = match sort_dir {
            SortDirection::Asc => "asc",
            SortDirection::Desc => "desc",
        };
        let (works, total) = self
            .db
            .list_works_paginated(
                user_id, page, page_size, sort_col, dir, media_type, language,
            )
            .await
            .map_err(WorkServiceError::Db)?;

        let work_ids: Vec<i64> = works.iter().map(|w| w.id).collect();
        let items = self
            .db
            .list_library_items_by_work_ids(user_id, &work_ids)
            .await
            .map_err(WorkServiceError::Db)?;

        // Pre-index items by work_id to avoid O(works×items) filtering.
        let mut items_by_work: HashMap<WorkId, Vec<LibraryItem>> =
            HashMap::with_capacity(work_ids.len());
        for item in items {
            items_by_work.entry(item.work_id).or_default().push(item);
        }

        let work_views = works
            .into_iter()
            .map(|w| {
                let work_items = items_by_work.remove(&w.id).unwrap_or_default();
                WorkDetailView {
                    work: w,
                    library_items: work_items,
                    cover_mtime: None,
                    audiobook_cover_mtime: None,
                }
            })
            .collect();

        Ok(PaginatedWorksView {
            works: work_views,
            total,
            page,
            page_size,
        })
    }

    async fn update(
        &self,
        user_id: UserId,
        work_id: WorkId,
        req: UpdateWorkRequest,
    ) -> Result<Work, WorkServiceError> {
        self.db
            .get_work(user_id, work_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
            })?;

        let has_title = req.title.is_some();
        let has_author = req.author_name.is_some();
        let series_name_cleared = matches!(req.series_name, Some(None));
        let series_position_cleared = matches!(req.series_position, Some(None));
        let has_series_name = req.series_name.is_some();
        let has_series_position = req.series_position.is_some();
        let cleaned_title = req.title.map(|t| crate::title_cleanup::clean_title(&t));
        let cleaned_author = req
            .author_name
            .map(|a| crate::title_cleanup::clean_author(&a));
        let normalized_title = cleaned_title
            .as_deref()
            .map(livrarr_domain::normalize_for_matching);
        let normalized_author = cleaned_author
            .as_deref()
            .map(livrarr_domain::normalize_for_matching);
        let db_req = UpdateWorkUserFieldsDbRequest {
            title: cleaned_title,
            author_name: cleaned_author,
            normalized_title,
            normalized_author,
            series_name: req.series_name,
            series_position: req.series_position,
            monitor_ebook: req.monitor_ebook,
            monitor_audiobook: req.monitor_audiobook,
        };

        let work = self
            .db
            .update_work_user_fields(user_id, work_id, db_req)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
            })?;

        let mut prov_reqs: Vec<SetFieldProvenanceRequest> = Vec::new();
        if has_title {
            prov_reqs.push(SetFieldProvenanceRequest {
                user_id,
                work_id,
                field: WorkField::Title,
                source: None,
                setter: ProvenanceSetter::User,
                cleared: false,
            });
        }
        if has_author {
            prov_reqs.push(SetFieldProvenanceRequest {
                user_id,
                work_id,
                field: WorkField::AuthorName,
                source: None,
                setter: ProvenanceSetter::User,
                cleared: false,
            });
        }
        if has_series_name {
            prov_reqs.push(SetFieldProvenanceRequest {
                user_id,
                work_id,
                field: WorkField::SeriesName,
                source: None,
                setter: ProvenanceSetter::User,
                cleared: series_name_cleared,
            });
        }
        if has_series_position {
            prov_reqs.push(SetFieldProvenanceRequest {
                user_id,
                work_id,
                field: WorkField::SeriesPosition,
                source: None,
                setter: ProvenanceSetter::User,
                cleared: series_position_cleared,
            });
        }
        if !prov_reqs.is_empty() {
            if let Err(e) = self.db.set_field_provenance_batch(prov_reqs).await {
                tracing::warn!(work_id, "user-edit provenance write failed: {e}");
            }
        }

        Ok(work)
    }

    async fn delete(&self, user_id: UserId, work_id: WorkId) -> Result<(), WorkServiceError> {
        self.db
            .get_work(user_id, work_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
            })?;

        let items = self
            .db
            .list_library_items_by_work(user_id, work_id)
            .await
            .map_err(WorkServiceError::Db)?;

        self.db
            .delete_work(user_id, work_id)
            .await
            .map(|_| ())
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
            })?;

        delete_cover_files(&self.data_dir, user_id, work_id).await;

        for item in &items {
            if let Err(e) = tokio::fs::remove_file(&item.path).await {
                tracing::warn!(
                    work_id = work_id,
                    item_id = item.id,
                    path = %item.path,
                    "failed to delete library file on work delete: {e}"
                );
            }
        }

        Ok(())
    }

    async fn refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<RefreshWorkResult, WorkServiceError> {
        let work = self.get(user_id, work_id).await?;

        let _guard = self.refresh_locks.lock((user_id, work_id)).await;

        if let Err(e) = self.db.reset_enrichment_for_refresh(user_id, work_id).await {
            tracing::warn!("reset_enrichment_for_refresh failed: {e}");
        }

        if let Err(e) = self
            .enrichment
            .reset_for_manual_refresh(user_id, work_id)
            .await
        {
            tracing::warn!("enrichment reset_for_manual_refresh failed: {e}");
        }

        // Unified enrichment: provider dispatch, merge, cover download, tag sync.
        let _enrichment_status = self.run_unified_enrichment(user_id, &work, None).await;

        let refreshed_work = match self.db.get_work(user_id, work_id).await {
            Ok(w) => w,
            Err(_) => work,
        };

        Ok(RefreshWorkResult {
            work: refreshed_work,
            messages: vec![],
            taggable_items: vec![],
            merge_deferred: false,
        })
    }

    // Dead: bulk refresh is implemented at the handler layer
    // (`crates/livrarr-handlers/src/work.rs::refresh_all`) per insight 9g
    // (handler-level spawning for long-running background work). This stub
    // never wired up — the handler does its own list + spawn + iterate +
    // finish_bulk_refresh directly.
    // async fn refresh_all(&self, user_id: UserId) -> Result<RefreshAllHandle, WorkServiceError> {
    //     let works = self
    //         .db
    //         .list_works(user_id)
    //         .await
    //         .map_err(WorkServiceError::Db)?;
    //
    //     let total_works = works.len();
    //
    //     if !self.try_start_bulk_refresh(user_id) {
    //         return Err(WorkServiceError::Enrichment(
    //             "bulk refresh already in progress".into(),
    //         ));
    //     }
    //
    //     Ok(RefreshAllHandle { total_works })
    // }

    async fn upload_cover(
        &self,
        user_id: UserId,
        work_id: WorkId,
        bytes: &[u8],
    ) -> Result<(), WorkServiceError> {
        // 5 MB — accommodates high-resolution covers from providers like Google Books.
        // TODO(alpha6+): reduce stored cover resolution to limit on-disk footprint.
        const MAX_COVER_BYTES: usize = 5 * 1024 * 1024;

        if bytes.len() > MAX_COVER_BYTES {
            return Err(WorkServiceError::Enrichment(format!(
                "cover too large: {} bytes (max {})",
                bytes.len(),
                MAX_COVER_BYTES
            )));
        }
        if bytes.is_empty() {
            return Err(WorkServiceError::Enrichment("empty image data".into()));
        }
        if !is_supported_image(bytes) {
            return Err(WorkServiceError::Enrichment(
                "unrecognized image format (expected JPEG, PNG, GIF, or WebP)".into(),
            ));
        }

        let _work = self.get(user_id, work_id).await?;

        let covers_dir = self.data_dir.join("covers").join(user_id.to_string());
        tokio::fs::create_dir_all(&covers_dir)
            .await
            .map_err(|e| WorkServiceError::Enrichment(format!("create covers dir: {e}")))?;

        let cover_path = covers_dir.join(format!("{work_id}.jpg"));
        let tmp_path = cover_path.with_extension("jpg.tmp");
        let tmp_clone = tmp_path.clone();
        let target = cover_path.clone();
        let bytes_vec = bytes.to_vec();
        let write_result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp_clone)?;
            f.write_all(&bytes_vec)?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&tmp_clone, &target)
        })
        .await
        .map_err(|e| WorkServiceError::Enrichment(format!("spawn error: {e}")))?;

        if let Err(e) = write_result {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(WorkServiceError::Enrichment(format!("write cover: {e}")));
        }

        let thumb_path = covers_dir.join(format!("{work_id}_thumb.jpg"));
        let _ = tokio::fs::remove_file(&thumb_path).await;

        self.db
            .set_cover_manual(user_id, work_id, true)
            .await
            .map_err(WorkServiceError::Db)?;

        Ok(())
    }

    async fn download_cover(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<u8>, WorkServiceError> {
        let _work = self.get(user_id, work_id).await?;

        // Try new tenant-aware path first, fall back to old flat layout.
        let new_path = self
            .data_dir
            .join("covers")
            .join(user_id.to_string())
            .join(format!("{work_id}.jpg"));
        let cover_path = if new_path.exists() {
            new_path
        } else {
            self.data_dir.join("covers").join(format!("{work_id}.jpg"))
        };
        let bytes = tokio::fs::read(&cover_path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                WorkServiceError::NotFound
            } else {
                WorkServiceError::Enrichment(format!("read cover: {e}"))
            }
        })?;
        Ok(bytes)
    }

    async fn lookup(&self, req: LookupRequest) -> Result<Vec<LookupResult>, WorkServiceError> {
        let term = req.term.trim().to_string();
        if term.is_empty() {
            return Ok(vec![]);
        }

        let cfg = self.db.get_metadata_config().await.ok();
        let default_lang = cfg
            .as_ref()
            .and_then(|c| c.languages.first().cloned())
            .unwrap_or_else(|| "en".to_string());
        let lang = req.lang_override.as_deref().unwrap_or(&default_lang);

        if lang != "en" && !livrarr_external_data::language::is_supported_language(lang) {
            return Err(WorkServiceError::Enrichment(format!(
                "unsupported language: {lang}"
            )));
        }

        // Step 1: Google Books (primary).
        match self.lookup_google_books(&term, lang).await {
            Ok(results) if !results.is_empty() => return Ok(results),
            Ok(_) => tracing::debug!(term = %term, lang = %lang, "GoogleBooks returned empty"),
            Err(e) => tracing::warn!(
                term = %term, lang = %lang, error = %e,
                "GoogleBooks lookup failed; falling back to next provider"
            ),
        }

        // Step 2: OpenLibrary (fallback).
        match self.lookup_openlibrary(&term, lang).await {
            Ok(results) if !results.is_empty() => return Ok(results),
            Ok(_) => tracing::debug!(term = %term, lang = %lang, "OpenLibrary returned empty"),
            Err(e) => tracing::warn!(
                term = %term, lang = %lang, error = %e,
                "OpenLibrary lookup failed; falling back to next provider"
            ),
        }

        // Step 3: Hardcover (if token configured).
        match self.lookup_hardcover(&term, lang).await {
            Ok(results) if !results.is_empty() => return Ok(results),
            Ok(_) => tracing::debug!(term = %term, lang = %lang, "Hardcover returned empty"),
            Err(e) => tracing::warn!(
                term = %term, lang = %lang, error = %e,
                "Hardcover lookup failed; falling back to next provider"
            ),
        }

        // Step 4: Goodreads scrape (foreign-language only).
        if lang != "en" {
            return self.lookup_goodreads(&term, lang).await;
        }

        Ok(vec![])
    }

    async fn lookup_filtered(
        &self,
        user_id: UserId,
        req: LookupRequest,
        raw: bool,
    ) -> Result<LookupResponse, WorkServiceError> {
        let term = req.term.trim().to_lowercase();
        if term.is_empty() {
            return Ok(LookupResponse {
                results: vec![],
                filtered_count: 0,
                raw_count: 0,
                raw_available: false,
            });
        }

        let lang = req
            .lang_override
            .clone()
            .unwrap_or_else(|| "en".to_string());

        // Resolver path (#97 fix): route through the multi-provider fan-out ONLY
        // when the term identifies a specific book (an `isbn:` lookup or another
        // provider key). A bare-title term is a free-text discovery search — it
        // carries no identifier for the resolver to act on (resolve() abstains as
        // EmptySeed), so it falls through to the legacy provider search below.
        let seed = lookup_term_to_seed(&term, &lang);
        if seed_carries_identifier(&seed) {
            if let Some(resolver) = self.resolver.clone() {
                use livrarr_domain::services::IdentityResolver;
                let resolution = resolver
                    .resolve(
                        user_id,
                        &seed,
                        livrarr_domain::identity::LatencyTier::Interactive,
                    )
                    .await
                    .map_err(|e| WorkServiceError::Validation(format!("resolve failed: {e}")))?;
                let mut results = lookup_results_from_resolution(resolution);
                for r in &mut results {
                    r.title = crate::title_cleanup::title_case(&r.title);
                }
                let count = results.len();
                return Ok(LookupResponse {
                    results,
                    filtered_count: count,
                    raw_count: count,
                    raw_available: false,
                });
            }
        }

        let cache_key = (term.clone(), lang.clone());

        // Check cache (15 min TTL)
        {
            let cache = self.lookup_cache.lock().unwrap();
            if let Some(cached) = cache.get(&cache_key) {
                if cached.created_at.elapsed() < Duration::from_secs(900) {
                    let results = if raw || !cached.raw_available {
                        cached.raw.clone()
                    } else {
                        cached.filtered.clone()
                    };
                    return Ok(LookupResponse {
                        filtered_count: cached.filtered.len(),
                        raw_count: cached.raw.len(),
                        raw_available: cached.raw_available,
                        results,
                    });
                }
            }
        }

        let mut raw_results: Vec<LookupResult> = self.lookup(req).await?;
        for r in &mut raw_results {
            r.title = crate::title_cleanup::title_case(&r.title);
        }
        if raw_results.is_empty() {
            return Ok(LookupResponse {
                results: vec![],
                filtered_count: 0,
                raw_count: 0,
                raw_available: false,
            });
        }

        let raw_count = raw_results.len();

        // Attempt LLM filtering
        let (filtered, raw_available) = match self.llm_filter_search(&raw_results).await {
            Some(indices) if indices.len() < raw_count => {
                let filtered: Vec<LookupResult> = indices
                    .into_iter()
                    .filter_map(|i| raw_results.get(i).cloned())
                    .collect();
                (filtered, true)
            }
            _ => (raw_results.clone(), false),
        };

        let filtered_count = filtered.len();

        // Cache both
        {
            let mut cache = self.lookup_cache.lock().unwrap();
            // Evict stale entries
            cache.retain(|_, v| v.created_at.elapsed() < Duration::from_secs(900));
            cache.insert(
                cache_key,
                CachedLookup {
                    filtered: filtered.clone(),
                    raw: raw_results.clone(),
                    raw_available,
                    created_at: Instant::now(),
                },
            );
        }

        let results = if raw || !raw_available {
            raw_results
        } else {
            filtered
        };

        Ok(LookupResponse {
            results,
            filtered_count,
            raw_count,
            raw_available,
        })
    }

    async fn search_works(
        &self,
        user_id: UserId,
        query: &str,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<Work>, i64), WorkServiceError> {
        WorkDb::search_works(&self.db, user_id, query, page, page_size)
            .await
            .map_err(WorkServiceError::Db)
    }

    async fn download_cover_from_url(
        &self,
        user_id: i64,
        work_id: i64,
        cover_url: &str,
    ) -> Result<(), WorkServiceError> {
        let covers_dir = self.data_dir.join("covers").join(user_id.to_string());
        download_cover_to_disk(&self.http, cover_url, &covers_dir, work_id, "")
            .await
            .map_err(|e| WorkServiceError::Cover(e.to_string()))?;
        let thumb = covers_dir.join(format!("{work_id}_thumb.jpg"));
        let _ = tokio::fs::remove_file(&thumb).await;
        Ok(())
    }

    fn try_start_bulk_refresh(&self, user_id: i64) -> bool {
        let mut guard = self.bulk_refresh_users.lock().unwrap();
        guard.insert(user_id)
    }

    fn finish_bulk_refresh(&self, user_id: i64) {
        let mut guard = self.bulk_refresh_users.lock().unwrap();
        guard.remove(&user_id);
    }
}

impl<D, E, H, L, M, T> WorkServiceImpl<D, E, H, L, M, T>
where
    D: WorkDb + ConfigDb + Send + Sync,
    H: HttpFetcher + Send + Sync,
    L: LlmCaller + Send + Sync,
    M: crate::MergeEngine + Send + Sync,
    T: livrarr_domain::services::TagService + Send + Sync,
{
    async fn llm_filter_search(&self, results: &[LookupResult]) -> Option<Vec<usize>> {
        let mut listing = String::new();
        for (i, r) in results.iter().enumerate() {
            listing.push_str(&format!(
                "{}: \"{}\" by {} ({})\n",
                i,
                r.title,
                r.author_name,
                r.year.map(|y| y.to_string()).unwrap_or_default(),
            ));
        }

        let system = "You are a librarian assistant. Clean up book search results.";
        let user_prompt = format!(
            "These are search results from a book database:\n\n\
             {listing}\n\
             Clean up this list:\n\
             1. Remove non-book items (study guides, journals, blank notebooks, merchandise, board games)\n\
             2. Remove duplicate editions of the same work — keep the one with the best metadata\n\
             3. Remove comic/manga adaptations, movie tie-in editions, and abridged versions\n\
             4. Remove anthologies and compilations unless they are a well-known standalone work\n\
             5. Keep results that are legitimate different works even if titles are similar\n\n\
             Return a JSON array of the original indices to keep, e.g. [0, 2, 5].\n\
             Return ONLY the JSON array, no other text."
        );

        let mut context = HashMap::new();
        context.insert(LlmField::BibliographyHtml, LlmValue::Text(listing));

        let req = LlmCallRequest {
            system_template: system.to_string(),
            user_template: user_prompt,
            context,
            allowed_fields: &[LlmField::BibliographyHtml],
            timeout: Duration::from_secs(30),
            purpose: LlmPurpose::SearchResultCleanup,
        };

        let resp = self.llm.call(req).await.ok()?;

        let json_str = resp
            .content
            .trim()
            .strip_prefix("```json")
            .or_else(|| resp.content.trim().strip_prefix("```"))
            .unwrap_or(resp.content.trim())
            .strip_suffix("```")
            .unwrap_or(resp.content.trim())
            .trim();

        let indices: Vec<usize> = serde_json::from_str(json_str).ok()?;
        let max_idx = results.len();
        let valid: Vec<usize> = indices.into_iter().filter(|&i| i < max_idx).collect();

        if valid.is_empty() {
            return None;
        }

        Some(valid)
    }

    async fn lookup_goodreads(
        &self,
        term: &str,
        lang: &str,
    ) -> Result<Vec<LookupResult>, WorkServiceError> {
        let search_url = format!(
            "https://www.goodreads.com/search?q={}",
            urlencoding::encode(term)
        );

        let fetch_req = FetchRequest {
            url: search_url,
            method: HttpMethod::Get,
            headers: vec![("Accept-Language".into(), "en-US,en;q=0.9".into())],
            body: None,
            timeout: std::time::Duration::from_secs(10),
            rate_bucket: RateBucket::Goodreads,
            max_body_bytes: 2 * 1024 * 1024,
            anti_bot_check: true,
            user_agent: UserAgentProfile::Browser,
        };

        let resp = match self.http.fetch(fetch_req).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Goodreads search fetch failed: {e}");
                return Ok(vec![]);
            }
        };

        if resp.status >= 400 {
            tracing::warn!(
                status = resp.status,
                "Goodreads search returned non-success"
            );
            return Ok(vec![]);
        }

        let raw_html = String::from_utf8_lossy(&resp.body);

        if livrarr_external_data::provider_util::is_anti_bot_page(&raw_html) {
            tracing::warn!("Goodreads search: anti-bot page detected");
            return Ok(vec![]);
        }

        let parsed = livrarr_external_data::goodreads::parse_search_html(&raw_html);

        if parsed.is_empty() && raw_html.contains("itemtype=\"http") {
            tracing::warn!(
                "Goodreads parser drift: HTML contains schema.org Book rows but 0 passed \
                 validation. HTML structure may have changed."
            );
        }

        let lang_owned = lang.to_string();
        let results = parsed
            .into_iter()
            .map(|r| {
                let full_url = if r.detail_url.starts_with('/') {
                    format!("https://www.goodreads.com{}", r.detail_url)
                } else {
                    r.detail_url.clone()
                };
                let validated_url =
                    if livrarr_external_data::goodreads::validate_detail_url(&full_url) {
                        Some(full_url)
                    } else {
                        None
                    };
                LookupResult {
                    ol_key: None,
                    title: r.title,
                    author_name: r.author.unwrap_or_default(),
                    author_ol_key: None,
                    year: r.year,
                    cover_url: r.cover_url,
                    description: None,
                    series_name: r.series_name,
                    series_position: r.series_position,
                    source: Some("Goodreads".to_string()),
                    source_type: Some("goodreads".to_string()),
                    language: Some(lang_owned.clone()),
                    detail_url: validated_url,
                    rating: r.rating,
                    isbn_13: None,
                    candidate_id: None,
                    hc_key: None,
                    gr_key: None,
                    asin: None,
                }
            })
            .collect();

        Ok(results)
    }

    async fn lookup_openlibrary(
        &self,
        term: &str,
        lang: &str,
    ) -> Result<Vec<LookupResult>, WorkServiceError> {
        let lang_param = if lang != "en" {
            let ol_lang = iso639_1_to_3(lang);
            format!("&language={}", urlencoding::encode(ol_lang))
        } else {
            String::new()
        };
        let url = format!(
            "https://openlibrary.org/search.json?q={}&limit=50&fields=key,title,author_name,author_key,first_publish_year,cover_i{lang_param}",
            urlencoding::encode(term)
        );

        let fetch_req = FetchRequest {
            url,
            method: HttpMethod::Get,
            headers: vec![],
            body: None,
            timeout: std::time::Duration::from_secs(10),
            rate_bucket: RateBucket::OpenLibrary,
            max_body_bytes: 2 * 1024 * 1024,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
        };

        let resp = match self.http.fetch(fetch_req).await {
            Ok(r) => r,
            Err(e) => {
                return Err(WorkServiceError::Enrichment(format!(
                    "OpenLibrary request failed: {e}"
                )));
            }
        };

        if resp.status >= 400 {
            return Err(WorkServiceError::Enrichment(format!(
                "OpenLibrary returned {}",
                resp.status
            )));
        }

        let data: serde_json::Value = serde_json::from_slice(&resp.body)
            .map_err(|e| WorkServiceError::Enrichment(format!("OpenLibrary parse error: {e}")))?;

        let docs = data
            .get("docs")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        let results = docs
            .iter()
            .filter_map(|doc| {
                let key = doc.get("key")?.as_str()?;
                let title = doc.get("title")?.as_str()?;
                let ol_key = key.trim_start_matches("/works/").to_string();

                let author_name = doc
                    .get("author_name")
                    .and_then(|a| a.as_array())
                    .and_then(|a| a.first())
                    .and_then(|a| a.as_str())
                    .unwrap_or("Unknown")
                    .to_string();

                let author_ol_key = doc
                    .get("author_key")
                    .and_then(|a| a.as_array())
                    .and_then(|a| a.first())
                    .and_then(|a| a.as_str())
                    .map(|k| k.trim_start_matches("/authors/").to_string());

                let year = doc
                    .get("first_publish_year")
                    .and_then(|y| y.as_i64())
                    .map(|y| y as i32);

                let cover_url = doc
                    .get("cover_i")
                    .and_then(|c| c.as_i64())
                    .map(|c| format!("https://covers.openlibrary.org/b/id/{c}-M.jpg"));

                Some(LookupResult {
                    ol_key: Some(ol_key),
                    title: title.to_string(),
                    author_name,
                    author_ol_key,
                    year,
                    cover_url,
                    description: None,
                    series_name: None,
                    series_position: None,
                    source: None,
                    source_type: None,
                    language: Some(lang.to_string()),
                    detail_url: None,
                    rating: None,
                    isbn_13: None,
                    candidate_id: None,
                    hc_key: None,
                    gr_key: None,
                    asin: None,
                })
            })
            .collect();

        Ok(results)
    }

    async fn lookup_google_books(
        &self,
        term: &str,
        lang: &str,
    ) -> Result<Vec<LookupResult>, WorkServiceError> {
        let lang_norm = lang.split('-').next().unwrap_or(lang).to_lowercase();

        let api_key = match self.db.get_metadata_config().await {
            Ok(cfg) => match cfg
                .google_books_api_key
                .as_deref()
                .filter(|s| !s.is_empty())
            {
                Some(k) => k.to_string(),
                None => {
                    tracing::debug!(term = %term, "GoogleBooks: no API key configured; skipping");
                    return Ok(vec![]);
                }
            },
            Err(_) => return Ok(vec![]),
        };

        let url = format!(
            "https://www.googleapis.com/books/v1/volumes\
             ?q={}&langRestrict={}&maxResults=20",
            urlencoding::encode(term),
            urlencoding::encode(&lang_norm),
        );

        let volumes =
            livrarr_external_data::google_books::fetch_gb_volumes(&self.http, &api_key, url)
                .await
                .map_err(WorkServiceError::Enrichment)?;

        let results = volumes
            .iter()
            .filter_map(|vol| {
                let vi = vol.volume_info.as_ref()?;
                let title = vi.title.as_ref()?.clone();
                let author_name = vi
                    .authors
                    .as_ref()
                    .and_then(|a| a.first())
                    .cloned()
                    .unwrap_or_else(|| "Unknown".to_string());
                let year = vi
                    .published_date
                    .as_deref()
                    .and_then(|d| d.get(..4))
                    .and_then(|y| y.parse::<i32>().ok());
                let cover_url = vi
                    .image_links
                    .as_ref()
                    .and_then(livrarr_external_data::google_books::normalize_cover_url);
                let language = vi.language.clone().or_else(|| Some(lang_norm.clone()));

                Some(LookupResult {
                    ol_key: None,
                    title,
                    author_name,
                    author_ol_key: None,
                    year,
                    cover_url,
                    description: None,
                    series_name: None,
                    series_position: None,
                    source: Some("google_books".into()),
                    source_type: Some("search".into()),
                    language,
                    detail_url: None,
                    rating: None,
                    isbn_13: livrarr_external_data::google_books::extract_isbn13(
                        &vi.industry_identifiers,
                    ),
                    candidate_id: None,
                    hc_key: None,
                    gr_key: None,
                    asin: None,
                })
            })
            .collect();

        Ok(results)
    }

    async fn lookup_hardcover(
        &self,
        term: &str,
        _lang: &str,
    ) -> Result<Vec<LookupResult>, WorkServiceError> {
        let cfg = match self.db.get_metadata_config().await {
            Ok(c) => c,
            Err(_) => return Ok(vec![]),
        };

        if !cfg.hardcover_enabled {
            return Ok(vec![]);
        }

        let token = match cfg
            .hardcover_api_token
            .as_deref()
            .map(|t| {
                t.trim()
                    .trim_start_matches("Bearer ")
                    .trim_start_matches("bearer ")
            })
            .filter(|t| !t.is_empty())
        {
            Some(t) => t.to_string(),
            None => return Ok(vec![]),
        };

        let query = r#"query SearchBooks($query: String!) {
            search(query: $query, query_type: "books", per_page: 15) {
                results
            }
        }"#;

        let body = serde_json::json!({
            "query": query,
            "variables": {"query": term}
        });

        let body_bytes = serde_json::to_vec(&body)
            .map_err(|e| WorkServiceError::Enrichment(format!("HC serialize: {e}")))?;

        let resp = self
            .http
            .fetch(livrarr_domain::services::FetchRequest {
                url: livrarr_external_data::hardcover::HARDCOVER_API_URL.to_string(),
                method: livrarr_domain::services::HttpMethod::Post,
                headers: vec![
                    ("Authorization".into(), format!("Bearer {token}")),
                    ("Content-Type".into(), "application/json".into()),
                ],
                body: Some(body_bytes),
                timeout: std::time::Duration::from_secs(10),
                rate_bucket: livrarr_domain::services::RateBucket::Hardcover,
                max_body_bytes: 2 * 1024 * 1024,
                anti_bot_check: false,
                user_agent: livrarr_domain::services::UserAgentProfile::Server,
            })
            .await
            .map_err(|e| WorkServiceError::Enrichment(format!("HC search: {e}")))?;

        if resp.status >= 400 {
            return Err(WorkServiceError::Enrichment(format!(
                "HC search HTTP {}",
                resp.status
            )));
        }

        let data: serde_json::Value = serde_json::from_slice(&resp.body)
            .map_err(|e| WorkServiceError::Enrichment(format!("HC parse: {e}")))?;

        let hits = data
            .pointer("/data/search/results/hits")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();

        let results: Vec<LookupResult> = hits
            .iter()
            .filter_map(|hit| {
                let doc = hit.get("document")?;
                let title = doc.get("title")?.as_str()?.to_string();
                let author_name = doc
                    .get("author_names")
                    .and_then(|a| a.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown")
                    .to_string();
                let isbn_13 = doc.get("isbns").and_then(|v| v.as_array()).and_then(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .find(|s| s.len() == 13 && (s.starts_with("978") || s.starts_with("979")))
                        .map(|s| s.to_string())
                });
                let cover_url = doc
                    .pointer("/image/url")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                Some(LookupResult {
                    ol_key: None,
                    title,
                    author_name,
                    author_ol_key: None,
                    year: None,
                    cover_url,
                    description: None,
                    series_name: None,
                    series_position: None,
                    source: Some("hardcover".into()),
                    source_type: Some("search".into()),
                    language: None,
                    detail_url: None,
                    rating: None,
                    isbn_13,
                    candidate_id: None,
                    hc_key: None,
                    gr_key: None,
                    asin: None,
                })
            })
            .collect();

        Ok(results)
    }
}

// =============================================================================
// add() helpers
// =============================================================================

impl<D, E, H, L, M, T> WorkServiceImpl<D, E, H, L, M, T>
where
    D: WorkDb
        + WorkDbCreate
        + AuthorDb
        + LibraryItemDb
        + ProvenanceDb
        + EnrichmentRetryDb
        + livrarr_domain::services::WorkIdentityRepository
        + Send
        + Sync,
    E: EnrichmentWorkflow + Send + Sync,
    H: HttpFetcher + Clone + Send + Sync + 'static,
    L: LlmCaller + Send + Sync,
    M: crate::MergeEngine + Send + Sync,
    T: livrarr_domain::services::TagService + Send + Sync,
{
    async fn try_dedup_by_normalized(
        &self,
        user_id: UserId,
        normalized_title: &str,
        normalized_author: &str,
        source_provider_data: &Option<SourceProviderData>,
    ) -> Result<Option<(Work, EnrichmentStatus)>, WorkServiceError> {
        let existing = self
            .db
            .find_by_normalized_match(user_id, normalized_title, normalized_author)
            .await
            .map_err(WorkServiceError::Db)?;
        if let Some(work) = existing.into_iter().next() {
            let (work, enrichment_status) = if source_provider_data.is_some() {
                let status = self
                    .run_unified_enrichment(user_id, &work, source_provider_data.clone())
                    .await;
                let refreshed = self.db.get_work(user_id, work.id).await.unwrap_or(work);
                (refreshed, status)
            } else {
                let status = work.enrichment_status;
                (work, status)
            };
            Ok(Some((work, enrichment_status)))
        } else {
            Ok(None)
        }
    }

    async fn find_or_create_author(
        &self,
        user_id: UserId,
        cleaned_author: &str,
        author_ol_key: Option<&str>,
    ) -> Result<(bool, Option<i64>), WorkServiceError> {
        if cleaned_author.is_empty() {
            return Ok((false, None));
        }
        let normalized = cleaned_author.to_lowercase();
        match self
            .db
            .find_author_by_name(user_id, &normalized)
            .await
            .map_err(WorkServiceError::Db)?
        {
            Some(existing) => Ok((false, Some(existing.id))),
            None => {
                let author = self
                    .db
                    .create_author(CreateAuthorDbRequest {
                        user_id,
                        name: cleaned_author.to_string(),
                        sort_name: None,
                        ol_key: author_ol_key.map(|s| s.to_string()),
                        gr_key: None,
                        hc_key: None,
                        import_id: None,
                    })
                    .await
                    .map_err(WorkServiceError::Db)?;
                Ok((true, Some(author.id)))
            }
        }
    }

    async fn handle_race_loser(
        &self,
        user_id: UserId,
        work: Work,
        author_created: bool,
        author_id: Option<i64>,
        source_provider_data: Option<SourceProviderData>,
    ) -> Result<AddWorkResult, WorkServiceError> {
        let (work, enrichment_status) = if source_provider_data.is_some() {
            let status = self
                .run_unified_enrichment(user_id, &work, source_provider_data)
                .await;
            let refreshed = self.db.get_work(user_id, work.id).await.unwrap_or(work);
            (refreshed, status)
        } else {
            let status = work.enrichment_status;
            (work, status)
        };
        Ok(AddWorkResult {
            work,
            created: false,
            author_created,
            author_id,
            messages: vec![],
            cover_mtime: None,
            audiobook_cover_mtime: None,
            enrichment_status,
        })
    }

    async fn finish_created_work(
        &self,
        user_id: UserId,
        work: Work,
        author_created: bool,
        author_id: Option<i64>,
        source_provider_data: Option<SourceProviderData>,
        skip_sync_enrichment: bool,
    ) -> Result<AddWorkResult, WorkServiceError> {
        // Phase 1: synchronous cover download within 3s budget (REQ-010).
        let is_user_initiated = source_provider_data.is_none();
        let covers_dir = self.data_dir.join("covers").join(user_id.to_string());
        let phase1_mtime = crate::cover::fetch_phase1_cover(
            &self.http,
            &self.http_client,
            &work.title,
            &work.author_name,
            work.cover_url.as_deref(),
            None,
            &covers_dir,
            work.id,
        )
        .await;

        // Assign trust based on how the work was added.
        if phase1_mtime.is_some() || work.cover_url.is_some() {
            let is_fallback = phase1_mtime.is_some() && work.cover_url.is_none();
            let trust = crate::cover_resolution::phase1_trust(is_user_initiated, is_fallback);
            let source = work.cover_source.as_deref().unwrap_or("add");
            let _ = self
                .db
                .update_cover_metadata(
                    user_id,
                    work.id,
                    work.cover_url.as_deref(),
                    source,
                    trust,
                    0,
                    0,
                )
                .await;
        }

        // Skip sync enrichment: return Unenriched immediately (REQ-009).
        // Background retry job will pick up works with Unenriched status.
        // Readarr imports (source_provider_data.is_some()) still get sync enrichment.
        if skip_sync_enrichment && source_provider_data.is_none() {
            let updated_work = self
                .db
                .get_work(user_id, work.id)
                .await
                .map_err(WorkServiceError::Db)?;
            let cover_mtime =
                crate::cover::cover_file_mtime(&covers_dir, updated_work.id).or_else(|| {
                    crate::cover::cover_file_mtime(&self.data_dir.join("covers"), updated_work.id)
                });
            let audiobook_cover_mtime =
                crate::cover::audiobook_cover_file_mtime(&covers_dir, updated_work.id).or_else(
                    || {
                        crate::cover::audiobook_cover_file_mtime(
                            &self.data_dir.join("covers"),
                            updated_work.id,
                        )
                    },
                );
            return Ok(AddWorkResult {
                work: updated_work,
                created: true,
                author_created,
                author_id,
                messages: vec![],
                cover_mtime,
                audiobook_cover_mtime,
                enrichment_status: EnrichmentStatus::Unenriched,
            });
        }

        let enrichment_status = self
            .run_unified_enrichment(user_id, &work, source_provider_data)
            .await;
        let updated_work = self
            .db
            .get_work(user_id, work.id)
            .await
            .map_err(WorkServiceError::Db)?;
        let covers_dir = self.data_dir.join("covers").join(user_id.to_string());
        let cover_mtime =
            crate::cover::cover_file_mtime(&covers_dir, updated_work.id).or_else(|| {
                crate::cover::cover_file_mtime(&self.data_dir.join("covers"), updated_work.id)
            });
        let audiobook_cover_mtime =
            crate::cover::audiobook_cover_file_mtime(&covers_dir, updated_work.id).or_else(|| {
                crate::cover::audiobook_cover_file_mtime(
                    &self.data_dir.join("covers"),
                    updated_work.id,
                )
            });
        Ok(AddWorkResult {
            work: updated_work,
            created: true,
            author_created,
            author_id,
            messages: vec![],
            cover_mtime,
            audiobook_cover_mtime,
            enrichment_status,
        })
    }
}

// =============================================================================
// Unified enrichment pipeline
// =============================================================================

impl<D, E, H, L, M, T> WorkServiceImpl<D, E, H, L, M, T>
where
    D: WorkDb + LibraryItemDb + ProvenanceDb + EnrichmentRetryDb + Send + Sync,
    E: EnrichmentWorkflow + Send + Sync,
    H: HttpFetcher + Clone + Send + Sync + 'static,
    L: LlmCaller + Send + Sync,
    M: crate::MergeEngine + Send + Sync,
    T: livrarr_domain::services::TagService + Send + Sync,
{
    /// Run the full enrichment pipeline synchronously.
    ///
    /// Steps:
    ///   1. Inject source provider data (if present) via `enrichment.inject_source_data`
    ///   2. Dispatch to providers via `enrichment.enrich_work`
    ///   3. Collect per-provider provenance from DB
    ///   4. Merge using `merge_engine.merge`
    ///   5. Apply merge to DB via `db.apply_enrichment_merge`
    ///   6. Download cover (if cover_url present and not manual)
    ///   7. Tag sync all existing library items via `tag_service.retag_library_items`
    ///
    /// Returns the final `EnrichmentStatus`. Never returns `Err` — all failures
    /// are absorbed and produce `Failed` status, never a caller error.
    async fn run_unified_enrichment(
        &self,
        user_id: UserId,
        work: &Work,
        source_provider_data: Option<livrarr_domain::services::SourceProviderData>,
    ) -> EnrichmentStatus {
        let work_id = work.id;

        // Step 1: Inject source provider data (Readarr import etc.)
        if let Some(src) = source_provider_data {
            self.enrichment
                .inject_source_data(user_id, work_id, src)
                .await;
        }

        // Step 2: Provider dispatch — scatter-gather enrichment
        let enrich_result = match self
            .enrichment
            .enrich_work(user_id, work_id, EnrichmentMode::Background)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(work_id, "run_unified_enrichment: enrich_work failed: {e}");
                return EnrichmentStatus::Failed;
            }
        };

        // Step 3: After enrichment, reload work and provenance from DB.
        let post_enrich_work = match self.db.get_work(user_id, work_id).await {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(work_id, "run_unified_enrichment: get_work failed: {e}");
                return EnrichmentStatus::Failed;
            }
        };

        // Use the enrichment_status from the enrich_work pipeline
        // (it already ran merge internally via EnrichmentServiceImpl).
        let final_status = enrich_result.enrichment_status;

        // Step 4: Trust-aware cover upgrade (non-fatal). Ebook and audiobook
        // covers are independent; upgrade each from its own resolution.
        let covers_dir = self.data_dir.join("covers").join(user_id.to_string());
        match crate::cover_resolution::maybe_upgrade_cover(
            &post_enrich_work,
            enrich_result.cover_resolution,
            &covers_dir,
            &self.http,
        )
        .await
        {
            Ok(Some(upgrade)) => {
                if let Err(e) = self
                    .db
                    .update_cover_metadata(
                        user_id,
                        work_id,
                        Some(&upgrade.url),
                        &upgrade.source,
                        upgrade.trust,
                        upgrade.width as i32,
                        upgrade.height as i32,
                    )
                    .await
                {
                    tracing::warn!(work_id, "cover metadata update failed: {e}");
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(work_id, "cover upgrade failed: {e}");
            }
        }
        match crate::cover_resolution::maybe_upgrade_cover(
            &post_enrich_work,
            enrich_result.audiobook_cover_resolution,
            &covers_dir,
            &self.http,
        )
        .await
        {
            Ok(Some(upgrade)) => {
                if let Err(e) = self
                    .db
                    .update_audiobook_cover_metadata(
                        user_id,
                        work_id,
                        Some(&upgrade.url),
                        &upgrade.source,
                        upgrade.trust,
                        upgrade.width as i32,
                        upgrade.height as i32,
                    )
                    .await
                {
                    tracing::warn!(work_id, "audiobook cover metadata update failed: {e}");
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(work_id, "audiobook cover upgrade failed: {e}");
            }
        }

        // Step 5: Tag sync all existing library items (non-fatal)
        let items = self
            .db
            .list_taggable_items_by_work(user_id, work_id)
            .await
            .unwrap_or_default();

        if !items.is_empty() {
            let tag_results = self
                .tag_service
                .retag_library_items(&post_enrich_work, &items)
                .await;

            let merge_generation = self
                .db
                .get_merge_generation(user_id, work_id)
                .await
                .unwrap_or(0);
            for result in &tag_results {
                let tag_status = if result.succeeded {
                    livrarr_domain::TagStatus::Synced
                } else {
                    livrarr_domain::TagStatus::Failed
                };
                if let Err(e) = self
                    .db
                    .update_library_item_tag_status(
                        result.library_item_id,
                        tag_status,
                        merge_generation,
                    )
                    .await
                {
                    tracing::warn!(
                        work_id,
                        item_id = result.library_item_id,
                        "run_unified_enrichment: update_library_item_tag_status failed: {e}"
                    );
                }
            }
        }

        final_status
    }
}

async fn write_addtime_provenance<D: ProvenanceDb>(
    db: &D,
    user_id: i64,
    work: &Work,
    setter: ProvenanceSetter,
) {
    crate::provenance::write_addtime_provenance(db, user_id, work, setter).await;
}

pub fn unproxy_cover_url(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("/api/v1/coverproxy?url=") {
        urlencoding::decode(rest)
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| url.to_string())
    } else {
        url.to_string()
    }
}

pub async fn download_cover_to_disk<H: HttpFetcher>(
    http: &H,
    url: &str,
    covers_dir: &std::path::Path,
    work_id: i64,
    suffix: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tokio::fs::create_dir_all(covers_dir).await?;

    let req = FetchRequest {
        url: url.to_string(),
        method: HttpMethod::Get,
        headers: vec![],
        body: None,
        timeout: std::time::Duration::from_secs(30),
        rate_bucket: RateBucket::None,
        max_body_bytes: 10 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
    };

    let resp = http
        .fetch_ssrf_safe(req)
        .await
        .map_err(|e| format!("fetch: {e}"))?;
    if resp.status >= 400 {
        return Err(format!("cover download returned {}", resp.status).into());
    }

    let cover_path = covers_dir.join(format!("{work_id}{suffix}.jpg"));
    let tmp_path = cover_path.with_extension("jpg.tmp");
    let tmp_clone = tmp_path.clone();
    let target = cover_path.clone();
    let bytes = resp.body;
    let result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp_clone)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp_clone, &target)
    })
    .await;
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            Err(Box::new(e))
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            Err(format!("spawn error: {e}").into())
        }
    }
}

pub async fn delete_cover_files(data_dir: &std::path::Path, user_id: i64, work_id: i64) {
    for dir in [
        data_dir.join("covers").join(user_id.to_string()),
        data_dir.join("covers"),
    ] {
        let _ = tokio::fs::remove_file(dir.join(format!("{work_id}.jpg"))).await;
        let _ = tokio::fs::remove_file(dir.join(format!("{work_id}_thumb.jpg"))).await;
        let _ = tokio::fs::remove_file(dir.join(format!("{work_id}_audio.jpg"))).await;
        let _ = tokio::fs::remove_file(dir.join(format!("{work_id}_audio_thumb.jpg"))).await;
        let _ = tokio::fs::remove_file(dir.join(format!("{work_id}.candidate.tmp"))).await;
        let _ = tokio::fs::remove_file(dir.join(format!("{work_id}_audio.candidate.tmp"))).await;
    }
}

fn is_supported_image(bytes: &[u8]) -> bool {
    if bytes.len() < 12 {
        return false;
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return true;
    }
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return true;
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return true;
    }
    if &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return true;
    }
    false
}
