//! Behavioral red gate for S6 user-triggered retry-all-incomplete recovery.
//!
//! `WorkService::retry_all_incomplete` replaces the deleted recurring background
//! retry job. These tests assert a one-shot sweep over the union of incomplete
//! status axes: enrichment Failed, enrichment Unenriched, or identity Pending.

use livrarr_behavioral::stubs::{create_test_user, StubEnrichmentWorkflow, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{CreateWorkDbRequest, UpdateWorkEnrichmentDbRequest, WorkDb, WorkDbCreate};
use livrarr_domain::services::WorkService;
use livrarr_domain::{normalize_for_matching, EnrichmentStatus, IdentityStatus, UserId, Work};
use livrarr_metadata::english_identity_resolver::LiveEnglishIdentityResolver;
use livrarr_metadata::work_service::WorkServiceImpl;
use std::sync::Arc;
use std::time::Duration;

type TestWorkService = WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher>;

const AUTHOR: &str = "S6 Contract Author";

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

    livrarr_db::test_helpers::set_identity_status_fixture(db, user_id, work.id, identity_status)
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
    let svc = WorkServiceImpl::new(db, workflow, StubHttpFetcher::new(), test_data_dir());

    match resolver {
        Some(resolver) => svc.with_resolver(Arc::new(resolver)),
        None => svc,
    }
}

fn assert_same_work_ids(mut actual: Vec<i64>, expected: &[i64]) {
    let mut expected = expected.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);
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
