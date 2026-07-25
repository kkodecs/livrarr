#![allow(dead_code, unused_imports)]

//! Feature-gated red-first behavioral suite for design identity-edit r4.
//! MERGED SUITE (2026-07-24): codex-authored base (chosen for fixture fidelity — the
//! HTTP fixture stops at the outbound boundary, so previews traverse the real Goodreads
//! client/parser, provider queue, enrichment workflow adapter, work service, handler,
//! and SQLite repository; deterministic SQLite-trigger race barriers) + CC-authored
//! net-new pins appended at the end (marked "CC-merged"). Merge record:
//! build/reviews/identity-edit/suite-merge-notes.md. Fixes applied at merge: conflicts
//! table name (work_identity_conflicts, 5 sites), exact +1 generation asserts relaxed
//! to monotonic (the design allows >1 increment per composite transaction).
//!
//! Registered behind `required-features = ["identity_edit_red"]` (staged-red protocol;
//! see test_identity_edit_durable.rs for the pins that run red TODAY). At
//! implementation kickoff: land signatures -> this file compiles -> every test RED ->
//! implement -> green. Import paths are alignment-only; contracts are not.
//!
//! Known gaps deferred to implementation time (add before the code gate closes):
//! - AC-24 global-saturation arm (64 live tokens -> new tenant 503 preview_capacity).
//! - AC-4 OL-agrees/OL-disagrees/outage-drop sibling arms (need OL/HC endpoints on the
//!   local provider fixture; only the GR leg is wired today).
//! - AC-12 SQLITE_BUSY-exhausted / SQLITE_FULL 503 taxonomy injection.
//! - FE vitest + Playwright per the design's Frontend section.

