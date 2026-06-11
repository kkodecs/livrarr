//! Behavioral RED tests for metadata-correctness refresh orchestration.
//!
//! Each test wires a real `WorkServiceImpl` with a stub-backed
//! `LiveEnglishIdentityResolver` (the test_idu_bulk_import_identity pattern), so
//! REQ-008's completion-before-scatter, suppression, and resume semantics are
//! observable: stub provider call counts prove whether completion ran; persisted
//! DB anchors prove what it resolved.

use livrarr_behavioral::stubs::{create_test_user, StubEnrichmentWorkflow, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{CreateWorkDbRequest, ProviderRetryStateDb, WorkDb, WorkDbCreate};
use livrarr_domain::identity::{AnchorSetter, AnchorType};
use livrarr_domain::services::{WorkIdentityRepository, WorkService};
use livrarr_domain::{normalize_for_matching, MetadataProvider, OutcomeClass, Work};
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
async fn refresh_completes_missing_gr_key_before_scatter_runs() {
    // REQ-008/AC-010: refresh runs identity anchor-completion BEFORE the
    // enrichment scatter; the resolver's fan-out is observed and the completed
    // anchor is persisted, then the scatter still runs.
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_gr_keyless_work(&db, user_id, "Completion Before Scatter").await;
    let workflow = StubEnrichmentWorkflow::succeeding();
    let ol = gr_key_bearing_ol_stub("Completion Before Scatter");
    let svc = service_with_resolver(
        db.clone(),
        workflow.clone(),
        resolver_with_stubs(vec![ol.clone()]),
    );

    let result = svc.refresh(user_id, work.id).await.expect("refresh work");
    let refreshed = db
        .get_work(user_id, work.id)
        .await
        .expect("read refreshed work");

    assert!(
        ol.call_count() >= 1,
        "AC-010: refresh must run the completion fan-out for a gr_key-less work"
    );
    assert_eq!(workflow.reset_call_count(), 1);
    assert_eq!(
        workflow.call_count(),
        1,
        "scatter should still run after completion"
    );
    assert_eq!(result.work.id, work.id);
    assert_eq!(
        refreshed.gr_key.as_deref(),
        Some("234225"),
        "AC-010: completion should persist the missing Goodreads key before scatter"
    );
}

#[tokio::test]
async fn terminal_not_found_completion_suppression_survives_plain_refresh() {
    // REQ-008/AC-010: when EVERY missing anchor's provider is suppressed (here:
    // only gr_key is missing and Goodreads carries a terminal not-found), a
    // plain consecutive refresh makes ZERO resolver calls — completion is
    // bounded; resume needs an identity-input change or explicit retry.
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (work, created) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Suppressed Completion".to_string(),
            author_name: "Refresh Audit".to_string(),
            normalized_title: normalize_for_matching("Suppressed Completion"),
            normalized_author: normalize_for_matching("Refresh Audit"),
            ol_key: Some("OL777000W".to_string()),
            isbn_13: Some("9780441013593".to_string()),
            asin: Some("B000TEST".to_string()),
            language: Some("en".to_string()),
            monitor_ebook: true,
            ..Default::default()
        })
        .await
        .expect("seed work missing only gr_key");
    assert!(created);
    db.confirm_anchor(
        work.id,
        AnchorType::new(AnchorType::HC_WORK),
        "HC-SUPPRESSED",
        AnchorSetter::User,
    )
    .await
    .expect("seed hc anchor");
    db.record_terminal_outcome(
        user_id,
        work.id,
        MetadataProvider::Goodreads,
        OutcomeClass::NotFound,
        None,
    )
    .await
    .expect("record terminal provider outcome");
    let workflow = StubEnrichmentWorkflow::succeeding();
    let ol = gr_key_bearing_ol_stub("Suppressed Completion");
    let svc = service_with_resolver(
        db.clone(),
        workflow.clone(),
        resolver_with_stubs(vec![ol.clone()]),
    );

    svc.refresh(user_id, work.id).await.expect("refresh work");
    let state = db
        .get_retry_state(user_id, work.id, MetadataProvider::Goodreads)
        .await
        .expect("read retry state");

    assert_eq!(
        ol.call_count(),
        0,
        "AC-010: a fully-suppressed completion makes zero resolver calls"
    );
    assert!(
        state.is_some(),
        "plain refresh must not clear suppression state"
    );
    assert_eq!(
        workflow.call_count(),
        1,
        "refresh still runs the scatter once"
    );
    assert_eq!(
        db.get_work(user_id, work.id)
            .await
            .expect("read refreshed work")
            .gr_key,
        None,
        "suppressed completion should leave the missing provider anchor absent"
    );
}

#[tokio::test]
async fn explicit_retry_reset_reenables_completion_suppression_matrix() {
    // REQ-008/AC-010 resume matrix: plain refresh does not clear suppression;
    // an explicit retry reset does — and the NEXT refresh then actually
    // re-attempts completion (resolver called, anchor gained).
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_gr_keyless_work(&db, user_id, "Retry Matrix").await;
    db.confirm_anchor(
        work.id,
        AnchorType::new(AnchorType::ISBN_13),
        "9780441013593",
        AnchorSetter::AutoIsbn,
    )
    .await
    .expect("seed bridge anchor");
    db.record_terminal_outcome(
        user_id,
        work.id,
        MetadataProvider::Goodreads,
        OutcomeClass::NotFound,
        None,
    )
    .await
    .expect("record terminal provider outcome");
    let workflow = StubEnrichmentWorkflow::succeeding();
    let ol = gr_key_bearing_ol_stub("Retry Matrix");
    let svc = service_with_resolver(db.clone(), workflow, resolver_with_stubs(vec![ol.clone()]));

    svc.refresh(user_id, work.id)
        .await
        .expect("plain refresh should not clear suppression");
    assert!(
        db.get_retry_state(user_id, work.id, MetadataProvider::Goodreads)
            .await
            .expect("read retry state")
            .is_some(),
        "plain consecutive refresh must not re-enable completion"
    );

    db.reset_all_retry_states(user_id, work.id)
        .await
        .expect("explicit retry reset");
    assert!(
        db.get_retry_state(user_id, work.id, MetadataProvider::Goodreads)
            .await
            .expect("read retry state")
            .is_none(),
        "explicit retry reset should re-enable the next completion attempt"
    );

    let calls_before_retry = ol.call_count();
    svc.refresh(user_id, work.id)
        .await
        .expect("refresh after explicit retry");
    assert!(
        ol.call_count() > calls_before_retry,
        "AC-010: after an explicit retry reset, the next refresh re-attempts completion"
    );
    assert_eq!(
        db.get_work(user_id, work.id)
            .await
            .expect("read refreshed work")
            .gr_key
            .as_deref(),
        Some("234225"),
        "AC-010: the re-attempted completion persists the recovered anchor"
    );
}
