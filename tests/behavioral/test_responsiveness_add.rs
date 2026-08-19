mod common;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use livrarr_behavioral::stubs::{create_test_user, StubEnrichmentWorkflow, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{CreateWorkDbRequest, UpdateWorkEnrichmentDbRequest, WorkDb, WorkDbCreate};
use livrarr_domain::identity::{
    CandidateId, CapturedIdentity, ConflictSource, IdentityMethod, IdentityMode, IdentityState,
    PendingReason, RawHarvest,
};
use livrarr_domain::seed::{seed_add_box, SeedInput, SeedLanguage};
use livrarr_domain::services::{
    EnrichmentMode, EnrichmentResult, EnrichmentWorkflow, EnrichmentWorkflowError, RefreshSurface,
    WorkService,
};
use livrarr_domain::{
    normalize_for_matching, EnrichmentStatus, IdentityStatus, MetadataProvider, OutcomeClass,
    RequestPriority, UserId, Work, WorkId,
};
use livrarr_external_data::transport_cache::TransportCache;
use livrarr_external_data::{
    NormalizedWorkDetail, ProviderClient, ProviderOutcome, StubProviderClient,
};
use livrarr_metadata::english_identity_resolver::{LiveEnglishIdentityResolver, ResolverConfig};
use livrarr_metadata::work_service::WorkServiceImpl;
use tokio::sync::Notify;

type TestWorkService<E = StubEnrichmentWorkflow> = WorkServiceImpl<SqliteDb, E, StubHttpFetcher>;

fn service<E>(db: SqliteDb, workflow: E) -> TestWorkService<E> {
    WorkServiceImpl::new(
        db,
        workflow,
        StubHttpFetcher::new(),
        tempfile::tempdir()
            .expect("test data dir")
            .path()
            .to_path_buf(),
    )
}

fn seed_input(title: &str, author: &str) -> SeedInput {
    SeedInput {
        title: title.to_string(),
        author_name: author.to_string(),
        language: SeedLanguage::resolve(Some("en"), "en"),
        author_ol_key: None,
        year: Some(2024),
        cover_url: None,
        detail_url: None,
        description: None,
        series_name: None,
        series_position: None,
    }
}

fn captured_identity(
    title: &str,
    author: &str,
    ol_key: Option<&str>,
    isbn_13: Option<&str>,
) -> CapturedIdentity {
    CapturedIdentity {
        ol_key: ol_key.map(str::to_string),
        gr_key: None,
        hc_key: None,
        isbn_13: isbn_13.map(str::to_string),
        asin: None,
        title: title.to_string(),
        author_name: author.to_string(),
        language: Some("en".to_string()),
    }
}

fn stub_isbn_resolver() -> LiveEnglishIdentityResolver {
    let ol = StubProviderClient::new(MetadataProvider::OpenLibrary, ProviderOutcome::NotFound);
    let hc = StubProviderClient::new(
        MetadataProvider::Hardcover,
        ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
            hc_key: Some("hc_dune".to_string()),
            isbn_13: Some("9780441013593".to_string()),
            title: Some("Dune".to_string()),
            author_name: Some("Frank Herbert".to_string()),
            language: Some("en".to_string()),
            ..NormalizedWorkDetail::default()
        })),
    );
    let clients = [
        (MetadataProvider::OpenLibrary, ProviderClient::Stub(ol)),
        (MetadataProvider::Hardcover, ProviderClient::Stub(hc)),
    ]
    .into_iter()
    .collect::<HashMap<_, _>>();
    LiveEnglishIdentityResolver {
        clients,
        cache: Arc::new(TransportCache::new(Duration::from_secs(30))),
        config: ResolverConfig {
            gb_key_present: false,
            ..ResolverConfig::default()
        },
    }
}

fn confirmed_candidate(
    title: &str,
    author: &str,
    ol_key: &str,
) -> livrarr_domain::identity::WorkCandidate {
    seed_add_box(
        seed_input(title, author),
        IdentityState::Confirmed {
            anchors: captured_identity(title, author, Some(ol_key), None),
            method: IdentityMethod::UserSelected,
            score: None,
        },
        Some(CandidateId(format!("candidate-{ol_key}"))),
        false,
    )
}

