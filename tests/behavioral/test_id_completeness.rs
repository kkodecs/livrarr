use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use livrarr_behavioral::stubs::{
    create_second_test_user, create_test_user, StubEnrichmentWorkflow, StubHttpFetcher,
};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    CreateWorkDbRequest, UpdateWorkEnrichmentDbRequest, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::identity::{
    AnchorConfidence, AnchorProvenance, AnchorSetter, AnchorType, Candidate, CandidateId,
    CapturedIdentity, ConflictSource, IdentityMethod, IdentityMode, MatchBasis, Resolution,
    ResolutionScore,
};
use livrarr_domain::services::{
    ConvergeOutcome, RefreshSurface, WorkIdentityError, WorkIdentityRepository, WorkService,
};
use livrarr_domain::{
    normalize_for_matching, AuthType, EnrichmentStatus, IdentityStatus, MetadataProvider, UserId,
    Work,
};
use livrarr_external_data::transport_cache::TransportCache;
use livrarr_external_data::{
    NormalizedWorkDetail, ProviderClient, ProviderOutcome, StubProviderClient,
};
use livrarr_handlers::context::{HasWorkIdentityRepository, HasWorkService};
use livrarr_handlers::work::{affirm_pending_anchor, list_pending_anchors};
use livrarr_handlers::AuthContext;
use livrarr_metadata::async_resolver::settle_identity;
use livrarr_metadata::english_identity_resolver::EnglishIdentityResolver;
use livrarr_metadata::english_identity_resolver::{LiveEnglishIdentityResolver, ResolverConfig};
use livrarr_metadata::work_service::{StubNoLlm, WorkServiceImpl};

type TestWorkService =
    WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher, StubNoLlm>;

struct ScriptedResolver {
    calls: AtomicUsize,
    result: Mutex<Resolution>,
}

impl ScriptedResolver {
    fn new(result: Resolution) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            result: Mutex::new(result),
        }
    }
}

impl EnglishIdentityResolver for ScriptedResolver {
    async fn resolve(
        &self,
        _user_id: UserId,
        _seed: &livrarr_domain::identity::WorkSeed,
        _tier: livrarr_domain::identity::LatencyTier,
    ) -> Result<Resolution, WorkIdentityError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.result.lock().expect("scripted result").clone())
    }
}

#[derive(Clone)]
struct TestState {
    work_service: Arc<TestWorkService>,
    identity_repo: SqliteDb,
}

impl HasWorkService for TestState {
    type WorkSvc = TestWorkService;

    fn work_service(&self) -> &Self::WorkSvc {
        &self.work_service
    }
}

impl HasWorkIdentityRepository for TestState {
    type WorkIdentityRepo = SqliteDb;

    fn work_identity_repo(&self) -> &Self::WorkIdentityRepo {
        &self.identity_repo
    }
}

fn service(
    db: SqliteDb,
    workflow: StubEnrichmentWorkflow,
    resolver: Option<LiveEnglishIdentityResolver>,
) -> TestWorkService {
    let svc = WorkServiceImpl::new_with_all(
        db,
        workflow,
        StubHttpFetcher::new(),
        StubNoLlm,
        tempfile::tempdir()
            .expect("test data dir")
            .path()
            .to_path_buf(),
    );

    match resolver {
        Some(resolver) => svc.with_resolver(Arc::new(resolver)),
        None => svc,
    }
}

fn resolver_with_stubs(stubs: Vec<StubProviderClient>) -> LiveEnglishIdentityResolver {
    let clients = stubs
        .into_iter()
        .map(|s| (s.provider, ProviderClient::Stub(s)))
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

fn work_req(user_id: UserId, title: &str) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: "Id Completeness Author".to_string(),
        normalized_title: normalize_for_matching(title),
        normalized_author: normalize_for_matching("Id Completeness Author"),
        language: Some("en".to_string()),
        monitor_ebook: true,
        monitor_audiobook: true,
        ..Default::default()
    }
}

