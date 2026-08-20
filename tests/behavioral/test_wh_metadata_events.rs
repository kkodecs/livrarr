mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use livrarr_behavioral::stubs::{create_test_user, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{
    CreateLibraryItemDbRequest, CreateWorkDbRequest, HistoryDb, LibraryItemDb, RootFolderDb,
    TagStatus, WorkDb, WorkDbCreate,
};
use livrarr_domain::identity::{
    CandidateId, CapturedIdentity, IdentityMethod, IdentityState, WorkCandidate,
};
use livrarr_domain::seed::{seed_add_box, SeedInput, SeedLanguage};
use livrarr_domain::services::{
    EnrichmentMode, EnrichmentResult, EnrichmentWorkflow, EnrichmentWorkflowError, RefreshSurface,
    SourceProviderData, WorkService,
};
use livrarr_domain::{
    normalize_for_matching, EnrichmentStatus, EventType, Freshness, HistoryFilter, IdentityStatus,
    MediaType, MetadataProvider, OutcomeClass, RequestPriority, UserId, Work, WorkId,
};
use livrarr_external_data::transport_cache::TransportCache;
use livrarr_external_data::NormalizedWorkDetail;
use livrarr_metadata::enrichment_workflow_service::EnrichmentWorkflowImpl;
use livrarr_metadata::work_service::WorkServiceImpl;
use livrarr_metadata::{
    DefaultMergeEngine, EnrichmentContext, EnrichmentServiceImpl, PriorityModel, ProviderQueue,
    ProviderQueueError, ScatterGatherResult,
};

type ScriptedService = WorkServiceImpl<SqliteDb, ScriptedWorkflow, StubHttpFetcher>;

#[derive(Clone)]
struct ScriptedWorkflow {
    outcome: ScriptedOutcome,
}

#[derive(Clone)]
enum ScriptedOutcome {
    Completed { changed: bool },
    Failed,
    MergeDeferred,
    NoAttempt,
}

impl ScriptedWorkflow {
    fn completed(changed: bool) -> Self {
        Self {
            outcome: ScriptedOutcome::Completed { changed },
        }
    }

    fn failed() -> Self {
        Self {
            outcome: ScriptedOutcome::Failed,
        }
    }

    fn merge_deferred() -> Self {
        Self {
            outcome: ScriptedOutcome::MergeDeferred,
        }
    }

    fn no_attempt() -> Self {
        Self {
            outcome: ScriptedOutcome::NoAttempt,
        }
    }
}

impl EnrichmentWorkflow for ScriptedWorkflow {
    async fn enrich_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
        mode: EnrichmentMode,
        _candidate_id: Option<CandidateId>,
        _priority: RequestPriority,
        _freshness: Freshness,
    ) -> Result<EnrichmentResult, EnrichmentWorkflowError> {
        match self.outcome {
            ScriptedOutcome::Failed => Err(EnrichmentWorkflowError::Queue(
                "scripted enrichment failure".to_string(),
            )),
            ScriptedOutcome::Completed { changed } => Ok(EnrichmentResult {
                enrichment_status: if changed {
                    EnrichmentStatus::Enriched
                } else {
                    EnrichmentStatus::Thin
                },
                enrichment_source: Some("scripted".to_string()),
                work: Work {
                    id: work_id,
                    user_id,
                    title: "Metadata Fixture".to_string(),
                    author_name: "Metadata Author".to_string(),
                    enrichment_status: if changed {
                        EnrichmentStatus::Enriched
                    } else {
                        EnrichmentStatus::Thin
                    },
                    ..Work::default()
                },
                merge_deferred: false,
                provider_outcomes: HashMap::from([(
                    MetadataProvider::Hardcover,
                    OutcomeClass::Success,
                )]),
                cover_resolution: None,
                audiobook_cover_resolution: None,
                identity_not_found: false,
                changed,
                attempted: true,
                captured_provider_identity: Vec::new(),
                captured_route_proposals: Vec::new(),
                provider_chase_attempted: false,
                search_leg_fired: false,
                search_ledger_burnable: false,
            }),
            ScriptedOutcome::MergeDeferred => Ok(EnrichmentResult {
                enrichment_status: EnrichmentStatus::Unenriched,
                enrichment_source: None,
                work: Work {
                    id: work_id,
                    user_id,
                    ..Work::default()
                },
                merge_deferred: mode == EnrichmentMode::Background,
                provider_outcomes: HashMap::from([(
                    MetadataProvider::Hardcover,
                    OutcomeClass::WillRetry,
                )]),
                cover_resolution: None,
                audiobook_cover_resolution: None,
                identity_not_found: false,
                changed: false,
                attempted: true,
                captured_provider_identity: Vec::new(),
                captured_route_proposals: Vec::new(),
                provider_chase_attempted: false,
                search_leg_fired: false,
                search_ledger_burnable: false,
            }),
            ScriptedOutcome::NoAttempt => Ok(EnrichmentResult {
                enrichment_status: EnrichmentStatus::Unenriched,
                enrichment_source: None,
                work: Work {
                    id: work_id,
                    user_id,
                    ..Work::default()
                },
                merge_deferred: false,
                provider_outcomes: HashMap::new(),
                cover_resolution: None,
                audiobook_cover_resolution: None,
                identity_not_found: false,
                changed: false,
                attempted: false,
                captured_provider_identity: Vec::new(),
                captured_route_proposals: Vec::new(),
                provider_chase_attempted: false,
                search_leg_fired: false,
                search_ledger_burnable: false,
            }),
        }
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

#[derive(Clone, Default)]
struct PanicProviderQueue;

impl ProviderQueue for PanicProviderQueue {
    async fn dispatch_enrichment(
        &self,
        work: &Work,
        _context: EnrichmentContext,
    ) -> Result<ScatterGatherResult, ProviderQueueError> {
        panic!(
            "cached-candidate fixture should reuse TransportCache, not dispatch providers for work {}",
            work.id
        );
    }
}

fn scripted_service(db: SqliteDb, workflow: ScriptedWorkflow) -> ScriptedService {
    WorkServiceImpl::new(
        db,
        workflow,
        StubHttpFetcher::new(),
        tempfile::tempdir().expect("test data dir").keep(),
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

fn confirmed_candidate(title: &str, author: &str, ol_key: &str) -> WorkCandidate {
    seed_add_box(
        seed_input(title, author),
        IdentityState::Confirmed {
            anchors: CapturedIdentity {
                ol_key: Some(ol_key.to_string()),
                gr_key: None,
                hc_key: None,
                isbn_13: None,
                asin: None,
                title: title.to_string(),
                author_name: author.to_string(),
                language: Some("en".to_string()),
            },
            method: IdentityMethod::UserSelected,
            score: None,
        },
        Some(CandidateId(format!("candidate-{ol_key}"))),
        false,
    )
}

async fn seed_confirmed_work(db: &SqliteDb, user_id: UserId, suffix: &str) -> Work {
    let (work, created) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: format!("Metadata Fixture {suffix}"),
            author_name: "Metadata Author".to_string(),
            normalized_title: normalize_for_matching(&format!("Metadata Fixture {suffix}")),
            normalized_author: normalize_for_matching("Metadata Author"),
            language: Some("en".to_string()),
            ol_key: Some(format!("/works/WH-META-{suffix}")),
            monitor_ebook: true,
            monitor_audiobook: true,
            ..CreateWorkDbRequest::default()
        })
        .await
        .expect("seed work");
    assert!(created);
    // Fixture-satisfiability: create_work alone does not confirm identity, and
    // the refresh road blocks enrichment for non-Confirmed works — without this
    // the "completed pass" fixtures could never produce their event even after
    // the writer lands (n2 precedent: set the status explicitly).
    db.set_identity_status(user_id, work.id, IdentityStatus::Confirmed)
        .await
        .expect("confirm seeded identity");
    work
}

async fn metadata_rows(
    db: &SqliteDb,
    user_id: UserId,
    work_id: WorkId,
) -> Vec<livrarr_domain::HistoryEvent> {
    let enriched = db
        .list_history(
            user_id,
            HistoryFilter {
                event_type: Some(EventType::Enriched),
                work_id: Some(work_id),
                start_date: None,
                end_date: None,
            },
        )
        .await
        .expect("list enriched history");
    let failed = db
        .list_history(
            user_id,
            HistoryFilter {
                event_type: Some(EventType::EnrichmentFailed),
                work_id: Some(work_id),
                start_date: None,
                end_date: None,
            },
        )
        .await
        .expect("list failed history");
    enriched.into_iter().chain(failed.into_iter()).collect()
}

async fn refresh_fixture(workflow: ScriptedWorkflow, suffix: &str) -> (SqliteDb, UserId, WorkId) {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_confirmed_work(&db, user_id, suffix).await;
    let svc = scripted_service(db.clone(), workflow);

    svc.refresh(user_id, work.id, RefreshSurface::Interactive)
        .await
        .expect("refresh should complete");

    (db, user_id, work.id)
}

#[tokio::test]
async fn wh_refresh_completed_changed_writes_one_enriched_changed_true() {
    let (db, user_id, work_id) =
        refresh_fixture(ScriptedWorkflow::completed(true), "changed").await;
    let rows = metadata_rows(&db, user_id, work_id).await;

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].event_type, EventType::Enriched);
    assert_eq!(
        rows[0].data.get("changed").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        rows[0].data.get("work_title").and_then(|v| v.as_str()),
        Some("Metadata Fixture changed")
    );
    assert_eq!(
        rows[0].data.get("tags_written").and_then(|v| v.as_bool()),
        Some(false),
        "a pass with no library files writes no tags — the flag is present and false"
    );
}