fn bridge_only_candidate(
    title: &str,
    author: &str,
    isbn_13: &str,
) -> livrarr_domain::identity::WorkCandidate {
    seed_add_box(
        seed_input(title, author),
        IdentityState::Pending {
            reason: PendingReason::NoCandidates,
            seed_anchors: Some(captured_identity(title, author, None, Some(isbn_13))),
            top_candidates: vec![],
        },
        Some(CandidateId(format!("candidate-{isbn_13}-{title}"))),
        false,
    )
}

fn work_req(user_id: UserId, title: &str, author: &str) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: author.to_string(),
        normalized_title: normalize_for_matching(title),
        normalized_author: normalize_for_matching(author),
        language: Some("en".to_string()),
        monitor_ebook: true,
        monitor_audiobook: true,
        ..Default::default()
    }
}

async fn seed_isbn_work(
    db: &SqliteDb,
    user_id: UserId,
    title: &str,
    author: &str,
    isbn_13: &str,
) -> Work {
    let (work, created) = db
        .create_work(CreateWorkDbRequest {
            isbn_13: Some(isbn_13.to_string()),
            ..work_req(user_id, title, author)
        })
        .await
        .expect("seed isbn work");
    assert!(created, "fixture should create a fresh work row");
    work
}

#[derive(Clone)]
struct SleepingWorkflow {
    delay: Duration,
    fail: bool,
}

impl SleepingWorkflow {
    fn succeeding(delay: Duration) -> Self {
        Self { delay, fail: false }
    }

    fn failing(delay: Duration) -> Self {
        Self { delay, fail: true }
    }
}

