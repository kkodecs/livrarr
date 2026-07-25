//! FINDING B repro — one question, one test.
//!
//! **Not a contract file.** This is a probe written for the identity-edit
//! implementation contest (2026-07-24) to settle a disputed claim with evidence
//! instead of argument. It is deliberately standalone: the merged contract suites
//! (`test_identity_edit.rs`, `test_identity_edit_durable.rs`) are frozen and
//! byte-audited across the competing trees, so nothing here touches them.
//!
//! # The claim under test
//!
//! Design `docs/design-identity-edit.md` — AC-10 "writer-coverage race matrix",
//! first row of the writer-coverage table (`refresh settle`):
//!
//! > for refresh settle, `complete_add`, add-time anchorless settle, mid-enrichment
//! > completion, retry-incomplete, and convergence, capture generation G, block
//! > after resolve, commit edit/clear, release → completion returns Superseded and
//! > writes no old anchor, pending guess, review state, conflict, or badge.
//!
//! The merged contract suite's three AC-10 tests bind only the four *user-action*
//! doors (pending affirm, review apply/dismiss, conflict resolve/dismiss) over
//! HTTP. None of them drives a *background* road. So an implementation can pass the
//! whole suite while leaving every road in that table unguarded — which is exactly
//! the gap FINDING B alleges against one contest entry.
//!
//! This test closes that hole in the evidence. It drives the real refresh road
//! through the real service, parks it inside the real provider await, clears the
//! slot through the real HTTP door, and then asks the only question that matters:
//!
//! **After the user says "this is not that book", does the in-flight resolver put
//! the old book back?**
//!
//! # Why this shape
//!
//! The barrier is the provider call itself: a `StubProviderClient` with a delay is
//! the harness's sanctioned way to hold a resolver inside its fetch
//! (`crates/livrarr-external-data/src/provider_client.rs` — `with_delay`, "so tests
//! can drive the resolver's per-call timeout"). That is precisely AC-10's "block
//! after resolve". The test never proceeds on a timer: it waits until the stub's
//! `call_count` proves the resolver is genuinely in flight, and it fails loudly if
//! the road never dispatched at all — a silent no-dispatch must not be mistaken for
//! a passing race.
//!
//! Everything the final assertion reads was written by production code on a
//! production path: the real `WorkService::refresh`, the real resolver, the real
//! completion primitive, and the real HTTP clear door.
//!
//! The test binds only the `WorkService` trait and the HTTP route — the one surface
//! both competing trees are known to share — so it compiles and runs unmodified on
//! either. An implementation that threads the expected generation into its
//! completion primitives passes. One that leaves `merge_missing_anchors`
//! unconditional fails, and the message names the resurrected key.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::routing::put;
use axum::Router;
use livrarr_behavioral::stubs::{create_test_user, StubEnrichmentWorkflow, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{CreateWorkDbRequest, UserDb, WorkDbCreate};
use livrarr_domain::identity::{AnchorSetter, AnchorType};
use livrarr_domain::normalize_for_matching;
use livrarr_domain::services::{RefreshSurface, WorkIdentityRepository, WorkService};
use livrarr_domain::{AuthType, MetadataProvider, UserId, WorkId};
use livrarr_external_data::transport_cache::TransportCache;
use livrarr_external_data::{
    NormalizedWorkDetail, ProviderClient, ProviderOutcome, StubProviderClient,
};
use livrarr_handlers::context::{
    HasHistoryService, HasIdentityConflictService, HasWorkIdentityRepository, HasWorkService,
};
use livrarr_handlers::AuthContext;
use livrarr_metadata::english_identity_resolver::{LiveEnglishIdentityResolver, ResolverConfig};
use livrarr_metadata::work_service::WorkServiceImpl;
use livrarr_server::history_service::HistoryServiceImpl;
use livrarr_server::services::identity_conflict_service::LiveIdentityConflictService;
use tower::ServiceExt;

/// The book the work was wrongly settled on before the user intervened.
const OLD_GR_KEY: &str = "9999";

/// Long enough that the clear lands well inside the window, short enough that a
/// broken run fails fast rather than hanging the suite.
const PROVIDER_HOLD: Duration = Duration::from_secs(5);

type TestWorkService = WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher>;
type TestHistoryService = HistoryServiceImpl<SqliteDb>;

// ---------------------------------------------------------------------------
// Route scaffolding (mirrors the contract suite's, trimmed to the clear door)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct RouteState {
    work_service: Arc<TestWorkService>,
    identity_repo: SqliteDb,
    history_service: Arc<TestHistoryService>,
    conflict_service: Arc<LiveIdentityConflictService>,
}

