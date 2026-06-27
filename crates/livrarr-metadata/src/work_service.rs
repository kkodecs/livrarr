use futures::stream::{self, StreamExt};
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

use livrarr_domain::seed::{iso639_1_to_3, lookup_term_to_seed, seed_carries_identifier};

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
    // merge_engine and tag_service are reserved for future slices (S8+).
    // materialize now owns the save step (S7); direct use will resume
    // when the provider-policy pipeline is wired end-to-end.
    #[allow(dead_code)]
    merge_engine: M,
    #[allow(dead_code)]
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
        _candidate_id: Option<livrarr_domain::identity::CandidateId>,
    ) -> Result<EnrichmentResult, EnrichmentWorkflowError> {
        Ok(EnrichmentResult {
            identity_not_found: false,
            changed: false,
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

/// The dead-end attempt threshold above which a missing anchor is no longer
/// chased (REQ-009, PO-locked at 3). The background convergence job reads its
/// threshold from `[convergence]` config; the synchronous refresh gate uses
/// this default directly.
const DEAD_END_THRESHOLD: u32 = 3;

/// The hard-anchor types still worth chasing on a work: a `works.*` column that
/// is NULL, holds no pending (fuzzy-guessed) ledger row, and has not reached the
/// dead-end attempt `threshold`. Shared by the refresh gate (Insertion B) and the
/// background convergence loop so both agree on what "still obtainable" means
/// (REQ-006, RE-007).
fn chaseable_anchor_types(
    work: &Work,
    anchors: &[livrarr_domain::identity::WorkIdentityAnchor],
    dead_ends: &[livrarr_domain::identity::AnchorDeadEnd],
    threshold: u32,
) -> Vec<livrarr_domain::identity::AnchorType> {
    use livrarr_domain::identity::{AnchorConfidence, AnchorType};
    [
        (AnchorType::OL_WORK, work.ol_key.is_none()),
        (AnchorType::GR_WORK, work.gr_key.is_none()),
        (AnchorType::HC_WORK, work.hc_key.is_none()),
        (AnchorType::ISBN_13, work.isbn_13.is_none()),
        (AnchorType::ASIN, work.asin.is_none()),
    ]
    .into_iter()
    .filter(|&(anchor_type, missing)| {
        missing
            && !anchors.iter().any(|a| {
                a.anchor_type.as_str() == anchor_type && a.confidence == AnchorConfidence::Pending
            })
            && !dead_ends
                .iter()
                .any(|d| d.anchor_type.as_str() == anchor_type && d.attempt_count >= threshold)
    })
    .map(|(anchor_type, _)| AnchorType::new(anchor_type))
    .collect()
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

/// Take one provider's discovery result (relevance-ordered), logging a failure or
/// timeout rather than failing the whole search. Generic over the provider error
/// type so every provider lookup can share one helper.
fn take_lookup<E: std::fmt::Display>(
    provider: &str,
    term: &str,
    res: Result<Result<Vec<LookupResult>, E>, tokio::time::error::Elapsed>,
) -> Vec<LookupResult> {
    match res {
        Ok(Ok(found)) => found,
        Ok(Err(e)) => {
            tracing::warn!(provider, term, "discovery provider failed: {e}");
            Vec::new()
        }
        Err(_) => {
            tracing::warn!(provider, term, "discovery provider timed out");
            Vec::new()
        }
    }
}

/// Round-robin the per-provider lists in fixed-size chunks so the strongest
/// matches from every provider lead and quality degrades evenly down the combined
/// list — instead of a naive concat (good→bad, good→bad, …). With `chunk = 3` the
/// first rows are the top 3 of each provider, then the next 3 of each, and so on.
fn interleave_by(lists: Vec<Vec<LookupResult>>, chunk: usize) -> Vec<LookupResult> {
    let mut iters: Vec<_> = lists.into_iter().map(|l| l.into_iter()).collect();
    let mut out = Vec::new();
    loop {
        let mut any = false;
        for it in &mut iters {
            for _ in 0..chunk {
                match it.next() {
                    Some(item) => {
                        out.push(item);
                        any = true;
                    }
                    None => break,
                }
            }
        }
        if !any {
            break;
        }
    }
    out
}

/// Map a resolved/confirmable identity into a wire `LookupResult`, carrying the
/// federated anchors + the `candidate_id` payload handle (REQ-014/R-009) and
/// the contributing providers as the result's source attribution (#147 — a
/// source-less result renders chip-less in the search UI).
fn lookup_result_from_captured(
    captured: livrarr_domain::identity::CapturedIdentity,
    candidate_id: Option<livrarr_domain::identity::CandidateId>,
    cover_url: Option<String>,
    sources: &[livrarr_domain::MetadataProvider],
) -> LookupResult {
    let source = if sources.is_empty() {
        None
    } else {
        Some(
            sources
                .iter()
                .map(|p| p.record_key())
                .collect::<Vec<_>>()
                .join("+"),
        )
    };
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
        source_type: source.clone(),
        source,
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
            &[],
        )],
        Resolution::NeedsConfirmation { candidates } => candidates
            .into_iter()
            .map(|c| {
                lookup_result_from_captured(
                    c.anchors,
                    Some(c.candidate_id),
                    c.cover_url,
                    &c.sources,
                )
            })
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
        + livrarr_db::ProviderRetryStateDb
        + ConfigDb
        + livrarr_db::SeriesDb
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

        // The persisted identity badge derived from the candidate's anchors
        // (REQ-014/016, D-013) — written at create and used to gate enrichment.
        let derived_identity = candidate.identity.derived_identity_status();

        // The originating door's identity patience (REQ-005) + conflict
        // attribution (REQ-020), threaded to the one identity road through the
        // chokepoint (ensure_identity_and_enrichment / settle_identity).
        // Spawned/batch import doors resolve in Background; a person-facing add
        // resolves Interactive. Author-monitor seeds a hard key and never reaches
        // the anchorless leg (the RE-009 exception).
        let identity_setter = candidate
            .provenance_setter
            .unwrap_or(ProvenanceSetter::User);
        let identity_source = conflict_source_for(identity_setter);
        let identity_mode = match identity_setter {
            ProvenanceSetter::Import | ProvenanceSetter::Imported => {
                livrarr_domain::identity::IdentityMode::Background
            }
            _ => livrarr_domain::identity::IdentityMode::Interactive,
        };

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
                    // REQ-010: the matched work takes the same identity +
                    // enrichment road as every other add outcome (the
                    // existing-work doors previously bypassed it).
                    let (enrichment_status, _identity_not_found) = self
                        .ensure_identity_and_enrichment(
                            user_id,
                            work.id,
                            candidate.source_provider_data,
                            None,
                            identity_mode,
                            identity_source,
                        )
                        .await;
                    let work = self.db.get_work(user_id, work.id).await.unwrap_or(work);
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
                    // REQ-010: the adopted work takes the same identity +
                    // enrichment road as every other add outcome.
                    let (enrichment_status, _identity_not_found) = self
                        .ensure_identity_and_enrichment(
                            user_id,
                            existing.id,
                            candidate.source_provider_data.clone(),
                            None,
                            identity_mode,
                            identity_source,
                        )
                        .await;
                    let work = self
                        .db
                        .get_work(user_id, existing.id)
                        .await
                        .unwrap_or(existing);
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
                    // REQ-010: the deduped work takes the same identity +
                    // enrichment road as every other add outcome.
                    let (enrichment_status, _identity_not_found) = self
                        .ensure_identity_and_enrichment(
                            user_id,
                            work.id,
                            candidate.source_provider_data.clone(),
                            None,
                            identity_mode,
                            identity_source,
                        )
                        .await;
                    let work = self.db.get_work(user_id, work.id).await.unwrap_or(work);
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
                            identity_mode,
                            identity_source,
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
                    derived_identity,
                    candidate.candidate_id.as_ref(),
                    identity_mode,
                    identity_source,
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
                            identity_mode,
                            identity_source,
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

                // A Pending identity reaches ensure_identity_and_enrichment via
                // finish_created_work: the add-time identity leg may resolve it
                // (REQ-010); a still-held identity skips the fan-out there.
                // Display/cover (best-in-hand) is materialized either way.
                self.finish_created_work(
                    user_id,
                    work,
                    author_created,
                    author_id,
                    candidate.source_provider_data,
                    derived_identity,
                    candidate.candidate_id.as_ref(),
                    identity_mode,
                    identity_source,
                )
                .await
            }
        }
    }

    async fn resolve_identity(
        &self,
        user_id: UserId,
        harvest: livrarr_domain::identity::RawHarvest,
        tier: livrarr_domain::identity::LatencyTier,
    ) -> Result<livrarr_domain::identity::ResolvedIdentity, WorkServiceError> {
        use livrarr_domain::identity::{
            CapturedIdentity, IdentityState, PendingReason, Resolution, ResolvedIdentity, WorkSeed,
        };

        // Preserve any language the door already knew, before `sanitized` consumes
        // the harvest.
        let harvest_language = harvest.language.clone();

        // Sanitize at the boundary: normalize keys, drop malformed ones, and only
        // resolve when a real anchor survives — identical to the Add-Work handler,
        // now the shared path for every door (P1 convergence).
        let seed = WorkSeed::sanitized(harvest).ok().filter(|s| {
            s.ol_key.is_some()
                || s.gr_key.is_some()
                || s.hc_key.is_some()
                || s.isbn_13.is_some()
                || s.asin.is_some()
        });

        let Some(seed) = seed else {
            // No usable anchor — a fuzzy title/author seed. Pending; enrichment or a
            // later pass converges it.
            return Ok(ResolvedIdentity {
                identity: IdentityState::Pending {
                    reason: PendingReason::NoCandidates,
                    seed_anchors: None,
                    top_candidates: vec![],
                },
                candidate_id: None,
                language: harvest_language,
                conflict: None,
            });
        };

        let captured_from_seed = |s: &WorkSeed| CapturedIdentity {
            ol_key: s.ol_key.clone(),
            gr_key: s.gr_key.clone(),
            hc_key: s.hc_key.clone(),
            isbn_13: s.isbn_13.clone(),
            asin: s.asin.clone(),
            title: s.title.clone().unwrap_or_default(),
            author_name: s.author_name.clone().unwrap_or_default(),
            language: s.language.clone(),
        };

        let Some(resolver) = self.resolver.clone() else {
            // No resolver wired (some headless/test configs): keep the anchors as a
            // Pending seed so a background pass can converge — never fabricate a
            // Confirmed badge from un-corroborated keys.
            let language = seed.language.clone();
            return Ok(ResolvedIdentity {
                identity: IdentityState::Pending {
                    reason: PendingReason::NoCandidates,
                    seed_anchors: Some(captured_from_seed(&seed)),
                    top_candidates: vec![],
                },
                candidate_id: None,
                language,
                conflict: None,
            });
        };

        use livrarr_domain::services::IdentityResolver;
        let resolution = resolver
            .resolve(user_id, &seed, tier)
            .await
            .map_err(|e| WorkServiceError::Validation(format!("identity resolve: {e}")))?;

        Ok(match resolution {
            Resolution::Resolved {
                identity,
                method,
                candidate_id,
                ..
            } => ResolvedIdentity {
                language: identity.language.clone(),
                identity: IdentityState::Confirmed {
                    anchors: identity,
                    method,
                    score: None,
                },
                candidate_id: Some(candidate_id),
                conflict: None,
            },
            Resolution::NeedsConfirmation { candidates } => ResolvedIdentity {
                identity: IdentityState::Pending {
                    reason: PendingReason::LowConfidence,
                    seed_anchors: None,
                    top_candidates: candidates,
                },
                candidate_id: None,
                language: None,
                conflict: None,
            },
            Resolution::Unresolved {
                captured,
                reason,
                candidate_id,
                ..
            } => ResolvedIdentity {
                language: captured.language.clone(),
                identity: IdentityState::Pending {
                    reason,
                    seed_anchors: Some(captured),
                    top_candidates: vec![],
                },
                candidate_id,
                conflict: None,
            },
            Resolution::Conflict {
                conflict, captured, ..
            } => ResolvedIdentity {
                language: captured.language.clone(),
                identity: IdentityState::Pending {
                    reason: PendingReason::LowConfidence,
                    seed_anchors: Some(captured),
                    top_candidates: vec![],
                },
                candidate_id: None,
                conflict: Some(conflict),
            },
        })
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
        if let Some(ref language) = filter.language {
            works.retain(|w| w.language.as_deref() == Some(language.as_str()));
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

        // Series reconcile (REQ-001, user origin — always wins): the returned
        // work carries the new series_name and the pre-edit series_id, which
        // is exactly the state reconcile arbitrates (relink/unlink + stub GC).
        // Failures PROPAGATE here: a user-edit unlink/relink has no self-heal
        // (the startup back-fill only links series_id-NULL works), so a
        // swallowed error would leave a visibly stale catalog. Reconcile is
        // idempotent — the client retries the edit safely.
        if has_series_name {
            crate::series_link::reconcile_work_series(
                &self.db,
                &work,
                crate::series_link::SeriesLinkOrigin::User,
            )
            .await
            .map_err(WorkServiceError::Db)?;
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

        let deleted = self
            .db
            .delete_work(user_id, work_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
            })?;

        // REQ-001/AC-012: deleting a work unlinks it; GC an unmonitored stub
        // left with zero linked works.
        if let Some(series_id) = deleted.series_id {
            if let Err(e) = crate::series_link::gc_stub_if_empty(&self.db, user_id, series_id).await
            {
                tracing::warn!(work_id, series_id, error = %e, "stub GC on work delete failed");
            }
        }

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
        let _refresh_span = livrarr_domain::perf::StageTimer::start("refresh_total", work_id);

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

        // REQ-002/REQ-006 (id-completeness): re-chase a work's still-obtainable
        // hard anchors via the one identity road on every refresh door (single +
        // bulk + retry all funnel through here). "Obtainable" = a NULL works.*
        // column with no pending guess and below the dead-end threshold
        // (chaseable_anchor_types). This SUPERSEDES the Sprint-E `!= Confirmed`
        // gate (insight 55): a Confirmed work missing a secondary id is now
        // topped up, while a fully-anchored or fully-dead-ended work skips the
        // resolver fan-out entirely — the cost Sprint-E removed. settle_identity
        // is the identity authority; the smart-skip it deliberately lacks
        // (ST-002) is re-applied here via the chaseable gate.
        //
        // NOTE: reset_for_manual_refresh above already DELETED provider_retry_state,
        // so a refresh always re-attempts providers — no suppression survives.
        let mut work = work;
        if let Some(resolver) = self.resolver.as_ref() {
            let _id_span = livrarr_domain::perf::StageTimer::start("identity", work_id);
            let anchors = self.db.list_anchors(work.id).await.unwrap_or_default();
            let dead_ends = self
                .db
                .list_anchor_dead_ends(work.id)
                .await
                .unwrap_or_default();
            if !chaseable_anchor_types(&work, &anchors, &dead_ends, DEAD_END_THRESHOLD).is_empty() {
                match crate::async_resolver::settle_identity(
                    resolver.as_ref(),
                    &self.db,
                    user_id,
                    &work,
                    livrarr_domain::identity::IdentityMode::Interactive,
                    livrarr_domain::identity::ConflictSource::Refresh,
                )
                .await
                {
                    Ok(_) => {
                        if let Ok(w) = self.db.get_work(user_id, work_id).await {
                            work = w;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            work_id,
                            "refresh identity settle failed; scatter proceeds: {e}"
                        );
                    }
                }
            }
        }

        // Unified enrichment: provider dispatch, merge, cover download, tag sync.
        // Manual mode (not Background) so a transiently-unavailable provider
        // (e.g. Google Books quota 429) does not defer the entire merge and
        // discard the data other providers returned — best-effort merge. (#117)
        // No candidate_id for a manual refresh — always re-fetches from network.
        let _enrichment_status = self
            .run_unified_enrichment(user_id, &work, None, EnrichmentMode::Manual, None)
            .await;

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

    async fn retry_all_incomplete(
        &self,
        user_id: UserId,
    ) -> Result<livrarr_domain::services::RetrySummary, WorkServiceError> {
        // Single pass over every "incomplete" work — Failed, Unenriched, or
        // identity-Pending — filtered in memory (like refresh_all). This REPLACES
        // the deleted background retry job: user-triggered, one pass, no recurring
        // loop (REQ-011 / PO §7).
        let works = self
            .db
            .list_works(user_id)
            .await
            .map_err(WorkServiceError::Db)?;
        let incomplete: Vec<Work> = works
            .into_iter()
            .filter(|w| {
                matches!(
                    w.enrichment_status,
                    EnrichmentStatus::Failed | EnrichmentStatus::Unenriched
                ) || w.identity_status == IdentityStatus::Pending
            })
            .collect();

        let total = incomplete.len();
        let mut recovered = 0usize;

        for work in &incomplete {
            // A Pending work re-resolves identity first via the one identity road
            // (settle_identity) — Background mode so Audnexus stays eligible
            // (REQ-001). The promoted anchor survives the refresh below
            // (reset_enrichment_for_refresh touches only enrichment).
            if work.identity_status == IdentityStatus::Pending {
                if let Some(resolver) = self.resolver.as_ref() {
                    if let Err(e) = crate::async_resolver::settle_identity(
                        resolver.as_ref(),
                        &self.db,
                        user_id,
                        work,
                        livrarr_domain::identity::IdentityMode::Background,
                        livrarr_domain::identity::ConflictSource::ManualRetry,
                    )
                    .await
                    {
                        tracing::warn!(
                            work_id = work.id,
                            "retry-incomplete identity settle failed: {e}"
                        );
                    }
                }
            }

            // Re-enrich through the one road (refresh -> run_unified ->
            // materialize). A refresh error never blocks the rest of the sweep.
            if self.refresh(user_id, work.id).await.is_ok() {
                if let Ok(after) = self.db.get_work(user_id, work.id).await {
                    let still_incomplete = matches!(
                        after.enrichment_status,
                        EnrichmentStatus::Failed | EnrichmentStatus::Unenriched
                    ) || after.identity_status == IdentityStatus::Pending;
                    if !still_incomplete {
                        recovered += 1;
                    }
                }
            }
        }

        Ok(RetrySummary {
            total,
            recovered,
            still_incomplete: total - recovered,
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

        // #97 + WCC chunk A: query every provider in parallel and union the
        // results, instead of returning the first that answers. Goodreads joins
        // as a co-equal provider via its WAF-free autocomplete endpoint. Each
        // lookup is timeout-bounded so a slow scrape can't stall the search.
        let provider_timeout = Duration::from_secs(10);
        let (gb, ol, hc, gr) = tokio::join!(
            tokio::time::timeout(provider_timeout, self.lookup_google_books(&term, lang)),
            tokio::time::timeout(provider_timeout, self.lookup_openlibrary(&term, lang)),
            tokio::time::timeout(provider_timeout, self.lookup_hardcover(&term, lang)),
            tokio::time::timeout(provider_timeout, self.lookup_goodreads(&term, lang)),
        );

        // Cap each provider to its top 9 (relevance-ordered), then round-robin in
        // chunks of 3 so the strongest matches from every provider lead. Order is
        // language-aware: English leads with the anchor-id providers (Hardcover,
        // OpenLibrary), then Google Books, then Goodreads (scrape, often blocked)
        // last. Non-English leads with Google Books — the foreign-language
        // metadata provider — then OpenLibrary, Hardcover, Goodreads.
        const PER_PROVIDER: usize = 9;
        let mut lists = if lang == "en" {
            vec![
                take_lookup("Hardcover", &term, hc),
                take_lookup("OpenLibrary", &term, ol),
                take_lookup("GoogleBooks", &term, gb),
                take_lookup("Goodreads", &term, gr),
            ]
        } else {
            vec![
                take_lookup("GoogleBooks", &term, gb),
                take_lookup("OpenLibrary", &term, ol),
                take_lookup("Hardcover", &term, hc),
                take_lookup("Goodreads", &term, gr),
            ]
        };
        for l in &mut lists {
            l.truncate(PER_PROVIDER);
        }
        let merged = interleave_by(lists, 3);

        Ok(dedupe_lookup_results(merged))
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

        // Keep the top 9 (the strongest cross-provider matches after the chunked
        // interleave) untouched; LLM-filter only the lower-ranked tail (item 10+)
        // for relevance to the query, so a genuine match in the head is never
        // dropped — only long-tail noise is pruned.
        const KEEP_HEAD: usize = 9;
        let (filtered, raw_available) = if raw_count > KEEP_HEAD {
            let tail = &raw_results[KEEP_HEAD..];
            match self.llm_filter_search(&term, tail).await {
                Some(keep) if keep.len() < tail.len() => {
                    let mut filtered: Vec<LookupResult> = raw_results[..KEEP_HEAD].to_vec();
                    filtered.extend(keep.into_iter().filter_map(|i| tail.get(i).cloned()));
                    (filtered, true)
                }
                _ => (raw_results.clone(), false),
            }
        } else {
            (raw_results.clone(), false)
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

    async fn eager_match_by_author(
        &self,
        _user_id: UserId,
        queries: Vec<EagerQuery>,
    ) -> Result<Vec<(usize, LookupResult)>, WorkServiceError> {
        // Group files by author (case-insensitive). Manual imports cluster
        // heavily by author, so one author-scoped query per provider serves all
        // of that author's files instead of one search per title.
        let mut groups: HashMap<String, Vec<EagerQuery>> = HashMap::new();
        for q in queries {
            groups
                .entry(q.author.trim().to_lowercase())
                .or_default()
                .push(q);
        }

        let mut out: Vec<(usize, LookupResult)> = Vec::new();
        // Files the author-batch could not confidently match. Each gets a
        // per-file 4-way title+author fallback after the batch pass.
        let mut abstained: Vec<EagerQuery> = Vec::new();

        for group in groups.into_values() {
            let author = group[0].author.trim().to_string();
            if author.is_empty() {
                continue;
            }
            let lang = group
                .iter()
                .find_map(|q| q.language.clone())
                .unwrap_or_else(|| "en".to_string());

            // One author-scoped query per provider, in parallel. Google Books
            // (`inauthor:`) leads on coverage; OpenLibrary (`author:`) adds work
            // anchors. Each is timeout-bounded so a slow provider can't stall the
            // batch; a provider that errors or times out simply abstains. Google
            // Books returns empty without a fetch when unconfigured (no API key),
            // which makes the pass OpenLibrary-only for keyless installs.
            let gb_term = format!("inauthor:\"{author}\"");
            let ol_term = format!("author:\"{author}\"");
            let provider_timeout = Duration::from_secs(8);
            let gb_fut = async {
                let t = Instant::now();
                let r = tokio::time::timeout(
                    provider_timeout,
                    self.lookup_google_books(&gb_term, &lang),
                )
                .await;
                (r, t.elapsed().as_millis() as u64)
            };
            let ol_fut = async {
                let t = Instant::now();
                let r = tokio::time::timeout(
                    provider_timeout,
                    self.lookup_openlibrary(&ol_term, &lang),
                )
                .await;
                (r, t.elapsed().as_millis() as u64)
            };
            let ((gb, gb_ms), (ol, ol_ms)) = tokio::join!(gb_fut, ol_fut);
            tracing::info!(author = %author, gb_ms, ol_ms, "perf eager: provider fetch");

            // Union the author's corpus: Google Books first (coverage/covers),
            // then OpenLibrary (work anchors).
            let mut corpus: Vec<LookupResult> = Vec::new();
            if let Ok(Ok(mut r)) = gb {
                corpus.append(&mut r);
            }
            if let Ok(Ok(mut r)) = ol {
                corpus.append(&mut r);
            }
            if corpus.is_empty() {
                // The whole author corpus is empty (provider error/timeout, or no
                // author-facet hits). Every file in the group falls through to the
                // per-file 4-way fallback.
                abstained.extend(group);
                continue;
            }

            let cand_refs: Vec<(&str, &str)> = corpus
                .iter()
                .map(|c| (c.title.as_str(), c.author_name.as_str()))
                .collect();
            let cand_langs: Vec<Option<&str>> =
                corpus.iter().map(|c| c.language.as_deref()).collect();

            for q in group {
                // The file's *actual* language (None when unknown) drives the HARD
                // language filter on selection — NOT the per-author query `lang`,
                // which defaults to "en" and would otherwise force an unknown file
                // onto English-only candidates.
                let file_lang = q.language.as_deref();

                // ISBN first: a file's embedded ISBN-13 pins the exact edition in
                // the corpus (Google Books carries isbn_13; OpenLibrary does not),
                // beating any title heuristic. Fall back to the strict title+author
                // cascade when there's no ISBN or no ISBN hit in the corpus.
                let chosen = q
                    .isbn
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .and_then(|isbn| {
                        corpus
                            .iter()
                            .position(|c| c.isbn_13.as_deref() == Some(isbn))
                    })
                    .or_else(|| {
                        livrarr_matching::work_dedup::best_candidate_index_lang(
                            &cand_refs,
                            &cand_langs,
                            &q.title,
                            &q.author,
                            file_lang,
                        )
                    });
                match chosen {
                    Some(idx) => out.push((q.id, finalize_eager_pick(idx, &corpus, file_lang))),
                    // No confident author-batch match: defer to the per-file
                    // 4-way title+author fallback.
                    None => abstained.push(q),
                }
            }
        }

        // Per-file fallback for abstained files (#6). The author-scoped batch
        // misses books that ARE findable by title on providers whose author
        // facet is incomplete (e.g. Hardcover returns a title but not an author
        // query). For each abstained file, run the SAME full 4-way discovery the
        // search box uses (`self.lookup`: Google Books + OpenLibrary + Hardcover
        // + Goodreads, parallel, interleaved, deduped) on `"<title> <author>"`,
        // then select with the SAME confident-match guard
        // (`best_candidate_index_lang`: HARD language guard + title/author match)
        // so a wrong book is never auto-picked. A fallback hit receives the same
        // anchor-graft + cover upgrade as a batch hit via `finalize_eager_pick`.
        // Fires only for abstained files (bounded) and runs with bounded
        // concurrency so several abstains don't serialize into many sequential
        // 4-way searches. Goodreads is in the 4-way but only on abstains, so its
        // volume stays low (anti-bot-safe).
        if !abstained.is_empty() {
            const FALLBACK_CONCURRENCY: usize = 4;
            let fallback_hits: Vec<Option<(usize, LookupResult)>> = stream::iter(abstained)
                .map(|q| async move {
                    let file_lang = q.language.as_deref();
                    let term = format!("{} {}", q.title, q.author);
                    let req = LookupRequest {
                        term,
                        lang_override: q.language.clone(),
                    };
                    // A lookup error (e.g. unsupported language) is treated as an
                    // abstain, mirroring how the batch treats a provider failure.
                    let candidates = match self.lookup(req).await {
                        Ok(c) => c,
                        Err(_) => return None,
                    };
                    if candidates.is_empty() {
                        return None;
                    }
                    let cand_refs: Vec<(&str, &str)> = candidates
                        .iter()
                        .map(|c| (c.title.as_str(), c.author_name.as_str()))
                        .collect();
                    let cand_langs: Vec<Option<&str>> =
                        candidates.iter().map(|c| c.language.as_deref()).collect();
                    let chosen = livrarr_matching::work_dedup::best_candidate_index_lang(
                        &cand_refs,
                        &cand_langs,
                        &q.title,
                        &q.author,
                        file_lang,
                    );
                    chosen.map(|idx| (q.id, finalize_eager_pick(idx, &candidates, file_lang)))
                })
                .buffer_unordered(FALLBACK_CONCURRENCY)
                .collect()
                .await;
            out.extend(fallback_hits.into_iter().flatten());
        }

        Ok(out)
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

    fn try_start_bulk_refresh(
        &self,
        user_id: i64,
    ) -> Option<livrarr_domain::services::BulkRefreshGuard> {
        let inserted = {
            // Poison-proof: a panicked peer must not wedge the slot set.
            let mut slots = self
                .bulk_refresh_users
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            slots.insert(user_id)
        };
        inserted.then(|| {
            livrarr_domain::services::BulkRefreshGuard::new(
                self.bulk_refresh_users.clone(),
                user_id,
            )
        })
    }

    async fn converge_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
        threshold: u32,
    ) -> Result<ConvergeOutcome, WorkServiceError> {
        use livrarr_domain::identity::{
            AnchorConfidence, AnchorType, ConflictSource, IdentityMode,
        };
        use livrarr_domain::IdentityStatus;

        // Fresh row (R-10): the job hands us an id; re-read so we settle on truth.
        let work = self.get(user_id, work_id).await?;
        let was_pending = work.identity_status == IdentityStatus::Pending;

        // The anchor slots that are currently NULL on works.*.
        let missing_of = |w: &Work| -> Vec<String> {
            [
                (AnchorType::OL_WORK, w.ol_key.is_none()),
                (AnchorType::GR_WORK, w.gr_key.is_none()),
                (AnchorType::HC_WORK, w.hc_key.is_none()),
                (AnchorType::ISBN_13, w.isbn_13.is_none()),
                (AnchorType::ASIN, w.asin.is_none()),
            ]
            .into_iter()
            .filter(|(_, missing)| *missing)
            .map(|(t, _)| t.to_string())
            .collect()
        };
        let before_missing = missing_of(&work);
        let holds_anchor = before_missing.len() < 5;

        let anchors = self.db.list_anchors(work_id).await.unwrap_or_default();
        let dead_ends = self
            .db
            .list_anchor_dead_ends(work_id)
            .await
            .unwrap_or_default();
        let chaseable = chaseable_anchor_types(&work, &anchors, &dead_ends, threshold);

        // Step 0 — Pending dead-end (M9 / the convergence trap). settle_identity treats
        // a NoCandidates Unresolved as TRANSIENT (ST-002) and keeps the work Pending, so
        // re-settling a hopeless Pending work would fan out to providers every cadence
        // forever. Terminalize to NeedsReview when a Pending work has no identity path:
        // it holds NO hard anchor to resolve from (an anchorless, title-only work is not
        // chased in the background), OR every still-missing anchor is already
        // pending-guessed / at the dead-end threshold (chaseable empty).
        //
        // DIVERGENCE from ir-v2 convergence-orchestration step 0: the IR's
        // `chaseable.is_empty()` (missing-based) does NOT catch an anchorless Pending
        // work (all 5 missing -> chaseable non-empty). The behavioral contract
        // (test_id_completeness converge_work_terminal, "Converge Pending No Chase")
        // requires it to terminalize on the first pass — hence the `!holds_anchor`
        // clause. [Flagged for cross-family review; Codex authored that test.]
        if was_pending && (!holds_anchor || chaseable.is_empty()) {
            self.db.set_needs_review(work_id).await.map_err(|e| {
                WorkServiceError::Validation(format!("convergence set_needs_review failed: {e}"))
            })?;
            return Ok(ConvergeOutcome::Terminal);
        }

        // Step 1 — identity / ID-chasing leg via the one identity road. Settle ONLY when
        // a chaseable missing anchor remains (R-5): a fully-anchored or fully-dead-ended
        // Confirmed work is never fanned out; a Pending work that reached here still
        // holds a chaseable bridge. Background keeps Audnexus eligible; Convergence
        // attributes any raised conflict.
        let mut work = work;
        if !chaseable.is_empty() {
            if let Some(resolver) = self.resolver.as_ref() {
                if let Err(e) = crate::async_resolver::settle_identity(
                    resolver.as_ref(),
                    &self.db,
                    user_id,
                    &work,
                    IdentityMode::Background,
                    ConflictSource::Convergence,
                )
                .await
                {
                    tracing::warn!(work_id, "convergence identity settle failed: {e}");
                }
                work = self.get(user_id, work_id).await?;
            }
        }

        // Step 2 — enrichment leg (Background path — NEVER refresh, RE-005). Runs when
        // identity permits (settled) and enrichment is still incomplete.
        let identity_permits = !matches!(
            work.identity_status,
            IdentityStatus::Pending | IdentityStatus::Conflict | IdentityStatus::NeedsReview
        );
        let enrichment_incomplete = matches!(
            work.enrichment_status,
            EnrichmentStatus::Unenriched | EnrichmentStatus::Failed
        );
        if identity_permits && enrichment_incomplete {
            let _ = self
                .run_unified_enrichment(user_id, &work, None, EnrichmentMode::Background, None)
                .await;
            work = self.get(user_id, work_id).await?;
        }

        // Step 3 — dead-end accounting (R-1/R-2). A harvested anchor clears its counter;
        // a chaseable anchor still missing and unguessed gets +1 (an at-threshold anchor
        // is already excluded from `chaseable`, so it is never re-bumped).
        let still_missing = missing_of(&work);
        let anchors_after = self.db.list_anchors(work_id).await.unwrap_or_default();
        let pending_after: Vec<String> = anchors_after
            .iter()
            .filter(|a| a.confidence == AnchorConfidence::Pending)
            .map(|a| a.anchor_type.as_str().to_string())
            .collect();
        for t in &before_missing {
            if !still_missing.contains(t) {
                let _ = self
                    .db
                    .clear_anchor_dead_end(work_id, AnchorType::new(t))
                    .await;
            }
        }
        for at in &chaseable {
            let key = at.as_str().to_string();
            if still_missing.contains(&key) && !pending_after.contains(&key) {
                let _ = self.db.bump_anchor_attempt(work_id, at.clone()).await;
            }
        }

        // Step 4 — outcome for the job's pacing.
        let outcome = if matches!(
            work.identity_status,
            IdentityStatus::NeedsReview | IdentityStatus::Conflict | IdentityStatus::NotFound
        ) {
            ConvergeOutcome::Terminal
        } else if matches!(
            work.identity_status,
            IdentityStatus::Confirmed | IdentityStatus::Provisional
        ) && matches!(
            work.enrichment_status,
            EnrichmentStatus::Enriched | EnrichmentStatus::Thin
        ) {
            ConvergeOutcome::Completed
        } else {
            ConvergeOutcome::StillIncomplete
        };
        Ok(outcome)
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
    async fn llm_filter_search(&self, query: &str, results: &[LookupResult]) -> Option<Vec<usize>> {
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
            "A user searched a book database for: \"{query}\"\n\n\
             These are lower-ranked results for that query:\n\n\
             {listing}\n\
             Clean up this list:\n\
             1. Remove items not relevant to the query \"{query}\"\n\
             2. Remove non-book items (study guides, journals, blank notebooks, merchandise, board games)\n\
             3. Remove duplicate editions of the same work — keep the one with the best metadata\n\
             4. Remove comic/manga adaptations, movie tie-in editions, and abridged versions\n\
             5. Remove anthologies and compilations unless they are a well-known standalone work\n\
             6. Keep results that are legitimate different works even if titles are similar\n\n\
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
        // Autocomplete is language-agnostic; the real GR language comes from
        // enrichment (detail-page JSON-LD `inLanguage`), not from discovery.
        _lang: &str,
    ) -> Result<Vec<LookupResult>, WorkServiceError> {
        // Discovery uses the WAF-free `/book/auto_complete` JSON endpoint. The
        // HTML `/search` page is AWS-WAF 202-challenged (dead); autocomplete
        // returns structured title/author/cover/rating/id with no LLM. Query the
        // term as-is — adding the author demotes the canonical book (author-in-
        // title substring matches rank study guides / adaptations first).
        let url = format!(
            "https://www.goodreads.com/book/auto_complete?format=json&q={}",
            urlencoding::encode(term)
        );

        let fetch_req = FetchRequest {
            url,
            method: HttpMethod::Get,
            headers: vec![("Accept".into(), "application/json".into())],
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
                tracing::warn!("Goodreads autocomplete fetch failed: {e}");
                return Ok(vec![]);
            }
        };

        // 200 is the only "door open" status; a 202 challenge / 4xx / 5xx are
        // transient blocks for discovery — the other providers carry the search.
        if resp.status != 200 {
            tracing::warn!(
                status = resp.status,
                "Goodreads autocomplete returned non-200"
            );
            return Ok(vec![]);
        }

        let body = String::from_utf8_lossy(&resp.body);
        // A non-array body (WAF interstitial / format change) parses to empty.
        let parsed = livrarr_external_data::goodreads::parse_autocomplete_json(&body);

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
                // Canonical Goodreads work anchor from the structured endpoint,
                // normalized to the bare numeric id (the domain canonical form per
                // normalize_gr_key) so it persists and matches consistently.
                let gr_key = validated_url
                    .as_deref()
                    .and_then(livrarr_external_data::goodreads::extract_gr_key)
                    .and_then(|k| livrarr_domain::normalization::normalize_gr_key(&k));
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
                    source: Some("goodreads".to_string()),
                    source_type: Some("goodreads".to_string()),
                    // Discovery has no language — don't fabricate it from the query
                    // term (#11 / 三体=es). Enrichment supplies the real one.
                    language: None,
                    detail_url: validated_url,
                    rating: r.rating,
                    isbn_13: None,
                    candidate_id: None,
                    hc_key: None,
                    gr_key,
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
                    .map(|c| format!("https://covers.openlibrary.org/b/id/{c}-L.jpg"));

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
                    source: Some("openlibrary".to_string()),
                    source_type: Some("openlibrary".to_string()),
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
                // REQ-011: never stamp the query language onto a result — a
                // payload without one stays language-unknown (#11, GB path).
                let language = vi.language.clone();

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

                let cover_url = doc
                    .pointer("/image/url")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                // The Hardcover work id (same extraction the HC client uses) is a
                // work anchor, so a picked HC result is trusted (zero-network add)
                // instead of falling back to an ISBN re-resolve.
                let hc_key = doc
                    .get("id")
                    .map(|v| v.to_string().trim_matches('"').to_string());

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
                    isbn_13: None,
                    candidate_id: None,
                    hc_key,
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
        + livrarr_db::ProviderRetryStateDb
        + livrarr_db::SeriesDb
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
                // No candidate_id: dedup path re-enriches existing work from network.
                let (status, _) = self
                    .run_unified_enrichment(
                        user_id,
                        &work,
                        source_provider_data.clone(),
                        EnrichmentMode::Background,
                        None,
                    )
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

    /// REQ-010 (#144): the single identity+enrichment decision EVERY add
    /// outcome takes (created, anchor-matched, adopted, deduped, race-loser).
    /// An anchor-less work first runs the add-time identity leg via the one
    /// identity road (`settle_identity`) — the engine resolves the seed,
    /// partitions hard vs fuzzy anchors (REQ-004), and raises the badge itself.
    /// Enrichment then runs only when the identity permits and the work needs it
    /// — an already-Enriched dedup re-add is never re-enriched. `(mode, source)`
    /// are threaded from the originating door (REQ-001/005).
    async fn ensure_identity_and_enrichment(
        &self,
        user_id: UserId,
        work_id: WorkId,
        source_provider_data: Option<SourceProviderData>,
        candidate_id: Option<livrarr_domain::identity::CandidateId>,
        mode: livrarr_domain::identity::IdentityMode,
        source: livrarr_domain::identity::ConflictSource,
    ) -> (EnrichmentStatus, bool) {
        use livrarr_domain::IdentityStatus;

        let mut work = match self.db.get_work(user_id, work_id).await {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(
                    work_id,
                    "ensure_identity_and_enrichment: get_work failed: {e}"
                );
                return (EnrichmentStatus::Failed, false);
            }
        };

        let anchorless = work.ol_key.is_none()
            && work.gr_key.is_none()
            && work.hc_key.is_none()
            && work.isbn_13.is_none()
            && work.asin.is_none();
        if anchorless && work.identity_status != IdentityStatus::Conflict {
            if let Some(resolver) = self.resolver.as_ref() {
                // The add-time identity leg routes through the one identity road
                // (settle_identity): resolve the anchorless seed, hard/fuzzy
                // split (REQ-004), monotonic badge raise. (mode, source) come
                // from the door.
                match crate::async_resolver::settle_identity(
                    resolver.as_ref(),
                    &self.db,
                    user_id,
                    &work,
                    mode,
                    source,
                )
                .await
                {
                    Ok(_) => {
                        if let Ok(w) = self.db.get_work(user_id, work.id).await {
                            work = w;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(work_id, "add-time identity settle failed: {e}");
                    }
                }
            } else {
                tracing::warn!(
                    work_id,
                    "no resolver composed — skipping add-time identity leg"
                );
            }
        }

        // Identity gate (unchanged shape): a held identity does not enrich
        // here — the fan-out waits for identity convergence.
        if matches!(
            work.identity_status,
            IdentityStatus::Pending | IdentityStatus::Conflict | IdentityStatus::NeedsReview
        ) {
            return (work.enrichment_status, false);
        }

        // Needs-enrichment gate: Unenriched/Failed or fresh source data; an
        // already-Enriched dedup re-add returns untouched.
        let needs = matches!(
            work.enrichment_status,
            EnrichmentStatus::Unenriched | EnrichmentStatus::Failed
        ) || source_provider_data.is_some();
        if !needs {
            return (work.enrichment_status, false);
        }

        self.run_unified_enrichment(
            user_id,
            &work,
            source_provider_data,
            EnrichmentMode::Background,
            candidate_id,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_race_loser(
        &self,
        user_id: UserId,
        work: Work,
        author_created: bool,
        author_id: Option<i64>,
        source_provider_data: Option<SourceProviderData>,
        mode: livrarr_domain::identity::IdentityMode,
        source: livrarr_domain::identity::ConflictSource,
    ) -> Result<AddWorkResult, WorkServiceError> {
        let (enrichment_status, _identity_not_found) = self
            .ensure_identity_and_enrichment(
                user_id,
                work.id,
                source_provider_data,
                None,
                mode,
                source,
            )
            .await;
        let work = self.db.get_work(user_id, work.id).await.unwrap_or(work);
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

    #[allow(clippy::too_many_arguments)]
    async fn finish_created_work(
        &self,
        user_id: UserId,
        work: Work,
        author_created: bool,
        author_id: Option<i64>,
        source_provider_data: Option<SourceProviderData>,
        derived_identity: livrarr_domain::IdentityStatus,
        candidate_id: Option<&livrarr_domain::identity::CandidateId>,
        mode: livrarr_domain::identity::IdentityMode,
        source: livrarr_domain::identity::ConflictSource,
    ) -> Result<AddWorkResult, WorkServiceError> {
        use livrarr_domain::IdentityStatus;

        // Persist the identity-confidence badge derived at resolution time
        // (REQ-014/D-013) — independent of enrichment, written once at create.
        self.db
            .set_identity_status(user_id, work.id, derived_identity)
            .await
            .map_err(WorkServiceError::Db)?;

        // Series reconcile (REQ-001): a metadata-provided series_name gets a
        // series row (stub if absent) and the FK link. Worker-created works
        // arrive already linked (series_id set) — reconcile no-ops on them.
        // Warn-only by design: a failed link here self-heals at the next
        // startup back-fill (REQ-002 targets series_id-NULL works), and a
        // successful add must not fail over series bookkeeping.
        if work.series_id.is_none() && work.series_name.is_some() {
            if let Err(e) = crate::series_link::reconcile_work_series(
                &self.db,
                &work,
                crate::series_link::SeriesLinkOrigin::System,
            )
            .await
            {
                tracing::warn!(work_id = work.id, error = %e, "series reconcile at create failed");
            }
        }

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
            // A user-picked cover (cover_manual, set from the selected search
            // result) is locked at User trust so enrichment never overrides it
            // (resolve_cover bails on User) and update_cover_metadata keeps the
            // cover_manual flag set. Without this, the phase-1 write would assign
            // Validated trust and reset cover_manual to false, letting background
            // enrichment replace the user's chosen cover.
            let trust = if work.cover_manual && work.cover_url.is_some() {
                livrarr_domain::CoverTrust::User
            } else {
                let is_fallback = phase1_mtime.is_some() && work.cover_url.is_none();
                crate::cover_resolution::phase1_trust(is_user_initiated, is_fallback)
            };
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

        let (enrichment_status, identity_not_found) = self
            .ensure_identity_and_enrichment(
                user_id,
                work.id,
                source_provider_data,
                candidate_id.cloned(),
                mode,
                source,
            )
            .await;

        // Seam-2 (REQ-002/D-013): enrichment SIGNALS that it could not verify the
        // work's identity — the LLM rejected every provider payload as not-this-book.
        // The caller — not enrichment — writes the identity badge (the one-way
        // identity←enrichment seam; no EstablishedIdentity contract yet). This is an
        // identity-not-found, distinct from an open anchor `Conflict`.
        if identity_not_found {
            self.db
                .set_identity_status(user_id, work.id, IdentityStatus::NotFound)
                .await
                .map_err(WorkServiceError::Db)?;
        }

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
    D: WorkDb
        + LibraryItemDb
        + ProvenanceDb
        + EnrichmentRetryDb
        + livrarr_db::SeriesDb
        + livrarr_db::ProviderRetryStateDb
        + livrarr_domain::services::WorkIdentityRepository
        + Send
        + Sync,
    E: EnrichmentWorkflow + Send + Sync,
    H: HttpFetcher + Clone + Send + Sync + 'static,
    L: LlmCaller + Send + Sync,
    M: crate::MergeEngine + Send + Sync,
    T: livrarr_domain::services::TagService + Send + Sync,
{
    /// Run the full enrichment pipeline synchronously (REQ-001/012).
    ///
    /// Steps:
    ///   1. Inject source provider data (if present) via `enrichment.inject_source_data`
    ///   2. Dispatch to providers via `enrichment.enrich_work` (candidate_id → cache reuse)
    ///   3. Reload work from DB (post-merge state)
    ///   4. Materialize: cover download + tag write, change-gated (REQ-012)
    ///
    /// Returns `(enrichment_status, identity_not_found)`. Never returns `Err` — all
    /// failures are absorbed, producing `Failed` status when enrichment itself fails
    /// and otherwise continuing past materialize errors (non-fatal, warned).
    async fn run_unified_enrichment(
        &self,
        user_id: UserId,
        work: &Work,
        source_provider_data: Option<livrarr_domain::services::SourceProviderData>,
        mode: EnrichmentMode,
        candidate_id: Option<livrarr_domain::identity::CandidateId>,
    ) -> (EnrichmentStatus, bool) {
        let work_id = work.id;

        // REQ-008 parity at the add door: an anchor-poor work starves the
        // scatter — every provider skips on "no anchor" and the status lands
        // Failed with zero network (e.g. a GR-link add carries only gr_key,
        // which no enrich provider consumes). Run the same identity
        // anchor-completion the refresh door runs, so fresh anchors are in
        // the DB before the scatter reads it. One-shot per add — the
        // refresh door keeps the terminal-outcome bookkeeping for its loop.
        if work.ol_key.is_none()
            && work.isbn_13.is_none()
            && work.asin.is_none()
            && work.hc_key.is_none()
        {
            if let Some(resolver) = self.resolver.as_ref() {
                // Same identity anchor-completion the refresh door runs, via the
                // one identity road (settle_identity). Background mode: this
                // fires mid-enrichment for an anchor-poor work (e.g. a GR-only
                // add) so fresh anchors land in the DB before the scatter reads.
                if let Err(e) = crate::async_resolver::settle_identity(
                    resolver.as_ref(),
                    &self.db,
                    user_id,
                    work,
                    livrarr_domain::identity::IdentityMode::Background,
                    livrarr_domain::identity::ConflictSource::Refresh,
                )
                .await
                {
                    tracing::warn!(work_id, "add-door anchor completion failed: {e}");
                }
            }
        }

        // Step 1: Inject source provider data (Readarr import etc.)
        if let Some(src) = source_provider_data {
            self.enrichment
                .inject_source_data(user_id, work_id, src)
                .await;
        }

        // Step 2: Provider dispatch — scatter-gather enrichment. `candidate_id`
        // enables cache reuse (zero network) when the user picked a specific
        // candidate during discovery (AC-001).
        let enrich_result = match self
            .enrichment
            .enrich_work(user_id, work_id, mode, candidate_id)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(work_id, "run_unified_enrichment: enrich_work failed: {e}");
                return (EnrichmentStatus::Failed, false);
            }
        };

        // Step 3: After enrichment, reload work from DB (reflects merged state).
        let post_enrich_work = match self.db.get_work(user_id, work_id).await {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(work_id, "run_unified_enrichment: get_work failed: {e}");
                return (EnrichmentStatus::Failed, false);
            }
        };

        // Series reconcile (REQ-001, system origin): an enriched series_name
        // gets its stub/link; a work already linked to a GR-backed series is
        // left alone (string-only — system writes never displace GR-grounded
        // assignment, AC-021).
        if let Err(e) = crate::series_link::reconcile_work_series(
            &self.db,
            &post_enrich_work,
            crate::series_link::SeriesLinkOrigin::System,
        )
        .await
        {
            tracing::warn!(work_id, error = %e, "series reconcile after enrichment failed");
        }

        let final_status = enrich_result.enrichment_status;
        let identity_not_found = enrich_result.identity_not_found;

        // Step 4: Materialize — change-gated cover download + tag write (REQ-012).
        // Non-fatal: a materialize error is warned and enrichment still returns
        // the merged status (REQ-013 / P-G: partial beats wrong beats empty).
        let covers_dir = self.data_dir.join("covers").join(user_id.to_string());
        let items = self
            .db
            .list_taggable_items_by_work(user_id, work_id)
            .await
            .unwrap_or_default();
        let file_paths: Vec<std::path::PathBuf> = items
            .iter()
            .map(|i| std::path::PathBuf::from(&i.path))
            .collect();

        let mat_req = livrarr_domain::services::MaterializeRequest {
            work_id,
            changed: enrich_result.changed,
            tag_fields_changed: enrich_result.changed,
            ebook_cover: livrarr_domain::services::CoverSlotState {
                chosen_new_url: enrich_result
                    .cover_resolution
                    .as_ref()
                    .map(|r| r.url.clone()),
                current_url: post_enrich_work.cover_url.clone(),
                current_path: None,
                user_locked: post_enrich_work.cover_trust == livrarr_domain::CoverTrust::User,
            },
            audiobook_cover: livrarr_domain::services::CoverSlotState {
                chosen_new_url: enrich_result
                    .audiobook_cover_resolution
                    .as_ref()
                    .map(|r| r.url.clone()),
                current_url: post_enrich_work.audiobook_cover_url.clone(),
                current_path: None,
                user_locked: post_enrich_work.audiobook_cover_trust
                    == livrarr_domain::CoverTrust::User,
            },
            file_paths,
            tags: livrarr_domain::services::MaterializeTags {
                title: post_enrich_work.title.clone(),
                subtitle: post_enrich_work.subtitle.clone(),
                author: post_enrich_work.author_name.clone(),
                narrator: post_enrich_work.narrator.clone(),
                year: post_enrich_work.year,
                genre: post_enrich_work.genres.clone(),
                description: post_enrich_work.description.clone(),
                publisher: post_enrich_work.publisher.clone(),
                isbn: post_enrich_work.isbn_13.clone(),
                language: post_enrich_work.language.clone(),
                series_name: post_enrich_work.series_name.clone(),
                series_position: post_enrich_work.series_position,
            },
            covers_dir,
        };

        let materialize =
            livrarr_materialize::LiveMaterializeService::new(Arc::new(self.http.clone()));
        let mat_outcome = {
            let _mat_span = livrarr_domain::perf::StageTimer::start("materialize", work_id);
            livrarr_domain::services::MaterializeService::materialize(&materialize, mat_req).await
        };
        match mat_outcome {
            Ok(outcome) => {
                // REQ-017: persist freshly decoded ebook-cover dimensions via
                // the existing writer. (The audiobook slot has no dims writer
                // today — its SavedCover is computed but not yet persisted.)
                if let Some(saved) = outcome.saved_cover.as_ref() {
                    if let Err(e) = self
                        .db
                        .update_cover_dimensions(user_id, work_id, saved.width, saved.height)
                        .await
                    {
                        tracing::warn!(work_id, "cover dimension persist failed: {e}");
                    }
                } else if post_enrich_work.cover_width == 0 && post_enrich_work.cover_url.is_some()
                {
                    // REQ-017 trust-independent backfill: nothing was saved this
                    // pass (incl. user-locked covers) but the stored file exists
                    // and the work has no dims — decode the stored image once.
                    let path = self
                        .data_dir
                        .join("covers")
                        .join(user_id.to_string())
                        .join(format!("{work_id}.jpg"));
                    if let Ok(bytes) = tokio::fs::read(&path).await {
                        let dims = tokio::task::spawn_blocking(move || {
                            image::load_from_memory(&bytes)
                                .map(|img| (img.width() as i32, img.height() as i32))
                                .ok()
                        })
                        .await
                        .ok()
                        .flatten();
                        if let Some((w, h)) = dims {
                            if let Err(e) = self
                                .db
                                .update_cover_dimensions(user_id, work_id, w, h)
                                .await
                            {
                                tracing::warn!(work_id, "cover dimension backfill failed: {e}");
                            }
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(work_id, "run_unified_enrichment: materialize failed: {e}");
            }
        }

        (final_status, identity_not_found)
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

/// Collapse duplicate works merged from multiple discovery providers. Prefers an
/// ISBN-13 match; otherwise keys on normalized title + author. First occurrence
/// wins, so provider order (Google Books, OpenLibrary, Hardcover) breaks ties.
fn dedupe_lookup_results(results: Vec<LookupResult>) -> Vec<LookupResult> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(results.len());
    for r in results {
        let key = match r
            .isbn_13
            .as_deref()
            .and_then(livrarr_domain::normalization::normalize_isbn13)
        {
            Some(isbn) => format!("isbn:{isbn}"),
            None => format!(
                "ta:{}|{}",
                r.title.trim().to_lowercase(),
                r.author_name.trim().to_lowercase()
            ),
        };
        if seen.insert(key) {
            out.push(r);
        }
    }
    out
}

/// Rank a cover by the quality of its hosting source, inferred from the URL
/// host. Higher is better. Google Books, Hardcover, and Amazon serve
/// full-resolution covers; OpenLibrary's search path serves small images. An
/// empty/unrecognized URL ranks lowest so a real cover always wins.
fn cover_source_rank(url: &str) -> u8 {
    let u = url.to_ascii_lowercase();
    if u.is_empty() {
        return 0;
    }
    if u.contains("books.google")
        || u.contains("books.googleusercontent")
        || u.contains("googleusercontent")
        || u.contains("hardcover.app")
        || u.contains("assets.hardcover")
        || u.contains("images-amazon")
        || u.contains("media-amazon")
        || u.contains("ssl-images-amazon")
    {
        3
    } else if u.contains("covers.openlibrary.org") {
        2
    } else {
        // A non-empty cover from an unrecognized host still beats OpenLibrary's
        // small search thumbnails but ranks below the known high-res sources.
        1
    }
}

/// Finalize a confident eager-match pick: take the selected candidate from a
/// candidate corpus and apply the two consistency upgrades that every eager hit
/// receives — an anchor-graft (so an ISBN/Google-Books-only pick gains a work
/// anchor and can be created Confirmed) and a best-source cover upgrade. Both
/// upgrades enforce the same HARD language guard (`file_lang`, when known): an
/// anchor or cover is never borrowed across languages. Shared by the
/// author-batch pass and the per-file 4-way fallback so both treat a hit
/// identically.
fn finalize_eager_pick(
    idx: usize,
    corpus: &[LookupResult],
    file_lang: Option<&str>,
) -> LookupResult {
    let mut result = corpus[idx].clone();
    // The pick is often a Google Books / ISBN hit, which carries a cover + ISBN
    // but NO work anchor. Graft an anchor from a same-title candidate in the
    // corpus so the work can be created Confirmed (and enrich directly) rather
    // than landing ISBN-only and relying on background convergence.
    let has_anchor = result.ol_key.is_some() || result.gr_key.is_some() || result.hc_key.is_some();
    if !has_anchor {
        let norm = livrarr_matching::work_dedup::normalize_title_for_match(&result.title);
        // HARD language guard (#8): when the file's language is known, only graft
        // an anchor from a same-language candidate — never lend a
        // different-language work's anchor.
        let want_lang = file_lang.and_then(livrarr_domain::normalization::normalize_language);
        if let Some(anchored) = corpus.iter().find(|c| {
            (c.ol_key.is_some() || c.gr_key.is_some() || c.hc_key.is_some())
                && livrarr_matching::work_dedup::normalize_title_for_match(&c.title) == norm
                && livrarr_matching::work_dedup::authors_match(&c.author_name, &result.author_name)
                && match want_lang {
                    Some(ref want) => {
                        c.language
                            .as_deref()
                            .and_then(livrarr_domain::normalization::normalize_language)
                            == Some(want.clone())
                    }
                    None => true,
                }
        }) {
            result.ol_key = anchored.ol_key.clone();
            result.author_ol_key = anchored.author_ol_key.clone();
            if result.gr_key.is_none() {
                result.gr_key = anchored.gr_key.clone();
            }
            if result.hc_key.is_none() {
                result.hc_key = anchored.hc_key.clone();
            }
        }
    }
    // Cover-quality upgrade: the matched work/edition stays as selected, but its
    // cover is replaced with the best-source cover among same-work corpus
    // candidates (e.g. a Google Books full-res image instead of an OpenLibrary
    // `-M` thumbnail). The same language guard as the anchor-graft applies so a
    // cover is never borrowed across languages.
    if let Some(better) = best_same_work_cover(&result, corpus, file_lang) {
        result.cover_url = Some(better);
    }
    result
}

/// Among `corpus` candidates that represent the SAME work as `selected`, return
/// the best-quality cover URL by source rank. "Same work" = matching normalized
/// title + author; when `want_lang` is set (the file's known language) only
/// same-language candidates are considered, so a cover is never borrowed across
/// languages. Returns `None` when no same-work candidate has a cover that
/// outranks the selected candidate's own cover (stable: ties keep the original).
fn best_same_work_cover(
    selected: &LookupResult,
    corpus: &[LookupResult],
    want_lang: Option<&str>,
) -> Option<String> {
    let norm = livrarr_matching::work_dedup::normalize_title_for_match(&selected.title);
    let want = want_lang.and_then(livrarr_domain::normalization::normalize_language);
    let mut best_url: Option<&str> = selected.cover_url.as_deref().filter(|u| !u.is_empty());
    let mut best_rank = best_url.map(cover_source_rank).unwrap_or(0);

    for c in corpus {
        let url = match c.cover_url.as_deref().filter(|u| !u.is_empty()) {
            Some(u) => u,
            None => continue,
        };
        if livrarr_matching::work_dedup::normalize_title_for_match(&c.title) != norm {
            continue;
        }
        if !livrarr_matching::work_dedup::authors_match(&c.author_name, &selected.author_name) {
            continue;
        }
        // HARD language guard: when the file's language is known, only consider
        // same-language candidates for the cover upgrade.
        if let Some(ref want) = want {
            let cand = c
                .language
                .as_deref()
                .and_then(livrarr_domain::normalization::normalize_language);
            if cand.as_ref() != Some(want) {
                continue;
            }
        }
        let rank = cover_source_rank(url);
        if rank > best_rank {
            best_rank = rank;
            best_url = Some(url);
        }
    }

    match best_url {
        Some(u) if Some(u) != selected.cover_url.as_deref() => Some(u.to_string()),
        _ => None,
    }
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

#[cfg(test)]
mod discovery_tests {
    use super::*;

    fn lr(title: &str, author: &str, isbn: Option<&str>) -> LookupResult {
        LookupResult {
            ol_key: None,
            title: title.into(),
            author_name: author.into(),
            author_ol_key: None,
            year: None,
            cover_url: None,
            description: None,
            series_name: None,
            series_position: None,
            source: None,
            source_type: None,
            language: None,
            detail_url: None,
            rating: None,
            isbn_13: isbn.map(|s| s.into()),
            candidate_id: None,
            hc_key: None,
            gr_key: None,
            asin: None,
        }
    }

    #[test]
    fn dedupe_keeps_distinct_works_from_all_providers() {
        // A Hardcover-only book must survive a merge where Google Books already
        // returned results (the #97 regression); duplicates collapse to one.
        let merged = dedupe_lookup_results(vec![
            lr("Google Result", "Author A", Some("9780000000001")),
            lr("Hardcover Only", "Author B", Some("9780000000002")),
            lr("Google Result", "Author A", Some("9780000000001")), // dup by isbn
            lr("No ISBN Book", "Author C", None),
            lr("No ISBN Book", "Author C", None), // dup by title+author
        ]);
        let titles: Vec<&str> = merged.iter().map(|r| r.title.as_str()).collect();
        assert!(
            titles.contains(&"Hardcover Only"),
            "HC-only book was dropped"
        );
        assert_eq!(merged.len(), 3, "expected 3 distinct works after dedupe");
    }

    #[test]
    fn interleave_round_robins_in_chunks() {
        // chunk=2: first 2 of A, first 2 of B, then A's remainder — so each
        // provider's strongest hits lead and quality degrades evenly.
        let a = vec![
            lr("A0", "x", None),
            lr("A1", "x", None),
            lr("A2", "x", None),
        ];
        let b = vec![lr("B0", "y", None), lr("B1", "y", None)];
        let out = interleave_by(vec![a, b], 2);
        let titles: Vec<&str> = out.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, vec!["A0", "A1", "B0", "B1", "A2"]);
    }

    #[test]
    fn interleave_handles_uneven_and_empty_lists() {
        let a = vec![lr("A0", "x", None)];
        let empty: Vec<LookupResult> = vec![];
        let c = vec![lr("C0", "z", None), lr("C1", "z", None)];
        let out = interleave_by(vec![a, empty, c], 3);
        let titles: Vec<&str> = out.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, vec!["A0", "C0", "C1"]);
    }

    #[test]
    fn take_lookup_passes_ok_and_swallows_err() {
        let ok: Result<Result<Vec<LookupResult>, String>, tokio::time::error::Elapsed> =
            Ok(Ok(vec![lr("Hit", "a", None)]));
        assert_eq!(take_lookup("P", "t", ok).len(), 1);

        // A provider error degrades to an empty contribution, never failing the
        // whole search (the timeout arm behaves identically).
        let err: Result<Result<Vec<LookupResult>, String>, tokio::time::error::Elapsed> =
            Ok(Err("provider boom".to_string()));
        assert!(take_lookup("P", "t", err).is_empty());
    }

    fn lr_cover(
        title: &str,
        author: &str,
        lang: Option<&str>,
        cover: Option<&str>,
    ) -> LookupResult {
        LookupResult {
            language: lang.map(|s| s.into()),
            cover_url: cover.map(|s| s.into()),
            ..lr(title, author, None)
        }
    }

    #[test]
    fn cover_rank_prefers_high_res_sources_over_openlibrary() {
        let gb = "https://books.google.com/books/content?id=abc&img=1";
        let ol = "https://covers.openlibrary.org/b/id/123-L.jpg";
        let amazon = "https://images-amazon.com/images/I/x.jpg";
        let hc = "https://assets.hardcover.app/cover.jpg";
        assert!(cover_source_rank(gb) > cover_source_rank(ol));
        assert!(cover_source_rank(amazon) > cover_source_rank(ol));
        assert!(cover_source_rank(hc) > cover_source_rank(ol));
        assert!(cover_source_rank(ol) > cover_source_rank(""));
    }

    #[test]
    fn cover_upgrade_picks_google_over_openlibrary_for_same_work() {
        let selected = lr_cover(
            "The Hobbit",
            "Tolkien",
            None,
            Some("https://covers.openlibrary.org/b/id/123-M.jpg"),
        );
        let corpus = vec![
            selected.clone(),
            lr_cover(
                "The Hobbit",
                "Tolkien",
                None,
                Some("https://books.google.com/books/content?id=hobbit"),
            ),
        ];
        let better = best_same_work_cover(&selected, &corpus, None);
        assert_eq!(
            better.as_deref(),
            Some("https://books.google.com/books/content?id=hobbit")
        );
    }

    #[test]
    fn cover_upgrade_keeps_openlibrary_when_only_source() {
        let selected = lr_cover(
            "The Hobbit",
            "Tolkien",
            None,
            Some("https://covers.openlibrary.org/b/id/123-L.jpg"),
        );
        let corpus = vec![selected.clone()];
        // No higher-ranked same-work cover exists, so no upgrade is returned.
        assert_eq!(best_same_work_cover(&selected, &corpus, None), None);
    }

    #[test]
    fn cover_upgrade_does_not_borrow_other_language_cover() {
        // German pick; an English same-title edition has a Google cover, but the
        // known file language is German, so its cover must NOT be borrowed.
        let selected = lr_cover(
            "Der Hobbit",
            "Tolkien",
            Some("de"),
            Some("https://covers.openlibrary.org/b/id/123-M.jpg"),
        );
        let corpus = vec![
            selected.clone(),
            lr_cover(
                "Der Hobbit",
                "Tolkien",
                Some("en"),
                Some("https://books.google.com/books/content?id=eng"),
            ),
        ];
        assert_eq!(best_same_work_cover(&selected, &corpus, Some("de")), None);
    }
}
