//! Door-gate behavioral suite for `design-door-gate.md`.
//!
//! Exclusions, verbatim from the design:
//! - `manual_import.rs::import` and `list_import.rs::confirm` handler bodies — composite handler
//!   contexts (multipart/preview machinery, many services). Their work-creation halves are covered
//!   at Layers A/B (B2/B3, A3-conditional); the manual-import +5s refresh chain (manual_import.rs:886-895)
//!   has the same shape as C1's chain tail. A future incident in those handler bodies is NOT caught
//!   by this suite — named residual.
//! - Readarr `ImportRunner` — private, one production construction site (F11); covered by the
//!   documented stand-in (B6). No handler/job row possible without a production visibility change.
//! - `source_provider_data` — not observable at the workflow seam (F10). Pinning it would need a
//!   deeper spy at `run_unified_enrichment` (a WorkServiceImpl-internal seam) or a full
//!   ProviderQueue-level fixture; out of BUILD-LIGHT scope. Recorded, not asserted.
//! - `IdentityMode`/`ConflictSource` per door — identity-side parameters (settle_identity), invisible
//!   to the enrichment spy at Layer B. Layer C's C1 row pins the literals the add handler threads
//!   (work.rs:262-269); Layer A pins the provenance values that drive the derivation (F7/F8) for the
//!   batch doors.
//!
//! Conditional arms taken:
//! - A2 series-monitor worker: constructible in the behavioral crate via
//!   `SeriesQueryServiceImpl::new(SqliteDb, StubHttpFetcher, Arc<RecordingWorkService>, StubNoLlm)`;
//!   the real worker arm is used.
//! - A3 list import confirm: constructible in the behavioral crate via
//!   `ListServiceImpl::new(SqliteDb, RecordingWorkService, StubHttpFetcher, NoOpBibliographyTrigger)`;
//!   the real confirm arm is used.
//! - A4 Readarr: no row, matching the Readarr precedent; the private `ImportRunner` is represented
//!   by B6's Readarr-shaped candidate through `WorkService::add`.
//!
//! Convention: a new R1/R2 door is not done until its row exists here.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::Utc;
use livrarr_behavioral::stubs::{create_test_user, StubEnrichmentWorkflow, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    AuthorDb, CreateAuthorDbRequest, CreateSeriesDbRequest, CreateWorkDbRequest, ListImportDb,
    SeriesDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::identity::{
    AnchorConfidence, AnchorDeadEnd, AnchorSetter, AnchorType, Candidate, CandidateId,
    CapturedIdentity, ConflictSource, ConsistencyDivergence, IdentityMethod, IdentityMode,
    IdentityState, LatencyTier, NewIdentityConflict, PendingReason, RawHarvest, Resolution,
    ResolvedIdentity, WorkCandidate, WorkIdentityAnchor,
};
use livrarr_domain::seed::{
    seed_add_box, seed_author_monitor, seed_list_import, seed_manual_import, seed_readarr_import,
    seed_series_monitor, SeedInput, SeedLanguage,
};
use livrarr_domain::services::*;
use livrarr_domain::{
    normalize_for_matching, AuthType, Author, AuthorId, DbError, EnrichmentStatus, Freshness,
    IdentityStatus, LibraryItem, MediaType, ProvenanceSetter, RequestPriority, User, UserId,
    UserRole, Work, WorkId,
};
use livrarr_handlers::context::{
    HasAppConfigService, HasAuthorService, HasEnrichmentWorkflow, HasHistoryService,
    HasIdentityResolver, HasNotificationService, HasSeriesQueryService, HasTagService,
    HasWorkIdentityRepository, HasWorkService,
};
use livrarr_handlers::work::RefreshAllParams;
use livrarr_handlers::work::{affirm_pending_anchor, refresh, refresh_all, retry_all_incomplete};
use livrarr_handlers::AuthContext;
use livrarr_metadata::author_monitor_workflow::AuthorMonitorWorkflowImpl;
use livrarr_metadata::discovery_service::StubNoLlm;
use livrarr_metadata::list_service::{ListServiceImpl, NoOpBibliographyTrigger};
use livrarr_metadata::series_query_service::SeriesQueryServiceImpl;
use livrarr_metadata::work_service::WorkServiceImpl;
use tokio_util::sync::CancellationToken;

type TestWorkService = WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher>;

fn test_data_dir(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("livrarr-door-gate-{label}-{}", std::process::id()))
}

fn real_work_service(db: SqliteDb, workflow: StubEnrichmentWorkflow) -> TestWorkService {
    WorkServiceImpl::new(db, workflow, StubHttpFetcher::new(), test_data_dir("work"))
}

fn seed_input(title: &str, author: &str) -> SeedInput {
    SeedInput {
        title: title.to_string(),
        author_name: author.to_string(),
        language: SeedLanguage::resolve(Some("en"), "en"),
        author_ol_key: None,
        year: Some(2026),
        cover_url: None,
        detail_url: None,
        description: None,
        series_name: None,
        series_position: None,
    }
}

fn captured(title: &str, author: &str, ol_key: Option<&str>) -> CapturedIdentity {
    CapturedIdentity {
        ol_key: ol_key.map(str::to_string),
        gr_key: None,
        hc_key: None,
        isbn_13: None,
        asin: None,
        title: title.to_string(),
        author_name: author.to_string(),
        language: Some("en".to_string()),
    }
}

fn confirmed_identity(title: &str, author: &str, ol_key: &str) -> IdentityState {
    IdentityState::Confirmed {
        anchors: captured(title, author, Some(ol_key)),
        method: IdentityMethod::UserSelected,
        score: None,
    }
}

fn pending_identity(title: &str, author: &str) -> IdentityState {
    IdentityState::Pending {
        reason: PendingReason::NoCandidates,
        seed_anchors: Some(captured(title, author, None)),
        top_candidates: vec![],
    }
}

fn work_req(
    user_id: UserId,
    title: &str,
    author: &str,
    ol_key: Option<&str>,
) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: author.to_string(),
        normalized_title: normalize_for_matching(title),
        normalized_author: normalize_for_matching(author),
        language: Some("en".to_string()),
        ol_key: ol_key.map(str::to_string),
        monitor_ebook: true,
        monitor_audiobook: false,
        ..Default::default()
    }
}

