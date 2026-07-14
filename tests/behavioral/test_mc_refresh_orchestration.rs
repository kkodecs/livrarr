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
use livrarr_domain::identity::{AnchorSetter, AnchorType, IdentityState, LatencyTier, RawHarvest};
use livrarr_domain::services::{
    EnrichmentMode, RefreshSurface, WorkIdentityRepository, WorkService,
};
use livrarr_domain::{
    normalize_for_matching, IdentityStatus, MetadataProvider, OutcomeClass, RequestPriority, Work,
};
use livrarr_external_data::transport_cache::TransportCache;
use livrarr_external_data::{
    NormalizedWorkDetail, ProviderClient, ProviderOutcome, StubProviderClient,
};
use livrarr_metadata::english_identity_resolver::{LiveEnglishIdentityResolver, ResolverConfig};
use livrarr_metadata::enrichment_workflow_service::ResetOnlyEnrichmentWorkflow;
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
async fn refresh_syncs_gr_key_on_flm_match_then_runs_scatter() {
    // FLM/REQ-008: refresh runs identity completion BEFORE the enrichment scatter;
    // the resolver's fan-out is observed, and a title+author (FLM) match syncs
    // gr_key to works.* immediately — no confirmation needed. Scatter still runs.
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

    let result = svc
        .refresh(user_id, work.id, RefreshSurface::Interactive)
        .await
        .expect("refresh work");
    let refreshed = db
        .get_work(user_id, work.id)
        .await
        .expect("read refreshed work");

    assert!(
        ol.call_count() >= 1,
        "REQ-008: refresh must run the completion fan-out for a gr_key-less work"
    );
    assert_eq!(workflow.reset_call_count(), 1);
    assert_eq!(
        workflow.call_count(),
        1,
        "scatter should still run after the completion attempt"
    );
    assert_eq!(result.work.id, work.id);
    assert_eq!(
        refreshed.gr_key.as_deref(),
        Some("234225"),
        "FLM: title+author match syncs gr_key to works.*"
    );
}

#[tokio::test]
async fn dead_ended_completion_suppression_survives_plain_refresh() {
    // REQ-002/REQ-009 (id-completeness): the refresh chaseable gate
    // (`chaseable_anchor_types`) suppresses re-chasing via the DURABLE
    // per-(work, anchor) dead-end marker (`work_anchor_dead_ends`), never
    // `provider_retry_state`. Here gr_key is the only missing anchor and it has
    // reached the dead-end threshold (3), so a plain refresh makes ZERO resolver
    // calls, leaves the anchor absent, still runs the scatter once, and the
    // durable marker survives the refresh (ST-009). The complementary case — a
    // missing anchor still below threshold IS re-chased — is in
    // test_id_completeness::..._refresh_gate_confirmed_rechases_only_when_missing_obtainable_id.
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
    // The refresh enrichment gate requires a settled identity; this fixture's
    // point is the chaseable gate, so the anchor-rich work is marked Confirmed.
    db.set_identity_status(user_id, work.id, IdentityStatus::Confirmed)
        .await
        .expect("mark work confirmed");
    db.confirm_anchor(
        work.id,
        AnchorType::new(AnchorType::HC_WORK),
        "HC-SUPPRESSED",
        AnchorSetter::User,
    )
    .await
    .expect("seed hc anchor");
    // gr_key (the only missing anchor) has hit the dead-end threshold
    // (DEAD_END_THRESHOLD, PO-locked at 3) — the durable marker the refresh
    // chaseable gate honors.
    for _ in 0..3 {
        db.bump_anchor_attempt(work.id, AnchorType::new(AnchorType::GR_WORK))
            .await
            .expect("seed gr_key dead-end at threshold");
    }
    let workflow = StubEnrichmentWorkflow::succeeding();
    let ol = gr_key_bearing_ol_stub("Suppressed Completion");
    let svc = service_with_resolver(
        db.clone(),
        workflow.clone(),
        resolver_with_stubs(vec![ol.clone()]),
    );

    svc.refresh(user_id, work.id, RefreshSurface::Interactive)
        .await
        .expect("refresh work");

    assert_eq!(
        ol.call_count(),
        0,
        "a dead-ended missing anchor is not re-chased on a plain refresh"
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
        "a dead-ended completion leaves the missing provider anchor absent"
    );
    let dead_ends = db
        .list_anchor_dead_ends(work.id)
        .await
        .expect("read dead-ends after refresh");
    assert_eq!(
        dead_ends
            .iter()
            .find(|d| d.anchor_type.as_str() == AnchorType::GR_WORK)
            .expect("gr_key dead-end survives plain refresh")
            .attempt_count,
        3,
        "the durable dead-end marker survives a plain refresh (ST-009)"
    );
}