async fn seed_work(
    db: &SqliteDb,
    user_id: UserId,
    title: &str,
    identity_status: IdentityStatus,
    enrichment_status: EnrichmentStatus,
    anchors: SeedAnchors,
) -> Work {
    let (work, created) = db
        .create_work(CreateWorkDbRequest {
            ol_key: anchors.ol_key.map(str::to_string),
            gr_key: anchors.gr_key.map(str::to_string),
            isbn_13: anchors.isbn_13.map(str::to_string),
            asin: anchors.asin.map(str::to_string),
            ..work_req(user_id, title)
        })
        .await
        .expect("seed work");
    assert!(created, "fixture titles must be unique");

    if let Some(hc_key) = anchors.hc_key {
        db.confirm_anchor(
            work.id,
            AnchorType::new(AnchorType::HC_WORK),
            hc_key,
            AnchorSetter::Import,
        )
        .await
        .expect("seed hc_key anchor");
    }

    if enrichment_status != EnrichmentStatus::Unenriched {
        db.update_work_enrichment(
            user_id,
            work.id,
            UpdateWorkEnrichmentDbRequest {
                enrichment_status,
                enrichment_source: Some("test-seed".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("seed enrichment status");
    }

    db.set_identity_status(user_id, work.id, identity_status)
        .await
        .expect("seed identity status");

    db.get_work(user_id, work.id)
        .await
        .expect("read seeded work")
}

#[derive(Clone, Copy, Default)]
struct SeedAnchors {
    ol_key: Option<&'static str>,
    gr_key: Option<&'static str>,
    hc_key: Option<&'static str>,
    isbn_13: Option<&'static str>,
    asin: Option<&'static str>,
}

fn captured(
    ol_key: Option<&str>,
    gr_key: Option<&str>,
    hc_key: Option<&str>,
    isbn_13: Option<&str>,
    asin: Option<&str>,
    title: &str,
) -> CapturedIdentity {
    CapturedIdentity {
        ol_key: ol_key.map(str::to_string),
        gr_key: gr_key.map(str::to_string),
        hc_key: hc_key.map(str::to_string),
        isbn_13: isbn_13.map(str::to_string),
        asin: asin.map(str::to_string),
        title: title.to_string(),
        author_name: "Id Completeness Author".to_string(),
        language: Some("en".to_string()),
    }
}

fn hard_provenance(identity: &CapturedIdentity) -> AnchorProvenance {
    AnchorProvenance {
        ol_key: identity.ol_key.as_ref().map(|_| MatchBasis::Hard),
        gr_key: identity.gr_key.as_ref().map(|_| MatchBasis::Hard),
        hc_key: identity.hc_key.as_ref().map(|_| MatchBasis::Hard),
        isbn_13: identity.isbn_13.as_ref().map(|_| MatchBasis::Hard),
        asin: identity.asin.as_ref().map(|_| MatchBasis::Hard),
    }
}

fn resolved_with_provenance(
    identity: CapturedIdentity,
    provenance: Option<AnchorProvenance>,
) -> Resolution {
    let provenance = provenance.unwrap_or_else(|| hard_provenance(&identity));
    Resolution::Resolved {
        identity,
        method: IdentityMethod::IsbnDirect,
        candidate_id: CandidateId("id-completeness-candidate".to_string()),
        provenance,
    }
}

async fn confirm_anchor(db: &SqliteDb, work_id: i64, anchor_type: &str, value: &str) {
    db.confirm_anchor(
        work_id,
        AnchorType::new(anchor_type),
        value,
        AnchorSetter::User,
    )
    .await
    .expect("confirm anchor");
}

async fn auth_context(db: &SqliteDb, user_id: UserId) -> AuthContext {
    AuthContext {
        user: db.get_user(user_id).await.expect("get auth user"),
        auth_type: AuthType::Session,
        session_token_hash: Some("test-session".to_string()),
    }
}

fn test_state(db: SqliteDb) -> TestState {
    TestState {
        work_service: Arc::new(service(
            db.clone(),
            StubEnrichmentWorkflow::succeeding(),
            None,
        )),
        identity_repo: db,
    }
}

#[tokio::test]
async fn test_id_completeness_pending_anchor_firewall_blank_and_monotonic() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_work(
        &db,
        user_id,
        "Pending Anchor Firewall",
        IdentityStatus::Confirmed,
        EnrichmentStatus::Enriched,
        SeedAnchors::default(),
    )
    .await;

    db.record_pending_anchor(work.id, AnchorType::new(AnchorType::ASIN), "B000PEND12")
        .await
        .expect("record pending ASIN");
    let after = db.get_work(user_id, work.id).await.expect("read work");
    let anchors = db.list_anchors(work.id).await.expect("list anchors");

    assert_eq!(after.asin, None, "pending ASIN must not sync works.asin");
    let pending = anchors
        .iter()
        .find(|a| a.anchor_type.as_str() == AnchorType::ASIN && a.anchor_value == "B000PEND12")
        .expect("pending ASIN ledger row");
    assert_eq!(pending.confidence, AnchorConfidence::Pending);
    assert_eq!(pending.setter, AnchorSetter::AutoSearch);

    let err = db
        .record_pending_anchor(work.id, AnchorType::new(AnchorType::ASIN), "   ")
        .await
        .expect_err("blank pending anchor is rejected");
    assert!(matches!(err, WorkIdentityError::InvalidAnchorValue));

    confirm_anchor(&db, work.id, AnchorType::ASIN, "B000CONF12").await;
    db.record_pending_anchor(work.id, AnchorType::new(AnchorType::ASIN), "B000CONF12")
        .await
        .expect("re-offering confirmed anchor as fuzzy guess");
    let anchors = db.list_anchors(work.id).await.expect("list anchors again");
    let confirmed = anchors
        .iter()
        .find(|a| a.anchor_type.as_str() == AnchorType::ASIN && a.anchor_value == "B000CONF12")
        .expect("confirmed same-value ASIN");
    assert_eq!(confirmed.confidence, AnchorConfidence::Confirmed);
}

#[tokio::test]
async fn test_id_completeness_dead_end_counters_are_durable_and_clearable() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_work(
        &db,
        user_id,
        "Dead End Counters",
        IdentityStatus::Confirmed,
        EnrichmentStatus::Enriched,
        SeedAnchors::default(),
    )
    .await;

    for _ in 0..3 {
        db.bump_anchor_attempt(work.id, AnchorType::new(AnchorType::GR_WORK))
            .await
            .expect("bump gr_key dead-end");
    }
    db.bump_anchor_attempt(work.id, AnchorType::new(AnchorType::ASIN))
        .await
        .expect("bump ASIN dead-end");

    let dead_ends = db
        .list_anchor_dead_ends(work.id)
        .await
        .expect("list dead-ends");
    let gr = dead_ends
        .iter()
        .find(|d| d.anchor_type.as_str() == AnchorType::GR_WORK)
        .expect("gr_key dead-end");
    assert_eq!(gr.attempt_count, 3);

    db.reset_for_manual_refresh(user_id, work.id)
        .await
        .expect("manual refresh reset");
    let dead_ends = db
        .list_anchor_dead_ends(work.id)
        .await
        .expect("dead-ends survive refresh reset");
    assert_eq!(
        dead_ends
            .iter()
            .find(|d| d.anchor_type.as_str() == AnchorType::GR_WORK)
            .expect("gr_key dead-end after refresh")
            .attempt_count,
        3
    );

    db.clear_anchor_dead_end(work.id, AnchorType::new(AnchorType::GR_WORK))
        .await
        .expect("clear only gr_key");
    let dead_ends = db.list_anchor_dead_ends(work.id).await.expect("list");
    assert!(!dead_ends
        .iter()
        .any(|d| d.anchor_type.as_str() == AnchorType::GR_WORK));
    assert!(dead_ends
        .iter()
        .any(|d| d.anchor_type.as_str() == AnchorType::ASIN));

    db.clear_anchor_dead_ends(work.id)
        .await
        .expect("clear all dead-ends");
    assert!(db
        .list_anchor_dead_ends(work.id)
        .await
        .expect("list after clear all")
        .is_empty());
}

#[tokio::test]
async fn test_id_completeness_selector_branches_guards_and_next_clock() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let now = chrono::Utc::now();

    let pending = seed_work(
        &db,
        user_id,
        "Selector Pending",
        IdentityStatus::Pending,
        EnrichmentStatus::Unenriched,
        SeedAnchors::default(),
    )
    .await;
    let full_but_failed = seed_work(
        &db,
        user_id,
        "Selector Full Failed",
        IdentityStatus::Confirmed,
        EnrichmentStatus::Failed,
        SeedAnchors {
            ol_key: Some("OL100001W"),
            gr_key: Some("100001"),
            hc_key: Some("100001"),
            isbn_13: Some("9780000000001"),
            asin: Some("B0FULLFAIL"),
        },
    )
    .await;
    let missing_chaseable = seed_work(
        &db,
        user_id,
        "Selector Missing Chaseable",
        IdentityStatus::Confirmed,
        EnrichmentStatus::Enriched,
        SeedAnchors {
            ol_key: Some("OL100002W"),
            hc_key: Some("100002"),
            isbn_13: Some("9780000000002"),
            asin: Some("B0SELONE02"),
            ..Default::default()
        },
    )
    .await;
    let missing_pending_guess = seed_work(
        &db,
        user_id,
        "Selector Pending Guess",
        IdentityStatus::Confirmed,
        EnrichmentStatus::Enriched,
        SeedAnchors {
            ol_key: Some("OL100003W"),
            hc_key: Some("100003"),
            isbn_13: Some("9780000000003"),
            asin: Some("B0SELONE03"),
            ..Default::default()
        },
    )
    .await;
    db.record_pending_anchor(
        missing_pending_guess.id,
        AnchorType::new(AnchorType::GR_WORK),
        "100003",
    )
    .await
    .expect("record pending gr_key guess");
    let missing_at_threshold = seed_work(
        &db,
        user_id,
        "Selector At Threshold",
        IdentityStatus::Confirmed,
        EnrichmentStatus::Enriched,
        SeedAnchors {
            ol_key: Some("OL100004W"),
            hc_key: Some("100004"),
            isbn_13: Some("9780000000004"),
            asin: Some("B0SELONE04"),
            ..Default::default()
        },
    )
    .await;
    for _ in 0..3 {
        db.bump_anchor_attempt(
            missing_at_threshold.id,
            AnchorType::new(AnchorType::GR_WORK),
        )
        .await
        .expect("bump threshold gr_key");
    }
    let future = seed_work(
        &db,
        user_id,
        "Selector Future",
        IdentityStatus::Pending,
        EnrichmentStatus::Unenriched,
        SeedAnchors::default(),
    )
    .await;
    db.set_next_convergence_at(user_id, future.id, Some(now + chrono::Duration::hours(1)))
        .await
        .expect("set future convergence clock");

    let due = db
        .list_convergence_due(user_id, now, 3, 20)
        .await
        .expect("list convergence due");
    assert!(
        due.contains(&pending.id),
        "branch 1 selects identity-pending work"
    );
    assert!(
        due.contains(&full_but_failed.id),
        "branch 2 selects enrichment-incomplete work even when fully anchored"
    );
    assert!(
        due.contains(&missing_chaseable.id),
        "branch 3 selects confirmed work with a chaseable missing anchor"
    );
    assert!(
        !due.contains(&missing_pending_guess.id),
        "branch 3 ignores a missing anchor already held as a pending guess"
    );
    assert!(
        !due.contains(&missing_at_threshold.id),
        "branch 3 ignores a missing anchor at the dead-end threshold"
    );
    assert!(
        !due.contains(&future.id),
        "next_convergence_at in the future excludes the work"
    );

    db.set_next_convergence_at(user_id, future.id, None)
        .await
        .expect("clear convergence clock");
    let due = db
        .list_convergence_due(user_id, now, 3, 20)
        .await
        .expect("list convergence due after clear");
    assert!(
        due.contains(&future.id),
        "clearing next_convergence_at makes work due"
    );
}