async fn seed_db_work(
    db: &SqliteDb,
    user_id: UserId,
    title: &str,
    author: &str,
    ol_key: Option<&str>,
    status: EnrichmentStatus,
) -> Work {
    let (mut work, created) = db
        .create_work(work_req(user_id, title, author, ol_key))
        .await
        .expect("seed work");
    assert!(created);
    db.update_work_enrichment(
        user_id,
        work.id,
        livrarr_db::UpdateWorkEnrichmentDbRequest {
            enrichment_status: status,
            enrichment_source: Some("fixture".to_string()),
            title: None,
            author_name: None,
            year: None,
            language: None,
            description: None,
            publisher: None,
            genres: None,
            page_count: None,
            duration_seconds: None,
            rating: None,
            rating_count: None,
            cover_url: None,
            series_name: None,
            series_position: None,
            subtitle: None,
            original_title: None,
            narration_type: None,
            publish_date: None,
            narrator: None,
            abridged: None,
        },
    )
    .await
    .expect("set enrichment status");
    db.set_identity_status(user_id, work.id, IdentityStatus::Confirmed)
        .await
        .expect("set identity status");
    work.enrichment_status = status;
    work.identity_status = IdentityStatus::Confirmed;
    work
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExpectedSeam {
    contexts: Vec<(EnrichmentMode, RequestPriority)>,
    freshness: Vec<Freshness>,
    candidate_ids: Vec<Option<CandidateId>>,
}

async fn assert_seam<F, Fut>(row: &str, drive: F, expected: ExpectedSeam)
where
    F: FnOnce(SqliteDb, UserId, TestWorkService, StubEnrichmentWorkflow) -> Fut,
    Fut: std::future::Future<Output = Vec<WorkId>>,
{
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let workflow = StubEnrichmentWorkflow::succeeding();
    let service = real_work_service(db.clone(), workflow.clone());

    let expected_work_ids = drive(db, user_id, service, workflow.clone()).await;

    assert_eq!(workflow.enrich_contexts(), expected.contexts, "{row}");
    assert_eq!(workflow.freshness_calls(), expected.freshness, "{row}");
    assert_eq!(workflow.candidate_ids(), expected.candidate_ids, "{row}");
    assert_eq!(workflow.work_ids(), expected_work_ids, "{row} work_ids");
}

fn canonical_expected(candidate_id: Option<CandidateId>) -> ExpectedSeam {
    ExpectedSeam {
        contexts: vec![(EnrichmentMode::Background, RequestPriority::High)],
        freshness: vec![Freshness::PreferCache],
        candidate_ids: vec![candidate_id],
    }
}

fn no_calls() -> ExpectedSeam {
    ExpectedSeam {
        contexts: vec![],
        freshness: vec![],
        candidate_ids: vec![],
    }
}

#[tokio::test]
async fn b1_add_box_threads_canonical_background_high_prefer_cache() {
    let cid = CandidateId("B1".to_string());
    assert_seam(
        "B1",
        move |_db, user_id, service, _workflow| {
            let cid = cid.clone();
            async move {
                let result = service
                    .add(
                        user_id,
                        seed_add_box(
                            seed_input("B1 Add Box", "Door Author"),
                            confirmed_identity("B1 Add Box", "Door Author", "OLB1W"),
                            Some(cid),
                            false,
                        ),
                    )
                    .await
                    .expect("add box");
                vec![result.work.id]
            }
        },
        canonical_expected(Some(CandidateId("B1".to_string()))),
    )
    .await;
}

#[tokio::test]
async fn b2_manual_import_threads_canonical_background_high_prefer_cache() {
    let cid = CandidateId("B2".to_string());
    assert_seam(
        "B2",
        move |_db, user_id, service, _workflow| {
            let cid = cid.clone();
            async move {
                let result = service
                    .add(
                        user_id,
                        seed_manual_import(
                            seed_input("B2 Manual", "Door Author"),
                            confirmed_identity("B2 Manual", "Door Author", "OLB2W"),
                            Some(cid),
                        ),
                    )
                    .await
                    .expect("manual import add");
                vec![result.work.id]
            }
        },
        canonical_expected(Some(CandidateId("B2".to_string()))),
    )
    .await;
}

#[tokio::test]
async fn b3_list_import_threads_canonical_none_candidate() {
    assert_seam(
        "B3",
        |_db, user_id, service, _workflow| async move {
            let result = service
                .add(
                    user_id,
                    seed_list_import(
                        seed_input("B3 List", "Door Author"),
                        confirmed_identity("B3 List", "Door Author", "OLB3W"),
                        None,
                    ),
                )
                .await
                .expect("list import add");
            vec![result.work.id]
        },
        canonical_expected(None),
    )
    .await;
}

#[tokio::test]
async fn b4_author_monitor_threads_canonical_none_candidate() {
    assert_seam(
        "B4",
        |_db, user_id, service, _workflow| async move {
            let result = service
                .add(
                    user_id,
                    seed_author_monitor(
                        seed_input("B4 Author", "Door Author"),
                        confirmed_identity("B4 Author", "Door Author", "OLB4W"),
                    ),
                )
                .await
                .expect("author monitor add");
            vec![result.work.id]
        },
        canonical_expected(None),
    )
    .await;
}

#[tokio::test]
async fn b5_series_monitor_threads_canonical_and_persists_series_flags() {
    assert_seam(
        "B5",
        |db, user_id, service, _workflow| async move {
            let (author, _) = db
                .create_author(CreateAuthorDbRequest {
                    user_id,
                    name: "Door Author".to_string(),
                    sort_name: None,
                    ol_key: None,
                    gr_key: None,
                    hc_key: None,
                    import_id: None,
                })
                .await
                .expect("B5 author");
            let series = db
                .upsert_series(CreateSeriesDbRequest {
                    user_id,
                    author_id: author.id,
                    name: "Door Series".to_string(),
                    gr_key: "B5S".to_string(),
                    monitor_ebook: true,
                    monitor_audiobook: false,
                    monitor_language: Some("en".to_string()),
                    work_count: 1,
                })
                .await
                .expect("B5 series");
            let result = service
                .add(
                    user_id,
                    seed_series_monitor(
                        SeedInput {
                            series_name: Some("Door Series".to_string()),
                            series_position: Some(1.0),
                            ..seed_input("B5 Series", "Door Author")
                        },
                        confirmed_identity("B5 Series", "Door Author", "OLB5W"),
                        series.id,
                        true,
                        false,
                    ),
                )
                .await
                .expect("series monitor add");
            assert_eq!(result.work.series_id, Some(series.id), "B5 series id");
            assert!(result.work.monitor_ebook, "B5 ebook flag");
            assert!(!result.work.monitor_audiobook, "B5 audiobook flag");
            vec![result.work.id]
        },
        canonical_expected(None),
    )
    .await;
}

#[tokio::test]
async fn b6_readarr_stand_in_threads_canonical_none_candidate() {
    assert_seam(
        "B6",
        |db, user_id, service, _workflow| async move {
            db.create_list_import_record(
                "readarr-import",
                user_id,
                "readarr",
                &Utc::now().to_rfc3339(),
            )
            .await
            .expect("B6 import record");
            let result = service
                .add(
                    user_id,
                    seed_readarr_import(
                        seed_input("B6 Readarr", "Door Author"),
                        confirmed_identity("B6 Readarr", "Door Author", "OLB6W"),
                        SourceProviderData::default(),
                        true,
                        false,
                        "readarr-import".to_string(),
                    ),
                )
                .await
                .expect("readarr-shaped add");
            vec![result.work.id]
        },
        canonical_expected(None),
    )
    .await;
}

#[tokio::test]
async fn b7_add_anchor_dedup_to_enriched_target_skips_enrichment() {
    assert_seam(
        "B7",
        |db, user_id, service, workflow| async move {
            let _target = seed_db_work(
                &db,
                user_id,
                "B7 Dedup",
                "Door Author",
                Some("OLB7W"),
                EnrichmentStatus::Enriched,
            )
            .await;
            service
                .add(
                    user_id,
                    seed_add_box(
                        seed_input("B7 Dedup", "Door Author"),
                        confirmed_identity("B7 Dedup", "Door Author", "OLB7W"),
                        None,
                        false,
                    ),
                )
                .await
                .expect("dedup add");
            assert_eq!(workflow.call_count(), 0, "B7 needs-gate");
            vec![]
        },
        no_calls(),
    )
    .await;
}

#[tokio::test]
async fn b8_add_anchor_dedup_to_unenriched_target_runs_ensure() {
    assert_seam(
        "B8",
        |db, user_id, service, _workflow| async move {
            let target = seed_db_work(
                &db,
                user_id,
                "B8 Dedup",
                "Door Author",
                Some("OLB8W"),
                EnrichmentStatus::Unenriched,
            )
            .await;
            let result = service
                .add(
                    user_id,
                    seed_add_box(
                        seed_input("B8 Dedup", "Door Author"),
                        confirmed_identity("B8 Dedup", "Door Author", "OLB8W"),
                        None,
                        false,
                    ),
                )
                .await
                .expect("dedup add");
            assert_eq!(
                result.work.id, target.id,
                "B8 dedup must adopt the existing work"
            );
            vec![target.id]
        },
        canonical_expected(None),
    )
    .await;
}

#[tokio::test]
async fn b9_refresh_interactive_threads_manual_normal_bypass() {
    assert_seam(
        "B9",
        |db, user_id, service, _workflow| async move {
            let work = seed_db_work(
                &db,
                user_id,
                "B9 Refresh",
                "Door Author",
                Some("OLB9W"),
                EnrichmentStatus::Unenriched,
            )
            .await;
            service
                .refresh(user_id, work.id, RefreshSurface::Interactive)
                .await
                .expect("interactive refresh");
            vec![work.id]
        },
        ExpectedSeam {
            contexts: vec![(EnrichmentMode::Manual, RequestPriority::Normal)],
            freshness: vec![Freshness::Bypass],
            candidate_ids: vec![None],
        },
    )
    .await;
}

#[tokio::test]
async fn b10_refresh_bulk_threads_manual_low_bypass() {
    assert_seam(
        "B10",
        |db, user_id, service, _workflow| async move {
            let work = seed_db_work(
                &db,
                user_id,
                "B10 Refresh",
                "Door Author",
                Some("OLB10W"),
                EnrichmentStatus::Unenriched,
            )
            .await;
            service
                .refresh(user_id, work.id, RefreshSurface::Bulk)
                .await
                .expect("bulk refresh");
            vec![work.id]
        },
        ExpectedSeam {
            contexts: vec![(EnrichmentMode::Manual, RequestPriority::Low)],
            freshness: vec![Freshness::Bypass],
            candidate_ids: vec![None],
        },
    )
    .await;
}

#[tokio::test]
async fn b11_retry_all_incomplete_rides_bulk_refresh() {
    assert_seam(
        "B11",
        |db, user_id, service, _workflow| async move {
            let work = seed_db_work(
                &db,
                user_id,
                "B11 Retry",
                "Door Author",
                Some("OLB11W"),
                EnrichmentStatus::Failed,
            )
            .await;
            service
                .retry_all_incomplete(user_id)
                .await
                .expect("retry all incomplete");
            vec![work.id]
        },
        ExpectedSeam {
            contexts: vec![(EnrichmentMode::Manual, RequestPriority::Low)],
            freshness: vec![Freshness::Bypass],
            candidate_ids: vec![None],
        },
    )
    .await;
}

#[tokio::test]
async fn b12_converge_work_runs_background_low_prefer_cache_for_unenriched() {
    assert_seam(
        "B12",
        |db, user_id, service, _workflow| async move {
            let work = seed_db_work(
                &db,
                user_id,
                "B12 Converge",
                "Door Author",
                Some("OLB12W"),
                EnrichmentStatus::Unenriched,
            )
            .await;
            service
                .converge_work(user_id, work.id, 3)
                .await
                .expect("converge");
            vec![work.id]
        },
        ExpectedSeam {
            contexts: vec![(EnrichmentMode::Background, RequestPriority::Low)],
            freshness: vec![Freshness::PreferCache],
            candidate_ids: vec![None],
        },
    )
    .await;
}

#[tokio::test]
async fn b13_converge_work_on_enriched_work_skips_enrichment() {
    assert_seam(
        "B13",
        |db, user_id, service, _workflow| async move {
            let work = seed_db_work(
                &db,
                user_id,
                "B13 Converge",
                "Door Author",
                Some("OLB13W"),
                EnrichmentStatus::Enriched,
            )
            .await;
            service
                .converge_work(user_id, work.id, 3)
                .await
                .expect("converge");
            vec![]
        },
        no_calls(),
    )
    .await;
}

#[tokio::test]
async fn b14_add_identity_pending_candidate_blocks_enrichment() {
    assert_seam(
        "B14",
        |_db, user_id, service, _workflow| async move {
            let mut bridge_anchors = captured("B14 Pending", "Door Author", None);
            bridge_anchors.isbn_13 = Some("9780441172719".to_string());
            service
                .add(
                    user_id,
                    seed_add_box(
                        seed_input("B14 Pending", "Door Author"),
                        IdentityState::Pending {
                            reason: PendingReason::NoCandidates,
                            seed_anchors: Some(bridge_anchors),
                            top_candidates: vec![],
                        },
                        None,
                        false,
                    ),
                )
                .await
                .expect("pending add");
            vec![]
        },
        no_calls(),
    )
    .await;
}

#[derive(Debug, Clone)]
enum WorkCall {
    ResolveIdentityLocal,
    Add(UserId, Box<WorkCandidate>),
    AddFast(UserId),
    CompleteAdd {
        user_id: UserId,
        work_id: WorkId,
        source_provider_data: Option<Box<SourceProviderData>>,
        candidate_id: Option<CandidateId>,
        mode: IdentityMode,
        source: ConflictSource,
    },
    Refresh(UserId, WorkId, RefreshSurface),
    RetryAllIncomplete(UserId),
    TryStartBulkRefresh(UserId),
    Get,
}

#[derive(Clone)]
struct RecordingWorkService {
    calls: Arc<Mutex<Vec<WorkCall>>>,
    listed_works: Arc<Mutex<Vec<Work>>>,
    next_work_id: WorkId,
    add_created: bool,
    author_created: bool,
    bulk_slot_available: Arc<AtomicBool>,
}

impl RecordingWorkService {
    fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            listed_works: Arc::new(Mutex::new(Vec::new())),
            next_work_id: 7001,
            add_created: true,
            author_created: false,
            bulk_slot_available: Arc::new(AtomicBool::new(true)),
        }
    }

    fn with_listed_works(works: Vec<Work>) -> Self {
        let this = Self::new();
        *this.listed_works.lock().expect("listed works") = works;
        this
    }

    async fn calls(&self) -> Vec<WorkCall> {
        self.calls.lock().expect("work calls").clone()
    }

    async fn add_calls(&self) -> Vec<WorkCandidate> {
        self.calls
            .lock()
            .expect("work calls")
            .iter()
            .filter_map(|call| match call {
                WorkCall::Add(user_id, candidate) => {
                    let _ = user_id;
                    Some((**candidate).clone())
                }
                _ => None,
            })
            .collect()
    }

    async fn refresh_calls(&self) -> Vec<(UserId, WorkId, RefreshSurface)> {
        self.calls
            .lock()
            .expect("work calls")
            .iter()
            .filter_map(|call| match call {
                WorkCall::Refresh(user_id, work_id, surface) => {
                    Some((*user_id, *work_id, *surface))
                }
                _ => None,
            })
            .collect()
    }
}

