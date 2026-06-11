//! Behavioral RED tests for metadata-correctness add-door identity/enrichment routing.
//!
//! Each red door test wires a real `WorkServiceImpl` with a stub-backed
//! `LiveEnglishIdentityResolver` (the test_idu_bulk_import_identity pattern), so the
//! AC-012 RED directive is asserted in full: resolver/provider fan-out OBSERVED and
//! resolved anchors PERSISTED — not merely that the add road was reached.

use livrarr_behavioral::stubs::{create_test_user, StubEnrichmentWorkflow, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{CreateWorkDbRequest, UpdateWorkEnrichmentDbRequest, WorkDb, WorkDbCreate};
use livrarr_domain::identity::{
    CapturedIdentity, IdentityMethod, IdentityState, PendingReason, WorkCandidate, WorkSeedFields,
};
use livrarr_domain::services::WorkService;
use livrarr_domain::{
    normalize_for_matching, EnrichmentStatus, MetadataProvider, ProvenanceSetter, Work,
};
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

fn service(db: SqliteDb, workflow: StubEnrichmentWorkflow) -> TestWorkService {
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

/// An OpenLibrary stub whose payload deterministically resolves the candidate:
/// matching title/author plus canonical anchors (ol_key + isbn_13).
fn resolving_ol_stub(title: &str, author: &str) -> StubProviderClient {
    StubProviderClient::new(
        MetadataProvider::OpenLibrary,
        ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
            title: Some(title.to_string()),
            author_name: Some(author.to_string()),
            ol_key: Some("OL900100W".to_string()),
            isbn_13: Some("9780000000002".to_string()),
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
            llm_configured: false,
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

fn candidate_fields(title: &str, author: &str, language: &str) -> WorkSeedFields {
    WorkSeedFields {
        title: title.to_string(),
        author_name: author.to_string(),
        language: language.to_string(),
        author_ol_key: None,
        year: Some(2024),
        cover_url: None,
        detail_url: None,
        description: None,
        series_name: None,
        series_position: None,
    }
}

fn anchorless_pending_candidate(title: &str, author: &str) -> WorkCandidate {
    WorkCandidate {
        fields: candidate_fields(title, author, "en"),
        identity: IdentityState::Pending {
            reason: PendingReason::NoCandidates,
            seed_anchors: None,
            top_candidates: vec![],
        },
        candidate_id: None,
        source_provider_data: None,
        file_path: None,
        delete_existing_after_import: false,
        series_id: None,
        monitor_ebook: None,
        monitor_audiobook: None,
        provenance_setter: Some(ProvenanceSetter::Import),
        import_id: None,
        cover_manual: false,
    }
}

fn anchorless_confirmed_candidate(title: &str, author: &str) -> WorkCandidate {
    WorkCandidate {
        fields: candidate_fields(title, author, "en"),
        identity: IdentityState::Confirmed {
            anchors: CapturedIdentity {
                ol_key: None,
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
        candidate_id: None,
        source_provider_data: None,
        file_path: None,
        delete_existing_after_import: false,
        series_id: None,
        monitor_ebook: None,
        monitor_audiobook: None,
        provenance_setter: Some(ProvenanceSetter::Import),
        import_id: None,
        cover_manual: false,
    }
}

fn work_req(user_id: i64, title: &str, author: &str) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: author.to_string(),
        normalized_title: normalize_for_matching(title),
        normalized_author: normalize_for_matching(author),
        language: Some("en".to_string()),
        monitor_ebook: true,
        monitor_audiobook: false,
        ..Default::default()
    }
}

async fn seed_anchorless_work(db: &SqliteDb, user_id: i64, title: &str, author: &str) -> Work {
    let (work, created) = db
        .create_work(work_req(user_id, title, author))
        .await
        .expect("seed anchorless work");
    assert!(created);
    work
}

#[tokio::test]
async fn anchorless_candidate_runs_identity_road_and_sync_enrichment() {
    // REQ-010/AC-012 (the IR's RED directive): a title/author/language-only
    // candidate performs resolver/provider fan-out (provider calls OBSERVED),
    // persists the resolved anchors, and runs the sync enrichment leg — proving
    // the anchor-less path actually resolves, not just that a helper is reached.
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let workflow = StubEnrichmentWorkflow::succeeding();
    let ol = resolving_ol_stub("Anchorless GB Result", "Door Audit");
    let svc = service_with_resolver(
        db.clone(),
        workflow.clone(),
        resolver_with_stubs(vec![ol.clone()]),
    );

    let added = svc
        .add(
            user_id,
            anchorless_pending_candidate("Anchorless GB Result", "Door Audit"),
        )
        .await
        .expect("add anchorless candidate");
    let persisted = db
        .get_work(user_id, added.work.id)
        .await
        .expect("read added work");

    assert!(
        ol.call_count() >= 1,
        "AC-012: the add door must fan out through the identity resolver's providers"
    );
    assert_eq!(
        persisted.ol_key.as_deref(),
        Some("OL900100W"),
        "AC-012: resolved anchors must be persisted on the work"
    );
    assert_eq!(
        persisted.isbn_13.as_deref(),
        Some("9780000000002"),
        "AC-012: the resolver's bridge identifier must be persisted"
    );
    assert_eq!(
        workflow.call_count(),
        1,
        "sync enrichment should run on add"
    );
    // The returned status carries the enrichment outcome; the PERSISTED status
    // is written inside the real enrichment service's merge apply, which the
    // stub workflow here does not reach — pinned by the pipeline suites.
    assert_ne!(added.enrichment_status, EnrichmentStatus::Unenriched);
}

#[tokio::test]
async fn anchorless_readd_adopts_existing_unenriched_work_and_enriches_it() {
    // REQ-010/AC-012: an anchor-less re-add matching an existing Unenriched
    // anchor-less work takes the same identity + enrichment road — fan-out
    // observed, anchors persisted onto the adopted work, enrichment run.
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let existing = seed_anchorless_work(&db, user_id, "Adopt Me", "Door Audit").await;
    let workflow = StubEnrichmentWorkflow::succeeding();
    let ol = resolving_ol_stub("Adopt Me", "Door Audit");
    let svc = service_with_resolver(
        db.clone(),
        workflow.clone(),
        resolver_with_stubs(vec![ol.clone()]),
    );

    let added = svc
        .add(
            user_id,
            anchorless_confirmed_candidate("Adopt Me", "Door Audit"),
        )
        .await
        .expect("re-add anchorless candidate");
    let persisted = db
        .get_work(user_id, existing.id)
        .await
        .expect("read adopted work");

    assert!(!added.created);
    assert_eq!(added.work.id, existing.id);
    assert!(
        ol.call_count() >= 1,
        "AC-012: the adopt branch must run the identity leg for an anchor-less work"
    );
    assert_eq!(
        persisted.ol_key.as_deref(),
        Some("OL900100W"),
        "AC-012: anchors resolved on re-add must persist onto the adopted work"
    );
    assert_eq!(
        workflow.call_count(),
        1,
        "adopt branch should run enrichment"
    );
    // The returned status carries the enrichment outcome; the PERSISTED status
    // is written inside the real enrichment service's merge apply, which the
    // stub workflow here does not reach — pinned by the pipeline suites.
    assert_ne!(added.enrichment_status, EnrichmentStatus::Unenriched);
}

#[tokio::test]
async fn enriched_dedup_readd_does_not_reenrich_or_touch_source() {
    // REQ-010/AC-012: re-adding a candidate deduped to an already-Enriched work preserves behavior.
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let existing = seed_anchorless_work(&db, user_id, "Already Enriched", "Door Audit").await;
    db.update_work_enrichment(
        user_id,
        existing.id,
        UpdateWorkEnrichmentDbRequest {
            enrichment_status: EnrichmentStatus::Enriched,
            enrichment_source: Some("existing-source".to_string()),
            description: Some("Existing description".to_string()),
            cover_url: Some("https://covers.example/existing.jpg".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("mark existing work enriched");

    let workflow = StubEnrichmentWorkflow::succeeding();
    let svc = service(db.clone(), workflow.clone());
    let added = svc
        .add(
            user_id,
            anchorless_confirmed_candidate("Already Enriched", "Door Audit"),
        )
        .await
        .expect("re-add enriched dedup");
    let persisted = db
        .get_work(user_id, existing.id)
        .await
        .expect("read deduped work");

    assert!(!added.created);
    assert_eq!(added.work.id, existing.id);
    assert_eq!(
        workflow.call_count(),
        0,
        "already enriched dedup must not re-enrich"
    );
    assert_eq!(persisted.enrichment_status, EnrichmentStatus::Enriched);
    assert_eq!(
        persisted.enrichment_source.as_deref(),
        Some("existing-source")
    );
}
