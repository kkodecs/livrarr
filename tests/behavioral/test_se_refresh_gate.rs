//! Behavioral RED tests for sprint-e-refresh-gate.
//!
//! Door-1 identity completion is observed through the resolver provider stub's
//! call count; enrichment scatter is observed through `StubEnrichmentWorkflow`.

use livrarr_behavioral::stubs::{create_test_user, StubEnrichmentWorkflow, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{CreateWorkDbRequest, WorkDb, WorkDbCreate};
use livrarr_domain::services::{RefreshSurface, WorkService};
use livrarr_domain::{normalize_for_matching, IdentityStatus, MetadataProvider, Work};
use livrarr_external_data::transport_cache::TransportCache;
use livrarr_external_data::{
    NormalizedWorkDetail, ProviderClient, ProviderOutcome, StubProviderClient,
};
use livrarr_metadata::english_identity_resolver::{LiveEnglishIdentityResolver, ResolverConfig};
use livrarr_metadata::work_service::WorkServiceImpl;

type TestWorkService = WorkServiceImpl<
    SqliteDb,
    StubEnrichmentWorkflow,
    StubHttpFetcher,
    livrarr_metadata::work_service::StubNoLlm,
    livrarr_metadata::DefaultMergeEngine,
    livrarr_metadata::work_service::StubTagService,
>;

fn resolver_with_stubs(stubs: Vec<StubProviderClient>) -> LiveEnglishIdentityResolver {
    let clients = stubs
        .into_iter()
        .map(|s| (s.provider, ProviderClient::Stub(s)))
        .collect::<std::collections::HashMap<_, _>>();
    LiveEnglishIdentityResolver {
        clients,
        cache: std::sync::Arc::new(TransportCache::new(std::time::Duration::from_secs(30))),
        config: ResolverConfig {
            gb_key_present: false,
            ..ResolverConfig::default()
        },
    }
}

fn service_with_resolver(
    db: SqliteDb,
    workflow: StubEnrichmentWorkflow,
    resolver: LiveEnglishIdentityResolver,
) -> TestWorkService {
    WorkServiceImpl::new_with_all(
        db,
        workflow,
        StubHttpFetcher::new(),
        livrarr_http::HttpClient::builder()
            .build()
            .expect("test HttpClient"),
        livrarr_metadata::work_service::StubNoLlm,
        tempfile::tempdir()
            .expect("test data dir")
            .path()
            .to_path_buf(),
        livrarr_metadata::DefaultMergeEngine::new(livrarr_metadata::PriorityModel::english()),
        std::sync::Arc::new(livrarr_metadata::work_service::StubTagService),
    )
    .with_resolver(std::sync::Arc::new(resolver))
}

fn work_req(user_id: i64, title: &str) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: "Refresh Audit".to_string(),
        normalized_title: normalize_for_matching(title),
        normalized_author: normalize_for_matching("Refresh Audit"),
        language: Some("en".to_string()),
        monitor_ebook: true,
        monitor_audiobook: false,
        ..Default::default()
    }
}

fn gr_key_bearing_ol_stub(title: &str) -> StubProviderClient {
    StubProviderClient::new(
        MetadataProvider::OpenLibrary,
        ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
            title: Some(title.to_string()),
            author_name: Some("Refresh Audit".to_string()),
            ol_key: Some("OL777000W".to_string()),
            gr_key: Some("234225".to_string()),
            isbn_13: Some("9780441013593".to_string()),
            language: Some("en".to_string()),
            ..NormalizedWorkDetail::default()
        })),
    )
}

async fn seed_anchor_rich_gr_keyless_work(db: &SqliteDb, user_id: i64, title: &str) -> Work {
    let (work, created) = db
        .create_work(CreateWorkDbRequest {
            ol_key: Some("OL777000W".to_string()),
            isbn_13: Some("9780441013593".to_string()),
            ..work_req(user_id, title)
        })
        .await
        .expect("seed anchor-rich gr-keyless work");
    assert!(
        created,
        "REQ-001 fixture: anchor-rich work should be newly seeded"
    );
    work
}