impl WorkService for RecordingWorkService {
    async fn add(
        &self,
        user_id: UserId,
        candidate: WorkCandidate,
    ) -> Result<AddWorkResult, WorkServiceError> {
        self.calls
            .lock()
            .expect("work calls")
            .push(WorkCall::Add(user_id, Box::new(candidate)));
        Ok(AddWorkResult {
            work: Work {
                id: self.next_work_id,
                user_id,
                ..Work::default()
            },
            created: self.add_created,
            author_created: self.author_created,
            author_id: None,
            messages: vec![],
            cover_mtime: None,
            audiobook_cover_mtime: None,
            enrichment_status: EnrichmentStatus::Enriched,
        })
    }

    async fn resolve_identity(
        &self,
        _user_id: UserId,
        harvest: RawHarvest,
        _tier: LatencyTier,
    ) -> Result<ResolvedIdentity, WorkServiceError> {
        Ok(ResolvedIdentity {
            identity: pending_identity(
                harvest.title.as_deref().unwrap_or("pending"),
                harvest.author_name.as_deref().unwrap_or("pending"),
            ),
            candidate_id: None,
            language: harvest.language,
            conflict: None,
        })
    }

    fn resolve_identity_local(
        &self,
        harvest: RawHarvest,
    ) -> Result<ResolvedIdentity, WorkServiceError> {
        let title = harvest.title.clone().unwrap_or_else(|| "local".to_string());
        let author = harvest
            .author_name
            .clone()
            .unwrap_or_else(|| "local".to_string());
        self.calls
            .lock()
            .expect("work calls")
            .push(WorkCall::ResolveIdentityLocal);
        Ok(ResolvedIdentity {
            identity: confirmed_identity(
                &title,
                &author,
                harvest.ol_key.as_deref().unwrap_or("OLLOCALW"),
            ),
            candidate_id: None,
            language: harvest.language,
            conflict: None,
        })
    }