#[tokio::test]
async fn wh_refresh_completed_noop_writes_one_enriched_changed_false() {
    let (db, user_id, work_id) = refresh_fixture(ScriptedWorkflow::completed(false), "noop").await;
    let rows = metadata_rows(&db, user_id, work_id).await;

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].event_type, EventType::Enriched);
    assert_eq!(
        rows[0].data.get("changed").and_then(|v| v.as_bool()),
        Some(false)
    );
}

#[tokio::test]
async fn wh_refresh_failure_writes_one_enrichment_failed() {
    let (db, user_id, work_id) = refresh_fixture(ScriptedWorkflow::failed(), "failed").await;
    let rows = metadata_rows(&db, user_id, work_id).await;

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].event_type, EventType::EnrichmentFailed);
    assert!(
        rows[0].data.get("reason").is_some(),
        "enrichmentFailed payload should summarize the failure"
    );
}

#[tokio::test]
async fn wh_identity_parked_refresh_writes_zero_metadata_events() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (work, created) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Parked Metadata Fixture".to_string(),
            author_name: "Metadata Author".to_string(),
            normalized_title: normalize_for_matching("Parked Metadata Fixture"),
            normalized_author: normalize_for_matching("Metadata Author"),
            language: Some("en".to_string()),
            monitor_ebook: true,
            monitor_audiobook: true,
            ..CreateWorkDbRequest::default()
        })
        .await
        .expect("seed parked work");
    assert!(created);

    let svc = scripted_service(db.clone(), ScriptedWorkflow::completed(true));
    svc.refresh(user_id, work.id, RefreshSurface::Interactive)
        .await
        .expect("parked refresh should return successfully");

    assert!(
        metadata_rows(&db, user_id, work.id).await.is_empty(),
        "identity-pending work must not enrich and must not record metadata history"
    );
}