impl EnrichmentWorkflow for SleepingWorkflow {
    async fn enrich_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
        _mode: EnrichmentMode,
        _candidate_id: Option<CandidateId>,
        _priority: RequestPriority,
        _freshness: livrarr_domain::Freshness,
    ) -> Result<EnrichmentResult, EnrichmentWorkflowError> {
        tokio::time::sleep(self.delay).await;

        if self.fail {
            return Err(EnrichmentWorkflowError::Queue(
                "scripted failure".to_string(),
            ));
        }

        Ok(EnrichmentResult {
            enrichment_status: EnrichmentStatus::Enriched,
            enrichment_source: Some("sleeping-test".to_string()),
            work: Work {
                id: work_id,
                user_id,
                enrichment_status: EnrichmentStatus::Enriched,
                ..Work::default()
            },
            merge_deferred: false,
            provider_outcomes: HashMap::<MetadataProvider, OutcomeClass>::new(),
            cover_resolution: None,
            audiobook_cover_resolution: None,
            identity_not_found: false,
            changed: false,
            attempted: true,
            captured_provider_identity: Vec::new(),
            captured_route_proposals: Vec::new(),
            provider_chase_attempted: false,
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
    }
}

#[derive(Clone)]
struct GatedWorkflow {
    entered: Arc<Notify>,
    release: Arc<Notify>,
    fail: bool,
}

impl GatedWorkflow {
    fn succeeding() -> Self {
        Self {
            entered: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
            fail: false,
        }
    }

    fn failing() -> Self {
        Self {
            entered: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
            fail: true,
        }
    }

    fn entered(&self) -> Arc<Notify> {
        Arc::clone(&self.entered)
    }

    fn release(&self) {
        self.release.notify_one();
    }
}

impl EnrichmentWorkflow for GatedWorkflow {
    async fn enrich_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
        _mode: EnrichmentMode,
        _candidate_id: Option<CandidateId>,
        _priority: RequestPriority,
        _freshness: livrarr_domain::Freshness,
    ) -> Result<EnrichmentResult, EnrichmentWorkflowError> {
        self.entered.notify_one();
        self.release.notified().await;

        if self.fail {
            return Err(EnrichmentWorkflowError::Queue(
                "scripted failure".to_string(),
            ));
        }

        Ok(EnrichmentResult {
            enrichment_status: EnrichmentStatus::Enriched,
            enrichment_source: Some("gated-test".to_string()),
            work: Work {
                id: work_id,
                user_id,
                enrichment_status: EnrichmentStatus::Enriched,
                ..Work::default()
            },
            merge_deferred: false,
            provider_outcomes: HashMap::<MetadataProvider, OutcomeClass>::new(),
            cover_resolution: None,
            audiobook_cover_resolution: None,
            identity_not_found: false,
            changed: false,
            attempted: true,
            captured_provider_identity: Vec::new(),
            captured_route_proposals: Vec::new(),
            provider_chase_attempted: false,
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
    }
}

#[derive(Clone)]
struct RecordingPersistingWorkflow {
    db: SqliteDb,
    call_count: Arc<AtomicUsize>,
}

impl RecordingPersistingWorkflow {
    fn succeeding(db: SqliteDb) -> Self {
        Self {
            db,
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

impl EnrichmentWorkflow for RecordingPersistingWorkflow {
    async fn enrich_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
        _mode: EnrichmentMode,
        _candidate_id: Option<CandidateId>,
        _priority: RequestPriority,
        _freshness: livrarr_domain::Freshness,
    ) -> Result<EnrichmentResult, EnrichmentWorkflowError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.db
            .update_work_enrichment(
                user_id,
                work_id,
                UpdateWorkEnrichmentDbRequest {
                    enrichment_status: EnrichmentStatus::Enriched,
                    enrichment_source: Some("recording-test".to_string()),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| EnrichmentWorkflowError::Queue(format!("persist enriched: {e}")))?;

        Ok(EnrichmentResult {
            enrichment_status: EnrichmentStatus::Enriched,
            enrichment_source: Some("recording-test".to_string()),
            work: Work {
                id: work_id,
                user_id,
                enrichment_status: EnrichmentStatus::Enriched,
                ..Work::default()
            },
            merge_deferred: false,
            provider_outcomes: HashMap::<MetadataProvider, OutcomeClass>::new(),
            cover_resolution: None,
            audiobook_cover_resolution: None,
            identity_not_found: false,
            changed: false,
            attempted: true,
            captured_provider_identity: Vec::new(),
            captured_route_proposals: Vec::new(),
            provider_chase_attempted: false,
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
    }
}

/// AC: resolve_identity_local derives identity from local harvest only.
#[tokio::test]
async fn resolve_identity_local_derives_confirmed_pending_and_no_candidates() {
    let db = common::create_test_db().await;
    let svc = service(db, StubEnrichmentWorkflow::succeeding());

    let confirmed = svc
        .resolve_identity_local(RawHarvest {
            ol_key: Some("/works/OL27448W".to_string()),
            isbn: Some("9780441013593".to_string()),
            title: Some("Dune".to_string()),
            author_name: Some("Frank Herbert".to_string()),
            language: Some("en".to_string()),
            user_confirmed: true,
            ..RawHarvest::default()
        })
        .expect("local work-anchor harvest should resolve");
    match confirmed.identity {
        IdentityState::Confirmed { anchors, .. } => {
            assert_eq!(anchors.ol_key.as_deref(), Some("/works/OL27448W"));
            assert_eq!(anchors.isbn_13.as_deref(), Some("9780441013593"));
            assert_eq!(anchors.title, "Dune");
        }
        other => panic!("expected Confirmed identity, got {other:?}"),
    }
    assert!(confirmed.conflict.is_none());
    assert!(confirmed.candidate_id.is_none());

    let isbn_only = svc
        .resolve_identity_local(RawHarvest {
            isbn: Some("9780140328721".to_string()),
            title: Some("Matilda".to_string()),
            author_name: Some("Roald Dahl".to_string()),
            language: Some("en".to_string()),
            user_confirmed: true,
            ..RawHarvest::default()
        })
        .expect("local isbn-only harvest should resolve");
    match isbn_only.identity {
        IdentityState::Pending {
            reason,
            seed_anchors: Some(anchors),
            top_candidates,
        } => {
            assert_eq!(reason, PendingReason::NoCandidates);
            assert_eq!(anchors.isbn_13.as_deref(), Some("9780140328721"));
            assert_eq!(anchors.title, "Matilda");
            assert!(top_candidates.is_empty());
        }
        other => panic!("expected Pending identity with seed anchors, got {other:?}"),
    }
    assert!(isbn_only.conflict.is_none());

    let anchorless = svc
        .resolve_identity_local(RawHarvest {
            title: Some("Totally Anchorless".to_string()),
            author_name: Some("No Identifier".to_string()),
            language: Some("en".to_string()),
            user_confirmed: true,
            ..RawHarvest::default()
        })
        .expect("local anchorless harvest should resolve");
    match anchorless.identity {
        IdentityState::Pending {
            reason,
            seed_anchors: None,
            top_candidates,
        } => {
            assert_eq!(reason, PendingReason::NoCandidates);
            assert!(top_candidates.is_empty());
        }
        other => panic!("expected anchorless Pending identity, got {other:?}"),
    }
    assert!(anchorless.conflict.is_none());
}

/// AC-007/REQ-004: add_fast returns before any provider-bound enrichment work.
#[tokio::test]
async fn add_fast_returns_created_without_enrichment_calls() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let workflow = StubEnrichmentWorkflow::succeeding();
    let svc = service(db, workflow.clone());

    let result = svc
        .add_fast(
            user_id,
            confirmed_candidate("Dune", "Frank Herbert", "/works/OL27448W"),
        )
        .await
        .expect("add_fast should create the confirmed candidate");

    assert!(result.created);
    assert_eq!(
        workflow.call_count(),
        0,
        "add_fast must not call enrichment on the response path"
    );
}

/// AC-012/REQ-007: a repeated work-anchor add is idempotent for the user.
#[tokio::test]
async fn add_fast_is_idempotent_for_same_work_anchor_candidate() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let svc = service(db.clone(), StubEnrichmentWorkflow::succeeding());
    let candidate = confirmed_candidate("Dune", "Frank Herbert", "/works/OL27448W");

    let first = svc
        .add_fast(user_id, candidate.clone())
        .await
        .expect("first add should create");
    let second = svc
        .add_fast(user_id, candidate)
        .await
        .expect("second add should dedup");
    let works = db.list_works(user_id).await.expect("list works");

    assert!(first.created);
    assert!(!second.created);
    assert_eq!(second.work.id, first.work.id);
    assert_eq!(
        works.len(),
        1,
        "only one work row should exist for the user"
    );
}

/// AC-012/design 2.4: bridge matches dedup only when title verdicts agree.
#[tokio::test]
async fn add_fast_dedups_bridge_when_titles_match_and_splits_collision_titles() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let svc = service(db.clone(), StubEnrichmentWorkflow::succeeding());
    let seeded = seed_isbn_work(
        &db,
        user_id,
        "The Left Hand of Darkness",
        "Ursula K. Le Guin",
        "9780441478125",
    )
    .await;

    let same = svc
        .add_fast(
            user_id,
            bridge_only_candidate(
                "The Left Hand of Darkness",
                "Ursula K. Le Guin",
                "9780441478125",
            ),
        )
        .await
        .expect("matching bridge-only candidate should dedup");
    assert!(!same.created);
    assert_eq!(same.work.id, seeded.id);

    let collision = svc
        .add_fast(
            user_id,
            bridge_only_candidate(
                "Completely Unrelated Words",
                "Another Writer",
                "9780441478125",
            ),
        )
        .await
        .expect("different-title bridge collision should create separately");
    let works = db.list_works(user_id).await.expect("list works");

    assert!(collision.created);
    assert_ne!(collision.work.id, seeded.id);
    assert_eq!(collision.work.title, "Completely Unrelated Words");
    assert_eq!(collision.work.isbn_13.as_deref(), Some("9780441478125"));
    assert_eq!(works.len(), 2);
}

/// AC-009/REQ-005: complete_add toggles the in-memory enriching signal.
#[tokio::test]
async fn complete_add_tracks_enriching_until_background_work_finishes() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let workflow = GatedWorkflow::succeeding();
    let entered_notify = workflow.entered();
    let entered = entered_notify.notified();
    let svc = Arc::new(service(db.clone(), workflow.clone()));
    let created = svc
        .add_fast(
            user_id,
            confirmed_candidate("Lifecycle Book", "Progress Author", "/works/OL700W"),
        )
        .await
        .expect("add_fast should create work");
    assert!(created.created);

    let work_id = created.work.id;
    let task_svc = Arc::clone(&svc);
    let handle = tokio::spawn(async move {
        task_svc
            .complete_add(
                user_id,
                work_id,
                None,
                None,
                IdentityMode::Background,
                ConflictSource::ManualAdd,
            )
            .await;
    });

    tokio::time::timeout(Duration::from_secs(1), entered)
        .await
        .expect("enrichment workflow should start");
    assert!(
        svc.is_enriching(user_id, work_id),
        "is_enriching should be true after enrich_work has entered"
    );

    workflow.release();
    handle.await.expect("complete_add task should not panic");
    assert!(
        !svc.is_enriching(user_id, work_id),
        "is_enriching should be false after complete_add finishes"
    );
}

/// AC: complete_add chases bridge-only seed anchors before the enrichment gate.
#[tokio::test]
async fn complete_add_chases_bridge_only_identity_before_enrichment_gate() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let workflow = RecordingPersistingWorkflow::succeeding(db.clone());
    let svc = service(db.clone(), workflow.clone()).with_resolver(Arc::new(stub_isbn_resolver()));
    let created = svc
        .add_fast(
            user_id,
            bridge_only_candidate("Dune", "Frank Herbert", "9780441013593"),
        )
        .await
        .expect("add_fast should create a bridge-only work");
    assert!(created.created);
    assert_eq!(
        created.work.identity_status,
        IdentityStatus::Pending,
        "bridge-only pending seed anchors should start held before complete_add"
    );

    let work_id = created.work.id;
    svc.complete_add(
        user_id,
        work_id,
        None,
        None,
        IdentityMode::Background,
        ConflictSource::ManualAdd,
    )
    .await;

    let persisted = db.get_work(user_id, work_id).await.expect("read work");
    assert_eq!(
        persisted.identity_status,
        IdentityStatus::Confirmed,
        "complete_add should run the same identity chase as refresh for pending works with anchors"
    );
    assert_eq!(persisted.hc_key.as_deref(), Some("hc_dune"));
    assert!(
        workflow.call_count() >= 1,
        "enrichment workflow should run once the identity chase settles the gate"
    );
    assert_eq!(persisted.enrichment_status, EnrichmentStatus::Enriched);
    assert!(
        !svc.is_enriching(user_id, work_id),
        "complete_add should clear the enriching signal after completion"
    );
}

/// AC: manual refresh is visible through the same enriching signal as complete_add.
#[tokio::test]
async fn refresh_tracks_enriching_until_manual_enrichment_finishes() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let workflow = GatedWorkflow::succeeding();
    let entered_notify = workflow.entered();
    let entered = entered_notify.notified();
    let svc = Arc::new(service(db.clone(), workflow.clone()));
    let created = svc
        .add_fast(
            user_id,
            confirmed_candidate("Refresh Signal Book", "Progress Author", "/works/OL703W"),
        )
        .await
        .expect("add_fast should create work");
    assert!(created.created);
    let work_id = created.work.id;

    let task_svc = Arc::clone(&svc);
    let handle = tokio::spawn(async move {
        task_svc
            .refresh(user_id, work_id, RefreshSurface::Interactive)
            .await
            .expect("refresh should complete");
    });

    tokio::time::timeout(Duration::from_secs(1), entered)
        .await
        .expect("refresh enrichment workflow should start");
    assert!(
        svc.is_enriching(user_id, work_id),
        "is_enriching should be true while refresh enrichment is running"
    );

    workflow.release();
    handle.await.expect("refresh task should not panic");
    assert!(
        !svc.is_enriching(user_id, work_id),
        "is_enriching should be false after refresh finishes"
    );
}