#[tokio::test]
async fn test_id_completeness_converge_work_terminal_settle_enrich_clear_and_no_refresh_reset() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let pending_no_chase = seed_work(
        &db,
        user_id,
        "Converge Pending No Chase",
        IdentityStatus::Pending,
        EnrichmentStatus::Unenriched,
        SeedAnchors::default(),
    )
    .await;
    let pending_chaseable = seed_work(
        &db,
        user_id,
        "Converge Pending Chaseable",
        IdentityStatus::Pending,
        EnrichmentStatus::Unenriched,
        SeedAnchors {
            isbn_13: Some("9780000000101"),
            ..Default::default()
        },
    )
    .await;
    let full_enrichment_retry = seed_work(
        &db,
        user_id,
        "Converge Full Enrichment Retry",
        IdentityStatus::Confirmed,
        EnrichmentStatus::Failed,
        SeedAnchors {
            ol_key: Some("OL100102W"),
            gr_key: Some("100102"),
            hc_key: Some("100102"),
            isbn_13: Some("9780000000102"),
            asin: Some("B0CONVFULL"),
        },
    )
    .await;
    let harvested = seed_work(
        &db,
        user_id,
        "Converge Clears Dead End",
        IdentityStatus::Confirmed,
        EnrichmentStatus::Enriched,
        SeedAnchors {
            isbn_13: Some("9780000000103"),
            ..Default::default()
        },
    )
    .await;
    db.bump_anchor_attempt(harvested.id, AnchorType::new(AnchorType::GR_WORK))
        .await
        .expect("pre-bump gr_key");

    let workflow = StubEnrichmentWorkflow::succeeding();
    let ol = StubProviderClient::new(
        MetadataProvider::OpenLibrary,
        ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
            title: Some("Converge Clears Dead End".to_string()),
            author_name: Some("Id Completeness Author".to_string()),
            gr_key: Some("100103".to_string()),
            isbn_13: Some("9780000000103".to_string()),
            language: Some("en".to_string()),
            ..NormalizedWorkDetail::default()
        })),
    );
    let svc = service(
        db.clone(),
        workflow.clone(),
        Some(resolver_with_stubs(vec![ol.clone()])),
    );

    let outcome = svc
        .converge_work(user_id, pending_no_chase.id, 3)
        .await
        .expect("converge pending with no chaseable anchor");
    assert_eq!(outcome, ConvergeOutcome::Terminal);
    assert_eq!(
        db.get_work(user_id, pending_no_chase.id)
            .await
            .expect("read terminalized work")
            .identity_status,
        IdentityStatus::NeedsReview
    );

    let _ = svc
        .converge_work(user_id, pending_chaseable.id, 3)
        .await
        .expect("converge pending with chaseable anchor");
    assert!(
        ol.call_count() >= 1,
        "pending chaseable work settles identity"
    );

    let before_identity_calls = ol.call_count();
    let _ = svc
        .converge_work(user_id, full_enrichment_retry.id, 3)
        .await
        .expect("converge fully anchored enrichment retry");
    assert_eq!(
        ol.call_count(),
        before_identity_calls,
        "fully anchored enrichment retry does not fan out identity"
    );
    assert!(
        workflow.work_ids().contains(&full_enrichment_retry.id),
        "enrichment still runs for a fully anchored incomplete work"
    );

    let _ = svc
        .converge_work(user_id, harvested.id, 3)
        .await
        .expect("converge harvested anchor");
    let harvested_after = db
        .get_work(user_id, harvested.id)
        .await
        .expect("read harvested");
    assert_eq!(harvested_after.gr_key.as_deref(), Some("100103"));
    assert!(db
        .list_anchor_dead_ends(harvested.id)
        .await
        .expect("dead-end list after harvest")
        .iter()
        .all(|d| d.anchor_type.as_str() != AnchorType::GR_WORK));
    assert_eq!(
        workflow.reset_call_count(),
        0,
        "converge_work must never call reset_for_manual_refresh"
    );
}