#[path = "common.rs"]
mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::routing::{delete, get, post, put};
use axum::Router;
use livrarr_behavioral::stubs::{create_second_test_user, create_test_user, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{
    ApplyEnrichmentMergeRequest, CreateUserDbRequest, CreateWorkDbRequest, HistoryDb, UserDb,
    WorkDb, WorkDbCreate,
};
use livrarr_domain::history_events;
use livrarr_domain::identity::{
    AnchorConfidence, AnchorSetter, AnchorType, Candidate, CandidateId, CapturedIdentity,
    ConflictSource, IdentityConflictKind, IncomingConflictPayload, NewIdentityConflict,
    ResolutionScore,
};
use livrarr_domain::identity_edit::{classify_identifier_input, ClassifyError, IdentityEditError};
use livrarr_domain::seed::{seed_add_box, SeedInput, SeedLanguage};
use livrarr_domain::services::{IdentityConflictService, WorkIdentityRepository, WorkService};
use livrarr_domain::{
    ApplyMergeOutcome, AuthType, EnrichmentStatus, EventType, HistoryFilter, IdentityStatus,
    MetadataProvider, User, UserId, UserRole, WorkId,
};
use livrarr_enrichment::{DefaultProviderQueueBuilder, ProviderQueueConfig};
use livrarr_external_data::{GoodreadsClient, ProviderClient};
use livrarr_handlers::context::{
    HasHistoryService, HasIdentityConflictService, HasWorkIdentityRepository, HasWorkService,
};
use livrarr_handlers::AuthContext;
use livrarr_http::{fetcher::HttpFetcherImpl, HttpClient};
use livrarr_metadata::enrichment_workflow_service::EnrichmentWorkflowImpl;
use livrarr_metadata::work_service::WorkServiceImpl;
use livrarr_metadata::{DefaultMergeEngine, EnrichmentServiceImpl, PriorityModel};
use livrarr_server::history_service::HistoryServiceImpl;
use livrarr_server::services::identity_conflict_service::LiveIdentityConflictService;
use serde_json::{json, Value};
use sqlx::Row;
use tower::ServiceExt;

type TestHistoryService = HistoryServiceImpl<SqliteDb>;

struct RouteState<W> {
    work_service: Arc<W>,
    identity_repo: SqliteDb,
    history_service: Arc<TestHistoryService>,
    conflict_service: Arc<LiveIdentityConflictService>,
}

impl<W> Clone for RouteState<W> {
    fn clone(&self) -> Self {
        Self {
            work_service: self.work_service.clone(),
            identity_repo: self.identity_repo.clone(),
            history_service: self.history_service.clone(),
            conflict_service: self.conflict_service.clone(),
        }
    }
}

impl<W> HasWorkService for RouteState<W>
where
    W: WorkService + Send + Sync + 'static,
{
    type WorkSvc = W;

    fn work_service(&self) -> &Self::WorkSvc {
        &self.work_service
    }
}

impl<W> HasWorkIdentityRepository for RouteState<W>
where
    W: WorkService + Send + Sync + 'static,
{
    type WorkIdentityRepo = SqliteDb;

    fn work_identity_repo(&self) -> &Self::WorkIdentityRepo {
        &self.identity_repo
    }
}

impl<W> HasHistoryService for RouteState<W>
where
    W: WorkService + Send + Sync + 'static,
{
    type HistorySvc = TestHistoryService;

    fn history_service(&self) -> &Self::HistorySvc {
        &self.history_service
    }
}

impl<W> HasIdentityConflictService for RouteState<W>
where
    W: WorkService + Send + Sync + 'static,
{
    type IdentityConflictSvc = LiveIdentityConflictService;

    fn identity_conflict_service(&self) -> &Self::IdentityConflictSvc {
        &self.conflict_service
    }
}

fn identity_app<S>(state: S) -> Router
where
    S: HasWorkService + HasWorkIdentityRepository + HasHistoryService + HasIdentityConflictService,
{
    Router::new()
        .route(
            "/work/{id}/identity/preview",
            post(livrarr_handlers::work::preview_identity_edit::<S>),
        )
        .route(
            "/work/{id}/identity/{slot}",
            put(livrarr_handlers::work::commit_identity_edit::<S>)
                .delete(livrarr_handlers::work::clear_identity_slot::<S>),
        )
        .route(
            "/work/{id}/pending-anchors/{anchor_type}/affirm",
            post(livrarr_handlers::work::affirm_pending_anchor::<S>),
        )
        .route(
            "/identity-review/{work_id}/resolve",
            post(livrarr_handlers::identity_review::resolve::<S>),
        )
        .route(
            "/identity-review/{work_id}/dismiss",
            post(livrarr_handlers::identity_review::dismiss::<S>),
        )
        .route(
            "/identity-conflict/{id}/resolve",
            post(livrarr_handlers::identity_conflicts::resolve::<S>),
        )
        .route(
            "/identity-conflict/{id}/dismiss",
            post(livrarr_handlers::identity_conflicts::dismiss::<S>),
        )
        .with_state(state)
}

async fn spawn_goodreads(status: StatusCode) -> String {
    let html = r#"<html><script type="application/ld+json">
        {
          "@context":"https://schema.org",
          "@type":"Book",
          "name":"The Certified Book",
          "author":[{"@type":"Person","name":"Case Writer"}],
          "datePublished":"2004",
          "inLanguage":"English"
        }
        </script></html>"#
        .to_string();
    let app = Router::new().route(
        "/book/show/{id}",
        get(move || {
            let html = html.clone();
            async move {
                (
                    status,
                    [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    html,
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind Goodreads fixture");
    let address = listener.local_addr().expect("fixture address");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve Goodreads fixture");
    });
    format!("http://{address}")
}

fn work_service(
    db: SqliteDb,
    goodreads_base_url: String,
) -> impl WorkService + Send + Sync + 'static {
    let fetcher = HttpFetcherImpl::new().expect("real HTTP fetcher");
    let http = HttpClient::builder().build().expect("real HTTP client");
    let goodreads =
        GoodreadsClient::new(fetcher, http, goodreads_base_url).with_retry_backoff(0);
    let db_arc = Arc::new(db.clone());
    let queue = DefaultProviderQueueBuilder::new()
        .add_provider(
            MetadataProvider::Goodreads,
            ProviderClient::Goodreads(goodreads),
            ProviderQueueConfig {
                provider: MetadataProvider::Goodreads,
                max_attempts: 1,
            },
        )
        .build(db_arc.clone());
    let enrichment = EnrichmentServiceImpl::new(
        db_arc,
        Arc::new(queue),
        Arc::new(DefaultMergeEngine::new(PriorityModel::english())),
        false,
    );
    let workflow = EnrichmentWorkflowImpl::new(Arc::new(enrichment));
    WorkServiceImpl::new(
        db,
        workflow,
        StubHttpFetcher::new(),
        tempfile::tempdir().expect("identity-edit data dir").keep(),
    )
}

fn route_state<W>(db: SqliteDb, work_service: W) -> RouteState<W>
where
    W: WorkService + Send + Sync + 'static,
{
    RouteState {
        work_service: Arc::new(work_service),
        identity_repo: db.clone(),
        history_service: Arc::new(HistoryServiceImpl::new(db.clone())),
        conflict_service: Arc::new(LiveIdentityConflictService::new(db)),
    }
}

async fn auth_context(db: &SqliteDb, user_id: UserId) -> AuthContext {
    AuthContext {
        user: db.get_user(user_id).await.expect("seeded user"),
        auth_type: AuthType::Session,
        session_token_hash: Some("identity-edit-session".to_string()),
    }
}

async fn create_user(db: &SqliteDb, suffix: &str) -> UserId {
    db.create_user(CreateUserDbRequest {
        username: format!("identity-edit-{suffix}"),
        password_hash: "hash".to_string(),
        role: UserRole::User,
        api_key_hash: format!("identity-edit-key-{suffix}"),
    })
    .await
    .expect("create user")
    .id
}

async fn create_work(db: &SqliteDb, user_id: UserId, title: &str) -> WorkId {
    db.create_work(CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: "Case Writer".to_string(),
        normalized_title: title.to_ascii_lowercase(),
        normalized_author: "case writer".to_string(),
        language: Some("en".to_string()),
        ..CreateWorkDbRequest::default()
    })
    .await
    .expect("create work")
    .0
    .id
}

async fn call(
    app: &Router,
    method: Method,
    uri: String,
    body: Option<Value>,
    auth: Option<AuthContext>,
) -> (StatusCode, HeaderMap, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    let request_body = if let Some(value) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        Body::from(value.to_string())
    } else {
        Body::empty()
    };
    let mut request = builder.body(request_body).expect("build request");
    if let Some(auth) = auth {
        request.extensions_mut().insert(auth);
    }
    let response = app.clone().oneshot(request).await.expect("route response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("JSON response")
    };
    (status, headers, json)
}

async fn preview(
    app: &Router,
    db: &SqliteDb,
    user_id: UserId,
    work_id: WorkId,
    input: &str,
    slot: Option<&str>,
) -> (StatusCode, HeaderMap, Value) {
    call(
        app,
        Method::POST,
        format!("/work/{work_id}/identity/preview"),
        Some(json!({"input": input, "slot": slot})),
        Some(auth_context(db, user_id).await),
    )
    .await
}

async fn commit(
    app: &Router,
    db: &SqliteDb,
    user_id: UserId,
    work_id: WorkId,
    slot: &str,
    preview_id: &str,
) -> (StatusCode, HeaderMap, Value) {
    call(
        app,
        Method::PUT,
        format!("/work/{work_id}/identity/{slot}"),
        Some(json!({"preview_id": preview_id})),
        Some(auth_context(db, user_id).await),
    )
    .await
}

fn preview_id(body: &Value) -> &str {
    body["previewId"]
        .as_str()
        .expect("certifiable preview has previewId")
}

async fn generation(db: &SqliteDb, work_id: WorkId) -> i64 {
    sqlx::query_scalar("SELECT identity_generation FROM works WHERE id = ?")
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .expect("identity generation")
}

fn incoming(gr_key: Option<&str>, ol_key: Option<&str>) -> IncomingConflictPayload {
    IncomingConflictPayload {
        ol_key: ol_key.map(str::to_string),
        gr_key: gr_key.map(str::to_string),
        hc_key: None,
        isbn_13: None,
        asin: None,
        title: "The Certified Book".to_string(),
        author_name: "Case Writer".to_string(),
        year: Some(2004),
        cover_url: None,
        top_candidates: vec![],
    }
}

fn review_candidate(id: &str, title: &str) -> Candidate {
    Candidate {
        candidate_id: CandidateId(id.to_string()),
        anchors: CapturedIdentity {
            ol_key: Some("OL777W".to_string()),
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
            title: title.to_string(),
            author_name: "Case Writer".to_string(),
            language: Some("en".to_string()),
        },
        cover_url: None,
        sources: vec![MetadataProvider::OpenLibrary],
        score: ResolutionScore {
            title_jaccard: 0.95,
            author_overlap: 1,
            runner_up_delta: 0.25,
        },
        existing_work_id: None,
    }
}

async fn raise_conflict(
    db: &SqliteDb,
    user_id: UserId,
    work_id: WorkId,
    kind: IdentityConflictKind,
) -> i64 {
    db.raise_identity_conflict(NewIdentityConflict {
        user_id,
        existing_work_id: work_id,
        kind,
        incoming: incoming(Some("999999"), Some("OL999W")),
        raised_by: ConflictSource::Refresh,
        raised_source_path: None,
    })
    .await
    .expect("raise identity conflict")
}

async fn race_generation_claim(
    app: &Router,
    db: &SqliteDb,
    work_id: WorkId,
    request: Request<Body>,
) -> (StatusCode, Value) {
    // A one-shot SQLite trigger is the deterministic barrier at the exact
    // first-statement claim. The handler completes its coherent reads; when its
    // conditional generation UPDATE begins, the trigger performs the competing
    // generation bump and ignores the outer row. SQLite reports zero rows to
    // the production repository, exactly as when another writer wins between
    // the door read and claim, without scheduler-dependent timing.
    sqlx::query("PRAGMA recursive_triggers = OFF")
        .execute(db.pool())
        .await
        .expect("disable recursive trigger execution");
    sqlx::query(&format!(
        "CREATE TRIGGER lose_identity_generation_claim \
         BEFORE UPDATE OF identity_generation ON works WHEN OLD.id = {work_id} \
         BEGIN \
           UPDATE works SET identity_generation = OLD.identity_generation + 1 \
             WHERE id = OLD.id; \
           SELECT RAISE(IGNORE); \
         END"
    ))
    .execute(db.pool())
    .await
    .expect("install generation-claim barrier");

    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("generation-race route response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read race error");
    (
        status,
        serde_json::from_slice(&bytes).expect("race error JSON"),
    )
}

async fn authenticated_request(
    db: &SqliteDb,
    user_id: UserId,
    method: Method,
    uri: String,
    body: Option<Value>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    let request_body = if let Some(value) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        Body::from(value.to_string())
    } else {
        Body::empty()
    };
    let mut request = builder.body(request_body).expect("build race request");
    request
        .extensions_mut()
        .insert(auth_context(db, user_id).await);
    request
}

fn history_filter() -> HistoryFilter {
    HistoryFilter {
        event_type: Some(EventType::IdentityResolved),
        work_id: None,
        start_date: None,
        end_date: None,
    }
}

/// REQ-IDs: AC-2
/// Directive: The one pure classifier applies the complete precedence and normalization table.
#[test]
fn classification_authority_handles_urls_bare_values_checksums_and_overlap() {
    let cases = [
        (
            "https://www.goodreads.com/book/show/12345.Some_Title",
            None,
            AnchorType::GR_WORK,
            "12345",
        ),
        (
            "https://openlibrary.org/works/OL12345W",
            None,
            AnchorType::OL_WORK,
            "OL12345W",
        ),
        ("OL12345W", None, AnchorType::OL_WORK, "OL12345W"),
        (
            "978-0-306-40615-7",
            None,
            AnchorType::ISBN_13,
            "9780306406157",
        ),
        ("0-306-40615-2", None, AnchorType::ISBN_13, "9780306406157"),
        ("097522980X", None, AnchorType::ISBN_13, "9780975229804"),
        (
            "https://www.amazon.com/dp/B08N5WRWNW",
            None,
            AnchorType::ASIN,
            "B08N5WRWNW",
        ),
        ("B0ABC12345", None, AnchorType::ASIN, "B0ABC12345"),
        ("1234567891", None, AnchorType::GR_WORK, "1234567891"),
    ];

    for (input, hint, expected_type, expected_value) in cases {
        let (anchor_type, canonical) =
            classify_identifier_input(input, hint).expect("classify valid input");
        assert_eq!(anchor_type.as_str(), expected_type, "{input}");
        assert_eq!(canonical, expected_value, "{input}");
    }

    assert!(matches!(
        classify_identifier_input("OL12345M", None),
        Err(ClassifyError::EditionKey)
    ));
    assert!(matches!(
        classify_identifier_input("0-306-40615-2", Some(AnchorType::new(AnchorType::ASIN))),
        Err(ClassifyError::WrongSlot { .. })
    ));
    assert!(classify_identifier_input("not an identifier", None).is_err());
}

/// REQ-IDs: AC-1, AC-25
/// Directive: A pasted GR URL traverses the real provider stack and returns a certifiable canonical preview.
#[tokio::test]
async fn gr_url_preview_returns_certified_record_and_opaque_token() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "Preview Target").await;
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));

    let (status, _, body) = preview(
        &app,
        &db,
        user_id,
        work_id,
        "https://www.goodreads.com/book/show/12345.Some_Title",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["resolved"]["title"], "The Certified Book");
    assert_eq!(body["resolved"]["author"], "Case Writer");
    assert_eq!(body["resolved"]["slot"], "gr_work");
    assert_eq!(body["resolved"]["canonicalValue"], "12345");
    assert!(!preview_id(&body).is_empty());
}