#[tokio::test]
async fn not_found_interactive_refresh_clears_dead_ends_and_rechases() {
    // REQ-009: a single-work, user-clicked try-again refresh for an Unverified
    // (`NotFound`) work must clear durable anchor dead-ends before the chaseable
    // gate runs, so a previously exhausted missing anchor is actually re-chased.
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (work, created) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Try Again Completion".to_string(),
            author_name: "Refresh Audit".to_string(),
            normalized_title: normalize_for_matching("Try Again Completion"),
            normalized_author: normalize_for_matching("Refresh Audit"),
            ol_key: Some("OL777000W".to_string()),
            isbn_13: Some("9780441013593".to_string()),
            asin: Some("B000TEST".to_string()),
            language: Some("en".to_string()),
            monitor_ebook: true,
            ..Default::default()
        })
        .await
        .expect("seed not-found work missing only gr_key");
    assert!(created);
    db.set_identity_status(user_id, work.id, IdentityStatus::NotFound)
        .await
        .expect("mark work not found");
    db.confirm_anchor(
        work.id,
        AnchorType::new(AnchorType::HC_WORK),
        "HC-TRY-AGAIN",
        AnchorSetter::User,
    )
    .await
    .expect("seed hc anchor");
    for _ in 0..3 {
        db.bump_anchor_attempt(work.id, AnchorType::new(AnchorType::GR_WORK))
            .await
            .expect("seed gr_key dead-end at threshold");
    }
    // The try-again door depends on the workflow reset's REAL DB semantics:
    // `reset_for_manual_refresh` recovers the terminal NotFound from the anchor
    // columns before settle_identity runs. The counting stub performs no DB
    // write, which leaves the work NotFound and trips settle's REQ-006 terminal
    // guard — so this pin wires the reset-only production workflow (scatter is
    // outside this pin's charter).
    let ol = gr_key_bearing_ol_stub("Try Again Completion");
    let svc = WorkServiceImpl::new(
        db.clone(),
        ResetOnlyEnrichmentWorkflow::new(db.clone()),
        StubHttpFetcher::new(),
        tempfile::tempdir()
            .expect("test data dir")
            .path()
            .to_path_buf(),
    )
    .with_resolver(std::sync::Arc::new(resolver_with_stubs(vec![ol.clone()])));

    svc.refresh(user_id, work.id, RefreshSurface::Interactive)
        .await
        .expect("try-again refresh work");

    let dead_ends = db
        .list_anchor_dead_ends(work.id)
        .await
        .expect("read dead-ends after try-again refresh");
    let resolver_calls = ol.call_count();
    assert_eq!(
        (dead_ends.is_empty(), resolver_calls > 0),
        (true, true),
        "REQ-009: NotFound interactive try-again refresh must clear anchor dead-ends and re-chase; dead_ends={dead_ends:?}, resolver_calls={resolver_calls}"
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

    svc.refresh(user_id, work.id, RefreshSurface::Interactive)
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
    svc.refresh(user_id, work.id, RefreshSurface::Interactive)
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
async fn refresh_adopts_goodreads_key_for_one_sided_subtitle_with_isbn_bridge() {
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let isbn = "9780307346612";
    let (work, created) = db
        .create_work(CreateWorkDbRequest {
            isbn_13: Some(isbn.to_string()),
            ..work_req(user_id, "World War Z")
        })
        .await
        .expect("seed bare-main work with isbn bridge");
    assert!(created);
    db.set_identity_status(user_id, work.id, IdentityStatus::Confirmed)
        .await
        .expect("mark work confirmed");
    let gr = gr_stub(
        "World War Z: An Oral History of the Zombie War",
        None,
        None,
        Some(isbn),
        "234225",
    );
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
            .gr_key
            .as_deref(),
        Some("234225"),
        "one-sided subtitle plus matching ISBN should trust and persist the Goodreads key"
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
    db.set_identity_status(user_id, work.id, IdentityStatus::Confirmed)
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