/// Directive (Phase 5 REQ-008/AC-012): a work parked as `NeedsReview` — the
/// resolver could not confidently pick a candidate, so identity is grey — must
/// never have provider data written onto it by a background pass. Seeds a work
/// already at `NeedsReview` (the state a `Resolution::NeedsConfirmation`
/// verdict leaves it in) and drives it through the same background convergence
/// path a scheduled tick uses, with no resolver wired: the Step-0 dead-end exit
/// only applies to a `Pending` prior, so this exercises the Step-2 enrichment
/// gate (`identity_permits`) directly.
#[tokio::test]
async fn test_id_completeness_needs_review_work_never_dispatches_enrichment_ac012() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let parked = seed_work(
        &db,
        user_id,
        "AC-012 Parked Grey Candidate",
        IdentityStatus::NeedsReview,
        EnrichmentStatus::Unenriched,
        SeedAnchors::default(),
    )
    .await;

    let workflow = StubEnrichmentWorkflow::succeeding();
    let svc = service(db.clone(), workflow.clone(), None);

    let outcome = svc
        .converge_work(user_id, parked.id, 3)
        .await
        .expect("converge a needs-review work");

    assert_eq!(
        outcome,
        ConvergeOutcome::Terminal,
        "a NeedsReview identity is terminal regardless of enrichment/chaseable state"
    );
    assert_eq!(
        workflow.call_count(),
        0,
        "AC-012: background convergence must never dispatch enrichment for a grey/needs-review work"
    );
    assert!(
        !workflow.work_ids().contains(&parked.id),
        "AC-012: the parked work's id must never reach the enrichment workflow"
    );

    let after = db
        .get_work(user_id, parked.id)
        .await
        .expect("read parked work");
    assert_eq!(
        after.identity_status,
        IdentityStatus::NeedsReview,
        "a NeedsReview badge is not silently changed by convergence"
    );
    assert_eq!(
        after.enrichment_status,
        EnrichmentStatus::Unenriched,
        "AC-012: no provider data merged onto a grey-identity work"
    );
    assert_eq!(
        after.cover_url, None,
        "AC-012: no cover written from an uncertain candidate"
    );
    assert_eq!(
        after.description, None,
        "AC-012: no description written from an uncertain candidate"
    );
}

/// `refresh()` lacked the `identity_permits` gate the convergence path (the
/// test above) and the add door both apply — a parked work would still
/// enrich on a manual or bulk refresh. Same fixture shape as the test above,
/// driven through `refresh()` instead of `converge_work`, proving the gate
/// now makes the two paths consistent.
#[tokio::test]
async fn test_id_completeness_needs_review_work_never_enriches_on_refresh() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let parked = seed_work(
        &db,
        user_id,
        "Parked Refresh Guard",
        IdentityStatus::NeedsReview,
        EnrichmentStatus::Unenriched,
        SeedAnchors::default(),
    )
    .await;

    let workflow = StubEnrichmentWorkflow::succeeding();
    let svc = service(db.clone(), workflow.clone(), None);

    svc.refresh(user_id, parked.id, RefreshSurface::Interactive)
        .await
        .expect("refresh a needs-review work");

    assert_eq!(
        workflow.call_count(),
        0,
        "refresh must never dispatch enrichment for a grey/needs-review work"
    );
    assert!(
        !workflow.work_ids().contains(&parked.id),
        "the parked work's id must never reach the enrichment workflow via refresh"
    );

    let after = db
        .get_work(user_id, parked.id)
        .await
        .expect("read parked work");
    assert_eq!(
        after.identity_status,
        IdentityStatus::NeedsReview,
        "a NeedsReview badge is not silently changed by refresh"
    );
}