    async fn add_fast(
        &self,
        user_id: UserId,
        candidate: WorkCandidate,
    ) -> Result<AddWorkResult, WorkServiceError> {
        self.calls
            .lock()
            .expect("work calls")
            .push(WorkCall::AddFast(user_id));
        let _ = candidate;
        Ok(AddWorkResult {
            work: Work {
                id: self.next_work_id,
                user_id,
                title: "handler add".to_string(),
                author_name: "handler author".to_string(),
                ..Work::default()
            },
            created: self.add_created,
            author_created: self.author_created,
            author_id: None,
            messages: vec![],
            cover_mtime: None,
            audiobook_cover_mtime: None,
            enrichment_status: EnrichmentStatus::Unenriched,
        })
    }

    async fn complete_add(
        &self,
        user_id: UserId,
        work_id: WorkId,
        source_provider_data: Option<SourceProviderData>,
        candidate_id: Option<CandidateId>,
        mode: IdentityMode,
        source: ConflictSource,
    ) {
        self.calls
            .lock()
            .expect("work calls")
            .push(WorkCall::CompleteAdd {
                user_id,
                work_id,
                source_provider_data: source_provider_data.map(Box::new),
                candidate_id,
                mode,
                source,
            });
    }

    fn is_enriching(&self, _user_id: UserId, _work_id: WorkId) -> bool {
        false
    }

    async fn get(&self, user_id: UserId, work_id: WorkId) -> Result<Work, WorkServiceError> {
        let _ = (user_id, work_id);
        self.calls.lock().expect("work calls").push(WorkCall::Get);
        Ok(Work {
            id: work_id,
            user_id,
            ..Work::default()
        })
    }

    async fn get_detail(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<WorkDetailView, WorkServiceError> {
        Ok(WorkDetailView {
            work: Work {
                id: work_id,
                user_id,
                ..Work::default()
            },
            library_items: vec![],
            cover_mtime: None,
            audiobook_cover_mtime: None,
        })
    }

    async fn list(
        &self,
        _user_id: UserId,
        _filter: WorkFilter,
    ) -> Result<Vec<Work>, WorkServiceError> {
        Ok(self.listed_works.lock().expect("listed works").clone())
    }

    async fn list_paginated(
        &self,
        _user_id: UserId,
        _page: u32,
        _page_size: u32,
        _sort_by: WorkSortField,
        _sort_dir: SortDirection,
        _media_type: Option<MediaType>,
        _language: Option<&str>,
    ) -> Result<PaginatedWorksView, WorkServiceError> {
        todo!("not exercised by door-gate")
    }

    async fn update(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _req: livrarr_domain::services::UpdateWorkRequest,
    ) -> Result<Work, WorkServiceError> {
        todo!("not exercised by door-gate")
    }

    async fn delete(&self, _user_id: UserId, _work_id: WorkId) -> Result<(), WorkServiceError> {
        Ok(())
    }

    async fn refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
        surface: RefreshSurface,
    ) -> Result<RefreshWorkResult, WorkServiceError> {
        self.calls
            .lock()
            .expect("work calls")
            .push(WorkCall::Refresh(user_id, work_id, surface));
        Ok(RefreshWorkResult {
            work: Work {
                id: work_id,
                user_id,
                ..Work::default()
            },
            messages: vec![],
            taggable_items: vec![],
            merge_deferred: false,
        })
    }

    async fn retry_all_incomplete(
        &self,
        user_id: UserId,
    ) -> Result<RetrySummary, WorkServiceError> {
        self.calls
            .lock()
            .expect("work calls")
            .push(WorkCall::RetryAllIncomplete(user_id));
        Ok(RetrySummary {
            total: 1,
            recovered: 1,
            still_incomplete: 0,
        })
    }

    async fn upload_cover(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _bytes: &[u8],
    ) -> Result<(), WorkServiceError> {
        todo!("not exercised by door-gate")
    }

    async fn download_cover(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
    ) -> Result<Vec<u8>, WorkServiceError> {
        todo!("not exercised by door-gate")
    }

    async fn search_works(
        &self,
        _user_id: UserId,
        _query: &str,
        _page: u32,
        _page_size: u32,
    ) -> Result<(Vec<Work>, i64), WorkServiceError> {
        todo!("not exercised by door-gate")
    }

    fn try_start_bulk_refresh(&self, user_id: UserId) -> Option<BulkRefreshGuard> {
        self.calls
            .lock()
            .expect("work calls")
            .push(WorkCall::TryStartBulkRefresh(user_id));
        self.bulk_slot_available
            .swap(false, Ordering::SeqCst)
            .then_some(BulkRefreshGuard::new(
                Arc::new(Mutex::new(std::collections::HashSet::new())),
                user_id,
            ))
    }

