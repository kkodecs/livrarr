#![allow(dead_code)]

//! RED behavioral tests for the REQ-014/015/016/019 status two-state split.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use assert_matches::assert_matches;
use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_db::{create_test_db, CreateUserDbRequest, ProviderRetryStateDb, UserDb, WorkDb};
use livrarr_domain::identity::{
    CapturedIdentity, ConflictSource, IdentityConflictKind, IdentityMethod, IdentityState,
    IncomingConflictPayload, NewIdentityConflict, PendingReason, WorkCandidate, WorkSeedFields,
};
use livrarr_domain::services::{
    EnrichmentMode as WorkflowMode, EnrichmentResult as WorkflowResult, EnrichmentWorkflow,
    EnrichmentWorkflowError, SourceProviderData, WorkIdentityRepository, WorkService,
};
use livrarr_domain::{
    EnrichmentStatus, IdentityStatus, MetadataProvider, OutcomeClass, UserId, UserRole, Work,
    WorkId,
};
use livrarr_external_data::{NormalizedWorkDetail, ProviderOutcome};
use livrarr_metadata::enrichment_workflow_service::EnrichmentWorkflowImpl;
use livrarr_metadata::work_service::WorkServiceImpl;
use livrarr_metadata::{
    CircuitState, DefaultMergeEngine, EnrichmentContext, EnrichmentMode, EnrichmentServiceImpl,
    MergeEngine, MergeInput, MergeOutput, PriorityModel, ProviderQueue, ProviderQueueError,
    ReconstructedOutcome, ScatterGatherResult,
};

const MERGE_USER_ID: UserId = 7;
const MERGE_WORK_ID: WorkId = 41;

fn default_priority_model() -> PriorityModel {
    PriorityModel {
        content: vec![MetadataProvider::Hardcover, MetadataProvider::OpenLibrary],
        description: vec![MetadataProvider::Hardcover, MetadataProvider::OpenLibrary],
        cover: vec![MetadataProvider::Hardcover, MetadataProvider::OpenLibrary],
        audio: vec![MetadataProvider::Audnexus],
    }
}

fn make_engine() -> DefaultMergeEngine {
    DefaultMergeEngine::new(default_priority_model())
}

async fn merge(engine: &impl MergeEngine, input: MergeInput) -> MergeOutput {
    engine.merge(input).await.expect("merge should succeed")
}

fn work_with(subtitle: Option<&str>, description: Option<&str>, cover_url: Option<&str>) -> Work {
    Work {
        id: MERGE_WORK_ID,
        user_id: MERGE_USER_ID,
        title: "Current Title".to_string(),
        author_name: "Current Author".to_string(),
        subtitle: subtitle.map(str::to_owned),
        description: description.map(str::to_owned),
        cover_url: cover_url.map(str::to_owned),
        ..Default::default()
    }
}

fn empty_detail() -> NormalizedWorkDetail {
    NormalizedWorkDetail {
        title: None,
        subtitle: None,
        original_title: None,
        author_name: None,
        description: None,
        year: None,
        series_name: None,
        series_position: None,
        genres: None,
        language: None,
        page_count: None,
        duration_seconds: None,
        publisher: None,
        publish_date: None,
        hc_key: None,
        gr_key: None,
        ol_key: None,
        isbn_13: None,
        asin: None,
        narrator: None,
        narration_type: None,
        abridged: None,
        rating: None,
        rating_count: None,
        cover_url: None,
        additional_isbns: Vec::new(),
        additional_asins: Vec::new(),
    }
}

fn success(payload: NormalizedWorkDetail) -> ReconstructedOutcome {
    ReconstructedOutcome {
        class: OutcomeClass::Success,
        payload: Some(payload),
    }
}

fn merge_input(current_work: Work, detail: NormalizedWorkDetail) -> MergeInput {
    MergeInput {
        current_work,
        current_provenance: vec![],
        provider_results: HashMap::from([(MetadataProvider::Hardcover, success(detail))]),
        mode: EnrichmentMode::Background,
        priority_model: default_priority_model(),
    }
}

async fn setup_user() -> (livrarr_db::sqlite::SqliteDb, UserId) {
    let db = create_test_db().await;
    let user = db
        .create_user(CreateUserDbRequest {
            username: "status-two-state-user".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::Admin,
            api_key_hash: "api".to_string(),
        })
        .await
        .expect("test user should be created");
    (db, user.id)
}