/// The ranked candidates behind a `NeedsReview` park are now persisted
/// (queryable per work) instead of discarded, and carry their real computed
/// scores rather than a placeholder.
#[tokio::test]
async fn test_id_completeness_review_candidates_persist_and_round_trip() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_work(
        &db,
        user_id,
        "Candidate Round Trip",
        IdentityStatus::Pending,
        EnrichmentStatus::Unenriched,
        SeedAnchors::default(),
    )
    .await;

    assert_eq!(
        db.get_review_candidates(work.id)
            .await
            .expect("read before any park"),
        None,
        "a work with no recorded park has no candidates to read"
    );

    let candidates = vec![Candidate {
        candidate_id: CandidateId("cand-1".to_string()),
        anchors: CapturedIdentity {
            ol_key: Some("OL-CAND-1".to_string()),
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
            title: "Candidate Round Trip".to_string(),
            author_name: "Test Author".to_string(),
            language: Some("en".to_string()),
        },
        cover_url: None,
        sources: vec![MetadataProvider::OpenLibrary],
        score: ResolutionScore {
            title_jaccard: 0.83,
            author_overlap: 0,
            runner_up_delta: 0.0,
        },
        existing_work_id: None,
    }];
    db.record_review_candidates(work.id, &candidates)
        .await
        .expect("record candidates");

    let round_tripped = db
        .get_review_candidates(work.id)
        .await
        .expect("read after record")
        .expect("candidates present after recording");
    assert_eq!(round_tripped.len(), 1);
    assert_eq!(
        round_tripped[0].anchors.ol_key.as_deref(),
        Some("OL-CAND-1")
    );
    assert!(
        (round_tripped[0].score.title_jaccard - 0.83).abs() < 1e-9,
        "the real computed score survives the round trip, not a hardcoded value"
    );

    // A later park replaces the set wholesale rather than appending to it.
    let candidates_v2 = vec![Candidate {
        candidate_id: CandidateId("cand-2".to_string()),
        anchors: CapturedIdentity {
            ol_key: Some("OL-CAND-2".to_string()),
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
            title: "Candidate Round Trip".to_string(),
            author_name: "Test Author".to_string(),
            language: Some("en".to_string()),
        },
        cover_url: None,
        sources: vec![MetadataProvider::Hardcover],
        score: ResolutionScore {
            title_jaccard: 0.91,
            author_overlap: 0,
            runner_up_delta: 0.0,
        },
        existing_work_id: None,
    }];
    db.record_review_candidates(work.id, &candidates_v2)
        .await
        .expect("record replacement candidates");
    let replaced = db
        .get_review_candidates(work.id)
        .await
        .expect("read after replace")
        .expect("candidates present after replace");
    assert_eq!(
        replaced.len(),
        1,
        "a fresh park replaces the prior candidate set, never appends"
    );
    assert_eq!(replaced[0].anchors.ol_key.as_deref(), Some("OL-CAND-2"));
}

/// AC-013: a parked work is visible in a review list with its persisted
/// candidates and real scores. The list endpoint pairs
/// `list_needs_review_works` with `get_review_candidates` per work; a
/// Confirmed work never appears in it.
#[tokio::test]
async fn test_id_completeness_identity_review_list_shows_parked_work_with_real_scores() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let parked = seed_work(
        &db,
        user_id,
        "Review List Park",
        IdentityStatus::NeedsReview,
        EnrichmentStatus::Unenriched,
        SeedAnchors::default(),
    )
    .await;
    let _settled = seed_work(
        &db,
        user_id,
        "Review List Settled",
        IdentityStatus::Confirmed,
        EnrichmentStatus::Enriched,
        SeedAnchors::default(),
    )
    .await;

    let candidates = vec![Candidate {
        candidate_id: CandidateId("cand-list-1".to_string()),
        anchors: CapturedIdentity {
            ol_key: Some("OL-LIST-1".to_string()),
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
            title: "Review List Park".to_string(),
            author_name: "Id Completeness Author".to_string(),
            language: Some("en".to_string()),
        },
        cover_url: None,
        sources: vec![MetadataProvider::OpenLibrary],
        score: ResolutionScore {
            title_jaccard: 0.87,
            author_overlap: 1,
            runner_up_delta: 0.1,
        },
        existing_work_id: None,
    }];
    db.record_review_candidates(parked.id, &candidates)
        .await
        .expect("record park candidates");

    let state = test_state(db.clone());
    let parks =
        livrarr_handlers::identity_review::list(State(state), auth_context(&db, user_id).await)
            .await
            .expect("list identity review")
            .0;

    assert_eq!(
        parks.len(),
        1,
        "only the NeedsReview work is listed, not the Confirmed one"
    );
    assert_eq!(parks[0].work_id, parked.id);
    assert_eq!(parks[0].candidates.len(), 1);
    assert_eq!(parks[0].candidates[0].ol_key.as_deref(), Some("OL-LIST-1"));
    assert!(
        (parks[0].candidates[0].title_jaccard - 0.87).abs() < 1e-9,
        "the review list surfaces the real computed score, not a hardcoded 1.0"
    );
}

