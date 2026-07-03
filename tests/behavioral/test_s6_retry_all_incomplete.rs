//! Behavioral red gate for S6 user-triggered retry-all-incomplete recovery.
//!
//! `WorkService::retry_all_incomplete` replaces the deleted recurring background
//! retry job. These tests assert a one-shot sweep over the union of incomplete
//! status axes: enrichment Failed, enrichment Unenriched, or identity Pending.

use livrarr_behavioral::stubs::{
    create_second_test_user, create_test_user, StubEnrichmentWorkflow, StubHttpFetcher,
};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{CreateWorkDbRequest, UpdateWorkEnrichmentDbRequest, WorkDb, WorkDbCreate};
use livrarr_domain::services::WorkService;
use livrarr_domain::{
    normalize_for_matching, EnrichmentStatus, IdentityStatus, MetadataProvider, UserId, Work,
};
use livrarr_external_data::transport_cache::TransportCache;
use livrarr_external_data::{
    NormalizedWorkDetail, ProviderClient, ProviderOutcome, StubProviderClient,
};
use livrarr_metadata::english_identity_resolver::{LiveEnglishIdentityResolver, ResolverConfig};
use livrarr_metadata::work_service::{StubNoLlm, StubTagService, WorkServiceImpl};
use livrarr_metadata::{DefaultMergeEngine, PriorityModel};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

type TestWorkService = WorkServiceImpl<
    SqliteDb,
    StubEnrichmentWorkflow,
    StubHttpFetcher,
    StubNoLlm,
    DefaultMergeEngine,
    StubTagService,
>;

const AUTHOR: &str = "S6 Contract Author";
const PENDING_TITLE: &str = "Pending Identity Title";
const RESOLVED_OL_KEY: &str = "OL-S6-PENDING-W";

fn test_data_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("livrarr-s6-retry-test-{}", std::process::id()))
}

fn work_req(user_id: UserId, title: &str) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: AUTHOR.to_string(),
        normalized_title: normalize_for_matching(title),
        normalized_author: normalize_for_matching(AUTHOR),
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
    enrichment_status: EnrichmentStatus,
    identity_status: IdentityStatus,
) -> Work {
    let (work, created) = db
        .create_work(work_req(user_id, title))
        .await
        .expect("seed work");
    assert!(created, "test fixture titles must be unique");

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

    let seeded = db
        .get_work(user_id, work.id)
        .await
        .expect("read seeded work");
    assert_eq!(seeded.enrichment_status, enrichment_status);
    assert_eq!(seeded.identity_status, identity_status);
    seeded
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
        livrarr_http::HttpClient::builder()
            .build()
            .expect("test HttpClient"),
        StubNoLlm,
        test_data_dir(),
        DefaultMergeEngine::new(PriorityModel::english()),
        Arc::new(StubTagService),
    );

    match resolver {
        Some(resolver) => svc.with_resolver(Arc::new(resolver)),
        None => svc,
    }
}

fn resolving_openlibrary_stub() -> StubProviderClient {
    StubProviderClient::new(
        MetadataProvider::OpenLibrary,
        ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
            ol_key: Some(RESOLVED_OL_KEY.to_string()),
            title: Some(PENDING_TITLE.to_string()),
            author_name: Some(AUTHOR.to_string()),
            language: Some("en".to_string()),
            ..NormalizedWorkDetail::default()
        })),
    )
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

