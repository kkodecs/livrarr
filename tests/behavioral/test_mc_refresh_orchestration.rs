//! Behavioral RED tests for metadata-correctness refresh orchestration.
//!
//! Each test wires a real `WorkServiceImpl` with a stub-backed
//! `LiveEnglishIdentityResolver` (the test_idu_bulk_import_identity pattern), so
//! REQ-008's completion-before-scatter, suppression, and resume semantics are
//! observable: stub provider call counts prove whether completion ran; persisted
//! DB anchors prove what it resolved.

use livrarr_behavioral::stubs::{create_test_user, StubEnrichmentWorkflow, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{CreateWorkDbRequest, WorkDb, WorkDbCreate};
use livrarr_domain::identity::{AnchorSetter, AnchorType, IdentityState, LatencyTier, RawHarvest};
use livrarr_domain::services::{
    EnrichmentMode, RefreshSurface, WorkIdentityRepository, WorkService,
};
use livrarr_domain::{
    normalize_for_matching, IdentityStatus, MetadataProvider, RequestPriority, Work,
};
use livrarr_external_data::transport_cache::TransportCache;
use livrarr_external_data::{
    NormalizedWorkDetail, ProviderClient, ProviderOutcome, StubProviderClient,
};
use livrarr_metadata::english_identity_resolver::{LiveEnglishIdentityResolver, ResolverConfig};
use livrarr_metadata::work_service::WorkServiceImpl;

type TestWorkService = WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher>;

/// An OpenLibrary stub whose payload resolves the work AND carries the missing
/// Goodreads key — completion captures anchors from any provider's payload.
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

fn gr_stub(
    title: &str,
    ol_key: Option<&str>,
    hc_key: Option<&str>,
    isbn_13: Option<&str>,
    gr_key: &str,
) -> StubProviderClient {
    StubProviderClient::new(
        MetadataProvider::Goodreads,
        ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
            title: Some(title.to_string()),
            author_name: Some("Refresh Audit".to_string()),
            ol_key: ol_key.map(str::to_string),
            hc_key: hc_key.map(str::to_string),
            gr_key: Some(gr_key.to_string()),
            isbn_13: isbn_13.map(str::to_string),
            language: Some("en".to_string()),
            ..NormalizedWorkDetail::default()
        })),
    )
}

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
    WorkServiceImpl::new(
        db,
        workflow,
        StubHttpFetcher::new(),
        tempfile::tempdir()
            .expect("test data dir")
            .path()
            .to_path_buf(),
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

async fn seed_gr_keyless_work(db: &SqliteDb, user_id: i64, title: &str) -> Work {
    let (work, created) = db
        .create_work(work_req(user_id, title))
        .await
        .expect("seed gr-keyless work");
    assert!(created);
    work
}

#[tokio::test]
async fn refresh_interactive_surface_dispatches_scatter_at_normal_priority() {
    // B4 table: a watched single-work refresh dispatches the enrichment scatter
    // at Manual mode + Normal priority.
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_gr_keyless_work(&db, user_id, "Interactive Surface Priority").await;
    let workflow = StubEnrichmentWorkflow::succeeding();
    let ol = gr_key_bearing_ol_stub("Interactive Surface Priority");
    let svc = service_with_resolver(db.clone(), workflow.clone(), resolver_with_stubs(vec![ol]));

    svc.refresh(user_id, work.id, RefreshSurface::Interactive)
        .await
        .expect("refresh work");

    let contexts = workflow.enrich_contexts();
    assert!(
        !contexts.is_empty(),
        "refresh must dispatch the enrichment scatter"
    );
    assert!(
        contexts.iter().all(|(mode, priority)| matches!(
            (mode, priority),
            (EnrichmentMode::Manual, RequestPriority::Normal)
        )),
        "an interactive refresh dispatches at Manual mode / Normal priority"
    );
}

#[tokio::test]
async fn refresh_bulk_surface_dispatches_scatter_at_low_priority() {
    // B4 table: an unattended bulk sweep (refresh-all, retry-all-incomplete)
    // rides the outbound queue at Low priority; the mode stays Manual.
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_gr_keyless_work(&db, user_id, "Bulk Surface Priority").await;
    let workflow = StubEnrichmentWorkflow::succeeding();
    let ol = gr_key_bearing_ol_stub("Bulk Surface Priority");
    let svc = service_with_resolver(db.clone(), workflow.clone(), resolver_with_stubs(vec![ol]));

    svc.refresh(user_id, work.id, RefreshSurface::Bulk)
        .await
        .expect("refresh work");

    let contexts = workflow.enrich_contexts();
    assert!(
        !contexts.is_empty(),
        "refresh must dispatch the enrichment scatter"
    );
    assert!(
        contexts.iter().all(|(mode, priority)| matches!(
            (mode, priority),
            (EnrichmentMode::Manual, RequestPriority::Low)
        )),
        "a bulk refresh dispatches at Manual mode / Low priority"
    );
}

#[tokio::test]
async fn refresh_does_not_merge_containment_sibling_without_anchor_corroboration() {
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_gr_keyless_work(&db, user_id, "Dune").await;
    let ol = StubProviderClient::new(
        MetadataProvider::OpenLibrary,
        ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
            title: Some("Dune Messiah".to_string()),
            author_name: Some("Refresh Audit".to_string()),
            ol_key: Some("OL-DUNE-MESSIAH".to_string()),
            isbn_13: Some("9780000000002".to_string()),
            language: Some("en".to_string()),
            ..NormalizedWorkDetail::default()
        })),
    );
    let svc = service_with_resolver(
        db.clone(),
        StubEnrichmentWorkflow::succeeding(),
        resolver_with_stubs(vec![ol]),
    );

    svc.refresh(user_id, work.id, RefreshSurface::Interactive)
        .await
        .expect("refresh work");

    let refreshed = db
        .get_work(user_id, work.id)
        .await
        .expect("read refreshed work");
    assert_eq!(
        refreshed.ol_key, None,
        "containment sibling title must not merge an OpenLibrary anchor"
    );
    assert_eq!(
        refreshed.identity_status,
        IdentityStatus::Pending,
        "containment sibling title must leave the work pending"
    );
}