/// AC-013: choosing a candidate applies it and un-parks the work — driven
/// through the real `identity_review::resolve` handler (not injected state),
/// which reuses the anchor-confirm + badge-recompute transaction of the
/// existing pending-anchor affirm path, generalized to a candidate's full
/// anchor set.
#[tokio::test]
async fn test_id_completeness_identity_review_resolve_applies_candidate_and_unparks() {
    let db = create_test_db().await;
    let user_a = create_test_user(&db).await;
    let user_b = create_second_test_user(&db).await;
    let work = seed_work(
        &db,
        user_a,
        "Review Resolve Park",
        IdentityStatus::NeedsReview,
        EnrichmentStatus::Unenriched,
        SeedAnchors::default(),
    )
    .await;

    let candidates = vec![Candidate {
        candidate_id: CandidateId("cand-resolve-1".to_string()),
        anchors: CapturedIdentity {
            ol_key: Some("OL-RESOLVE-1".to_string()),
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
            title: "Review Resolve Park".to_string(),
            author_name: "Id Completeness Author".to_string(),
            language: Some("en".to_string()),
        },
        cover_url: None,
        sources: vec![MetadataProvider::OpenLibrary],
        score: ResolutionScore {
            title_jaccard: 0.9,
            author_overlap: 1,
            runner_up_delta: 0.2,
        },
        existing_work_id: None,
    }];
    db.record_review_candidates(work.id, &candidates)
        .await
        .expect("record resolve candidates");

    let state = test_state(db.clone());

    // Cross-user resolve is hidden, same as every other identity write path.
    let cross_user = livrarr_handlers::identity_review::resolve(
        State(state.clone()),
        auth_context(&db, user_b).await,
        Path(work.id),
        axum::Json(livrarr_handlers::identity_review::ResolveReviewRequest {
            candidate_id: "cand-resolve-1".to_string(),
        }),
    )
    .await;
    let Err(err) = cross_user else {
        panic!("cross-user resolve must be hidden");
    };
    assert_eq!(err.into_response().status(), StatusCode::NOT_FOUND);

    let status = livrarr_handlers::identity_review::resolve(
        State(state),
        auth_context(&db, user_a).await,
        Path(work.id),
        axum::Json(livrarr_handlers::identity_review::ResolveReviewRequest {
            candidate_id: "cand-resolve-1".to_string(),
        }),
    )
    .await
    .expect("resolve applies the chosen candidate");
    assert_eq!(status, StatusCode::NO_CONTENT);

    let after = db
        .get_work(user_a, work.id)
        .await
        .expect("read resolved work");
    assert_eq!(
        after.identity_status,
        IdentityStatus::Confirmed,
        "a work-anchor candidate un-parks straight to Confirmed"
    );
    assert_eq!(after.ol_key.as_deref(), Some("OL-RESOLVE-1"));

    let anchors = db.list_anchors(work.id).await.expect("list anchors");
    let ol_anchor = anchors
        .iter()
        .find(|a| a.anchor_type.as_str() == AnchorType::OL_WORK && a.anchor_value == "OL-RESOLVE-1")
        .expect("resolved OL anchor");
    assert_eq!(ol_anchor.confidence, AnchorConfidence::Confirmed);
    assert_eq!(ol_anchor.setter, AnchorSetter::User);

    assert_eq!(
        db.get_review_candidates(work.id)
            .await
            .expect("read after resolve"),
        None,
        "the resolved park's candidate row is cleared"
    );
}

/// AC-013: dismissing a park leaves the work standalone as Pending — no
/// anchors written, no merge. A duplicate surfaced this way is one click from
/// the separate merge-two-works action.
#[tokio::test]
async fn test_id_completeness_identity_review_dismiss_leaves_work_standalone_pending() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_work(
        &db,
        user_id,
        "Review Dismiss Park",
        IdentityStatus::NeedsReview,
        EnrichmentStatus::Unenriched,
        SeedAnchors::default(),
    )
    .await;

    let candidates = vec![Candidate {
        candidate_id: CandidateId("cand-dismiss-1".to_string()),
        anchors: CapturedIdentity {
            ol_key: Some("OL-DISMISS-1".to_string()),
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
            title: "Review Dismiss Park".to_string(),
            author_name: "Id Completeness Author".to_string(),
            language: Some("en".to_string()),
        },
        cover_url: None,
        sources: vec![MetadataProvider::OpenLibrary],
        score: ResolutionScore {
            title_jaccard: 0.8,
            author_overlap: 1,
            runner_up_delta: 0.1,
        },
        existing_work_id: None,
    }];
    db.record_review_candidates(work.id, &candidates)
        .await
        .expect("record dismiss candidates");

    let state = test_state(db.clone());
    let status = livrarr_handlers::identity_review::dismiss(
        State(state),
        auth_context(&db, user_id).await,
        Path(work.id),
    )
    .await
    .expect("dismiss the park");
    assert_eq!(status, StatusCode::NO_CONTENT);

    let after = db
        .get_work(user_id, work.id)
        .await
        .expect("read dismissed work");
    assert_eq!(
        after.identity_status,
        IdentityStatus::Pending,
        "dismissing un-parks to Pending, standalone — never a merge"
    );
    assert_eq!(
        after.ol_key, None,
        "dismiss never adopts the candidate's anchors"
    );

    let anchors = db.list_anchors(work.id).await.expect("list anchors");
    assert!(anchors.is_empty(), "dismiss must not write any anchors");

    assert_eq!(
        db.get_review_candidates(work.id)
            .await
            .expect("read after dismiss"),
        None,
        "the dismissed park's candidate row is cleared"
    );
}

/// R-1 guard: dismissing an owned work that is NOT parked NeedsReview is
/// rejected with 409 — a direct POST must never downgrade a settled work
/// (Confirmed here) to Pending. Status, anchors, and the (stale) candidates
/// row are all left untouched.
#[tokio::test]
async fn test_id_completeness_identity_review_dismiss_rejects_settled_work() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_work(
        &db,
        user_id,
        "Review Dismiss Settled",
        IdentityStatus::Confirmed,
        EnrichmentStatus::Enriched,
        SeedAnchors::default(),
    )
    .await;

    // A stale candidates row on a settled work — the guard is on the badge,
    // not the row, so this must be inert.
    let stale = vec![Candidate {
        candidate_id: CandidateId("cand-stale-dismiss".to_string()),
        anchors: CapturedIdentity {
            ol_key: Some("OL-STALE-D".to_string()),
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
            title: "Review Dismiss Settled".to_string(),
            author_name: "Id Completeness Author".to_string(),
            language: Some("en".to_string()),
        },
        cover_url: None,
        sources: vec![MetadataProvider::OpenLibrary],
        score: ResolutionScore {
            title_jaccard: 0.8,
            author_overlap: 1,
            runner_up_delta: 0.1,
        },
        existing_work_id: None,
    }];
    db.record_review_candidates(work.id, &stale)
        .await
        .expect("record stale candidates");

    let state = test_state(db.clone());
    let err = livrarr_handlers::identity_review::dismiss(
        State(state),
        auth_context(&db, user_id).await,
        Path(work.id),
    )
    .await
    .expect_err("dismissing a settled work must be rejected");
    assert_eq!(err.into_response().status(), StatusCode::CONFLICT);

    let after = db
        .get_work(user_id, work.id)
        .await
        .expect("read work after rejected dismiss");
    assert_eq!(
        after.identity_status,
        IdentityStatus::Confirmed,
        "a settled work's badge is never downgraded by dismiss"
    );
    let anchors = db.list_anchors(work.id).await.expect("list anchors");
    assert!(anchors.is_empty(), "rejected dismiss writes no anchors");
    assert!(
        db.get_review_candidates(work.id)
            .await
            .expect("read candidates after rejected dismiss")
            .is_some(),
        "rejected dismiss leaves the candidates row untouched"
    );
}