/// REQ-IDs: AC-5, AC-12
/// Directive: Provider failure is a non-certifiable 200 and an absent token cannot be committed.
#[tokio::test]
async fn provider_failure_returns_no_snapshot_and_commit_requires_one() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "Provider Failure").await;
    let base = spawn_goodreads(StatusCode::SERVICE_UNAVAILABLE).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));

    let (status, _, body) = preview(&app, &db, user_id, work_id, "12345", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["resolved"].is_null());
    assert!(body.get("previewId").is_none() || body["previewId"].is_null());

    let (status, _, error) = commit(&app, &db, user_id, work_id, "gr_work", "never-issued").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["details"]["code"], "preview_required");
}

/// REQ-IDs: AC-3, AC-17
/// Directive: Collision previews expose only a same-user owner and never leak another tenant.
#[tokio::test]
async fn collision_preview_unions_ledger_and_columns_without_cross_tenant_leakage() {
    let db = common::create_test_db().await;
    let user_a = create_test_user(&db).await;
    let user_b = create_second_test_user(&db).await;
    let owner = create_work(&db, user_a, "Visible Owner").await;
    let same_user_target = create_work(&db, user_a, "Same User Target").await;
    let other_user_target = create_work(&db, user_b, "Other User Target").await;
    db.confirm_anchor(
        owner,
        AnchorType::new(AnchorType::GR_WORK),
        "12345",
        AnchorSetter::User,
    )
    .await
    .expect("seed owner");
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));

    let (status, _, blocked) = preview(&app, &db, user_a, same_user_target, "12345", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(blocked["collision"]["owningWorkId"], owner);
    assert_eq!(blocked["collision"]["owningWorkTitle"], "Visible Owner");
    assert!(blocked.get("previewId").is_none() || blocked["previewId"].is_null());

    let (status, _, allowed) = preview(&app, &db, user_b, other_user_target, "12345", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(allowed.get("collision").is_none() || allowed["collision"].is_null());
    assert!(!preview_id(&allowed).is_empty());
    assert!(!allowed.to_string().contains("Visible Owner"));
    // RULING 2026-07-24 (contest): the original bare-substring form
    // `contains(&owner.to_string())` is unpassable, not merely weak — `owner` is the
    // first work in a fresh test DB, so its id is "1", and the certified response this
    // very test requires at :635 carries canonicalValue "12345". No design-conformant
    // response can satisfy both. (An opaque previewId makes it flaky besides.)
    // Three contest entries independently reached this conclusion; the value-level form
    // below keeps the leak check AC-3 actually names — the owner id is emitted only as
    // `owningWorkId`, inside the collision block already asserted absent at :634.
    // Applied to the shared suite so every side is judged against the same corrected
    // assertion; no entry is credited or penalized for the inherited defect.
    assert!(!allowed
        .to_string()
        .contains(&format!("\"owningWorkId\":{owner}")));
}

/// REQ-IDs: AC-3, AC-12
/// Directive: Commit rechecks collision ownership and returns the structured same-user merge handoff.
#[tokio::test]
async fn owner_claim_after_preview_is_anchor_collision_not_internal_error() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let target = create_work(&db, user_id, "Collision Race Target").await;
    let owner = create_work(&db, user_id, "Collision Race Winner").await;
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));
    let (_, _, body) = preview(&app, &db, user_id, target, "12345", None).await;

    db.confirm_anchor(
        owner,
        AnchorType::new(AnchorType::GR_WORK),
        "12345",
        AnchorSetter::User,
    )
    .await
    .expect("competing owner wins after preview");
    let (status, _, error) = commit(&app, &db, user_id, target, "gr_work", preview_id(&body)).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["details"]["code"], "anchor_collision");
    assert_eq!(error["details"]["owningWorkId"], owner);
    assert_eq!(error["details"]["owningWorkTitle"], "Collision Race Winner");
    let target_key: Option<String> = sqlx::query_scalar("SELECT gr_key FROM works WHERE id = ?")
        .bind(target)
        .fetch_one(db.pool())
        .await
        .expect("target GR key");
    assert!(target_key.is_none());
}

/// REQ-IDs: AC-6, AC-9
/// Directive: Empty-slot commit writes a user-confirmed ledger row, syncs the column, and advances generation once.
#[tokio::test]
async fn empty_gr_slot_commit_is_atomic_confirmed_and_single_use() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "Empty Slot").await;
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));
    let (_, _, body) = preview(&app, &db, user_id, work_id, "12345", None).await;
    let token = preview_id(&body).to_string();
    let before = generation(&db, work_id).await;

    let (status, _, response) = commit(&app, &db, user_id, work_id, "gr_work", &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["grKey"], "12345");
    assert_eq!(response["identityStatus"], "confirmed");
    assert!(generation(&db, work_id).await > before, "commit must advance generation");

    let anchor = sqlx::query(
        "SELECT confidence, setter FROM work_identity_anchors \
         WHERE work_id = ? AND anchor_type = 'gr_work' AND anchor_value = '12345'",
    )
    .bind(work_id)
    .fetch_one(db.pool())
    .await
    .expect("confirmed anchor");
    assert_eq!(anchor.get::<String, _>("confidence"), "confirmed");
    assert_eq!(anchor.get::<String, _>("setter"), "user");

    let (again, _, error) = commit(&app, &db, user_id, work_id, "gr_work", &token).await;
    assert_eq!(again, StatusCode::CONFLICT);
    assert_eq!(error["details"]["code"], "preview_required");
}

/// REQ-IDs: AC-4, AC-7, AC-8, AC-20
/// Directive: An unverifiable work-key sibling is previewed for drop, while a bridge is protected; commit applies that drop-set exactly.
#[tokio::test]
async fn overwrite_and_sibling_drop_clean_columns_pending_rows_and_dead_ends() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "Overwrite With Drop").await;
    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::GR_WORK),
        "11111",
        AnchorSetter::AutoSearch,
    )
    .await
    .expect("seed old GR");
    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::OL_WORK),
        "OL222W",
        AnchorSetter::AutoSearch,
    )
    .await
    .expect("seed unverifiable sibling");
    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::ISBN_13),
        "9780306406157",
        AnchorSetter::Import,
    )
    .await
    .expect("seed protected bridge");
    db.record_pending_anchor(work_id, AnchorType::new(AnchorType::OL_WORK), "OL333W")
        .await
        .expect("seed sibling pending");
    db.record_pending_anchor(work_id, AnchorType::new(AnchorType::GR_WORK), "44444")
        .await
        .expect("seed edited-slot pending");
    db.bump_anchor_attempt(work_id, AnchorType::new(AnchorType::OL_WORK))
        .await
        .expect("seed sibling dead end");

    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));
    let (_, _, preview_body) = preview(&app, &db, user_id, work_id, "12345", Some("gr_work")).await;
    assert!(preview_body["siblings"]
        .as_array()
        .expect("sibling assessments")
        .iter()
        .any(|s| s["slot"] == "ol_work" && s["action"] == "drop"));
    let token = preview_id(&preview_body).to_string();

    let (status, _, _) = commit(&app, &db, user_id, work_id, "gr_work", &token).await;
    assert_eq!(status, StatusCode::OK);

    let old = sqlx::query(
        "SELECT confidence, superseded_by FROM work_identity_anchors \
         WHERE work_id = ? AND anchor_type = 'gr_work' AND anchor_value = '11111'",
    )
    .bind(work_id)
    .fetch_one(db.pool())
    .await
    .expect("old GR row");
    assert_eq!(old.get::<String, _>("confidence"), "superseded");
    assert_eq!(
        old.get::<Option<String>, _>("superseded_by").as_deref(),
        Some("12345")
    );

    let row = sqlx::query("SELECT gr_key, ol_key, isbn_13 FROM works WHERE id = ?")
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .expect("work row");
    assert_eq!(
        row.get::<Option<String>, _>("gr_key").as_deref(),
        Some("12345")
    );
    assert!(row.get::<Option<String>, _>("ol_key").is_none());
    assert_eq!(
        row.get::<Option<String>, _>("isbn_13").as_deref(),
        Some("9780306406157")
    );
    let pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM work_identity_anchors \
         WHERE work_id = ? AND confidence = 'pending' \
         AND anchor_type IN ('gr_work', 'ol_work')",
    )
    .bind(work_id)
    .fetch_one(db.pool())
    .await
    .expect("pending count");
    assert_eq!(pending, 0);
    assert!(db
        .list_anchor_dead_ends(work_id)
        .await
        .expect("dead ends")
        .iter()
        .all(|d| d.anchor_type.as_str() != AnchorType::OL_WORK));
}