fn assert_same_work_ids(mut actual: Vec<i64>, expected: &[i64]) {
    let mut expected = expected.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn retry_all_incomplete_lists_exact_union_and_refreshes_each_incomplete_once() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let other_user_id = create_second_test_user(&db).await;

    let failed_a = seed_work(
        &db,
        user_id,
        "Failed A",
        EnrichmentStatus::Failed,
        IdentityStatus::Confirmed,
    )
    .await;
    let failed_b = seed_work(
        &db,
        user_id,
        "Failed B",
        EnrichmentStatus::Failed,
        IdentityStatus::Confirmed,
    )
    .await;
    let unenriched = seed_work(
        &db,
        user_id,
        "Unenriched",
        EnrichmentStatus::Unenriched,
        IdentityStatus::Confirmed,
    )
    .await;
    let pending_identity = seed_work(
        &db,
        user_id,
        PENDING_TITLE,
        EnrichmentStatus::Enriched,
        IdentityStatus::Pending,
    )
    .await;
    let complete = seed_work(
        &db,
        user_id,
        "Complete",
        EnrichmentStatus::Enriched,
        IdentityStatus::Confirmed,
    )
    .await;
    let other_users_failed = seed_work(
        &db,
        other_user_id,
        "Other User Failed",
        EnrichmentStatus::Failed,
        IdentityStatus::Confirmed,
    )
    .await;

    let workflow = StubEnrichmentWorkflow::succeeding();
    let resolver = resolver_with_stubs(vec![
        resolving_openlibrary_stub(),
        StubProviderClient::new(MetadataProvider::Hardcover, ProviderOutcome::NotFound),
    ]);
    let svc = service(db.clone(), workflow.clone(), Some(resolver));

    let summary = svc
        .retry_all_incomplete(user_id)
        .await
        .expect("retry_all_incomplete should return a summary");

    let expected = [failed_a.id, failed_b.id, unenriched.id, pending_identity.id];
    assert_eq!(
        summary.total,
        expected.len(),
        "AC-S6-1: total is the exact union of Failed, Unenriched, and identity-Pending works"
    );
    assert_eq!(
        workflow.call_count(),
        expected.len(),
        "AC-S6-2: every incomplete work is re-enriched exactly once"
    );
    assert_same_work_ids(workflow.work_ids(), &expected);

    assert!(
        !workflow.work_ids().contains(&complete.id),
        "AC-S6-1: complete works must not be swept"
    );
    assert!(
        !workflow.work_ids().contains(&other_users_failed.id),
        "retry_all_incomplete must remain scoped to the requested user"
    );
}

#[tokio::test]
async fn retry_all_incomplete_is_one_shot_and_does_not_continue_after_returning() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let failed = seed_work(
        &db,
        user_id,
        "One Shot Failed",
        EnrichmentStatus::Failed,
        IdentityStatus::Confirmed,
    )
    .await;

    let workflow = StubEnrichmentWorkflow::succeeding();
    let svc = service(db, workflow.clone(), None);

    let summary = tokio::time::timeout(Duration::from_secs(1), svc.retry_all_incomplete(user_id))
        .await
        .expect("AC-S6-4: retry_all_incomplete is a single pass and returns")
        .expect("retry_all_incomplete should return a summary");

    assert_eq!(summary.total, 1);
    assert_same_work_ids(workflow.work_ids(), &[failed.id]);

    let calls_after_return = workflow.call_count();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        workflow.call_count(),
        calls_after_return,
        "AC-S6-4: no recurring retry loop or tick should run after the one-shot summary returns"
    );
}

#[tokio::test]
async fn retry_all_incomplete_re_resolves_identity_pending_work() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let pending = seed_work(
        &db,
        user_id,
        PENDING_TITLE,
        EnrichmentStatus::Enriched,
        IdentityStatus::Pending,
    )
    .await;

    let ol = resolving_openlibrary_stub();
    let resolver = resolver_with_stubs(vec![
        ol.clone(),
        StubProviderClient::new(MetadataProvider::Hardcover, ProviderOutcome::NotFound),
    ]);
    let workflow = StubEnrichmentWorkflow::succeeding();
    let svc = service(db.clone(), workflow.clone(), Some(resolver));

    let summary = svc
        .retry_all_incomplete(user_id)
        .await
        .expect("retry_all_incomplete should return a summary");

    assert_eq!(
        summary.total, 1,
        "AC-S6-3: identity-Pending work is part of the incomplete sweep even when enrichment is already Enriched"
    );
    assert!(
        ol.call_count() >= 1,
        "AC-S6-3: Pending identity must be re-resolved through the resolver \
         (refresh now ALSO runs REQ-008 anchor completion, so the sweep may \
         legitimately resolve more than once)"
    );
    assert_same_work_ids(workflow.work_ids(), &[pending.id]);

    let saved = db
        .get_work(user_id, pending.id)
        .await
        .expect("read pending work after retry");
    assert_eq!(
        saved.identity_status,
        IdentityStatus::Confirmed,
        "FLM: title+author match confirms identity during the retry sweep"
    );
    assert_eq!(
        saved.ol_key.as_deref(),
        Some(RESOLVED_OL_KEY),
        "FLM: title+author match syncs ol_key to works.*"
    );
}