    async fn converge_work(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _threshold: u32,
    ) -> Result<ConvergeOutcome, WorkServiceError> {
        Ok(ConvergeOutcome::Completed)
    }

    async fn preview_merge_works(
        &self,
        _user_id: UserId,
        _survivor_id: WorkId,
        _loser_id: WorkId,
    ) -> Result<MergePreview, WorkServiceError> {
        todo!("not exercised by door-gate")
    }

    async fn merge_works(
        &self,
        _user_id: UserId,
        _survivor_id: WorkId,
        _loser_id: WorkId,
        _choices: Vec<MergeFieldChoiceEntry>,
    ) -> Result<MergeWorksResult, WorkServiceError> {
        todo!("not exercised by door-gate")
    }
}

const OL_WORKS_JSON: &str = r#"{
  "entries": [
    {"key": "/works/OLA1W", "title": "A1 Eligible", "first_publish_date": "2026"}
  ]
}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a1_author_monitor_adds_auto_added_pending_work_candidate() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author, _) = db
        .create_author(CreateAuthorDbRequest {
            user_id,
            name: "A1 Author".to_string(),
            sort_name: None,
            ol_key: Some("OL1001A".to_string()),
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("author");
    // The monitor reads the route ledger; startup ingestion gives every migrated
    // author this route before anything is served.
    livrarr_db::AuthorLinkDb::attach_route_as_user(
        &db,
        user_id,
        author.id,
        livrarr_domain::AuthorRouteKey::parse(
            livrarr_domain::AuthorProvider::OpenLibrary,
            "OL1001A",
        )
        .expect("canonical OL key"),
    )
    .await
    .expect("seeded OpenLibrary route");
    db.update_author(
        user_id,
        author.id,
        livrarr_db::UpdateAuthorDbRequest {
            name: None,
            sort_name: None,
            ol_key: None,
            gr_key: None,
            monitored: Some(true),
            monitor_new_items: Some(true),
            monitor_since: None,
            monitor_language: None,
        },
    )
    .await
    .expect("monitor author");

    let work_service = Arc::new(RecordingWorkService::new());
    let workflow = AuthorMonitorWorkflowImpl::new(
        Arc::new(db),
        work_service.clone(),
        Arc::new(StubHttpFetcher::with_ok(
            200,
            OL_WORKS_JSON.as_bytes().to_vec(),
        )),
    );
    workflow
        .run_monitor(user_id, CancellationToken::new())
        .await
        .expect("run monitor");

    let adds = work_service.add_calls().await;
    assert_eq!(adds.len(), 1, "A1 exactly one eligible work");
    let candidate = &adds[0];
    assert_eq!(
        candidate.provenance_setter,
        Some(ProvenanceSetter::AutoAdded),
        "A1 provenance"
    );
    assert_eq!(candidate.candidate_id, None, "A1 candidate_id");
    assert_eq!(
        candidate
            .identity
            .seed_or_confirmed_anchors()
            .and_then(|a| a.ol_key.as_deref()),
        Some("OLA1W"),
        "A1 identity carries the bibliography entry's OL work key"
    );
}

const SERIES_ROSTER_HTML: &str = r#"<html><body>
<div data-react-class="ReactComponents.SeriesHeader" data-react-props="{&quot;title&quot;:&quot;Door Series&quot;,&quot;subtitle&quot;:&quot;1 primary works • 1 total works&quot;,&quot;description&quot;:{&quot;html&quot;:&quot;&quot;}}"></div>
<div data-react-class="ReactComponents.SeriesList" data-react-props="{&quot;series&quot;:[{&quot;book&quot;:{&quot;bookId&quot;:&quot;47212&quot;,&quot;title&quot;:&quot;A2 Roster Book (Door Series, #1)&quot;,&quot;bookTitleBare&quot;:&quot;A2 Roster Book&quot;,&quot;publicationDate&quot;:&quot;2026&quot;}}]}"></div>
<div data-react-class="ReactComponents.FullPagePaginationControls" data-react-props="{&quot;numWorks&quot;:1,&quot;currentPageNumber&quot;:1,&quot;perPage&quot;:100}"></div>
</body></html>"#;

#[tokio::test]
async fn a2_series_monitor_worker_constructible_arm_adds_auto_added_series_candidate() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author, _) = db
        .create_author(CreateAuthorDbRequest {
            user_id,
            name: "A2 Author".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: Some("A2A".to_string()),
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("author");
    let series = db
        .upsert_series(CreateSeriesDbRequest {
            user_id,
            author_id: author.id,
            name: "Door Series".to_string(),
            gr_key: "A2S".to_string(),
            monitor_ebook: true,
            monitor_audiobook: false,
            monitor_language: Some("en".to_string()),
            work_count: 1,
        })
        .await
        .expect("series");
    let work_service = Arc::new(RecordingWorkService::new());
    let service = SeriesQueryServiceImpl::new(
        db,
        StubHttpFetcher::with_ok(200, SERIES_ROSTER_HTML.as_bytes().to_vec()),
        work_service.clone(),
        StubNoLlm,
    );

    service
        .run_series_monitor_worker(SeriesMonitorWorkerParams {
            cancel: tokio_util::sync::CancellationToken::new(),
            user_id,
            author_id: author.id,
            series_id: series.id,
            series_name: series.name.clone(),
            series_gr_key: series.gr_key.clone(),
            monitor_ebook: true,
            monitor_audiobook: false,
        })
        .await
        .expect("series worker");

    let adds = work_service.add_calls().await;
    assert_eq!(adds.len(), 1, "A2 one roster gap");
    let candidate = &adds[0];
    assert_eq!(
        candidate.provenance_setter,
        Some(ProvenanceSetter::AutoAdded)
    );
    assert_eq!(candidate.series_id, Some(series.id));
    assert_eq!(candidate.monitor_ebook, Some(true));
    assert_eq!(candidate.monitor_audiobook, Some(false));
}

#[tokio::test]
async fn a3_list_confirm_constructible_arm_adds_imported_candidate() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_service = RecordingWorkService::new();
    let service = ListServiceImpl::new(
        db,
        work_service.clone(),
        StubHttpFetcher::new(),
        NoOpBibliographyTrigger,
    );
    let preview = service
        .preview(
            user_id,
            b"Book Id,Title,Author,ISBN,ISBN13,My Rating,Exclusive Shelf\n1,A3 List,A3 Author,=\"\",=\"9780441172719\",5,read\n".to_vec(),
        )
        .await
        .expect("preview");
    service
        .confirm(user_id, &preview.preview_id, None, &[0], Some("en"))
        .await
        .expect("confirm");

    let adds = work_service.add_calls().await;
    assert_eq!(adds.len(), 1, "A3 one confirmed row");
    assert_eq!(
        adds[0].provenance_setter,
        Some(ProvenanceSetter::Imported),
        "A3 provenance"
    );
}

#[derive(Clone)]
struct InertAuthorService;