/// REQ-IDs: AC-9
/// Directive: Two preview intents share no write authority; the first commit makes the second stale.
#[tokio::test]
async fn two_previews_then_two_commits_enforce_generation_cas() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "Two Previews").await;
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));
    let (_, _, first) = preview(&app, &db, user_id, work_id, "12345", None).await;
    let (_, _, second) = preview(&app, &db, user_id, work_id, "67890", None).await;

    assert_eq!(
        commit(&app, &db, user_id, work_id, "gr_work", preview_id(&first))
            .await
            .0,
        StatusCode::OK
    );
    let (status, _, error) =
        commit(&app, &db, user_id, work_id, "gr_work", preview_id(&second)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["details"]["code"], "preview_required");
    let gr: String = sqlx::query_scalar("SELECT gr_key FROM works WHERE id = ?")
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .expect("GR key");
    assert_eq!(gr, "12345");
}

/// REQ-IDs: AC-9, AC-16
/// Directive: Clear advances durable generation and invalidates an older preview intent.
#[tokio::test]
async fn clear_between_preview_and_commit_makes_the_delayed_commit_stale() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "Clear Beats Preview").await;
    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::GR_WORK),
        "11111",
        AnchorSetter::User,
    )
    .await
    .expect("seed clearable GR");
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));
    let (_, _, body) = preview(&app, &db, user_id, work_id, "12345", None).await;

    assert_eq!(
        call(
            &app,
            Method::DELETE,
            format!("/work/{work_id}/identity/gr_work"),
            None,
            Some(auth_context(&db, user_id).await),
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, _, error) =
        commit(&app, &db, user_id, work_id, "gr_work", preview_id(&body)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["details"]["code"], "preview_required");
    let gr: Option<String> = sqlx::query_scalar("SELECT gr_key FROM works WHERE id = ?")
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .expect("cleared GR");
    assert!(gr.is_none());
}

/// REQ-IDs: AC-9, AC-11
/// Directive: A conflict raised after preview invalidates commit and remains open.
#[tokio::test]
async fn conflict_raised_after_preview_invalidates_the_snapshot_without_auto_resolution() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "Conflict After Preview").await;
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));
    let (_, _, body) = preview(&app, &db, user_id, work_id, "12345", None).await;
    let conflict_id = raise_conflict(
        &db,
        user_id,
        work_id,
        IdentityConflictKind::IncomingDifferentGrKey,
    )
    .await;

    let (status, _, error) =
        commit(&app, &db, user_id, work_id, "gr_work", preview_id(&body)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["details"]["code"], "preview_required");
    let conflict_status: String =
        sqlx::query_scalar("SELECT status FROM work_identity_conflicts WHERE id = ?")
            .bind(conflict_id)
            .fetch_one(db.pool())
            .await
            .expect("conflict status");
    assert_eq!(conflict_status, "open");
}

/// REQ-IDs: AC-9, AC-12
/// Directive: A stale first-statement CAS or a real mid-transaction SQLite failure leaves every edit table untouched.
#[tokio::test]
async fn repository_edit_cas_and_mid_transaction_failure_are_fully_atomic() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "Atomic Edit").await;
    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::GR_WORK),
        "11111",
        AnchorSetter::User,
    )
    .await
    .expect("seed GR");
    let stale_generation = generation(&db, work_id).await;
    db.record_pending_anchor(work_id, AnchorType::new(AnchorType::ASIN), "B0ABC12345")
        .await
        .expect("competing writer");
    let after_competitor = generation(&db, work_id).await;

    let stale = db
        .apply_identity_edit(
            work_id,
            user_id,
            AnchorType::new(AnchorType::GR_WORK),
            "22222",
            stale_generation,
            &[],
        )
        .await
        .expect_err("stale generation must lose");
    assert!(matches!(stale, IdentityEditError::StalePreview));
    assert_eq!(generation(&db, work_id).await, after_competitor);
    let gr: String = sqlx::query_scalar("SELECT gr_key FROM works WHERE id = ?")
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .expect("GR after stale edit");
    assert_eq!(gr, "11111");

    sqlx::query(&format!(
        "CREATE TRIGGER abort_identity_edit_gr \
         BEFORE UPDATE OF gr_key ON works WHEN OLD.id = {work_id} \
         BEGIN SELECT RAISE(ABORT, 'forced identity-edit rollback'); END"
    ))
    .execute(db.pool())
    .await
    .expect("install real SQLite abort trigger");
    let before_failure = generation(&db, work_id).await;
    db.apply_identity_edit(
        work_id,
        user_id,
        AnchorType::new(AnchorType::GR_WORK),
        "33333",
        before_failure,
        &[],
    )
    .await
    .expect_err("trigger aborts the real transaction");
    assert_eq!(generation(&db, work_id).await, before_failure);
    let old_confidence: String = sqlx::query_scalar(
        "SELECT confidence FROM work_identity_anchors \
         WHERE work_id = ? AND anchor_type = 'gr_work' AND anchor_value = '11111'",
    )
    .bind(work_id)
    .fetch_one(db.pool())
    .await
    .expect("old anchor survives rollback");
    assert_eq!(old_confidence, "confirmed");
}

/// REQ-IDs: AC-11
/// Directive: Work-key edit closes same-slot and QuorumTie disputes but leaves other-slot conflicts open.
#[tokio::test]
async fn edit_closes_only_conflicts_the_certified_slot_actually_settles() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "Conflict Closure").await;
    let same_slot = raise_conflict(
        &db,
        user_id,
        work_id,
        IdentityConflictKind::IncomingDifferentGrKey,
    )
    .await;
    let quorum = raise_conflict(&db, user_id, work_id, IdentityConflictKind::QuorumTie).await;
    let other_slot = raise_conflict(
        &db,
        user_id,
        work_id,
        IdentityConflictKind::IncomingDifferentOlKey,
    )
    .await;
    let expected = generation(&db, work_id).await;

    db.apply_identity_edit(
        work_id,
        user_id,
        AnchorType::new(AnchorType::GR_WORK),
        "12345",
        expected,
        &[],
    )
    .await
    .expect("apply user-certified GR");

    for id in [same_slot, quorum] {
        let row =
            sqlx::query("SELECT status, resolution_notes FROM work_identity_conflicts WHERE id = ?")
                .bind(id)
                .fetch_one(db.pool())
                .await
                .expect("closed conflict");
        assert_eq!(row.get::<String, _>("status"), "resolved");
        assert_eq!(
            row.get::<Option<String>, _>("resolution_notes").as_deref(),
            Some("superseded by user identity edit")
        );
    }
    let status: String = sqlx::query_scalar("SELECT status FROM work_identity_conflicts WHERE id = ?")
        .bind(other_slot)
        .fetch_one(db.pool())
        .await
        .expect("other conflict");
    assert_eq!(status, "open");
}

