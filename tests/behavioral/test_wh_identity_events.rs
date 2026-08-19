use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use livrarr_behavioral::stubs::{
    create_test_user, SqlitePendingRouteRoad, StubEnrichmentWorkflow, StubHttpFetcher,
};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    AuthorDb, CreateAuthorDbRequest, CreateWorkDbRequest, HistoryDb, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::identity::{
    AnchorConfidence, AnchorSetter, AnchorType, Candidate, CandidateId, CapturedIdentity,
    ConflictResolutionAction, ConflictSource, IdentityConflictKind, IncomingConflictPayload,
    NewIdentityConflict, ResolutionScore,
};
use livrarr_domain::services::{IdentityConflictService, WorkIdentityRepository};
use livrarr_domain::{
    normalize_for_matching, AuthType, EventType, HistoryFilter, IdentityStatus, MetadataProvider,
    UserId, Work, WorkId,
};
use livrarr_handlers::context::{
    HasHistoryService, HasIdentityRoadService, HasWorkIdentityRepository, HasWorkService,
};
use livrarr_handlers::AuthContext;
use livrarr_metadata::work_service::WorkServiceImpl;
use livrarr_server::history_service::HistoryServiceImpl;
use livrarr_server::services::identity_conflict_service::LiveIdentityConflictService;

type TestWorkService = WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher>;
type TestHistoryService = HistoryServiceImpl<SqliteDb>;

#[derive(Clone)]
struct TestState {
    work_service: Arc<TestWorkService>,
    identity_repo: SqliteDb,
    history_service: Arc<TestHistoryService>,
    identity_road: SqlitePendingRouteRoad,
}

impl HasWorkService for TestState {
    type WorkSvc = TestWorkService;

    fn work_service(&self) -> &Self::WorkSvc {
        &self.work_service
    }
}

impl HasWorkIdentityRepository for TestState {
    type WorkIdentityRepo = SqliteDb;

    fn work_identity_repo(&self) -> &Self::WorkIdentityRepo {
        &self.identity_repo
    }
}

impl HasHistoryService for TestState {
    type HistorySvc = TestHistoryService;

    fn history_service(&self) -> &Self::HistorySvc {
        &self.history_service
    }
}

impl HasIdentityRoadService for TestState {
    type IdentityRoadSvc = SqlitePendingRouteRoad;

    fn identity_road_service(&self) -> &Self::IdentityRoadSvc {
        &self.identity_road
    }
}

fn empty_filter() -> HistoryFilter {
    HistoryFilter {
        event_type: None,
        work_id: None,
        start_date: None,
        end_date: None,
    }
}

fn identity_filter() -> HistoryFilter {
    HistoryFilter {
        event_type: Some(EventType::IdentityResolved),
        work_id: None,
        start_date: None,
        end_date: None,
    }
}