/// REQ-008: complete_add absorbs enrichment failure and persists Failed.
#[tokio::test]
async fn complete_add_absorbs_failure_and_marks_work_failed() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let svc = service(
        db.clone(),
        SleepingWorkflow::failing(Duration::from_millis(10)),
    );
    let created = svc
        .add_fast(
            user_id,
            confirmed_candidate("Failure Book", "Progress Author", "/works/OL701W"),
        )
        .await
        .expect("add_fast should create work");
    let work_id = created.work.id;

    svc.complete_add(
        user_id,
        work_id,
        None,
        None,
        IdentityMode::Background,
        ConflictSource::ManualAdd,
    )
    .await;

    let persisted = db.get_work(user_id, work_id).await.expect("read work");
    assert!(!svc.is_enriching(user_id, work_id));
    assert_eq!(persisted.enrichment_status, EnrichmentStatus::Failed);
}

/// AC-010: parked identity states gate add completion before enrichment starts.
#[tokio::test]
async fn complete_add_preserves_parked_identity_without_enrichment() {
    for (title, status, ol_key) in [
        (
            "Parked Conflict Book",
            IdentityStatus::Conflict,
            "/works/OL710W",
        ),
        (
            "Parked Needs Review Book",
            IdentityStatus::NeedsReview,
            "/works/OL711W",
        ),
    ] {
        let db = common::create_test_db().await;
        let user_id = create_test_user(&db).await;
        let workflow = GatedWorkflow::failing();
        let entered_notify = workflow.entered();
        let entered = entered_notify.notified();
        let svc = Arc::new(service(db.clone(), workflow.clone()));
        let created = svc
            .add_fast(
                user_id,
                confirmed_candidate(title, "Progress Author", ol_key),
            )
            .await
            .expect("add_fast should create work");
        let work_id = created.work.id;
        db.set_identity_status(user_id, work_id, status)
            .await
            .expect("park identity status");

        let task_svc = Arc::clone(&svc);
        let handle = tokio::spawn(async move {
            task_svc
                .complete_add(
                    user_id,
                    work_id,
                    None,
                    None,
                    IdentityMode::Background,
                    ConflictSource::ManualAdd,
                )
                .await;
        });

        let entered = tokio::time::timeout(Duration::from_millis(50), entered).await;
        workflow.release();
        handle.await.expect("complete_add task should not panic");

        assert!(
            entered.is_err(),
            "parked identity must gate complete_add before enrichment starts"
        );
        let after = db.get_work(user_id, work_id).await.expect("read work");
        assert!(!svc.is_enriching(user_id, work_id));
        assert_eq!(after.identity_status, status);
    }
}

