//! Behavioral RED gate for convergence-unified-path (Part 1).
//!
//! `WorkService::converge_pending_due` is the automatic background convergence
//! that restores M9: identity-`Pending` works seeded by monitors / Readarr /
//! list-import converge on their own instead of sitting in silent limbo. These
//! tests pin the spec's guardrails:
//!   AC-001/005 a DUE pending work converges to COMPLETE (identity + enrichment, option B)
//!   AC-002     a NOT-yet-due pending work is left alone (backoff-clock selection)
//!   AC-004     a Confirmed work is never swept (pending-only)
//!   AC-007     a dead-end terminates to needs-review and is NOT retried (no loop)
//!   AC-009     a single call processes at most `limit` works (bounded batch)

use livrarr_behavioral::stubs::{create_test_user, StubEnrichmentWorkflow, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    CreateWorkDbRequest, ProviderRetryStateDb, UpdateWorkEnrichmentDbRequest, WorkDb, WorkDbCreate,
};
use livrarr_domain::services::WorkService;
use livrarr_domain::{
    normalize_for_matching, EnrichmentStatus, IdentityStatus, MetadataProvider, UserId, Work,
};
use livrarr_external_data::transport_cache::TransportCache;
use livrarr_external_data::{
    NormalizedWorkDetail, ProviderClient, ProviderOutcome, StubProviderClient,
};
use livrarr_metadata::english_identity_resolver::{LiveEnglishIdentityResolver, ResolverConfig};
use livrarr_metadata::work_service::WorkServiceImpl;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

type TestWorkService = WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher>;

const AUTHOR: &str = "Convergence Author";
const PENDING_TITLE: &str = "Pending Identity Title";
const RESOLVED_OL_KEY: &str = "OL-CUP-PENDING-W";

fn test_data_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("livrarr-cup-test-{}", std::process::id()))
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

    db.get_work(user_id, work.id)
        .await
        .expect("read seeded work")
}

/// Write a `provider_retry_state` row so the work is DUE now (`next_attempt_at`
/// an hour in the past) — this is what `list_works_due_for_retry` selects on.
async fn make_due(db: &SqliteDb, user_id: UserId, work: &Work) {
    db.record_will_retry(
        user_id,
        work.id,
        MetadataProvider::OpenLibrary,
        chrono::Utc::now() - chrono::Duration::hours(1),
    )
    .await
    .expect("write due retry row");
}

/// Write a `provider_retry_state` row that is NOT yet due (an hour in the future).
async fn make_not_due(db: &SqliteDb, user_id: UserId, work: &Work) {
    db.record_will_retry(
        user_id,
        work.id,
        MetadataProvider::OpenLibrary,
        chrono::Utc::now() + chrono::Duration::hours(1),
    )
    .await
    .expect("write not-due retry row");
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
            llm_configured: false,
            ..ResolverConfig::default()
        },
    }
}

/// A resolver that resolves the pending work to a real OL work anchor.
fn resolving_resolver() -> LiveEnglishIdentityResolver {
    resolver_with_stubs(vec![
        StubProviderClient::new(
            MetadataProvider::OpenLibrary,
            ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
                ol_key: Some(RESOLVED_OL_KEY.to_string()),
                title: Some(PENDING_TITLE.to_string()),
                author_name: Some(AUTHOR.to_string()),
                language: Some("en".to_string()),
                ..NormalizedWorkDetail::default()
            })),
        ),
        StubProviderClient::new(MetadataProvider::Hardcover, ProviderOutcome::NotFound),
    ])
}

/// A resolver where every provider returns NotFound — a deterministic dead-end.
fn dead_end_resolver() -> LiveEnglishIdentityResolver {
    resolver_with_stubs(vec![
        StubProviderClient::new(MetadataProvider::OpenLibrary, ProviderOutcome::NotFound),
        StubProviderClient::new(MetadataProvider::Hardcover, ProviderOutcome::NotFound),
    ])
}

// AC-001 + AC-005: a DUE identity-pending work converges all the way to COMPLETE
// — identity resolved AND enrichment applied (option B), not identity alone.
#[tokio::test]
async fn converges_due_pending_work_to_complete() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let pending = seed_work(
        &db,
        user_id,
        PENDING_TITLE,
        EnrichmentStatus::Unenriched,
        IdentityStatus::Pending,
    )
    .await;
    make_due(&db, user_id, &pending).await;

    let workflow = StubEnrichmentWorkflow::succeeding();
    let svc = service(db.clone(), workflow.clone(), Some(resolving_resolver()));

    let summary = svc
        .converge_pending_due(user_id, 10)
        .await
        .expect("converge_pending_due should return a summary");

    assert_eq!(summary.total, 1, "the one due pending work is converged");
    let saved = db.get_work(user_id, pending.id).await.expect("read work");
    assert_eq!(
        saved.identity_status,
        IdentityStatus::Confirmed,
        "AC-001: a resolvable due pending work advances out of Pending"
    );
    assert!(
        workflow.work_ids().contains(&pending.id),
        "AC-005 (option B): the converged work is ALSO enriched, not identity-only"
    );
}