fn test_data_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("livrarr-status-two-state-{}", std::process::id()))
}

fn captured_identity(
    title: &str,
    author_name: &str,
    ol_key: Option<&str>,
    isbn_13: Option<&str>,
    asin: Option<&str>,
) -> CapturedIdentity {
    CapturedIdentity {
        ol_key: ol_key.map(str::to_owned),
        gr_key: None,
        hc_key: None,
        isbn_13: isbn_13.map(str::to_owned),
        asin: asin.map(str::to_owned),
        title: title.to_string(),
        author_name: author_name.to_string(),
        language: Some("en".to_string()),
    }
}

fn candidate_with_identity(
    title: &str,
    author_name: &str,
    identity: IdentityState,
) -> WorkCandidate {
    WorkCandidate {
        fields: WorkSeedFields {
            title: title.to_string(),
            author_name: author_name.to_string(),
            language: "en".to_string(),
            author_ol_key: None,
            year: None,
            cover_url: None,
            detail_url: None,
            description: None,
            series_name: None,
            series_position: None,
        },
        identity,
        candidate_id: None,
        source_provider_data: None,
        file_path: None,
        delete_existing_after_import: false,
        series_id: None,
        monitor_ebook: None,
        monitor_audiobook: None,
        provenance_setter: None,
        import_id: None,
        cover_manual: false,
    }
}

fn pending_candidate(title: &str, author_name: &str) -> WorkCandidate {
    candidate_with_identity(
        title,
        author_name,
        IdentityState::Pending {
            reason: PendingReason::NoCandidates,
            seed_anchors: None,
            top_candidates: vec![],
        },
    )
}

fn confirmed_candidate(
    title: &str,
    author_name: &str,
    ol_key: Option<&str>,
    isbn_13: Option<&str>,
    asin: Option<&str>,
) -> WorkCandidate {
    candidate_with_identity(
        title,
        author_name,
        IdentityState::Confirmed {
            anchors: captured_identity(title, author_name, ol_key, isbn_13, asin),
            method: if isbn_13.is_some() || asin.is_some() {
                IdentityMethod::IsbnDirect
            } else {
                IdentityMethod::UserSelected
            },
            score: None,
        },
    )
}

#[derive(Clone)]
struct SpyEnrichmentWorkflow {
    call_count: Arc<AtomicUsize>,
    status: EnrichmentStatus,
}

impl SpyEnrichmentWorkflow {
    fn returning(status: EnrichmentStatus) -> Self {
        Self {
            call_count: Arc::new(AtomicUsize::new(0)),
            status,
        }
    }

    fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

impl EnrichmentWorkflow for SpyEnrichmentWorkflow {
    async fn enrich_work(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _mode: WorkflowMode,
        _candidate_id: Option<livrarr_domain::identity::CandidateId>,
    ) -> Result<WorkflowResult, EnrichmentWorkflowError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        Ok(WorkflowResult {
            enrichment_status: self.status,
            identity_not_found: false,
            changed: false,
            enrichment_source: Some("spy".to_string()),
            work: Work {
                enrichment_status: self.status,
                ..Default::default()
            },
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
        _data: SourceProviderData,
    ) {
    }
}

#[derive(Clone)]
struct TextlessProviderQueue {
    db: livrarr_db::sqlite::SqliteDb,
    user_id: UserId,
}

impl TextlessProviderQueue {
    fn new(db: livrarr_db::sqlite::SqliteDb, user_id: UserId) -> Self {
        Self { db, user_id }
    }
}

impl ProviderQueue for TextlessProviderQueue {
    async fn dispatch_enrichment(
        &self,
        work: &Work,
        _context: EnrichmentContext,
    ) -> Result<ScatterGatherResult, ProviderQueueError> {
        let payload = empty_detail();
        self.db
            .record_terminal_outcome(
                self.user_id,
                work.id,
                MetadataProvider::Hardcover,
                OutcomeClass::Success,
                Some(serde_json::to_string(&payload).expect("payload should serialize")),
            )
            .await
            .expect("retry payload should be persisted");

        Ok(ScatterGatherResult {
            work_id: work.id,
            outcomes: HashMap::from([(
                MetadataProvider::Hardcover,
                ProviderOutcome::Success(Box::new(payload)),
            )]),
            merge_eligible: true,
            deferred: false,
        })
    }