/// AC-013/REQ-008: the progress signal is process-local and resets on restart.
#[tokio::test]
async fn is_enriching_is_false_for_fresh_service_over_same_db() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let workflow = GatedWorkflow::succeeding();
    let entered_notify = workflow.entered();
    let entered = entered_notify.notified();
    let svc = Arc::new(service(db.clone(), workflow.clone()));
    let created = svc
        .add_fast(
            user_id,
            confirmed_candidate("Restart Book", "Progress Author", "/works/OL702W"),
        )
        .await
        .expect("add_fast should create work");
    let work_id = created.work.id;

    let task_svc = Arc::clone(&svc);
    let handle = tokio::spawn(async move {
        task_svc
            .complete_add(
                user_id,
                work_id,
                None,
                None,
                IdentityMode::Background,
                ConflictSource::ManualAdd,
            )
            .await;
    });

    tokio::time::timeout(Duration::from_secs(1), entered)
        .await
        .expect("enrichment workflow should start");
    assert!(
        svc.is_enriching(user_id, work_id),
        "original service should report in-flight enrichment after enrich_work has entered"
    );

    let fresh = service(db, SleepingWorkflow::succeeding(Duration::from_millis(1)));
    assert!(
        !fresh.is_enriching(user_id, work_id),
        "fresh service over the same DB must not inherit in-memory progress state"
    );

    workflow.release();
    handle.await.expect("complete_add task should not panic");
}