// AC-002: a pending work whose next_attempt_at is in the FUTURE is not touched —
// selection follows the backoff clock, never "all pending every tick".
#[tokio::test]
async fn skips_pending_work_not_yet_due() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let pending = seed_work(
        &db,
        user_id,
        PENDING_TITLE,
        EnrichmentStatus::Unenriched,
        IdentityStatus::Pending,
    )
    .await;
    make_not_due(&db, user_id, &pending).await;

    let workflow = StubEnrichmentWorkflow::succeeding();
    let svc = service(db.clone(), workflow.clone(), Some(resolving_resolver()));

    let summary = svc
        .converge_pending_due(user_id, 10)
        .await
        .expect("summary");

    assert_eq!(
        summary.total, 0,
        "AC-002: a not-yet-due work is not selected"
    );
    let saved = db.get_work(user_id, pending.id).await.expect("read work");
    assert_eq!(
        saved.identity_status,
        IdentityStatus::Pending,
        "AC-002: a not-yet-due work is left untouched"
    );
    assert!(
        workflow.work_ids().is_empty(),
        "AC-002: no enrichment runs for a not-yet-due work"
    );
}

// AC-004: a Confirmed work is never swept — even if it has a DUE retry row.
#[tokio::test]
async fn skips_confirmed_work_even_when_due() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let confirmed = seed_work(
        &db,
        user_id,
        "Confirmed Complete",
        EnrichmentStatus::Enriched,
        IdentityStatus::Confirmed,
    )
    .await;
    make_due(&db, user_id, &confirmed).await;

    let workflow = StubEnrichmentWorkflow::succeeding();
    let svc = service(db.clone(), workflow.clone(), Some(resolving_resolver()));

    let summary = svc
        .converge_pending_due(user_id, 10)
        .await
        .expect("summary");

    assert_eq!(summary.total, 0, "AC-004: convergence is pending-only");
    assert!(
        !workflow.work_ids().contains(&confirmed.id),
        "AC-004: a Confirmed work is never re-enriched by convergence"
    );
}

// AC-007: a pending work the resolver cannot resolve terminates to needs-review
// and is NOT attempted again on the next call (no indefinite loop).
#[tokio::test]
async fn dead_end_goes_to_needs_review_and_is_not_retried() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let pending = seed_work(
        &db,
        user_id,
        "Unresolvable Work",
        EnrichmentStatus::Unenriched,
        IdentityStatus::Pending,
    )
    .await;
    make_due(&db, user_id, &pending).await;

    let workflow = StubEnrichmentWorkflow::succeeding();
    let svc = service(db.clone(), workflow.clone(), Some(dead_end_resolver()));

    let first = svc
        .converge_pending_due(user_id, 10)
        .await
        .expect("summary");
    assert_eq!(first.total, 1, "the dead-end work is attempted once");
    let saved = db.get_work(user_id, pending.id).await.expect("read work");
    assert_eq!(
        saved.identity_status,
        IdentityStatus::NeedsReview,
        "AC-007: a dead-end terminates to needs-review, never silent limbo"
    );

    // Second call: the now-terminal work must NOT be re-attempted (no loop).
    let second = svc
        .converge_pending_due(user_id, 10)
        .await
        .expect("summary");
    assert_eq!(
        second.total, 0,
        "AC-007: a needs-review work is terminal — convergence does not loop on it"
    );
}

// AC-009: a single call processes at most `limit` works (bounded batch).
#[tokio::test]
async fn respects_batch_limit() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    for i in 0..3 {
        let w = seed_work(
            &db,
            user_id,
            &format!("Pending {i}"),
            EnrichmentStatus::Unenriched,
            IdentityStatus::Pending,
        )
        .await;
        make_due(&db, user_id, &w).await;
    }

    let workflow = StubEnrichmentWorkflow::succeeding();
    let svc = service(db.clone(), workflow.clone(), Some(dead_end_resolver()));

    let summary = svc.converge_pending_due(user_id, 2).await.expect("summary");

    assert_eq!(
        summary.total, 2,
        "AC-009: a single call processes at most the batch limit (2 of 3 due works)"
    );
}