impl AuthorService for InertAuthorService {
    async fn add(
        &self,
        _user_id: UserId,
        _req: AddAuthorRequest,
    ) -> Result<AddAuthorResult, AuthorServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn merge(
        &self,
        _user_id: UserId,
        _survivor_id: AuthorId,
        _loser_id: AuthorId,
    ) -> Result<AuthorMergeReport, AuthorServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn get(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
    ) -> Result<Author, AuthorServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn list(&self, _user_id: UserId) -> Result<Vec<Author>, AuthorServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn update(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _req: livrarr_domain::services::UpdateAuthorRequest,
    ) -> Result<Author, AuthorServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn delete(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
    ) -> Result<(), AuthorServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn lookup(
        &self,
        _query: &str,
        _limit: u32,
    ) -> Result<Vec<AuthorLookupResult>, AuthorServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn bibliography(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _raw: bool,
    ) -> Result<BibliographyResult, AuthorServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn refresh_bibliography(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
    ) -> Result<BibliographyResult, AuthorServiceError> {
        todo!("not exercised by door-gate")
    }
    fn spawn_bibliography_refresh(&self, _author_id: i64, _user_id: i64) {}
    async fn lookup_authors(
        &self,
        _term: &str,
        _limit: u32,
    ) -> Result<Vec<AuthorLookupResult>, AuthorServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn rename(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _name: String,
    ) -> Result<Author, AuthorServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn select_name_variant(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _variant_id: i64,
    ) -> Result<Author, AuthorServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn set_monitoring(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _monitored: bool,
        _monitor_new_items: Option<bool>,
        _monitor_language: Option<String>,
    ) -> Result<Author, AuthorServiceError> {
        todo!("not exercised by door-gate")
    }
}

#[derive(Clone)]
struct InertSeriesQueryService;

impl SeriesQueryService for InertSeriesQueryService {
    async fn list_enriched(
        &self,
        _user_id: UserId,
    ) -> Result<Vec<SeriesListView>, SeriesServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn get_detail(
        &self,
        _user_id: UserId,
        _series_id: i64,
    ) -> Result<SeriesDetailView, SeriesServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn update_flags(
        &self,
        _user_id: UserId,
        _series_id: i64,
        _monitor_ebook: bool,
        _monitor_audiobook: bool,
        _language: Option<String>,
    ) -> Result<UpdateSeriesView, SeriesServiceError> {
        todo!("not exercised by door-gate")
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
        todo!("not exercised by door-gate")
    }
    async fn refresh_author_series(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
    ) -> Result<AuthorSeriesListView, SeriesServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn monitor_series(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _req: MonitorSeriesServiceRequest,
    ) -> Result<MonitorSeriesView, SeriesServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn run_series_monitor_worker(
        &self,
        _params: SeriesMonitorWorkerParams,
    ) -> Result<(), SeriesServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn promote_stub(
        &self,
        _user_id: UserId,
        _series_id: i64,
        _explicit_gr_key: Option<String>,
    ) -> Result<PromoteStubOutcome, SeriesServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn series_books(
        &self,
        _user_id: UserId,
        _series_id: i64,
    ) -> Result<SeriesBooksView, SeriesServiceError> {
        todo!("not exercised by door-gate")
    }
}

#[derive(Clone)]
struct InertConfigService;

impl AppConfigService for InertConfigService {
    async fn get_naming_config(&self) -> Result<livrarr_domain::settings::NamingConfig, DbError> {
        todo!("not exercised by door-gate")
    }
    async fn get_media_management_config(
        &self,
    ) -> Result<livrarr_domain::settings::MediaManagementConfig, DbError> {
        todo!("not exercised by door-gate")
    }
    async fn update_media_management_config(
        &self,
        _params: livrarr_domain::settings::UpdateMediaManagementParams,
    ) -> Result<livrarr_domain::settings::MediaManagementConfig, DbError> {
        todo!("not exercised by door-gate")
    }
    async fn get_metadata_config(
        &self,
    ) -> Result<livrarr_domain::settings::MetadataConfig, DbError> {
        todo!("not exercised by door-gate")
    }
    async fn update_metadata_config(
        &self,
        _params: livrarr_domain::settings::UpdateMetadataParams,
    ) -> Result<livrarr_domain::settings::MetadataConfig, DbError> {
        todo!("not exercised by door-gate")
    }
    async fn get_default_language(&self) -> Result<String, DbError> {
        Ok("en".to_string())
    }
    async fn update_default_language(&self, _language: &str) -> Result<String, DbError> {
        todo!("not exercised by door-gate")
    }
    async fn validate_default_language(&self, language: &str) -> Result<String, String> {
        Ok(language.to_string())
    }
    async fn get_email_config(&self) -> Result<livrarr_domain::settings::EmailConfig, DbError> {
        todo!("not exercised by door-gate")
    }
    async fn update_email_config(
        &self,
        _params: livrarr_domain::settings::UpdateEmailParams,
    ) -> Result<livrarr_domain::settings::EmailConfig, DbError> {
        todo!("not exercised by door-gate")
    }
    async fn validate_metadata_languages(
        &self,
        languages: &[String],
        _llm_enabled: Option<bool>,
        _llm_endpoint: Option<&str>,
        _llm_api_key: Option<&str>,
        _llm_model: Option<&str>,
        _google_books_api_key: Option<&str>,
    ) -> Result<Vec<String>, String> {
        Ok(languages.to_vec())
    }
}

#[derive(Clone)]
struct InertNotificationService;

impl NotificationService for InertNotificationService {
    async fn list_paginated(
        &self,
        _user_id: UserId,
        _unread_only: bool,
        _page: u32,
        _page_size: u32,
    ) -> Result<(Vec<livrarr_domain::Notification>, i64), NotificationServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn mark_read(&self, _user_id: UserId, _id: i64) -> Result<(), NotificationServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn dismiss(&self, _user_id: UserId, _id: i64) -> Result<(), NotificationServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn dismiss_all(&self, _user_id: UserId) -> Result<(), NotificationServiceError> {
        todo!("not exercised by door-gate")
    }
    async fn create(
        &self,
        req: CreateNotificationRequest,
    ) -> Result<livrarr_domain::Notification, NotificationServiceError> {
        Ok(livrarr_domain::Notification {
            id: 1,
            user_id: req.user_id,
            notification_type: req.notification_type,
            ref_key: req.ref_key,
            message: req.message,
            data: req.data,
            read: false,
            dismissed: false,
            created_at: Utc::now(),
        })
    }
}

#[derive(Clone)]
struct InertTagService;

impl TagService for InertTagService {
    async fn retag_library_items(
        &self,
        _work: &Work,
        _items: &[LibraryItem],
    ) -> Vec<TagSyncItemResult> {
        vec![]
    }
}

#[derive(Clone)]
struct InertIdentityResolver;

impl IdentityResolver for InertIdentityResolver {
    async fn resolve(
        &self,
        _user_id: UserId,
        _seed: &livrarr_domain::identity::WorkSeed,
        _tier: LatencyTier,
    ) -> Result<Resolution, WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
}

#[derive(Clone)]
struct RecordingIdentityRepo {
    anchors: Arc<Mutex<Vec<WorkIdentityAnchor>>>,
    confirm_count: Arc<AtomicUsize>,
}