#[derive(Clone)]
struct RecordingDelayedWorkflow {
    delay: std::time::Duration,
    events: std::sync::Arc<std::sync::Mutex<Vec<EnrichmentExecution>>>,
}

#[derive(Clone, Copy, Debug)]
struct EnrichmentExecution {
    enter: tokio::time::Instant,
    exit: tokio::time::Instant,
}

impl RecordingDelayedWorkflow {
    fn new(delay: std::time::Duration) -> Self {
        Self {
            delay,
            events: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    fn events(&self) -> Vec<EnrichmentExecution> {
        self.events.lock().unwrap().clone()
    }
}

impl livrarr_domain::services::EnrichmentWorkflow for RecordingDelayedWorkflow {
    async fn enrich_work(
        &self,
        user_id: livrarr_domain::UserId,
        work_id: livrarr_domain::WorkId,
        _mode: livrarr_domain::services::EnrichmentMode,
        _candidate_id: Option<livrarr_domain::identity::CandidateId>,
        _priority: livrarr_domain::RequestPriority,
        _freshness: livrarr_domain::Freshness,
    ) -> Result<
        livrarr_domain::services::EnrichmentResult,
        livrarr_domain::services::EnrichmentWorkflowError,
    > {
        let enter = tokio::time::Instant::now();
        tokio::time::sleep(self.delay).await;
        let exit = tokio::time::Instant::now();

        self.events
            .lock()
            .unwrap()
            .push(EnrichmentExecution { enter, exit });

        Ok(livrarr_domain::services::EnrichmentResult {
            enrichment_status: livrarr_domain::EnrichmentStatus::Enriched,
            enrichment_source: Some("recording-delayed-stub".into()),
            work: livrarr_domain::Work {
                id: work_id,
                user_id,
                ..Default::default()
            },
            merge_deferred: false,
            provider_outcomes: std::collections::HashMap::new(),
            cover_resolution: None,
            audiobook_cover_resolution: None,
            identity_not_found: false,
            changed: false,
            attempted: true,
            captured_provider_identity: Vec::new(),
            captured_route_proposals: Vec::new(),
            provider_chase_attempted: false,
        })
    }

    async fn reset_for_manual_refresh(
        &self,
        _user_id: livrarr_domain::UserId,
        _work_id: livrarr_domain::WorkId,
    ) -> Result<(), livrarr_domain::services::EnrichmentWorkflowError> {
        Ok(())
    }

    async fn inject_source_data(
        &self,
        _user_id: livrarr_domain::UserId,
        _work_id: livrarr_domain::WorkId,
        _data: livrarr_domain::services::SourceProviderData,
    ) {
    }
}

#[tokio::test]
async fn concurrent_retry_serializes_behind_active_refresh() {
    use livrarr_db::{WorkDb, WorkDbCreate};
    use livrarr_domain::services::WorkService;

    let db = livrarr_db::sqlite::SqliteDb::new_test().await;
    let user_id = livrarr_behavioral::stubs::create_test_user(&db).await;

    let (work, created) = db
        .create_work(livrarr_db::CreateWorkDbRequest {
            user_id,
            title: "Queued Refresh".into(),
            author_name: "Queue Author".into(),
            normalized_title: "queued refresh".into(),
            normalized_author: "queue author".into(),
            ol_key: Some("OL100000W".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(created, "AC-012: one seeded work must be created");

    db.set_identity_status(user_id, work.id, livrarr_domain::IdentityStatus::Confirmed)
        .await
        .unwrap();

    let workflow = RecordingDelayedWorkflow::new(std::time::Duration::from_millis(80));
    let service = livrarr_metadata::work_service::WorkServiceImpl::new(
        db.clone(),
        workflow.clone(),
        livrarr_behavioral::stubs::StubHttpFetcher::new(),
        std::env::temp_dir(),
    );

    let first = service.refresh(
        user_id,
        work.id,
        livrarr_domain::services::RefreshSurface::Interactive,
    );
    let second = async {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        service
            .refresh(
                user_id,
                work.id,
                livrarr_domain::services::RefreshSurface::Interactive,
            )
            .await
    };

    let (first, second) = tokio::join!(first, second);

    assert!(
        first.is_ok(),
        "AC-012: an active refresh must complete successfully while a retry waits behind it"
    );
    assert!(
        second.is_ok(),
        "AC-012: a retry issued during an active refresh must wait and then run, not be rejected"
    );

    let events = workflow.events();
    assert_eq!(
        events.len(),
        2,
        "AC-012: the active refresh and queued retry must both execute enrichment"
    );

    assert!(
        events[1].enter >= events[0].exit,
        "AC-012: per-(user, work) refresh locking must prevent overlapping enrichment executions"
    );
}