// R-001 + Codex-R-001: when identity resolves but enrichment fails, the work is
// NOT lost — it stays eligible (identified + incomplete) — and its retry-clock is
// advanced so it is not re-swept on the very next tick (no loop / no starvation).
#[tokio::test]
async fn incomplete_work_is_paced_and_not_immediately_reswept() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let pending = seed_work(
        &db,
        user_id,
        PENDING_TITLE,
        EnrichmentStatus::Unenriched,
        IdentityStatus::Pending,
    )
    .await;
    make_due(&db, user_id, &pending).await;

    // Identity resolves, but enrichment fails this pass.
    let workflow = StubEnrichmentWorkflow::failing();
    let svc = service(db.clone(), workflow.clone(), Some(resolving_resolver()));

    let first = svc
        .converge_pending_due(user_id, 10)
        .await
        .expect("summary");
    assert_eq!(first.total, 1);
    assert_eq!(
        first.recovered, 0,
        "enrichment failed, so it did not complete"
    );

    let saved = db.get_work(user_id, pending.id).await.expect("read work");
    assert_eq!(
        saved.identity_status,
        IdentityStatus::Confirmed,
        "Codex-R-001: identity progress is kept even when enrichment fails"
    );
    assert!(
        matches!(
            saved.enrichment_status,
            EnrichmentStatus::Unenriched | EnrichmentStatus::Failed
        ),
        "Codex-R-001: the work is still incomplete and remains eligible for a later tick"
    );

    // R-001: the next immediate tick must NOT re-attempt it — its retry-clock was
    // advanced, so it is no longer due.
    let second = svc
        .converge_pending_due(user_id, 10)
        .await
        .expect("summary");
    assert_eq!(
        second.total, 0,
        "R-001: an incomplete work is backoff-paced, not re-swept every tick"
    );
}

// R-003: convergence shares the bulk-refresh guard, so it does nothing while a
// manual Retry-Incomplete / refresh_all sweep holds the slot for that user.
#[tokio::test]
async fn bulk_guard_blocks_concurrent_run() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let pending = seed_work(
        &db,
        user_id,
        PENDING_TITLE,
        EnrichmentStatus::Unenriched,
        IdentityStatus::Pending,
    )
    .await;
    make_due(&db, user_id, &pending).await;

    let workflow = StubEnrichmentWorkflow::succeeding();
    let svc = service(db.clone(), workflow.clone(), Some(resolving_resolver()));

    // Simulate a manual bulk sweep already in progress for this user.
    let _held = svc
        .try_start_bulk_refresh(user_id)
        .expect("acquire bulk guard");

    let summary = svc
        .converge_pending_due(user_id, 10)
        .await
        .expect("summary");
    assert_eq!(
        summary.total, 0,
        "R-003: convergence yields while a bulk sweep holds the user's slot"
    );
    assert!(
        workflow.work_ids().is_empty(),
        "R-003: nothing is processed while the guard is held"
    );
}

// R-002: a Pending work seeded with a bare ISBN (no work anchor) resolves from its
// OWN seed when no provider responds — it must settle to Provisional and enrich,
// not loop as Pending. (Grounded in english_identity_resolver.rs:154 — no
// responders + seed_has_hard_id → Resolved from the seed.)
#[tokio::test]
async fn isbn_seed_resolves_to_provisional_and_enriches() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    // Seed a Pending work carrying ONLY an ISBN (no OL/GR/HC work anchor).
    let (work, created) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: PENDING_TITLE.to_string(),
            author_name: AUTHOR.to_string(),
            normalized_title: normalize_for_matching(PENDING_TITLE),
            normalized_author: normalize_for_matching(AUTHOR),
            language: Some("en".to_string()),
            isbn_13: Some("9780000000002".to_string()),
            monitor_ebook: true,
            monitor_audiobook: true,
            ..Default::default()
        })
        .await
        .expect("seed work");
    assert!(created);
    db.set_identity_status(user_id, work.id, IdentityStatus::Pending)
        .await
        .expect("set pending");
    let work = db.get_work(user_id, work.id).await.expect("read work");
    make_due(&db, user_id, &work).await;

    // No provider responds → the resolver resolves from the seed's own ISBN.
    let workflow = StubEnrichmentWorkflow::succeeding();
    let svc = service(db.clone(), workflow.clone(), Some(dead_end_resolver()));

    let summary = svc
        .converge_pending_due(user_id, 10)
        .await
        .expect("summary");
    assert_eq!(summary.total, 1);

    let saved = db.get_work(user_id, work.id).await.expect("read work");
    assert_eq!(
        saved.identity_status,
        IdentityStatus::Provisional,
        "R-002: a bare-ISBN resolve settles to Provisional, not stuck Pending"
    );
    assert!(
        workflow.work_ids().contains(&work.id),
        "R-002: a Provisional (bridge) work is enriched, not looped"
    );
}