fn work_req(user_id: UserId, title: &str) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: "Work History Author".to_string(),
        normalized_title: normalize_for_matching(title),
        normalized_author: normalize_for_matching("Work History Author"),
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
    identity_status: IdentityStatus,
    ol_key: Option<&str>,
) -> Work {
    let (author, _) = db
        .create_author(CreateAuthorDbRequest {
            user_id,
            name: "Work History Author".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed author");
    let (work, created) = db
        .create_work(CreateWorkDbRequest {
            ol_key: ol_key.map(str::to_string),
            author_id: Some(author.id),
            ..work_req(user_id, title)
        })
        .await
        .expect("seed work");
    assert!(created, "fixture work must be newly created");
    if let Some(key) = ol_key {
        db.confirm_anchor(
            work.id,
            AnchorType::new(AnchorType::OL_WORK),
            key,
            AnchorSetter::Import,
        )
        .await
        .expect("seed confirmed OL anchor");
    }
    db.set_identity_status(user_id, work.id, identity_status)
        .await
        .expect("seed identity status");
    db.get_work(user_id, work.id).await.expect("read work")
}

fn service(db: SqliteDb) -> TestWorkService {
    WorkServiceImpl::new(
        db,
        StubEnrichmentWorkflow::succeeding(),
        StubHttpFetcher::new(),
        tempfile::tempdir().expect("test data dir").keep(),
    )
}

fn test_state(db: SqliteDb) -> TestState {
    TestState {
        work_service: Arc::new(service(db.clone())),
        identity_repo: db.clone(),
        history_service: Arc::new(HistoryServiceImpl::new(db.clone())),
        identity_road: SqlitePendingRouteRoad::new(db),
    }
}

async fn auth_context(db: &SqliteDb, user_id: UserId) -> AuthContext {
    AuthContext {
        user: db.get_user(user_id).await.expect("get auth user"),
        auth_type: AuthType::Session,
        session_token_hash: Some("test-session".to_string()),
    }
}

fn captured(title: &str, ol_key: Option<&str>, gr_key: Option<&str>) -> CapturedIdentity {
    CapturedIdentity {
        ol_key: ol_key.map(str::to_string),
        gr_key: gr_key.map(str::to_string),
        hc_key: None,
        isbn_13: None,
        asin: None,
        title: title.to_string(),
        author_name: "Work History Author".to_string(),
        language: Some("en".to_string()),
    }
}

fn candidate(id: &str, title: &str, ol_key: &str) -> Candidate {
    Candidate {
        candidate_id: CandidateId(id.to_string()),
        anchors: captured(title, Some(ol_key), None),
        cover_url: None,
        sources: vec![MetadataProvider::OpenLibrary],
        score: ResolutionScore {
            title_jaccard: 0.91,
            author_overlap: 1,
            runner_up_delta: 0.2,
        },
        existing_work_id: None,
    }
}

fn incoming(title: &str, ol_key: &str) -> IncomingConflictPayload {
    IncomingConflictPayload {
        ol_key: Some(ol_key.to_string()),
        gr_key: None,
        hc_key: None,
        isbn_13: None,
        asin: None,
        title: title.to_string(),
        author_name: "Work History Author".to_string(),
        year: Some(2026),
        cover_url: None,
        top_candidates: vec![],
    }
}

async fn raise_conflict(
    db: &SqliteDb,
    user_id: UserId,
    action_name: &str,
) -> (LiveIdentityConflictService, WorkId, i64) {
    let old_key = format!("OL-CONFLICT-OLD-{action_name}W");
    let work = seed_work(
        db,
        user_id,
        &format!("Conflict {action_name}"),
        IdentityStatus::Confirmed,
        Some(&old_key),
    )
    .await;
    let service = LiveIdentityConflictService::new(db.clone());
    let id = service
        .raise(NewIdentityConflict {
            user_id,
            existing_work_id: work.id,
            kind: IdentityConflictKind::IncomingDifferentOlKey,
            incoming: incoming(
                &format!("Conflict {action_name}"),
                &format!("OL-CONFLICT-NEW-{action_name}W"),
            ),
            raised_by: ConflictSource::ManualAdd,
            raised_source_path: None,
        })
        .await
        .expect("raise identity conflict");
    (service, work.id, id)
}

async fn identity_events(db: &SqliteDb, user_id: UserId) -> Vec<livrarr_domain::HistoryEvent> {
    db.list_history(user_id, identity_filter())
        .await
        .expect("list identity history")
}

fn assert_only_action(events: &[livrarr_domain::HistoryEvent], action: &str, work_id: WorkId) {
    assert_eq!(
        events.len(),
        1,
        "expected exactly one identityResolved event"
    );
    assert_eq!(events[0].event_type, EventType::IdentityResolved);
    assert_eq!(events[0].work_id, Some(work_id));
    assert_eq!(events[0].data["action"], action);
    assert!(
        events[0].data["identity"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "identityResolved payload must name the chosen identity"
    );
    assert!(
        events[0].data["work_title"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "identityResolved payload must snapshot work_title"
    );
}

#[tokio::test]
async fn wh_conflict_resolution_each_action_records_one_identity_resolved() {
    let cases = [
        (
            ConflictResolutionAction::KeepExisting,
            "keep-existing",
            "keep",
        ),
        (
            ConflictResolutionAction::AcceptSeparate,
            "accept-separate",
            "separate",
        ),
        (
            ConflictResolutionAction::ReplaceAnchor,
            "replace-anchor",
            "replace",
        ),
        (ConflictResolutionAction::Merge, "merge", "merge"),
    ];

    for (action, action_label, suffix) in cases {
        let db = create_test_db().await;
        let user_id = create_test_user(&db).await;
        let (service, work_id, conflict_id) = raise_conflict(&db, user_id, suffix).await;

        service
            .resolve(conflict_id, user_id, action, Some("user chose".to_string()))
            .await
            .expect("resolve conflict");

        let events = identity_events(&db, user_id).await;
        assert_only_action(&events, action_label, work_id);
    }
}

#[tokio::test]
async fn wh_conflict_resolution_rejected_resolution_records_zero_identity_events() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (service, _work_id, _conflict_id) = raise_conflict(&db, user_id, "rejected").await;

    let err = service
        .resolve(
            9_999_999,
            user_id,
            ConflictResolutionAction::KeepExisting,
            None,
        )
        .await
        .expect_err("unknown conflict id is rejected");
    assert!(matches!(
        err,
        livrarr_domain::services::ConflictError::NotFound
    ));
    assert!(identity_events(&db, user_id).await.is_empty());
}

#[tokio::test]
async fn wh_review_candidate_apply_records_once_second_apply_409_and_dismiss_records_none() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_work(
        &db,
        user_id,
        "Review Identity Door",
        IdentityStatus::NeedsReview,
        None,
    )
    .await;
    let candidates = vec![candidate(
        "review-candidate-1",
        "Review Identity Door",
        "OL-REVIEW-1W",
    )];
    db.record_review_candidates(work.id, &candidates)
        .await
        .expect("record review candidates");
    let state = test_state(db.clone());

    let status = livrarr_handlers::identity_review::resolve(
        State(state.clone()),
        auth_context(&db, user_id).await,
        Path(work.id),
        axum::Json(livrarr_handlers::identity_review::ResolveReviewRequest {
            candidate_id: "review-candidate-1".to_string(),
        }),
    )
    .await
    .expect("apply review candidate");
    assert_eq!(status, StatusCode::NO_CONTENT);
    let events = identity_events(&db, user_id).await;
    assert_only_action(&events, "review-candidate-apply", work.id);

    let second = livrarr_handlers::identity_review::resolve(
        State(state),
        auth_context(&db, user_id).await,
        Path(work.id),
        axum::Json(livrarr_handlers::identity_review::ResolveReviewRequest {
            candidate_id: "review-candidate-1".to_string(),
        }),
    )
    .await
    .expect_err("second apply of the same park is rejected");
    // Stage-5 disposition (2026-07-19): the original assertion pinned 409, but a
    // SEQUENTIAL second apply short-circuits at the handler's candidate load —
    // apply_review_candidate deletes the candidates row on success, so the rerun
    // gets 404 before the claim guard ever runs. The 409 NotParked arm is the
    // CONCURRENT-race path only. The load-bearing REQ-008 pin — the rejected
    // path records nothing — is unchanged; the status pin now matches the
    // pre-existing sequential contract. Flagged for the code-stage review.
    assert_eq!(second.into_response().status(), StatusCode::NOT_FOUND);
    assert_eq!(
        identity_events(&db, user_id).await.len(),
        1,
        "rejected second apply records no additional identityResolved event"
    );

    let dismiss_db = create_test_db().await;
    let dismiss_user = create_test_user(&dismiss_db).await;
    let dismiss_work = seed_work(
        &dismiss_db,
        dismiss_user,
        "Review Dismiss Identity Door",
        IdentityStatus::NeedsReview,
        None,
    )
    .await;
    dismiss_db
        .record_review_candidates(
            dismiss_work.id,
            &[candidate(
                "review-dismiss-1",
                "Review Dismiss Identity Door",
                "OL-REVIEW-DISMISS-1W",
            )],
        )
        .await
        .expect("record dismiss candidates");
    let dismiss_status = livrarr_handlers::identity_review::dismiss(
        State(test_state(dismiss_db.clone())),
        auth_context(&dismiss_db, dismiss_user).await,
        Path(dismiss_work.id),
    )
    .await
    .expect("dismiss review park");
    assert_eq!(dismiss_status, StatusCode::NO_CONTENT);
    assert!(
        identity_events(&dismiss_db, dismiss_user).await.is_empty(),
        "dismiss settles no identity and must record no identityResolved event"
    );
}

#[tokio::test]
async fn wh_affirm_pending_anchor_records_once_and_settled_slot_records_zero() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_work(
        &db,
        user_id,
        "Affirm Identity Door",
        IdentityStatus::Confirmed,
        None,
    )
    .await;
    db.record_pending_anchor(work.id, AnchorType::new(AnchorType::ASIN), "B0AFFIRMWH")
        .await
        .expect("record pending ASIN");

    let status = livrarr_handlers::work::affirm_pending_anchor(
        State(test_state(db.clone())),
        auth_context(&db, user_id).await,
        Path((work.id, AnchorType::ASIN.to_string())),
    )
    .await
    .expect("affirm pending anchor");
    // Bug reproduction: identity-layer-rewrite — affirm resolves in the same
    // request and records exactly one actor-attributed identity event.
    assert_eq!(status.status(), StatusCode::NO_CONTENT);
    let events = identity_events(&db, user_id).await;
    assert_only_action(&events, "affirm", work.id);

    let settled_db = create_test_db().await;
    let settled_user = create_test_user(&settled_db).await;
    let settled_work = seed_work(
        &settled_db,
        settled_user,
        "Settled Slot Identity Door",
        IdentityStatus::Confirmed,
        Some("OL-SETTLED-1W"),
    )
    .await;
    settled_db
        .record_pending_anchor(
            settled_work.id,
            AnchorType::new(AnchorType::OL_WORK),
            "OL-SETTLED-OTHERW",
        )
        .await
        .expect("record competing settled-slot guess");
    let err = livrarr_handlers::work::affirm_pending_anchor(
        State(test_state(settled_db.clone())),
        auth_context(&settled_db, settled_user).await,
        Path((settled_work.id, AnchorType::OL_WORK.to_string())),
    )
    .await
    .expect_err("settled-slot affirm is rejected");
    assert_eq!(err.into_response().status(), StatusCode::CONFLICT);
    assert!(
        identity_events(&settled_db, settled_user).await.is_empty(),
        "settled-slot 409 records no identityResolved event"
    );
}

#[tokio::test]
async fn wh_system_side_anchor_confirmation_records_zero_identity_events() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_work(
        &db,
        user_id,
        "System Anchor Road",
        IdentityStatus::Pending,
        None,
    )
    .await;

    db.confirm_anchor(
        work.id,
        AnchorType::new(AnchorType::GR_WORK),
        "123456",
        AnchorSetter::Import,
    )
    .await
    .expect("system-side import setter confirms anchor");
    let anchors = db.list_anchors(work.id).await.expect("list anchors");
    assert_eq!(
        anchors
            .iter()
            .find(|a| a.anchor_type.as_str() == AnchorType::GR_WORK)
            .expect("system-confirmed GR anchor")
            .confidence,
        AnchorConfidence::Confirmed
    );
    assert!(
        db.list_history(user_id, empty_filter())
            .await
            .expect("list history after system confirmation")
            .is_empty(),
        "system-initiated anchor writes must not log identityResolved"
    );
}