/// R-2 guard: resolving an owned work that is NOT parked NeedsReview — but
/// still holds a stale candidates row — is rejected with 409 and writes
/// nothing. The guard lives inside apply_review_candidate's transaction, so
/// the handler's read-then-apply window cannot rewrite anchors on a settled
/// work.
#[tokio::test]
async fn test_id_completeness_identity_review_resolve_rejects_settled_work_with_stale_candidates() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_work(
        &db,
        user_id,
        "Review Resolve Settled",
        IdentityStatus::Confirmed,
        EnrichmentStatus::Enriched,
        SeedAnchors::default(),
    )
    .await;

    let stale = vec![Candidate {
        candidate_id: CandidateId("cand-stale-resolve".to_string()),
        anchors: CapturedIdentity {
            ol_key: Some("OL-STALE-R".to_string()),
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
            title: "Review Resolve Settled".to_string(),
            author_name: "Id Completeness Author".to_string(),
            language: Some("en".to_string()),
        },
        cover_url: None,
        sources: vec![MetadataProvider::OpenLibrary],
        score: ResolutionScore {
            title_jaccard: 0.9,
            author_overlap: 1,
            runner_up_delta: 0.2,
        },
        existing_work_id: None,
    }];
    db.record_review_candidates(work.id, &stale)
        .await
        .expect("record stale candidates");

    let state = test_state(db.clone());
    let err = livrarr_handlers::identity_review::resolve(
        State(state),
        auth_context(&db, user_id).await,
        Path(work.id),
        axum::Json(livrarr_handlers::identity_review::ResolveReviewRequest {
            candidate_id: "cand-stale-resolve".to_string(),
        }),
    )
    .await
    .expect_err("resolving a settled work must be rejected");
    assert_eq!(err.into_response().status(), StatusCode::CONFLICT);

    let after = db
        .get_work(user_id, work.id)
        .await
        .expect("read work after rejected resolve");
    assert_eq!(
        after.identity_status,
        IdentityStatus::Confirmed,
        "a settled work's badge is untouched by a rejected resolve"
    );
    assert_eq!(
        after.ol_key, None,
        "the stale candidate's anchors are never adopted"
    );
    let anchors = db.list_anchors(work.id).await.expect("list anchors");
    assert!(anchors.is_empty(), "rejected resolve writes no anchors");
    assert!(
        db.get_review_candidates(work.id)
            .await
            .expect("read candidates after rejected resolve")
            .is_some(),
        "rejected resolve leaves the candidates row untouched"
    );
}

#[tokio::test]
async fn test_id_completeness_pending_anchor_handlers_affirm_list_and_cross_user_404() {
    let db = create_test_db().await;
    let user_a = create_test_user(&db).await;
    let user_b = create_second_test_user(&db).await;
    let work = seed_work(
        &db,
        user_a,
        "Pending Handler",
        IdentityStatus::Confirmed,
        EnrichmentStatus::Enriched,
        SeedAnchors::default(),
    )
    .await;
    db.record_pending_anchor(work.id, AnchorType::new(AnchorType::ASIN), "B0AFFIRM12")
        .await
        .expect("record pending ASIN");

    let state = test_state(db.clone());
    let JsonLike(list) = JsonLike(
        list_pending_anchors(
            State(state.clone()),
            auth_context(&db, user_a).await,
            Path(work.id),
        )
        .await
        .expect("list pending anchors")
        .0,
    );
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].anchor_type, AnchorType::ASIN);
    assert_eq!(list[0].value, "B0AFFIRM12");
    assert_eq!(list[0].setter, "auto_search");

    let cross_user = list_pending_anchors(
        State(state.clone()),
        auth_context(&db, user_b).await,
        Path(work.id),
    )
    .await;
    let Err(err) = cross_user else {
        panic!("cross-user pending-anchor list is hidden");
    };
    assert_eq!(err.into_response().status(), StatusCode::NOT_FOUND);

    let status = affirm_pending_anchor(
        State(state.clone()),
        auth_context(&db, user_a).await,
        Path((work.id, AnchorType::ASIN.to_string())),
    )
    .await
    .expect("affirm pending ASIN");
    assert_eq!(status, StatusCode::NO_CONTENT);
    let after = db
        .get_work(user_a, work.id)
        .await
        .expect("read affirmed work");
    assert_eq!(after.asin.as_deref(), Some("B0AFFIRM12"));
    let anchors = db.list_anchors(work.id).await.expect("list anchors");
    assert_eq!(
        anchors
            .iter()
            .find(|a| a.anchor_type.as_str() == AnchorType::ASIN && a.anchor_value == "B0AFFIRM12")
            .expect("affirmed ASIN anchor")
            .confidence,
        AnchorConfidence::Confirmed
    );

    db.record_pending_anchor(work.id, AnchorType::new(AnchorType::GR_WORK), "100204")
        .await
        .expect("record second pending anchor");
    let cross_affirm = affirm_pending_anchor(
        State(state),
        auth_context(&db, user_b).await,
        Path((work.id, AnchorType::GR_WORK.to_string())),
    )
    .await;
    assert_eq!(
        cross_affirm
            .expect_err("cross-user affirm is hidden")
            .into_response()
            .status(),
        StatusCode::NOT_FOUND
    );
    let after_cross = db
        .get_work(user_a, work.id)
        .await
        .expect("read after cross-user");
    assert_eq!(after_cross.gr_key, None);
}