impl RecordingIdentityRepo {
    fn with_pending(work_id: WorkId, anchor_type: &str, value: &str) -> Self {
        Self {
            anchors: Arc::new(Mutex::new(vec![WorkIdentityAnchor {
                work_id,
                anchor_type: AnchorType::new(anchor_type),
                anchor_value: value.to_string(),
                confidence: AnchorConfidence::Pending,
                setter: AnchorSetter::AutoSearch,
                set_at: Utc::now(),
                superseded_by: None,
            }])),
            confirm_count: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl WorkIdentityRepository for RecordingIdentityRepo {
    async fn confirm_anchor(
        &self,
        _work_id: WorkId,
        _anchor_type: AnchorType,
        _value: &str,
        _setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn confirm_anchor_and_recompute_badge(
        &self,
        _work_id: WorkId,
        _anchor_type: AnchorType,
        _value: &str,
        _setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError> {
        self.confirm_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn read_anchors_with_generation(
        &self,
        _work_id: WorkId,
    ) -> Result<(i64, Vec<WorkIdentityAnchor>), WorkIdentityError> {
        Ok((0, self.anchors.lock().unwrap().clone()))
    }
    async fn affirm_anchor_claimed(
        &self,
        work_id: WorkId,
        anchor_type: AnchorType,
        value: &str,
        setter: AnchorSetter,
        _expected_generation: i64,
    ) -> Result<(), WorkIdentityError> {
        self.confirm_anchor_and_recompute_badge(work_id, anchor_type, value, setter)
            .await
    }
    async fn set_identity_pending(
        &self,
        _work_id: WorkId,
        _reason: PendingReason,
        _setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn set_needs_review(&self, _work_id: WorkId) -> Result<(), WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn record_review_candidates(
        &self,
        _work_id: WorkId,
        _candidates: &[Candidate],
    ) -> Result<(), WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn get_review_candidates(
        &self,
        _work_id: WorkId,
    ) -> Result<Option<Vec<Candidate>>, WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn list_needs_review_works(
        &self,
        _user_id: UserId,
    ) -> Result<Vec<Work>, WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn apply_review_candidate(
        &self,
        _work_id: WorkId,
        _candidate: &Candidate,
        _setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn dismiss_review(&self, _work_id: WorkId) -> Result<(), WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn set_identity_confirmed(&self, _work_id: WorkId) -> Result<(), WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn set_identity_provisional(&self, _work_id: WorkId) -> Result<(), WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn verify_anchor_cache_consistency(
        &self,
    ) -> Result<Vec<ConsistencyDivergence>, WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn find_work_by_anchor(
        &self,
        _user_id: UserId,
        _anchor_type: &AnchorType,
        _anchor_value: &str,
    ) -> Result<Option<WorkId>, WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn list_anchors(
        &self,
        _work_id: WorkId,
    ) -> Result<Vec<WorkIdentityAnchor>, WorkIdentityError> {
        Ok(self.anchors.lock().expect("anchors").clone())
    }
    async fn merge_missing_anchors(
        &self,
        _work_id: WorkId,
        _incoming: &CapturedIdentity,
    ) -> Result<Vec<AnchorType>, WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn detect_conflicting_anchors(
        &self,
        _existing_work_id: WorkId,
        _incoming: &CapturedIdentity,
        _source: ConflictSource,
    ) -> Result<Vec<NewIdentityConflict>, WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn raise_identity_conflict(
        &self,
        _conflict: NewIdentityConflict,
    ) -> Result<i64, WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn backfill_gr_numeric(&self) -> Result<(), WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn record_pending_anchor(
        &self,
        _work_id: WorkId,
        _anchor_type: AnchorType,
        _value: &str,
    ) -> Result<(), WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn bump_anchor_attempt(
        &self,
        _work_id: WorkId,
        _anchor_type: AnchorType,
    ) -> Result<(), WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn list_anchor_dead_ends(
        &self,
        _work_id: WorkId,
    ) -> Result<Vec<AnchorDeadEnd>, WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn clear_anchor_dead_end(
        &self,
        _work_id: WorkId,
        _anchor_type: AnchorType,
    ) -> Result<(), WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
    async fn clear_anchor_dead_ends(&self, _work_id: WorkId) -> Result<(), WorkIdentityError> {
        todo!("not exercised by door-gate")
    }
}

/// History recording is not under test here — the wh_* suites pin the affirm
/// door's event against a real DB; this double only satisfies the bound.
#[derive(Clone)]
struct InertHistoryService;

impl HistoryService for InertHistoryService {
    async fn list_paginated(
        &self,
        _user_id: UserId,
        _filter: livrarr_domain::HistoryFilter,
        _page: u32,
        _page_size: u32,
    ) -> Result<(Vec<livrarr_domain::HistoryEvent>, i64), HistoryServiceError> {
        Ok((vec![], 0))
    }

    async fn record(&self, _user_id: UserId, _draft: livrarr_domain::history_events::HistoryDraft) {
    }
}

#[derive(Clone)]
struct HandlerState {
    work: RecordingWorkService,
    identity_repo: RecordingIdentityRepo,
    author: InertAuthorService,
    series: InertSeriesQueryService,
    enrichment: StubEnrichmentWorkflow,
    resolver: InertIdentityResolver,
    config: InertConfigService,
    notifications: InertNotificationService,
    tags: InertTagService,
    history: InertHistoryService,
}

impl HandlerState {
    fn new(work: RecordingWorkService) -> Self {
        Self {
            work,
            identity_repo: RecordingIdentityRepo::with_pending(8801, AnchorType::OL_WORK, "OLC5W"),
            author: InertAuthorService,
            series: InertSeriesQueryService,
            enrichment: StubEnrichmentWorkflow::succeeding(),
            resolver: InertIdentityResolver,
            config: InertConfigService,
            notifications: InertNotificationService,
            tags: InertTagService,
            history: InertHistoryService,
        }
    }
}

impl HasWorkService for HandlerState {
    type WorkSvc = RecordingWorkService;
    fn work_service(&self) -> &Self::WorkSvc {
        &self.work
    }
}
impl HasWorkIdentityRepository for HandlerState {
    type WorkIdentityRepo = RecordingIdentityRepo;
    fn work_identity_repo(&self) -> &Self::WorkIdentityRepo {
        &self.identity_repo
    }
}
impl HasAuthorService for HandlerState {
    type AuthorSvc = InertAuthorService;
    fn author_service(&self) -> &Self::AuthorSvc {
        &self.author
    }
}
impl HasSeriesQueryService for HandlerState {
    type SeriesQuerySvc = InertSeriesQueryService;
    fn series_query_service(&self) -> &Self::SeriesQuerySvc {
        &self.series
    }
}
impl HasEnrichmentWorkflow for HandlerState {
    type EnrichmentWf = StubEnrichmentWorkflow;
    fn enrichment_workflow(&self) -> &Self::EnrichmentWf {
        &self.enrichment
    }
}
impl HasIdentityResolver for HandlerState {
    type IdentityResolverSvc = InertIdentityResolver;
    fn identity_resolver(&self) -> &Self::IdentityResolverSvc {
        &self.resolver
    }
}
impl HasAppConfigService for HandlerState {
    type AppConfigSvc = InertConfigService;
    fn app_config_service(&self) -> &Self::AppConfigSvc {
        &self.config
    }
}
impl HasNotificationService for HandlerState {
    type NotificationSvc = InertNotificationService;
    fn notification_service(&self) -> &Self::NotificationSvc {
        &self.notifications
    }
}
impl HasTagService for HandlerState {
    type TagSvc = InertTagService;
    fn tag_service(&self) -> &Self::TagSvc {
        &self.tags
    }
}
impl HasHistoryService for HandlerState {
    type HistorySvc = InertHistoryService;
    fn history_service(&self) -> &Self::HistorySvc {
        &self.history
    }
}

fn auth_context(user_id: UserId) -> AuthContext {
    AuthContext {
        user: User {
            id: user_id,
            username: "door-user".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::Admin,
            api_key_hash: "api".to_string(),
            setup_pending: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
        auth_type: AuthType::Session,
        session_token_hash: Some("test-session".to_string()),
    }
}

async fn wait_for_calls<F>(work: &RecordingWorkService, expected: usize, predicate: F)
where
    F: Fn(&[WorkCall]) -> usize,
{
    for _ in 0..20 {
        let calls = work.calls().await;
        if predicate(&calls) >= expected {
            return;
        }
        tokio::time::advance(std::time::Duration::from_millis(250)).await;
        tokio::task::yield_now().await;
    }
    panic!(
        "timed out waiting for {expected} matching calls; calls={:?}",
        work.calls().await
    );
}

#[tokio::test(start_paused = true)]
async fn c1_work_add_handler_chains_complete_add_then_delayed_refresh() {
    let work = RecordingWorkService::new();
    let state = HandlerState::new(work.clone());
    let candidate_id = CandidateId("C1".to_string());
    let _ = livrarr_handlers::work::add(
        State(state),
        auth_context(99),
        Json(livrarr_handlers::AddWorkRequest {
            ol_key: Some("OLC1W".to_string()),
            title: "C1 Add".to_string(),
            author_name: "C1 Author".to_string(),
            author_ol_key: None,
            year: Some(2026),
            cover_url: None,
            language: Some("en".to_string()),
            detail_url: None,
            cover_manual: false,
            isbn_13: None,
            candidate_id: Some(candidate_id.clone()),
            hc_key: None,
            gr_key: None,
            asin: None,
        }),
    )
    .await
    .expect("handler add");

    wait_for_calls(&work, 1, |calls| {
        calls
            .iter()
            .filter(|call| matches!(call, WorkCall::CompleteAdd { .. }))
            .count()
    })
    .await;
    let before_5s = work.refresh_calls().await;
    assert!(
        before_5s.is_empty(),
        "C1 refresh must not happen before +5s"
    );

    tokio::time::advance(std::time::Duration::from_secs(5)).await;
    wait_for_calls(&work, 1, |calls| {
        calls
            .iter()
            .filter(|call| matches!(call, WorkCall::Refresh(_, _, RefreshSurface::Interactive)))
            .count()
    })
    .await;

    let calls = work.calls().await;
    assert!(
        matches!(calls[0], WorkCall::ResolveIdentityLocal),
        "C1 call 1"
    );
    assert!(matches!(calls[1], WorkCall::AddFast(99)), "C1 call 2");
    assert!(
        matches!(
            &calls[2],
            WorkCall::CompleteAdd {
                user_id: 99,
                work_id: 7001,
                source_provider_data: None,
                candidate_id: Some(id),
                mode: IdentityMode::Interactive,
                source: ConflictSource::ManualAdd,
            } if *id == candidate_id
        ),
        "C1 call 3: {calls:?}"
    );
    assert_eq!(
        work.refresh_calls().await,
        vec![(99, 7001, RefreshSurface::Interactive)],
        "C1 delayed refresh"
    );
}

#[tokio::test]
async fn c2_work_refresh_handler_calls_interactive_refresh_once() {
    let work = RecordingWorkService::new();
    let state = HandlerState::new(work.clone());
    let _ = refresh(State(state), auth_context(99), Path(7701))
        .await
        .expect("refresh handler");
    assert_eq!(
        work.refresh_calls().await,
        vec![(99, 7701, RefreshSurface::Interactive)],
        "C2"
    );
}

#[tokio::test(start_paused = true)]
async fn c3_refresh_all_consults_guard_and_bulk_refreshes_every_listed_work() {
    let work = RecordingWorkService::with_listed_works(vec![
        Work {
            id: 1,
            user_id: 99,
            ..Work::default()
        },
        Work {
            id: 2,
            user_id: 99,
            ..Work::default()
        },
    ]);
    let state = HandlerState::new(work.clone());
    refresh_all(
        State(state),
        auth_context(99),
        Query(RefreshAllParams {
            language: None,
            monitored: None,
            enrichment_status: None,
            media_type: None,
        }),
    )
    .await
    .expect("refresh all");
    wait_for_calls(&work, 2, |calls| {
        calls
            .iter()
            .filter(|call| matches!(call, WorkCall::Refresh(_, _, RefreshSurface::Bulk)))
            .count()
    })
    .await;
    assert!(matches!(
        work.calls().await.first(),
        Some(WorkCall::TryStartBulkRefresh(99))
    ));
    let mut refreshes = work.refresh_calls().await;
    refreshes.sort_by_key(|(_, work_id, _)| *work_id);
    assert_eq!(
        refreshes,
        vec![(99, 1, RefreshSurface::Bulk), (99, 2, RefreshSurface::Bulk)],
        "C3"
    );
}

#[tokio::test(start_paused = true)]
async fn c4_retry_all_incomplete_handler_spawns_service_call() {
    let work = RecordingWorkService::new();
    let state = HandlerState::new(work.clone());
    retry_all_incomplete(State(state), auth_context(99))
        .await
        .expect("retry handler");
    wait_for_calls(&work, 1, |calls| {
        calls
            .iter()
            .filter(|call| matches!(call, WorkCall::RetryAllIncomplete(99)))
            .count()
    })
    .await;
    assert!(matches!(
        work.calls().await.first(),
        Some(WorkCall::TryStartBulkRefresh(99))
    ));
}

#[tokio::test(start_paused = true)]
async fn c5_affirm_pending_anchor_calls_affirm_then_spawns_interactive_refresh() {
    let work = RecordingWorkService::new();
    let state = HandlerState::new(work.clone());
    let repo = state.identity_repo.clone();
    affirm_pending_anchor(
        State(state),
        auth_context(99),
        Path((8801, AnchorType::OL_WORK.to_string())),
    )
    .await
    .expect("affirm handler");
    assert_eq!(
        repo.confirm_count.load(Ordering::SeqCst),
        1,
        "C5 affirm service call"
    );
    wait_for_calls(&work, 1, |calls| {
        calls
            .iter()
            .filter(|call| {
                matches!(
                    call,
                    WorkCall::Refresh(99, 8801, RefreshSurface::Interactive)
                )
            })
            .count()
    })
    .await;
}