#[tokio::test]
async fn refresh_rejects_goodreads_key_when_exact_title_has_contradicting_sibling_work_key() {
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (work, created) = db
        .create_work(CreateWorkDbRequest {
            ol_key: Some("OL-A".to_string()),
            ..work_req(user_id, "Mixed Evidence")
        })
        .await
        .expect("seed confirmed work with sibling anchors");
    assert!(created);
    livrarr_db::test_helpers::set_identity_status_fixture(
        &db,
        user_id,
        work.id,
        IdentityStatus::Confirmed,
    )
    .await
    .expect("mark work confirmed");
    // hc_key has no CreateWorkDbRequest field; the anchor write syncs the
    // denormalized works column (confirm_anchor_in_tx REQ-029).
    db.confirm_anchor(
        work.id,
        AnchorType::new(AnchorType::HC_WORK),
        "HC-A",
        AnchorSetter::User,
    )
    .await
    .expect("seed contradicting-slot hc anchor");
    let gr = gr_stub("Mixed Evidence", Some("OL-A"), Some("HC-B"), None, "777");
    let svc = service_with_resolver(
        db.clone(),
        StubEnrichmentWorkflow::succeeding(),
        resolver_with_stubs(vec![gr]),
    );

    svc.refresh(user_id, work.id, RefreshSurface::Interactive)
        .await
        .expect("refresh work");

    assert_eq!(
        db.get_work(user_id, work.id)
            .await
            .expect("read refreshed work")
            .gr_key,
        None,
        "exact title must not trust a Goodreads key when another work key contradicts"
    );
}

#[tokio::test]
async fn resolve_identity_adopts_goodreads_key_for_user_confirmed_bridge_only_subtitle_match() {
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let isbn = "9780307346612";
    let gr = gr_stub(
        "World War Z: An Oral History of the Zombie War",
        None,
        None,
        Some(isbn),
        "234225",
    );
    let svc = service_with_resolver(
        db,
        StubEnrichmentWorkflow::succeeding(),
        resolver_with_stubs(vec![gr]),
    );

    let resolved = svc
        .resolve_identity(
            user_id,
            RawHarvest {
                isbn: Some(isbn.to_string()),
                title: Some("World War Z".to_string()),
                author_name: Some("Refresh Audit".to_string()),
                language: Some("en".to_string()),
                user_confirmed: true,
                ..RawHarvest::default()
            },
            LatencyTier::Interactive,
        )
        .await
        .expect("resolve bridge-only identity");

    let anchors = match resolved.identity {
        IdentityState::Confirmed { anchors, .. } => anchors,
        other => panic!("expected confirmed identity, got {other:?}"),
    };
    assert_eq!(
        anchors.gr_key.as_deref(),
        Some("234225"),
        "bridge-only user-confirmed seed should trust the corroborated Goodreads key"
    );
}