// NOTE (id-completeness cutover): the Sprint-E test
// `refresh_skips_completion_for_confirmed_anchor_rich_work` was removed — this
// feature deliberately REVERSES that gate. A Confirmed work missing an
// obtainable id now RE-CHASES on refresh (chase missing IDs; the dead-end
// suppression bounds it). The replacement coverage — fully-anchored skips,
// missing-obtainable-id re-chases — lives in
// test_id_completeness::test_id_completeness_refresh_gate_confirmed_rechases_only_when_missing_obtainable_id.
// The Pending / Provisional / anchor-poor cases below are unchanged and remain valid.

#[tokio::test]
async fn refresh_runs_completion_for_pending_work() {
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let title = "Pending Anchor Rich";
    let work = seed_anchor_rich_gr_keyless_work(&db, user_id, title).await;
    db.set_identity_status(user_id, work.id, IdentityStatus::Pending)
        .await
        .expect("mark work pending");
    let workflow = StubEnrichmentWorkflow::succeeding();
    let ol = gr_key_bearing_ol_stub(title);
    let svc = service_with_resolver(
        db.clone(),
        workflow.clone(),
        resolver_with_stubs(vec![ol.clone()]),
    );

    svc.refresh(user_id, work.id, RefreshSurface::Interactive)
        .await
        .expect("refresh work");

    assert!(
        ol.call_count() >= 1,
        "AC-002/REQ-001: Pending refresh must still run door-1 identity completion"
    );
    assert_eq!(
        workflow.call_count(),
        1,
        "AC-003/REQ-001: Pending refresh must still run enrichment scatter once"
    );
}

#[tokio::test]
async fn refresh_completes_anchor_poor_confirmed_work_via_door2() {
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let title = "Confirmed Anchor Poor";
    let (work, created) = db
        .create_work(CreateWorkDbRequest {
            gr_key: Some("GR-ONLY-123".to_string()),
            ol_key: None,
            isbn_13: None,
            asin: None,
            ..work_req(user_id, title)
        })
        .await
        .expect("seed gr-only work");
    assert!(
        created,
        "AC-005/REQ-001 fixture: anchor-poor work should be newly seeded"
    );
    db.set_identity_status(user_id, work.id, IdentityStatus::Confirmed)
        .await
        .expect("mark work confirmed");
    let workflow = StubEnrichmentWorkflow::succeeding();
    let ol = gr_key_bearing_ol_stub(title);
    let svc = service_with_resolver(
        db.clone(),
        workflow.clone(),
        resolver_with_stubs(vec![ol.clone()]),
    );

    svc.refresh(user_id, work.id, RefreshSurface::Interactive)
        .await
        .expect("refresh work");

    assert!(
        ol.call_count() >= 1,
        "AC-005/REQ-001: Confirmed anchor-poor refresh must still complete via door 2"
    );
    assert_eq!(
        workflow.call_count(),
        1,
        "AC-003/REQ-001: Confirmed anchor-poor refresh must still run enrichment scatter once"
    );
}

#[tokio::test]
async fn refresh_runs_completion_for_provisional_work() {
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let title = "Provisional Anchor Rich";
    let work = seed_anchor_rich_gr_keyless_work(&db, user_id, title).await;
    db.set_identity_status(user_id, work.id, IdentityStatus::Provisional)
        .await
        .expect("mark work provisional");
    let workflow = StubEnrichmentWorkflow::succeeding();
    let ol = gr_key_bearing_ol_stub(title);
    let svc = service_with_resolver(
        db.clone(),
        workflow.clone(),
        resolver_with_stubs(vec![ol.clone()]),
    );

    svc.refresh(user_id, work.id, RefreshSurface::Interactive)
        .await
        .expect("refresh work");

    assert!(
        ol.call_count() >= 1,
        "AC-002/REQ-001: Provisional refresh must still run door-1 identity completion"
    );
    assert_eq!(
        workflow.call_count(),
        1,
        "AC-002/REQ-001: Provisional refresh must still run enrichment scatter once"
    );
}