impl HasWorkService for RouteState {
    type WorkSvc = TestWorkService;

    fn work_service(&self) -> &Self::WorkSvc {
        &self.work_service
    }
}

impl HasWorkIdentityRepository for RouteState {
    type WorkIdentityRepo = SqliteDb;

    fn work_identity_repo(&self) -> &Self::WorkIdentityRepo {
        &self.identity_repo
    }
}

impl HasHistoryService for RouteState {
    type HistorySvc = TestHistoryService;

    fn history_service(&self) -> &Self::HistorySvc {
        &self.history_service
    }
}

impl HasIdentityConflictService for RouteState {
    type IdentityConflictSvc = LiveIdentityConflictService;

    fn identity_conflict_service(&self) -> &Self::IdentityConflictSvc {
        &self.conflict_service
    }
}

fn identity_app(state: RouteState) -> Router {
    Router::new()
        .route(
            "/work/{id}/identity/{slot}",
            put(livrarr_handlers::work::commit_identity_edit::<RouteState>)
                .delete(livrarr_handlers::work::clear_identity_slot::<RouteState>),
        )
        .with_state(state)
}

// ---------------------------------------------------------------------------
// The barrier: a provider that still believes the old book, and answers slowly
// ---------------------------------------------------------------------------

/// A Goodreads payload that re-asserts the OLD book, held open for `PROVIDER_HOLD`
/// so the user can intervene while the resolver is mid-flight. Title and author
/// match the work so the resolver takes its auto-confirm (FLM) branch — the branch
/// that writes anchors.
fn stale_goodreads_provider() -> StubProviderClient {
    StubProviderClient::new(
        MetadataProvider::Goodreads,
        ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
            title: Some("The Certified Book".to_string()),
            author_name: Some("Case Writer".to_string()),
            gr_key: Some(OLD_GR_KEY.to_string()),
            language: Some("en".to_string()),
            ..NormalizedWorkDetail::default()
        })),
    )
    .with_delay(PROVIDER_HOLD)
}

fn service_with_resolver(db: SqliteDb, provider: StubProviderClient) -> TestWorkService {
    let resolver = LiveEnglishIdentityResolver {
        clients: std::collections::HashMap::from([(
            MetadataProvider::Goodreads,
            ProviderClient::Stub(provider),
        )]),
        cache: Arc::new(TransportCache::new(Duration::from_secs(30))),
        config: ResolverConfig {
            gb_key_present: false,
            // Must outlast the hold, or the fan-out abstains and the race never
            // happens (an abstention would silently "pass" this test).
            call_timeout: PROVIDER_HOLD * 4,
            ..ResolverConfig::default()
        },
    };
    // The enrichment scatter is stubbed on purpose: this test is about the identity
    // leg's write, and a live scatter would only add traffic to reason about.
    WorkServiceImpl::new(
        db,
        StubEnrichmentWorkflow::succeeding(),
        StubHttpFetcher::new(),
        tempfile::tempdir()
            .expect("repro data dir")
            .path()
            .to_path_buf(),
    )
    .with_resolver(Arc::new(resolver))
}

async fn auth_context(db: &SqliteDb, user_id: UserId) -> AuthContext {
    AuthContext {
        user: db.get_user(user_id).await.expect("seeded user"),
        auth_type: AuthType::Session,
        session_token_hash: Some("identity-edit-repro-session".to_string()),
    }
}