#[tokio::test]
async fn wh_background_merge_deferred_writes_zero_metadata_events() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_confirmed_work(&db, user_id, "deferred").await;
    let svc = scripted_service(db.clone(), ScriptedWorkflow::merge_deferred());

    svc.converge_work(user_id, work.id, 3)
        .await
        .expect("background convergence should run");

    assert!(
        metadata_rows(&db, user_id, work.id).await.is_empty(),
        "merge_deferred background result records no metadata event until a concluding pass"
    );
}

const CONTAINER_XML: &str = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

const MINIMAL_OPF: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Placeholder</dc:title>
  </metadata>
  <manifest>
  </manifest>
  <spine>
  </spine>
</package>"#;

fn write_minimal_epub(path: &std::path::Path) {
    use std::io::Write;
    let file = std::fs::File::create(path).expect("create epub");
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default();
    zip.start_file("META-INF/container.xml", opts)
        .expect("start container");
    zip.write_all(CONTAINER_XML.as_bytes()).expect("container");
    zip.start_file("OEBPS/content.opf", opts)
        .expect("start opf");
    zip.write_all(MINIMAL_OPF.as_bytes()).expect("opf");
    zip.finish().expect("finish epub");
}

#[tokio::test]
async fn wh_enriched_payload_carries_tags_written_when_materialization_wrote_tags() {
    // Tests-review disposition (withheld flag 1): the prior fixture scripted a
    // tags_written flag that the workflow double silently discarded — the test
    // was unsatisfiable, because the design derives the flag from the REAL
    // materialize step (MaterializeOutcome.tags_written at run_unified_enrichment
    // Step 5), never from the enrichment result. This drives the real
    // derivation: a real library item backed by a real minimal EPUB and a
    // completed changed pass make materialize write tags for real.
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_confirmed_work(&db, user_id, "tags").await;

    let books_dir = tempfile::tempdir().expect("books dir");
    let epub_path = books_dir.path().join("tags-fixture.epub");
    write_minimal_epub(&epub_path);
    let root = db
        .create_root_folder(books_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .expect("root folder");
    db.create_library_item(CreateLibraryItemDbRequest {
        user_id,
        work_id: work.id,
        root_folder_id: root.id,
        path: epub_path.to_string_lossy().into_owned(),
        media_type: MediaType::Ebook,
        file_size: 1024,
        import_id: None,
        tag_status: TagStatus::Pending,
        tagged_at_generation: 0,
    })
    .await
    .expect("library item");

    let svc = scripted_service(db.clone(), ScriptedWorkflow::completed(true));
    svc.refresh(user_id, work.id, RefreshSurface::Interactive)
        .await
        .expect("refresh should complete");

    let rows = metadata_rows(&db, user_id, work.id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].event_type, EventType::Enriched);
    assert_eq!(
        rows[0].data.get("tags_written").and_then(|v| v.as_bool()),
        Some(true),
        "materialized tag writes are part of the enriched event, not a separate tag event"
    );
}