/// REQ-IDs: AC-12, AC-17, AC-18
/// Directive: The route maps validation, ownership, authentication, and slot-method errors exactly.
#[tokio::test]
async fn route_error_contract_is_typed_and_does_not_mutate_foreign_work() {
    let db = common::create_test_db().await;
    let owner = create_test_user(&db).await;
    let stranger = create_second_test_user(&db).await;
    let work_id = create_work(&db, owner, "Route Errors").await;
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));

    for invalid in ["OL12345M", "not-an-id"] {
        let (status, _, _) = preview(&app, &db, owner, work_id, invalid, None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{invalid}");
    }
    let (status, _, wrong_slot) =
        preview(&app, &db, owner, work_id, "0-306-40615-2", Some("asin")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(wrong_slot["message"]
        .as_str()
        .is_some_and(|m| m.contains("Fix match")));

    let (foreign, _, _) = preview(&app, &db, stranger, work_id, "12345", None).await;
    assert_eq!(foreign, StatusCode::NOT_FOUND);
    let (foreign_clear, _, _) = call(
        &app,
        Method::DELETE,
        format!("/work/{work_id}/identity/gr_work"),
        None,
        Some(auth_context(&db, stranger).await),
    )
    .await;
    assert_eq!(foreign_clear, StatusCode::NOT_FOUND);
    let (unauthenticated, _, _) = call(
        &app,
        Method::POST,
        format!("/work/{work_id}/identity/preview"),
        Some(json!({"input":"12345","slot":null})),
        None,
    )
    .await;
    assert_eq!(unauthenticated, StatusCode::UNAUTHORIZED);

    let (unknown, _, _) = commit(&app, &db, owner, work_id, "unknown", "irrelevant").await;
    assert_eq!(unknown, StatusCode::BAD_REQUEST);
    let (hc_put, _, _) = commit(&app, &db, owner, work_id, "hc_work", "irrelevant").await;
    assert_eq!(hc_put, StatusCode::BAD_REQUEST);
    assert_eq!(generation(&db, work_id).await, 0);
}

/// REQ-IDs: AC-10
/// Directive: A pending-affirm door that loses its first generation claim returns the dedicated 409 and emits no success side effect.
#[tokio::test]
async fn pending_affirm_generation_loss_maps_to_pending_anchor_stale() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "Stale Pending Affirm").await;
    db.record_pending_anchor(work_id, AnchorType::new(AnchorType::GR_WORK), "12345")
        .await
        .expect("seed pending GR");
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));
    let request = authenticated_request(
        &db,
        user_id,
        Method::POST,
        format!("/work/{work_id}/pending-anchors/gr_work/affirm"),
        None,
    )
    .await;

    let (status, error) = race_generation_claim(&app, &db, work_id, request).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["details"]["code"], "pending_anchor_stale");
    assert_eq!(error["message"], "identity changed; reload pending anchors");
    let confidence: String = sqlx::query_scalar(
        "SELECT confidence FROM work_identity_anchors \
         WHERE work_id = ? AND anchor_type = 'gr_work' AND anchor_value = '12345'",
    )
    .bind(work_id)
    .fetch_one(db.pool())
    .await
    .expect("pending row remains");
    assert_eq!(confidence, "pending");
    assert!(db
        .list_history(user_id, history_filter())
        .await
        .expect("history")
        .is_empty());
}

/// REQ-IDs: AC-10
/// Directive: Review apply and dismiss generation losses share identity_review_stale and leave the park intact.
#[tokio::test]
async fn review_apply_and_dismiss_generation_losses_map_contextually() {
    for action in ["resolve", "dismiss"] {
        let db = common::create_test_db().await;
        let user_id = create_test_user(&db).await;
        let work_id = create_work(&db, user_id, &format!("Stale Review {action}")).await;
        db.set_identity_status(user_id, work_id, IdentityStatus::NeedsReview)
            .await
            .expect("park work");
        db.record_review_candidates(
            work_id,
            &[review_candidate(
                "review-stale",
                &format!("Stale Review {action}"),
            )],
        )
        .await
        .expect("seed review candidate");
        let base = spawn_goodreads(StatusCode::OK).await;
        let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));
        let body = (action == "resolve").then(|| json!({"candidateId":"review-stale"}));
        let request = authenticated_request(
            &db,
            user_id,
            Method::POST,
            format!("/identity-review/{work_id}/{action}"),
            body,
        )
        .await;

        let (status, error) = race_generation_claim(&app, &db, work_id, request).await;
        assert_eq!(status, StatusCode::CONFLICT, "{action}");
        assert_eq!(
            error["details"]["code"], "identity_review_stale",
            "{action}"
        );
        assert_eq!(
            error["message"], "identity changed; reload review candidates",
            "{action}"
        );
        let badge: String = sqlx::query_scalar("SELECT identity_status FROM works WHERE id = ?")
            .bind(work_id)
            .fetch_one(db.pool())
            .await
            .expect("review badge");
        assert_eq!(badge, "needs_review", "{action}");
        assert!(db
            .get_review_candidates(work_id)
            .await
            .expect("review candidates")
            .is_some());
        assert!(db
            .list_history(user_id, history_filter())
            .await
            .expect("history")
            .is_empty());
    }
}

/// REQ-IDs: AC-10
/// Directive: Conflict resolve and dismiss generation losses share identity_conflict_stale and leave the conflict open.
#[tokio::test]
async fn conflict_resolve_and_dismiss_generation_losses_map_contextually() {
    for action in ["resolve", "dismiss"] {
        let db = common::create_test_db().await;
        let user_id = create_test_user(&db).await;
        let work_id = create_work(&db, user_id, &format!("Stale Conflict {action}")).await;
        db.confirm_anchor(
            work_id,
            AnchorType::new(AnchorType::GR_WORK),
            "12345",
            AnchorSetter::AutoSearch,
        )
        .await
        .expect("seed existing identity");
        let conflict_id = raise_conflict(
            &db,
            user_id,
            work_id,
            IdentityConflictKind::IncomingDifferentGrKey,
        )
        .await;
        let base = spawn_goodreads(StatusCode::OK).await;
        let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));
        let body = (action == "resolve").then(|| json!({"action":"keep_existing","notes":null}));
        let request = authenticated_request(
            &db,
            user_id,
            Method::POST,
            format!("/identity-conflict/{conflict_id}/{action}"),
            body,
        )
        .await;

        let (status, error) = race_generation_claim(&app, &db, work_id, request).await;
        assert_eq!(status, StatusCode::CONFLICT, "{action}");
        assert_eq!(
            error["details"]["code"], "identity_conflict_stale",
            "{action}"
        );
        assert_eq!(
            error["message"], "identity changed; reload identity conflicts",
            "{action}"
        );
        let conflict_status: String =
            sqlx::query_scalar("SELECT status FROM work_identity_conflicts WHERE id = ?")
                .bind(conflict_id)
                .fetch_one(db.pool())
                .await
                .expect("conflict status");
        assert_eq!(conflict_status, "open", "{action}");
        assert!(db
            .list_history(user_id, history_filter())
            .await
            .expect("history")
            .is_empty());
    }
}

/// REQ-IDs: AC-13
/// Directive: Identity edit bumps merge_generation so an old enrichment merge is Superseded.
#[tokio::test]
async fn identity_edit_invalidates_an_in_flight_enrichment_merge() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "Merge CAS").await;
    let merge_generation: i64 =
        sqlx::query_scalar("SELECT merge_generation FROM works WHERE id = ?")
            .bind(work_id)
            .fetch_one(db.pool())
            .await
            .expect("merge generation");
    let identity_generation = generation(&db, work_id).await;

    db.apply_identity_edit(
        work_id,
        user_id,
        AnchorType::new(AnchorType::GR_WORK),
        "12345",
        identity_generation,
        &[],
    )
    .await
    .expect("identity edit");

    let outcome = db
        .apply_enrichment_merge(ApplyEnrichmentMergeRequest {
            user_id,
            work_id,
            expected_merge_generation: merge_generation,
            work_update: None,
            new_enrichment_status: EnrichmentStatus::Enriched,
            provenance_upserts: vec![],
            provenance_deletes: vec![],
        })
        .await
        .expect("merge CAS result");
    assert_eq!(outcome, ApplyMergeOutcome::Superseded);
}