/// The real user action: "this is not that book — clear it."
async fn clear_gr_slot(
    app: &Router,
    db: &SqliteDb,
    user_id: UserId,
    work_id: WorkId,
) -> (StatusCode, String) {
    let mut request = Request::builder()
        .method(Method::DELETE)
        .uri(format!("/work/{work_id}/identity/gr_work"))
        .body(Body::empty())
        .expect("build clear request");
    request
        .extensions_mut()
        .insert(auth_context(db, user_id).await);

    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("clear route response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read clear body");
    (status, String::from_utf8_lossy(&bytes).to_string())
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

/// REQ-IDs: AC-10 (writer-coverage race matrix — `refresh settle` row)
/// Directive: A refresh resolver parked mid-flight must not re-confirm the old
/// book into a slot the user cleared while it was waiting.
#[tokio::test]
async fn refresh_settle_must_not_restore_an_anchor_the_user_cleared_mid_flight() {
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;

    // A work already settled on the WRONG book, with its sibling slots empty so the
    // refresh chase gate opens and the identity leg genuinely runs.
    let (work, created) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "The Certified Book".to_string(),
            author_name: "Case Writer".to_string(),
            normalized_title: normalize_for_matching("The Certified Book"),
            normalized_author: normalize_for_matching("Case Writer"),
            language: Some("en".to_string()),
            ..CreateWorkDbRequest::default()
        })
        .await
        .expect("seed work");
    assert!(created, "the repro seeds a fresh work");
    let work_id = work.id;

    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::GR_WORK),
        OLD_GR_KEY,
        AnchorSetter::AutoSearch,
    )
    .await
    .expect("seed the old-book GR anchor");

    let provider = stale_goodreads_provider();
    let service = Arc::new(service_with_resolver(db.clone(), provider.clone()));
    let app = identity_app(RouteState {
        work_service: service.clone(),
        identity_repo: db.clone(),
        history_service: Arc::new(HistoryServiceImpl::new(db.clone())),
        conflict_service: Arc::new(LiveIdentityConflictService::new(db.clone())),
    });

    // 1. Start the background road. It takes its coherent read of the work, then
    //    parks inside the provider await still holding the OLD identity.
    let refresh_service = service.clone();
    let refresh = tokio::spawn(async move {
        refresh_service
            .refresh(user_id, work_id, RefreshSurface::Interactive)
            .await
    });

    // 2. Do not proceed on a timer — proceed on proof that the resolver dispatched.
    //    If the road never reaches the provider there is no race to observe, and a
    //    green result here would be meaningless.
    let dispatched = tokio::time::timeout(Duration::from_secs(20), async {
        while provider.call_count() == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        dispatched.is_ok(),
        "the refresh road never dispatched to the identity provider, so this test observed \
         nothing. Fix the harness before trusting any verdict from it."
    );

    // 3. While the resolver is parked, the user clears the slot through the real door.
    let (status, body) = clear_gr_slot(&app, &db, user_id, work_id).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "clear door rejected the edit: {body}"
    );

    let cleared_column: Option<String> =
        sqlx::query_scalar("SELECT gr_key FROM works WHERE id = ?")
            .bind(work_id)
            .fetch_one(db.pool())
            .await
            .expect("read works.gr_key after clear");
    assert_eq!(
        cleared_column, None,
        "precondition: the clear door must actually empty the slot before the race means anything"
    );

    // The post-clear ledger is the baseline. Whatever residue the clear itself
    // leaves (a superseded tombstone is legitimate) must survive the race
    // untouched — the contract is that the stale completion writes NOTHING, not
    // that the slot is scrubbed to a particular shape.
    let ledger_after_clear = gr_ledger(&db, work_id).await;

    // 4. Release the resolver into a world that changed under it.
    tokio::time::timeout(Duration::from_secs(60), refresh)
        .await
        .expect("refresh finished within 60s")
        .expect("refresh task did not panic")
        .expect("refresh returned Ok");

    // 5. The question. The slot the user emptied must still be empty.
    let gr_column: Option<String> = sqlx::query_scalar("SELECT gr_key FROM works WHERE id = ?")
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .expect("read works.gr_key after the race");
    let ledger_after_race = gr_ledger(&db, work_id).await;

    assert_eq!(
        gr_column, None,
        "works.gr_key came back as {gr_column:?} after the user cleared it — the in-flight \
         resolver wrote the old book into an emptied slot. The completion primitive on the \
         refresh road is not guarded by the identity generation."
    );
    assert_eq!(
        ledger_after_race, ledger_after_clear,
        "the gr_work ledger changed while a stale resolver completed: {ledger_after_clear:?} \
         before release, {ledger_after_race:?} after. AC-10 requires that completion to be \
         superseded and to write no anchor, pending guess, review state, conflict or badge."
    );
}

/// Every gr_work ledger row, ordered so two snapshots compare cleanly.
async fn gr_ledger(db: &SqliteDb, work_id: WorkId) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT anchor_value, confidence FROM work_identity_anchors \
         WHERE work_id = ? AND anchor_type = 'gr_work' \
         ORDER BY anchor_value, confidence",
    )
    .bind(work_id)
    .fetch_all(db.pool())
    .await
    .expect("read gr_work ledger rows")
}