#[tokio::test]
async fn wh_cached_reuse_nochange_records_enriched_false_with_real_stack() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let candidate_id = CandidateId("candidate-cache-nochange".to_string());
    let title = "Cached NoChange Fixture";
    let author = "Metadata Author";
    let ol_key = "/works/WH-CACHED-NOCHANGE";
    let cache = Arc::new(TransportCache::new(Duration::from_secs(60)));
    cache.cache_put(
        user_id,
        candidate_id.clone(),
        HashMap::from([(
            MetadataProvider::OpenLibrary,
            NormalizedWorkDetail {
                title: Some(title.to_string()),
                author_name: Some(author.to_string()),
                ol_key: Some(ol_key.to_string()),
                language: Some("en".to_string()),
                ..NormalizedWorkDetail::default()
            },
        )]),
    );

    let enrichment = EnrichmentServiceImpl::new(
        Arc::new(db.clone()),
        Arc::new(PanicProviderQueue),
        Arc::new(DefaultMergeEngine::new(PriorityModel::english())),
        false,
    )
    .with_transport_cache(cache);
    let workflow = EnrichmentWorkflowImpl::new(Arc::new(enrichment));
    let svc = WorkServiceImpl::new(
        db.clone(),
        workflow,
        StubHttpFetcher::new(),
        tempfile::tempdir().expect("test data dir").keep(),
    );

    let mut candidate = confirmed_candidate(title, author, ol_key);
    candidate.candidate_id = Some(candidate_id);
    let added = svc
        .add(user_id, candidate)
        .await
        .expect("add should run cached candidate reuse through complete_add");
    assert!(added.created);

    let rows = metadata_rows(&db, user_id, added.work.id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].event_type, EventType::Enriched);
    assert_eq!(
        rows[0].data.get("changed").and_then(|v| v.as_bool()),
        Some(false)
    );
}

#[tokio::test]
async fn wh_no_attempt_terminal_skip_writes_zero_metadata_events() {
    let (db, user_id, work_id) =
        refresh_fixture(ScriptedWorkflow::no_attempt(), "no-attempt").await;
    assert!(
        metadata_rows(&db, user_id, work_id).await.is_empty(),
        "a pass with no dispatch and no merge has attempted=false and records nothing"
    );
}

#[tokio::test]
async fn wh_bulk_refresh_writes_one_metadata_event_per_eligible_work() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_a = seed_confirmed_work(&db, user_id, "bulk-a").await;
    let work_b = seed_confirmed_work(&db, user_id, "bulk-b").await;
    let svc = scripted_service(db.clone(), ScriptedWorkflow::completed(false));
    let _guard = svc
        .try_start_bulk_refresh(user_id)
        .expect("bulk refresh slot should be available");

    for work_id in [work_a.id, work_b.id] {
        svc.refresh(user_id, work_id, RefreshSurface::Bulk)
            .await
            .expect("bulk refresh item should complete");
    }

    assert_eq!(metadata_rows(&db, user_id, work_a.id).await.len(), 1);
    assert_eq!(metadata_rows(&db, user_id, work_b.id).await.len(), 1);
}

#[tokio::test]
async fn wh_background_convergence_enrichment_writes_exactly_one_metadata_event() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_confirmed_work(&db, user_id, "convergence").await;
    let svc = scripted_service(db.clone(), ScriptedWorkflow::completed(false));

    svc.converge_work(user_id, work.id, 3)
        .await
        .expect("convergence should run");

    assert_eq!(
        metadata_rows(&db, user_id, work.id).await.len(),
        1,
        "background convergence should record one metadata event for the completed enrichment"
    );
}