/// REQ-IDs: AC-14, AC-19
/// Directive: A fully reconciled same-value user anchor is a zero-write, zero-history no-op.
#[tokio::test]
async fn true_no_op_consumes_the_preview_but_writes_nothing() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "True No-op").await;
    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::GR_WORK),
        "12345",
        AnchorSetter::User,
    )
    .await
    .expect("seed user anchor");
    db.set_identity_status(user_id, work_id, IdentityStatus::Confirmed)
        .await
        .expect("seed correct badge");
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));
    let (_, _, body) = preview(&app, &db, user_id, work_id, "12345", Some("gr_work")).await;
    let identity_before = generation(&db, work_id).await;
    let merge_before: i64 = sqlx::query_scalar("SELECT merge_generation FROM works WHERE id = ?")
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .expect("merge generation");

    let (status, _, _) = commit(&app, &db, user_id, work_id, "gr_work", preview_id(&body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(generation(&db, work_id).await, identity_before);
    let merge_after: i64 = sqlx::query_scalar("SELECT merge_generation FROM works WHERE id = ?")
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .expect("merge generation");
    assert_eq!(merge_after, merge_before);
    assert!(db
        .list_history(user_id, history_filter())
        .await
        .expect("identity history")
        .is_empty());
}

/// REQ-IDs: AC-15
/// Directive: ISBN edits create a Provisional bridge, preserve work-key slots, and allow same-user duplicates.
#[tokio::test]
async fn isbn_fix_match_is_a_non_unique_bridge_and_never_drops_work_keys() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let first = create_work(&db, user_id, "ISBN First").await;
    let second = create_work(&db, user_id, "ISBN Second").await;
    db.confirm_anchor(
        first,
        AnchorType::new(AnchorType::OL_WORK),
        "OLFIRSTW",
        AnchorSetter::Import,
    )
    .await
    .expect("seed first work key");

    for work_id in [first, second] {
        db.apply_identity_edit(
            work_id,
            user_id,
            AnchorType::new(AnchorType::ISBN_13),
            "9780306406157",
            generation(&db, work_id).await,
            &[],
        )
        .await
        .expect("shared ISBN commit");
    }

    let first_row = sqlx::query("SELECT isbn_13, ol_key, identity_status FROM works WHERE id = ?")
        .bind(first)
        .fetch_one(db.pool())
        .await
        .expect("first work");
    assert_eq!(
        first_row.get::<Option<String>, _>("isbn_13").as_deref(),
        Some("9780306406157")
    );
    assert_eq!(
        first_row.get::<Option<String>, _>("ol_key").as_deref(),
        Some("OLFIRSTW")
    );
    let second_badge: String = sqlx::query_scalar("SELECT identity_status FROM works WHERE id = ?")
        .bind(second)
        .fetch_one(db.pool())
        .await
        .expect("second badge");
    assert_eq!(second_badge, "provisional");
}

/// REQ-IDs: AC-16, AC-18
/// Directive: Clear handles pending-only and HC slots, deletes slot residue, and returns 404 only when truly empty.
#[tokio::test]
async fn clear_uses_union_truth_and_removes_all_slot_residue() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let pending_only = create_work(&db, user_id, "Pending-only Clear").await;
    db.record_pending_anchor(pending_only, AnchorType::new(AnchorType::GR_WORK), "12345")
        .await
        .expect("seed pending-only slot");
    db.bump_anchor_attempt(pending_only, AnchorType::new(AnchorType::GR_WORK))
        .await
        .expect("seed dead end");
    let hc_work = create_work(&db, user_id, "HC Clear").await;
    db.confirm_anchor(
        hc_work,
        AnchorType::new(AnchorType::HC_WORK),
        "hc-clear",
        AnchorSetter::User,
    )
    .await
    .expect("seed HC");
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));

    let (status, _, body) = call(
        &app,
        Method::DELETE,
        format!("/work/{pending_only}/identity/gr_work"),
        None,
        Some(auth_context(&db, user_id).await),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["parkedByConflicts"], false);
    let residue: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM work_identity_anchors \
         WHERE work_id = ? AND anchor_type = 'gr_work' AND confidence = 'pending'",
    )
    .bind(pending_only)
    .fetch_one(db.pool())
    .await
    .expect("pending residue");
    assert_eq!(residue, 0);
    assert!(db
        .list_anchor_dead_ends(pending_only)
        .await
        .expect("dead ends")
        .is_empty());

    let (empty, _, _) = call(
        &app,
        Method::DELETE,
        format!("/work/{pending_only}/identity/gr_work"),
        None,
        Some(auth_context(&db, user_id).await),
    )
    .await;
    assert_eq!(empty, StatusCode::NOT_FOUND);

    let (hc, _, _) = call(
        &app,
        Method::DELETE,
        format!("/work/{hc_work}/identity/hc_work"),
        None,
        Some(auth_context(&db, user_id).await),
    )
    .await;
    assert_eq!(hc, StatusCode::OK);
}

/// REQ-IDs: AC-16
/// Directive: Clear beside an open conflict keeps the work parked and does not resolve the conflict.
#[tokio::test]
async fn clear_with_open_conflict_returns_the_standard_parked_dto() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "Parked Clear").await;
    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::GR_WORK),
        "12345",
        AnchorSetter::User,
    )
    .await
    .expect("seed GR");
    let conflict_id = raise_conflict(
        &db,
        user_id,
        work_id,
        IdentityConflictKind::IncomingDifferentGrKey,
    )
    .await;
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));

    let (status, _, body) = call(
        &app,
        Method::DELETE,
        format!("/work/{work_id}/identity/gr_work"),
        None,
        Some(auth_context(&db, user_id).await),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["parkedByConflicts"], true);
    let conflict_status: String =
        sqlx::query_scalar("SELECT status FROM work_identity_conflicts WHERE id = ?")
            .bind(conflict_id)
            .fetch_one(db.pool())
            .await
            .expect("conflict status");
    assert_eq!(conflict_status, "open");
}

/// REQ-IDs: AC-19
/// Directive: Edit and clear each emit one truthful identityResolved event with old/new action context.
#[tokio::test]
async fn edit_and_clear_emit_exactly_one_history_event_each() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "History Identity").await;
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));
    let (_, _, body) = preview(&app, &db, user_id, work_id, "12345", None).await;
    assert_eq!(
        commit(&app, &db, user_id, work_id, "gr_work", preview_id(&body))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            format!("/work/{work_id}/identity/gr_work"),
            None,
            Some(auth_context(&db, user_id).await),
        )
        .await
        .0,
        StatusCode::OK
    );

    let events = db
        .list_history(user_id, history_filter())
        .await
        .expect("identity history");
    assert_eq!(events.len(), 2);
    // History is newest-first: clear follows edit.
    assert_eq!(events[0].data["action"], "clear");
    assert!(events[0].data["identity"]
        .as_str()
        .is_some_and(|s| s.contains("12345") && s.contains("cleared")));
    assert_eq!(events[1].data["action"], "edit");
    assert!(events[1].data["identity"]
        .as_str()
        .is_some_and(|s| s.contains("(empty)") && s.contains("12345")));
}