    fn circuit_state(&self, _provider: MetadataProvider) -> CircuitState {
        CircuitState::Closed
    }
}

fn real_textless_workflow(
    db: livrarr_db::sqlite::SqliteDb,
    user_id: UserId,
) -> EnrichmentWorkflowImpl<
    EnrichmentServiceImpl<livrarr_db::sqlite::SqliteDb, TextlessProviderQueue, DefaultMergeEngine>,
    livrarr_db::sqlite::SqliteDb,
> {
    let enrichment = EnrichmentServiceImpl::new(
        Arc::new(db.clone()),
        Arc::new(TextlessProviderQueue::new(db.clone(), user_id)),
        Arc::new(DefaultMergeEngine::new(PriorityModel::english())),
        false,
    );

    EnrichmentWorkflowImpl::new(Arc::new(enrichment), db)
}

// =============================================================================
// GROUP A — classifier is text-only; cover is not consulted (REQ-019).
// =============================================================================

#[tokio::test]
async fn test_group_a_description_without_cover_is_enriched() {
    // REQ-ID: REQ-019 | Contract: MergeEngine::merge | Behavior: description text alone classifies the merge as Enriched (AC-015)
    let engine = make_engine();
    let mut detail = empty_detail();
    detail.description = Some("Provider supplied a meaningful description.".to_string());

    let output = merge(&engine, merge_input(work_with(None, None, None), detail)).await;

    assert_eq!(output.enrichment_status, EnrichmentStatus::Enriched);
}

#[tokio::test]
async fn test_group_a_subtitle_without_description_or_cover_is_enriched() {
    // REQ-ID: REQ-019 | Contract: MergeEngine::merge | Behavior: a non-description meaningful text field (subtitle) alone classifies Enriched (AC-015)
    let engine = make_engine();
    let mut detail = empty_detail();
    detail.subtitle = Some("A Provider Supplied Subtitle".to_string());

    let output = merge(&engine, merge_input(work_with(None, None, None), detail)).await;

    assert_eq!(output.enrichment_status, EnrichmentStatus::Enriched);
}

#[tokio::test]
async fn test_group_a_cover_without_meaningful_text_is_thin() {
    // REQ-ID: REQ-019 | Contract: MergeEngine::merge | Behavior: cover presence alone does not make textless metadata Enriched (AC-015)
    let engine = make_engine();
    let mut detail = empty_detail();
    detail.cover_url = Some("https://example.test/cover.jpg".to_string());

    let output = merge(&engine, merge_input(work_with(None, None, None), detail)).await;

    assert_eq!(output.enrichment_status, EnrichmentStatus::Thin);
}

#[tokio::test]
async fn test_group_a_success_with_no_text_and_no_cover_is_thin_not_failed() {
    // REQ-ID: REQ-014 | Contract: MergeEngine::merge | Behavior: successful textless merge is Thin, not Failed (AC-010)
    let engine = make_engine();

    let output = merge(
        &engine,
        merge_input(work_with(None, None, None), empty_detail()),
    )
    .await;

    assert_eq!(output.enrichment_status, EnrichmentStatus::Thin);
}

// =============================================================================
// GROUP B — Pending identity holds enrichment (REQ-015).
// =============================================================================

#[tokio::test]
async fn test_group_b_pending_identity_does_not_invoke_enrichment_workflow() {
    // REQ-ID: REQ-015 | Contract: WorkService::add | Behavior: Pending identity holds enrichment fan-out (AC-011)
    let (db, user_id) = setup_user().await;
    let spy = SpyEnrichmentWorkflow::returning(EnrichmentStatus::Enriched);
    let svc = WorkServiceImpl::new(db, spy.clone(), StubHttpFetcher::new(), test_data_dir());

    let result = svc
        .add(
            user_id,
            pending_candidate("Fuzzy Pending Book", "Unknown Author"),
        )
        .await
        .expect("pending add should succeed");

    assert_eq!(spy.call_count(), 0);
    assert_matches!(result.work.enrichment_status, EnrichmentStatus::Unenriched);
}

// =============================================================================
// GROUP C — de-facto identity enriches (REQ-016).
// =============================================================================

#[tokio::test]
async fn test_group_c_isbn_bridge_without_work_anchor_is_provisional_identity() {
    // REQ-ID: REQ-016 | Contract: WorkService::add | Behavior: ISBN bridge without work anchor creates Provisional identity (AC-012)
    let (db, user_id) = setup_user().await;
    let spy = SpyEnrichmentWorkflow::returning(EnrichmentStatus::Enriched);
    let svc = WorkServiceImpl::new(db, spy, StubHttpFetcher::new(), test_data_dir());

    let result = svc
        .add(
            user_id,
            confirmed_candidate(
                "ISBN Only Book",
                "Bridge Author",
                None,
                Some("9780765326355"),
                None,
            ),
        )
        .await
        .expect("isbn bridge add should succeed");

    assert_eq!(result.work.identity_status, IdentityStatus::Provisional);
}

#[tokio::test]
async fn test_group_c_provisional_identity_still_invokes_enrichment_workflow() {
    // REQ-ID: REQ-016 | Contract: WorkService::add | Behavior: Provisional ISBN identity is eligible for enrichment (AC-012)
    let (db, user_id) = setup_user().await;
    let spy = SpyEnrichmentWorkflow::returning(EnrichmentStatus::Enriched);
    let svc = WorkServiceImpl::new(db, spy.clone(), StubHttpFetcher::new(), test_data_dir());

    let _result = svc
        .add(
            user_id,
            confirmed_candidate(
                "ISBN Only Enriches",
                "Bridge Author",
                None,
                Some("9780765326355"),
                None,
            ),
        )
        .await
        .expect("isbn bridge add should succeed");

    assert_eq!(spy.call_count(), 1);
}

// =============================================================================
// GROUP D — identity and enrichment are independent dimensions (AC-010).
// =============================================================================

#[tokio::test]
async fn test_group_d_confirmed_identity_with_no_text_is_simultaneously_confirmed_and_thin() {
    // REQ-ID: REQ-014 | Contract: WorkService::add | Behavior: Confirmed identity and Thin enrichment coexist independently (AC-010)
    let (db, user_id) = setup_user().await;
    let workflow = real_textless_workflow(db.clone(), user_id);
    let svc = WorkServiceImpl::new(db, workflow, StubHttpFetcher::new(), test_data_dir());

    let result = svc
        .add(
            user_id,
            confirmed_candidate(
                "Anchored Textless Book",
                "Sparse Author",
                Some("OL123W"),
                None,
                None,
            ),
        )
        .await
        .expect("anchored add should succeed");

    assert_eq!(
        (result.work.identity_status, result.work.enrichment_status),
        (IdentityStatus::Confirmed, EnrichmentStatus::Thin)
    );
}

#[tokio::test]
async fn test_group_d_open_identity_conflict_derives_conflict_identity_status() {
    // REQ-ID: REQ-014/D-013 | Contract: identity_status derivation | Behavior: an open work_identity_conflicts row derives IdentityStatus::Conflict
    let (db, user_id) = setup_user().await;
    let spy = SpyEnrichmentWorkflow::returning(EnrichmentStatus::Enriched);
    let svc = WorkServiceImpl::new(db.clone(), spy, StubHttpFetcher::new(), test_data_dir());

    let result = svc
        .add(
            user_id,
            confirmed_candidate(
                "Conflicted Identity Book",
                "Anchor Author",
                Some("OL123W"),
                None,
                None,
            ),
        )
        .await
        .expect("confirmed add should succeed");

    db.raise_identity_conflict(NewIdentityConflict {
        user_id,
        existing_work_id: result.work.id,
        kind: IdentityConflictKind::IncomingDifferentOlKey,
        incoming: IncomingConflictPayload {
            ol_key: Some("OL999W".to_string()),
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
            title: "Conflicted Identity Book".to_string(),
            author_name: "Anchor Author".to_string(),
            year: None,
            cover_url: None,
            top_candidates: Vec::new(),
        },
        raised_by: ConflictSource::ManualAdd,
        raised_source_path: None,
    })
    .await
    .expect("open identity conflict should be inserted");

    let reloaded = db
        .get_work(user_id, result.work.id)
        .await
        .expect("work should reload");

    assert_eq!(reloaded.identity_status, IdentityStatus::Conflict);
}
