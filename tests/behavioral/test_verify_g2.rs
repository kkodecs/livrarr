//! Behavioral verification test for Finding G2 (P1).
//!
//! G2: `retry_all_incomplete` filters works by:
//!   `EnrichmentStatus::Failed | EnrichmentStatus::Unenriched`
//!     || identity_status == IdentityStatus::Pending
//!
//! `EnrichmentStatus::Thin` is NOT in this filter. A work that received a
//! "we know the book but found no metadata" outcome is never re-attempted by
//! the user-triggered sweep, even though new metadata may have appeared since.
//!
//! Expected result: THIS TEST FAILS — `summary.total == 0` for a Thin work,
//! proving G2. The correct behaviour would be `summary.total == 1`.

use livrarr_behavioral::stubs::{create_test_user, StubEnrichmentWorkflow, StubHttpFetcher};
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{CreateWorkDbRequest, UpdateWorkEnrichmentDbRequest, WorkDb, WorkDbCreate};
use livrarr_domain::services::WorkService;
use livrarr_domain::{normalize_for_matching, EnrichmentStatus, IdentityStatus};
use livrarr_metadata::work_service::WorkServiceImpl;

const AUTHOR: &str = "G2 Verify Author";
const THIN_TITLE: &str = "Thin Status Title G2";

fn test_data_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("livrarr-g2-verify-{}", std::process::id()))
}

fn service(
    db: livrarr_db::sqlite::SqliteDb,
    workflow: StubEnrichmentWorkflow,
) -> WorkServiceImpl<livrarr_db::sqlite::SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher> {
    WorkServiceImpl::new(db, workflow, StubHttpFetcher::new(), test_data_dir())
}

/// G2 BUG: `EnrichmentStatus::Thin` is not in the `retry_all_incomplete` filter.
///
/// This test creates one work with `enrichment_status = Thin` and calls
/// `retry_all_incomplete`. The expected (buggy) result is `summary.total == 0`
/// — the sweep processes zero works because Thin is silently excluded from the
/// filter predicate. The assertion is written for the CORRECT behaviour
/// (`total == 1`), so the test will FAIL, proving G2.
#[tokio::test]
#[ignore = "Finding G2 (red-by-design gate): retry_all_incomplete excludes EnrichmentStatus::Thin, so 'known book, no metadata' works are never re-swept. In-scope for the convergence feature. Run with --ignored."]
async fn test_verify_g2_thin_work_excluded_from_retry_all_incomplete() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    // Create a work and manually set its enrichment_status to Thin.
    let (work, created) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: THIN_TITLE.to_string(),
            author_name: AUTHOR.to_string(),
            normalized_title: normalize_for_matching(THIN_TITLE),
            normalized_author: normalize_for_matching(AUTHOR),
            language: Some("en".to_string()),
            monitor_ebook: true,
            monitor_audiobook: true,
            ..Default::default()
        })
        .await
        .expect("create work");
    assert!(created, "work must be freshly created");

    db.update_work_enrichment(
        user_id,
        work.id,
        UpdateWorkEnrichmentDbRequest {
            enrichment_status: EnrichmentStatus::Thin,
            enrichment_source: Some("test-seed".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("set Thin status");

    // Works default to IdentityStatus::Pending after create_work (no anchor
    // supplied). Force Confirmed so the Pending branch in retry_all_incomplete
    // cannot rescue this work — we want only the enrichment filter to be tested.
    db.set_identity_status(user_id, work.id, IdentityStatus::Confirmed)
        .await
        .expect("set Confirmed identity status");

    // Confirm the DB actually stored Thin.
    let seeded = db
        .get_work(user_id, work.id)
        .await
        .expect("read seeded work");
    assert_eq!(
        seeded.enrichment_status,
        EnrichmentStatus::Thin,
        "precondition: work must be Thin before calling retry_all_incomplete"
    );
    assert_eq!(
        seeded.identity_status,
        IdentityStatus::Confirmed,
        "precondition: identity must be Confirmed so the Pending branch cannot rescue this work"
    );

    // Run the sweep.
    let workflow = StubEnrichmentWorkflow::succeeding();
    let svc = service(db, workflow.clone());

    let summary = svc
        .retry_all_incomplete(user_id)
        .await
        .expect("retry_all_incomplete should not error");

    // G2 BUG: the filter excludes Thin, so summary.total == 0.
    // The assertion below is written for the CORRECT behaviour (total == 1),
    // so this test is expected to FAIL.
    assert_eq!(
        summary.total, 1,
        "G2 BUG: retry_all_incomplete processed {} works instead of 1. \
         EnrichmentStatus::Thin is absent from the filter predicate \
         (matches!(w.enrichment_status, Failed | Unenriched)), so the Thin \
         work (id={}) is silently skipped and never re-enriched.",
        summary.total, work.id,
    );

    assert_eq!(
        workflow.call_count(),
        1,
        "G2 BUG: enrichment workflow was called {} times instead of 1 — \
         Thin work was not passed to refresh",
        workflow.call_count(),
    );
}