/// REQ-IDs: AC-21, AC-23
/// Directive: Startup Rust backfill normalizes valid columns, quarantines invalids, and marks completion atomically without generation bumps.
#[tokio::test]
async fn startup_ledger_backfill_is_idempotent_normalizing_and_non_destructive() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let valid = create_work(&db, user_id, "Backfill Valid").await;
    let invalid = create_work(&db, user_id, "Backfill Invalid").await;
    sqlx::query(
        "UPDATE works SET ol_key = 'OL123W', gr_key = '456.Some_Slug', \
         hc_key = 'hc-789', isbn_13 = '0-306-40615-2', asin = 'B0ABC12345' WHERE id = ?",
    )
    .bind(valid)
    .execute(db.pool())
    .await
    .expect("seed valid legacy columns");
    sqlx::query(
        "UPDATE works SET gr_key = 'bad-gr', isbn_13 = '9780000000000', \
         asin = 'not an asin' WHERE id = ?",
    )
    .bind(invalid)
    .execute(db.pool())
    .await
    .expect("seed quarantined columns");
    sqlx::query("DELETE FROM _livrarr_meta WHERE key = 'work_identity_ledger_backfill_complete'")
        .execute(db.pool())
        .await
        .expect("reset startup marker");

    sqlx::query(&format!(
        "CREATE TRIGGER abort_ledger_backfill \
         BEFORE INSERT ON work_identity_anchors \
         WHEN NEW.work_id = {valid} AND NEW.anchor_type = 'hc_work' \
         BEGIN SELECT RAISE(ABORT, 'forced startup backfill failure'); END"
    ))
    .execute(db.pool())
    .await
    .expect("install real mid-pass failure");
    livrarr_db::backfill_work_identity_ledger(db.pool())
        .await
        .expect_err("mid-pass storage failure must fail startup");
    let partial_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM work_identity_anchors WHERE work_id = ?")
            .bind(valid)
            .fetch_one(db.pool())
            .await
            .expect("partial row count");
    assert_eq!(partial_rows, 0, "all ledger inserts must roll back");
    let partial_marker: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM _livrarr_meta \
         WHERE key = 'work_identity_ledger_backfill_complete'",
    )
    .fetch_one(db.pool())
    .await
    .expect("partial marker count");
    assert_eq!(partial_marker, 0, "marker must be the last atomic write");
    sqlx::query("DROP TRIGGER abort_ledger_backfill")
        .execute(db.pool())
        .await
        .expect("remove failure trigger");

    livrarr_db::backfill_work_identity_ledger(db.pool())
        .await
        .expect("startup backfill");
    let first_rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT work_id, anchor_type, anchor_value FROM work_identity_anchors \
         ORDER BY work_id, anchor_type, anchor_value",
    )
    .fetch_all(db.pool())
    .await
    .expect("backfilled rows");
    assert!(first_rows
        .iter()
        .any(|r| r.0 == valid && r.1 == "gr_work" && r.2 == "456"));
    assert!(first_rows
        .iter()
        .any(|r| r.0 == valid && r.1 == "isbn_13" && r.2 == "9780306406157"));
    assert!(!first_rows.iter().any(|r| r.0 == invalid));
    let generations: Vec<i64> =
        sqlx::query_scalar("SELECT identity_generation FROM works ORDER BY id")
            .fetch_all(db.pool())
            .await
            .expect("backfill generations");
    assert!(generations.iter().all(|g| *g == 0));
    let marker: String = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta \
         WHERE key = 'work_identity_ledger_backfill_complete'",
    )
    .fetch_one(db.pool())
    .await
    .expect("completion marker");
    assert_eq!(marker, "1");

    livrarr_db::backfill_work_identity_ledger(db.pool())
        .await
        .expect("completed rerun");
    let second_rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT work_id, anchor_type, anchor_value FROM work_identity_anchors \
         ORDER BY work_id, anchor_type, anchor_value",
    )
    .fetch_all(db.pool())
    .await
    .expect("rerun rows");
    assert_eq!(second_rows, first_rows);
}

fn bridge_candidate(title: &str, isbn: &str) -> livrarr_domain::identity::WorkCandidate {
    seed_add_box(
        SeedInput {
            title: title.to_string(),
            author_name: "Case Writer".to_string(),
            language: SeedLanguage::resolve(Some("en"), "en"),
            author_ol_key: None,
            year: Some(2004),
            cover_url: None,
            detail_url: None,
            description: None,
            series_name: None,
            series_position: None,
        },
        livrarr_domain::identity::IdentityState::Confirmed {
            anchors: CapturedIdentity {
                ol_key: None,
                gr_key: None,
                hc_key: None,
                isbn_13: Some(isbn.to_string()),
                asin: None,
                title: title.to_string(),
                author_name: "Case Writer".to_string(),
                language: Some("en".to_string()),
            },
            method: livrarr_domain::identity::IdentityMethod::UserSelected,
            score: None,
        },
        None,
        false,
    )
}

/// REQ-IDs: AC-22
/// Directive: Multiple verdict-eligible bridge hits abstain, then normalized identity decides adopt versus create.
#[tokio::test]
async fn add_fast_abstains_on_multiple_bridge_hits_before_normalized_dedup() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let first = create_work(&db, user_id, "One Shared Bridge").await;
    let normalized_winner = create_work(&db, user_id, "Normalized Winner").await;
    for work_id in [first, normalized_winner] {
        db.confirm_anchor(
            work_id,
            AnchorType::new(AnchorType::ISBN_13),
            "9780306406157",
            AnchorSetter::Import,
        )
        .await
        .expect("seed shared bridge");
    }
    let base = spawn_goodreads(StatusCode::OK).await;
    let service = work_service(db.clone(), base);

    let adopted = service
        .add_fast(
            user_id,
            bridge_candidate("Normalized Winner", "9780306406157"),
        )
        .await
        .expect("normalized fallback");
    assert!(!adopted.created);
    assert_eq!(adopted.work.id, normalized_winner);

    let created = service
        .add_fast(
            user_id,
            bridge_candidate("No Normalized Match", "9780306406157"),
        )
        .await
        .expect("abstaining bridge creates");
    assert!(created.created);
    assert_ne!(created.work.id, first);
    assert_ne!(created.work.id, normalized_winner);
}

/// REQ-IDs: AC-24
/// Directive: A fifth tenant preview evicts only that tenant's oldest token and preserves another tenant's intent.
#[tokio::test]
async fn preview_per_user_capacity_is_oldest_first_and_tenant_isolated() {
    let db = common::create_test_db().await;
    let user_a = create_test_user(&db).await;
    let user_b = create_second_test_user(&db).await;
    let work_a = create_work(&db, user_a, "Preview Cap A").await;
    let work_b = create_work(&db, user_b, "Preview Cap B").await;
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));

    let (_, _, other) = preview(&app, &db, user_b, work_b, "90001", None).await;
    let other_token = preview_id(&other).to_string();
    let mut own = Vec::new();
    for key in ["10001", "10002", "10003", "10004", "10005"] {
        let (_, _, body) = preview(&app, &db, user_a, work_a, key, None).await;
        own.push(preview_id(&body).to_string());
    }

    let (evicted, _, error) = commit(&app, &db, user_a, work_a, "gr_work", &own[0]).await;
    assert_eq!(evicted, StatusCode::CONFLICT);
    assert_eq!(error["details"]["code"], "preview_required");
    assert_eq!(
        commit(&app, &db, user_b, work_b, "gr_work", &other_token)
            .await
            .0,
        StatusCode::OK
    );
}

/// REQ-IDs: AC-25
/// Directive: Domain owns the preview record and the repository accepts only plain edit data.
#[test]
fn compile_time_crate_boundary_uses_domain_record_and_plain_repository_arguments() {
    fn domain_record(
        value: livrarr_domain::services::IdentityPreviewRecord,
    ) -> livrarr_domain::services::IdentityPreviewRecord {
        value
    }
    fn repository_contract<R: WorkIdentityRepository>(repo: &R) {
        let future =
            repo.apply_identity_edit(1, 1, AnchorType::new(AnchorType::GR_WORK), "12345", 0, &[]);
        drop(future);
    }

    let _ = domain_record;
    let _ = repository_contract::<SqliteDb>;
}

// ===========================================================================
// CC-merged net-new pins (from the independent CC suite, adapted to this
// file's fixtures at merge time — see suite-merge-notes.md)
// ===========================================================================

/// REQ-IDs: AC-3 (CC-merged)
/// Directive: A same-user COLUMN-ONLY legacy owner (backfill-loser shape) collides
/// exactly like a ledger owner.
#[tokio::test]
async fn cc_collision_covers_column_only_legacy_owner() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let target = create_work(&db, user_id, "Column Collision Target").await;
    let column_owner = create_work(&db, user_id, "Column Only Owner").await;
    sqlx::query("UPDATE works SET gr_key = '12345' WHERE id = ?")
        .bind(column_owner)
        .execute(db.pool())
        .await
        .expect("seed column-only owner");
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));

    let (status, _, body) = preview(&app, &db, user_id, target, "12345", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["collision"]["owningWorkId"], column_owner);
    assert_eq!(body["collision"]["owningWorkTitle"], "Column Only Owner");
    assert!(body.get("previewId").is_none() || body["previewId"].is_null());
}