struct JsonLike<T>(T);

#[tokio::test]
async fn test_id_completeness_pending_anchor_list_empty_returns_empty_array() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_work(
        &db,
        user_id,
        "No Pending Guesses",
        IdentityStatus::Confirmed,
        EnrichmentStatus::Enriched,
        SeedAnchors::default(),
    )
    .await;
    let state = test_state(db.clone());

    let pending = list_pending_anchors(
        State(state),
        auth_context(&db, user_id).await,
        Path(work.id),
    )
    .await
    .expect("list pending anchors")
    .0;
    assert!(pending.is_empty());
}

#[tokio::test]
async fn test_id_completeness_refresh_gate_confirmed_rechases_only_when_missing_obtainable_id() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let full = seed_work(
        &db,
        user_id,
        "Refresh Fully Anchored",
        IdentityStatus::Confirmed,
        EnrichmentStatus::Enriched,
        SeedAnchors {
            ol_key: Some("OL100201W"),
            gr_key: Some("100201"),
            hc_key: Some("100201"),
            isbn_13: Some("9780000000201"),
            asin: Some("B0REFRESH1"),
        },
    )
    .await;
    let missing = seed_work(
        &db,
        user_id,
        "Refresh Missing Obtainable",
        IdentityStatus::Confirmed,
        EnrichmentStatus::Enriched,
        SeedAnchors {
            ol_key: Some("OL100202W"),
            isbn_13: Some("9780000000202"),
            ..Default::default()
        },
    )
    .await;
    let ol = StubProviderClient::new(
        MetadataProvider::OpenLibrary,
        ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
            title: Some("Refresh Missing Obtainable".to_string()),
            author_name: Some("Id Completeness Author".to_string()),
            ol_key: Some("OL100202W".to_string()),
            gr_key: Some("100202".to_string()),
            isbn_13: Some("9780000000202".to_string()),
            language: Some("en".to_string()),
            ..NormalizedWorkDetail::default()
        })),
    );
    let svc = service(
        db.clone(),
        StubEnrichmentWorkflow::succeeding(),
        Some(resolver_with_stubs(vec![ol.clone()])),
    );

    svc.refresh(user_id, full.id, RefreshSurface::Interactive)
        .await
        .expect("refresh full work");
    assert_eq!(
        ol.call_count(),
        0,
        "fully anchored Confirmed refresh skips identity fan-out"
    );

    svc.refresh(user_id, missing.id, RefreshSurface::Interactive)
        .await
        .expect("refresh missing-id work");
    assert!(
        ol.call_count() >= 1,
        "Confirmed work missing an obtainable id re-chases identity"
    );
}

#[tokio::test]
async fn test_id_completeness_hard_attach_confirms_flm_syncs_on_title_author_match() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let hard = seed_work(
        &db,
        user_id,
        "Hard Shared ISBN",
        IdentityStatus::Pending,
        EnrichmentStatus::Unenriched,
        SeedAnchors {
            isbn_13: Some("9780000000309"),
            ..Default::default()
        },
    )
    .await;
    let fuzzy = seed_work(
        &db,
        user_id,
        "Fuzzy ASIN Only",
        IdentityStatus::Pending,
        EnrichmentStatus::Unenriched,
        SeedAnchors::default(),
    )
    .await;

    let hard_identity = captured(
        None,
        Some("100301"),
        None,
        Some("9780000000309"),
        None,
        "Hard Shared ISBN",
    );
    let hard_resolver = ScriptedResolver::new(resolved_with_provenance(
        hard_identity,
        Some(AnchorProvenance {
            gr_key: Some(MatchBasis::Hard),
            isbn_13: Some(MatchBasis::Hard),
            ..AnchorProvenance::default()
        }),
    ));
    let hard_report = settle_identity(
        &hard_resolver,
        &db,
        user_id,
        &hard,
        IdentityMode::Background,
        ConflictSource::Convergence,
    )
    .await
    .expect("settle hard bridge");
    let hard_after = db.get_work(user_id, hard.id).await.expect("read hard");
    assert_eq!(hard_after.gr_key.as_deref(), Some("100301"));
    assert_eq!(hard_after.identity_status, IdentityStatus::Confirmed);
    assert!(hard_report
        .anchors_merged
        .iter()
        .any(|kind| kind == AnchorType::GR_WORK));

    let fuzzy_identity = captured(
        None,
        None,
        None,
        None,
        Some("B000FUZZY1"),
        "Fuzzy ASIN Only",
    );
    let fuzzy_resolver = ScriptedResolver::new(resolved_with_provenance(
        fuzzy_identity,
        Some(AnchorProvenance {
            asin: Some(MatchBasis::Fuzzy),
            ..AnchorProvenance::default()
        }),
    ));
    let _ = settle_identity(
        &fuzzy_resolver,
        &db,
        user_id,
        &fuzzy,
        IdentityMode::Background,
        ConflictSource::Convergence,
    )
    .await
    .expect("settle fuzzy-only guess");
    let fuzzy_after = db.get_work(user_id, fuzzy.id).await.expect("read fuzzy");
    let fuzzy_anchors = db.list_anchors(fuzzy.id).await.expect("list fuzzy anchors");
    // FLM: title + author match → ASIN is synced to works.asin and badge raises.
    assert_eq!(
        fuzzy_after.asin.as_deref(),
        Some("B000FUZZY1"),
        "FLM pass: ASIN synced to works.asin"
    );
    assert_eq!(
        fuzzy_after.identity_status,
        IdentityStatus::Provisional,
        "FLM pass: ASIN bridge raises badge to Provisional"
    );
    assert!(fuzzy_anchors.iter().any(|a| {
        a.anchor_type.as_str() == AnchorType::ASIN
            && a.anchor_value == "B000FUZZY1"
            && a.confidence == AnchorConfidence::Confirmed
    }));
}