/// REQ-IDs: AC-4 (CC-merged)
/// Directive: An unconfigured Hardcover sibling is KEPT (inert key, no poisoning road);
/// its key survives the commit untouched.
#[tokio::test]
async fn cc_hc_notconfigured_sibling_is_kept_through_commit() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "HC Keep").await;
    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::HC_WORK),
        "hc-keep-42",
        AnchorSetter::Import,
    )
    .await
    .expect("seed HC sibling");
    // The fixture wires ONLY Goodreads, so the HC preview leg reports NotConfigured.
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));

    let (_, _, body) = preview(&app, &db, user_id, work_id, "12345", Some("gr_work")).await;
    let siblings = body["siblings"].as_array().expect("assessments");
    let hc = siblings
        .iter()
        .find(|s| s["slot"] == "hc_work")
        .expect("hc assessment present");
    assert_eq!(hc["action"], "keep", "NotConfigured HC must be kept: {body}");

    let token = preview_id(&body).to_string();
    let (status, _, _) = commit(&app, &db, user_id, work_id, "gr_work", &token).await;
    assert_eq!(status, StatusCode::OK);
    let hc_key: Option<String> = sqlx::query_scalar("SELECT hc_key FROM works WHERE id = ?")
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .expect("hc column");
    assert_eq!(hc_key.as_deref(), Some("hc-keep-42"), "kept sibling untouched");
}

/// REQ-IDs: AC-9 (CC-merged, route-level d-arm)
/// Directive: ANY background identity writer between preview and commit stales the
/// token at the route boundary, and the lost claim writes nothing.
#[tokio::test]
async fn cc_background_writer_after_preview_stales_the_route_commit() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "Background Writer").await;
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));
    let (_, _, body) = preview(&app, &db, user_id, work_id, "12345", None).await;

    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::ASIN),
        "B0ABC12345",
        AnchorSetter::Import,
    )
    .await
    .expect("background writer");
    let gen_after_writer = generation(&db, work_id).await;

    let (status, _, error) =
        commit(&app, &db, user_id, work_id, "gr_work", preview_id(&body)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["details"]["code"], "preview_required");
    assert_eq!(generation(&db, work_id).await, gen_after_writer, "lost claim writes nothing");
    let gr: Option<String> = sqlx::query_scalar("SELECT gr_key FROM works WHERE id = ?")
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .expect("gr column");
    assert!(gr.is_none(), "stale commit must not confirm");
}

/// REQ-IDs: AC-14 (CC-merged negative: machine setter)
/// Directive: Same value with a MACHINE setter is NOT a no-op — the commit runs and
/// stamps user certification.
#[tokio::test]
async fn cc_same_value_machine_setter_commits_and_stamps_user() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "Machine Setter").await;
    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::GR_WORK),
        "12345",
        AnchorSetter::Import,
    )
    .await
    .expect("machine-set anchor");
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));
    let (_, _, body) = preview(&app, &db, user_id, work_id, "12345", Some("gr_work")).await;

    let (status, _, _) = commit(&app, &db, user_id, work_id, "gr_work", preview_id(&body)).await;
    assert_eq!(status, StatusCode::OK);
    let setter: String = sqlx::query_scalar(
        "SELECT setter FROM work_identity_anchors \
         WHERE work_id = ? AND anchor_type = 'gr_work' AND confidence = 'confirmed'",
    )
    .bind(work_id)
    .fetch_one(db.pool())
    .await
    .expect("setter");
    assert_eq!(setter, "user", "machine setter is not a no-op");
}

/// REQ-IDs: AC-14 (CC-merged negative: column drift)
/// Directive: Same value with ledger/column DISAGREEMENT is NOT a no-op — the commit
/// repairs both stores.
#[tokio::test]
async fn cc_same_value_column_drift_commits_and_repairs() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "Column Drift").await;
    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::GR_WORK),
        "12345",
        AnchorSetter::User,
    )
    .await
    .expect("user anchor");
    sqlx::query("UPDATE works SET gr_key = 'drifted' WHERE id = ?")
        .bind(work_id)
        .execute(db.pool())
        .await
        .expect("induce drift");
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));
    let (_, _, body) = preview(&app, &db, user_id, work_id, "12345", Some("gr_work")).await;

    let (status, _, _) = commit(&app, &db, user_id, work_id, "gr_work", preview_id(&body)).await;
    assert_eq!(status, StatusCode::OK);
    let gr: Option<String> = sqlx::query_scalar("SELECT gr_key FROM works WHERE id = ?")
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .expect("gr column");
    assert_eq!(gr.as_deref(), Some("12345"), "drift is not a no-op — commit repairs");
}

/// REQ-IDs: AC-23 (CC-merged: same-user duplicate work-key owner logic)
/// Directive: The backfill preserves an EXISTING confirmed owner over a lower-id
/// column-only member; with no owner, the lowest id wins; every loser column stays and
/// still earns its badge through the union projection.
#[tokio::test]
async fn cc_backfill_owner_preservation_for_duplicate_work_keys() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let dup_low = create_work(&db, user_id, "Dup Lower Id").await;
    let dup_high = create_work(&db, user_id, "Dup Higher Id Existing Owner").await;
    let orphan_a = create_work(&db, user_id, "No Owner A").await;
    let orphan_b = create_work(&db, user_id, "No Owner B").await;

    sqlx::query("UPDATE works SET ol_key = 'OL42W' WHERE id = ?")
        .bind(dup_low)
        .execute(db.pool())
        .await
        .expect("lower-id column");
    db.confirm_anchor(
        dup_high,
        AnchorType::new(AnchorType::OL_WORK),
        "OL42W",
        AnchorSetter::Import,
    )
    .await
    .expect("existing confirmed owner (higher id)");
    for w in [orphan_a, orphan_b] {
        sqlx::query("UPDATE works SET gr_key = '31337' WHERE id = ?")
            .bind(w)
            .execute(db.pool())
            .await
            .expect("ownerless duplicate columns");
    }
    sqlx::query("DELETE FROM _livrarr_meta WHERE key = 'work_identity_ledger_backfill_complete'")
        .execute(db.pool())
        .await
        .expect("reset marker");

    livrarr_db::backfill_work_identity_ledger(db.pool())
        .await
        .expect("backfill");

    let owner_rows = |work: WorkId, ty: &'static str| async move {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM work_identity_anchors \
             WHERE work_id = ? AND anchor_type = ? AND confidence = 'confirmed'",
        )
        .bind(work)
        .bind(ty)
        .fetch_one(db.pool())
        .await
        .expect("count")
    };
    assert_eq!(owner_rows(dup_high, "ol_work").await, 1, "existing owner preserved");
    assert_eq!(
        owner_rows(dup_low, "ol_work").await,
        0,
        "ownership never transfers to a lower id merely by ordering"
    );
    let low_col: Option<String> = sqlx::query_scalar("SELECT ol_key FROM works WHERE id = ?")
        .bind(dup_low)
        .fetch_one(db.pool())
        .await
        .expect("loser column");
    assert_eq!(low_col.as_deref(), Some("OL42W"), "loser column intact");
    assert_eq!(owner_rows(orphan_a, "gr_work").await, 1, "no owner -> lowest id wins");
    assert_eq!(owner_rows(orphan_b, "gr_work").await, 0);
}

/// REQ-IDs: AC-16 (CC-merged: populated-slot clear residue)
/// Directive: Clearing a populated slot supersedes the confirmed row (superseded_by
/// NULL) and NULLs the column in the same transaction.
#[tokio::test]
async fn cc_populated_clear_supersedes_row_and_nulls_column() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "Populated Clear").await;
    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::GR_WORK),
        "12345",
        AnchorSetter::User,
    )
    .await
    .expect("populated slot");
    let base = spawn_goodreads(StatusCode::OK).await;
    let app = identity_app(route_state(db.clone(), work_service(db.clone(), base)));

    let (status, _, _) = call(
        &app,
        Method::DELETE,
        format!("/work/{work_id}/identity/gr_work"),
        None,
        Some(auth_context(&db, user_id).await),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let row = sqlx::query(
        "SELECT confidence, superseded_by FROM work_identity_anchors \
         WHERE work_id = ? AND anchor_type = 'gr_work' AND anchor_value = '12345'",
    )
    .bind(work_id)
    .fetch_one(db.pool())
    .await
    .expect("superseded row");
    assert_eq!(row.get::<String, _>("confidence"), "superseded");
    assert!(row.get::<Option<String>, _>("superseded_by").is_none());
    let gr: Option<String> = sqlx::query_scalar("SELECT gr_key FROM works WHERE id = ?")
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .expect("column");
    assert!(gr.is_none());
}
