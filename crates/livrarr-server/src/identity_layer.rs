//! Identity-layer-rewrite (F2) composition root wiring. IR v1
//! `livrarr-server` module (ir-v1-identity-layer-rewrite.yaml:1353-1393).
//!
//! DELIBERATE SHADOW NAME: `StartupError` here is a *different*, new type
//! from the existing `crate::author_link::StartupError` (a struct carrying
//! an `AuthorRouteBackfillReport`). See STUBS-REPORT.md.

use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "test-helpers")]
use std::sync::atomic::{AtomicI64, Ordering};

use tokio_util::sync::CancellationToken;

use livrarr_domain::identity_layer::{
    EditionRepository, IdentityAuthorityReadiness, IdentityCutoverService, IdentityMigrationError,
    IdentityMigrationReport, IdentityRoadOutcome, ReviewActor, ReviewKind, ReviewResolutionCommand,
    WorkIdentityRepository,
};
use livrarr_domain::services::AuthorLinkWorkflow;
use livrarr_identity::identity_layer::IdentityEngine;
use livrarr_library::identity_layer::EpubCoverInspector;
use livrarr_metadata::identity_road::IdentityRoadServiceImpl;

pub type LiveIdentityRoad = IdentityRoadServiceImpl<
    livrarr_identity::identity_layer::DeterministicIdentityEngine,
    livrarr_db::sqlite::SqliteDb,
    livrarr_db::sqlite::SqliteDb,
    crate::services::author_linking_service::LiveAuthorLinkRoad,
>;

#[derive(Debug, Clone, PartialEq)]
pub enum IdentityRoadCall {
    Settle(livrarr_domain::identity_layer::IdentityRoadRequest),
    Resolve {
        actor: ReviewActor,
        command: ReviewResolutionCommand,
    },
}

#[derive(Debug, Clone, Default)]
pub struct IdentityRoadRecorder {
    calls: Arc<std::sync::Mutex<Vec<IdentityRoadCall>>>,
    outcomes: Arc<std::sync::Mutex<Vec<String>>>,
}

impl IdentityRoadRecorder {
    pub fn snapshot(&self) -> Vec<IdentityRoadCall> {
        self.calls.lock().expect("identity road recorder").clone()
    }

    pub fn clear(&self) {
        self.calls.lock().expect("identity road recorder").clear();
        self.outcomes
            .lock()
            .expect("identity road outcome recorder")
            .clear();
    }

    pub fn outcome_snapshot(&self) -> Vec<String> {
        self.outcomes
            .lock()
            .expect("identity road outcome recorder")
            .clone()
    }

    fn record(&self, call: IdentityRoadCall) {
        self.calls
            .lock()
            .expect("identity road recorder")
            .push(call);
    }

    fn record_outcome(
        &self,
        outcome: &Result<IdentityRoadOutcome, livrarr_domain::identity_layer::IdentityRoadError>,
    ) {
        self.outcomes
            .lock()
            .expect("identity road outcome recorder")
            .push(format!("{outcome:?}"));
    }
}

/// Static-dispatch recorder injected only by route/integration tests. It
/// records typed road calls, never provider credentials or response payloads.
pub struct RecordingIdentityRoad<S> {
    pub inner: S,
    pub recorder: IdentityRoadRecorder,
}

impl<S> livrarr_domain::identity_layer::IdentityRoadService for RecordingIdentityRoad<S>
where
    S: livrarr_domain::identity_layer::IdentityRoadService + Send + Sync,
{
    async fn settle(
        &self,
        request: livrarr_domain::identity_layer::IdentityRoadRequest,
    ) -> Result<IdentityRoadOutcome, livrarr_domain::identity_layer::IdentityRoadError> {
        self.recorder
            .record(IdentityRoadCall::Settle(request.clone()));
        let outcome = self.inner.settle(request).await;
        self.recorder.record_outcome(&outcome);
        outcome
    }

    async fn resolve_review(
        &self,
        actor: ReviewActor,
        command: ReviewResolutionCommand,
    ) -> Result<IdentityRoadOutcome, livrarr_domain::identity_layer::IdentityRoadError> {
        self.recorder.record(IdentityRoadCall::Resolve {
            actor: actor.clone(),
            command: command.clone(),
        });
        self.inner.resolve_review(actor, command).await
    }

    async fn apply_captured_route_handoff(
        &self,
        user_id: i64,
        work_id: i64,
        trigger: livrarr_domain::identity_layer::IdentityRoadOrigin,
        handoff: livrarr_domain::identity_layer::CapturedRouteHandoff,
    ) -> Result<Option<IdentityRoadOutcome>, livrarr_domain::identity_layer::IdentityRoadError>
    {
        self.inner
            .apply_captured_route_handoff(user_id, work_id, trigger, handoff)
            .await
    }
}

/// Production holds the live road directly. Tests opt into the recorder
/// through the same AppState field and the same enum-dispatch service seam.
pub enum AppIdentityRoad {
    Live(LiveIdentityRoad),
    Recording(RecordingIdentityRoad<LiveIdentityRoad>),
}

impl AppIdentityRoad {
    pub fn test_recorder(&self) -> &IdentityRoadRecorder {
        match self {
            Self::Recording(recording) => &recording.recorder,
            Self::Live(_) => panic!("identity road recorder is test-only"),
        }
    }
}

impl livrarr_domain::identity_layer::IdentityRoadService for AppIdentityRoad {
    async fn settle(
        &self,
        request: livrarr_domain::identity_layer::IdentityRoadRequest,
    ) -> Result<IdentityRoadOutcome, livrarr_domain::identity_layer::IdentityRoadError> {
        match self {
            Self::Live(road) => road.settle(request).await,
            Self::Recording(road) => road.settle(request).await,
        }
    }

    async fn resolve_review(
        &self,
        actor: ReviewActor,
        command: ReviewResolutionCommand,
    ) -> Result<IdentityRoadOutcome, livrarr_domain::identity_layer::IdentityRoadError> {
        match self {
            Self::Live(road) => road.resolve_review(actor, command).await,
            Self::Recording(road) => road.resolve_review(actor, command).await,
        }
    }

    async fn apply_captured_route_handoff(
        &self,
        user_id: i64,
        work_id: i64,
        trigger: livrarr_domain::identity_layer::IdentityRoadOrigin,
        handoff: livrarr_domain::identity_layer::CapturedRouteHandoff,
    ) -> Result<Option<IdentityRoadOutcome>, livrarr_domain::identity_layer::IdentityRoadError>
    {
        match self {
            Self::Live(road) => {
                road.apply_captured_route_handoff(user_id, work_id, trigger, handoff)
                    .await
            }
            Self::Recording(road) => {
                road.apply_captured_route_handoff(user_id, work_id, trigger, handoff)
                    .await
            }
        }
    }
}

/// Build the one production identity road from process-shared transports.
/// The concrete generic type keeps dispatch static and leaves a clean seam
/// for tests to wrap the road without a `dyn` service object.
pub fn build_live_identity_road(
    db: livrarr_db::sqlite::SqliteDb,
    http_fetcher: livrarr_http::fetcher::HttpFetcherImpl,
    http_client: livrarr_http::HttpClient,
    live_metadata_config: livrarr_external_data::live_config::LiveMetadataConfig,
) -> AppIdentityRoad {
    AppIdentityRoad::Live(build_identity_road(
        db,
        http_fetcher,
        http_client,
        live_metadata_config,
    ))
}

/// Test composition uses the exact production road and AppState seam, adding
/// only an injectable call recorder.
pub fn build_recording_identity_road(
    db: livrarr_db::sqlite::SqliteDb,
    http_fetcher: livrarr_http::fetcher::HttpFetcherImpl,
    http_client: livrarr_http::HttpClient,
    live_metadata_config: livrarr_external_data::live_config::LiveMetadataConfig,
) -> AppIdentityRoad {
    AppIdentityRoad::Recording(RecordingIdentityRoad {
        inner: build_identity_road(db, http_fetcher, http_client, live_metadata_config),
        recorder: IdentityRoadRecorder::default(),
    })
}

fn build_identity_road(
    db: livrarr_db::sqlite::SqliteDb,
    http_fetcher: livrarr_http::fetcher::HttpFetcherImpl,
    http_client: livrarr_http::HttpClient,
    live_metadata_config: livrarr_external_data::live_config::LiveMetadataConfig,
) -> LiveIdentityRoad {
    let author_gateway = livrarr_external_data::AuthorProviderGatewayImpl::new(
        livrarr_external_data::OpenLibraryClient::new(http_fetcher.clone()),
        livrarr_external_data::GoodreadsClient::new(
            http_fetcher.clone(),
            http_client,
            livrarr_external_data::goodreads::GOODREADS_BASE_URL,
        ),
        livrarr_external_data::HardcoverClient::new(http_fetcher, live_metadata_config),
    );
    IdentityRoadServiceImpl {
        identity_engine: livrarr_identity::identity_layer::DeterministicIdentityEngine,
        identity_repository: db.clone(),
        edition_repository: db.clone(),
        author_link_workflow: livrarr_metadata::author_linking::AuthorLinkingServiceImpl {
            db,
            gateway: author_gateway,
        },
    }
}

/// Generic/enum-dispatch wrapper implementing the domain `IdentityCutoverService`
/// trait — IR v1 names a concrete `IdentityCutoverServiceImpl<...>` in
/// `IdentityLayerComposition` without assigning it to a specific crate file
/// (unlike `IdentityRoadServiceImpl`, which is explicitly pinned to
/// `crates/livrarr-metadata/src/identity_road.rs`). Placed here since only
/// this composition struct references it. See STUBS-REPORT.md.
pub struct IdentityCutoverServiceImpl<C>
where
    C: IdentityCutoverService + Send + Sync + 'static,
{
    pub inner: C,
}

impl<C> IdentityCutoverServiceImpl<C>
where
    C: IdentityCutoverService + Send + Sync + 'static,
{
    /// Public concrete surface for the ratified cutover apply contract. The
    /// trait implementation below delegates here so the signature has one
    /// implementation while remaining visible to structural conformance.
    pub async fn apply(
        &self,
        approved_report: IdentityMigrationReport,
        cancel: CancellationToken,
    ) -> Result<IdentityMigrationReport, IdentityMigrationError> {
        self.inner.apply(approved_report, cancel).await
    }
}

impl<C> IdentityCutoverService for IdentityCutoverServiceImpl<C>
where
    C: IdentityCutoverService + Send + Sync + 'static,
{
    async fn rehearse(
        &self,
        snapshot: livrarr_domain::identity_layer::SnapshotDatabase,
        cancel: CancellationToken,
    ) -> Result<IdentityMigrationReport, IdentityMigrationError> {
        self.inner.rehearse(snapshot, cancel).await
    }

    async fn apply(
        &self,
        approved_report: IdentityMigrationReport,
        cancel: CancellationToken,
    ) -> Result<IdentityMigrationReport, IdentityMigrationError> {
        Self::apply(self, approved_report, cancel).await
    }

    async fn ensure_authority_ready(
        &self,
        cancel: CancellationToken,
    ) -> Result<IdentityAuthorityReadiness, IdentityMigrationError> {
        self.inner.ensure_authority_ready(cancel).await
    }
}

/// IR v1 leaves `IdentityRoadServiceImpl`'s `I`/`R`/`E`/`A` type parameters
/// (and `IdentityCutoverServiceImpl`'s `C`) unnamed — no module assigns a
/// concrete `IdentityEngine`/`WorkIdentityRepository`/etc. implementation
/// struct a name. Kept generic here rather than inventing concrete impl
/// names IR v1 never gives. See STUBS-REPORT.md.
pub struct IdentityLayerComposition<I, R, E, A, C>
where
    I: IdentityEngine + Send + Sync + 'static,
    R: livrarr_domain::identity_layer::WorkIdentityRepository + Send + Sync + 'static,
    E: EditionRepository + Send + Sync + 'static,
    A: AuthorLinkWorkflow + Send + Sync + 'static,
    C: IdentityCutoverService + Send + Sync + 'static,
{
    pub identity_road: Arc<IdentityRoadServiceImpl<I, R, E, A>>,
    pub cutover: Arc<IdentityCutoverServiceImpl<C>>,
    pub epub_inspector: Arc<EpubCoverInspector>,
}

#[derive(Debug, Clone)]
pub enum IdentityCutoverCliCommand {
    Rehearse {
        snapshot: PathBuf,
    },
    ListReviews,
    ShowReview {
        card_id: i64,
    },
    ResolveReview {
        card_id: i64,
        expected_generation: i64,
        action_file: PathBuf,
    },
    Apply {
        approved_report: PathBuf,
    },
}

/// IR v1 names `CutoverReviewSummary` without a field list. Shaped from
/// `pre_activation_review_cli.command_flow`'s `identity-cutover list` text
/// ("emits all unresolved card ids, generations, user scopes, and
/// ReviewKinds"). `user_id` covers "user scopes". See STUBS-REPORT.md.
#[derive(Debug, Clone)]
pub struct CutoverReviewSummary {
    pub card_id: i64,
    pub user_id: livrarr_domain::UserId,
    pub kind: ReviewKind,
    pub generation: i64,
}

/// IR v1 names `CutoverReviewDetail` without a field list. Shaped from the
/// same command_flow's `identity-cutover show` text ("emits the preserved
/// evidence, allowed actions, and current generation").
#[derive(Debug, Clone)]
pub struct CutoverReviewDetail {
    pub card_id: i64,
    pub kind: ReviewKind,
    pub generation: i64,
    pub evidence_json: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum IdentityCutoverCliOutcome {
    Report(IdentityMigrationReport),
    ReviewList(Vec<CutoverReviewSummary>),
    ReviewDetail(CutoverReviewDetail),
    ReviewResolved(IdentityRoadOutcome),
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum IdentityCutoverCommandError {
    #[error("review card not found")]
    NotFound,
    #[error("not a snapshot database")]
    NotSnapshot,
    #[error("rehearsal mismatch")]
    RehearsalMismatch,
    #[error("review kind mismatch")]
    ReviewKindMismatch,
    #[error("stale generation")]
    StaleGeneration,
    #[error("invalid action file")]
    InvalidActionFile,
    #[error("cancelled")]
    Cancelled,
    #[error("database error: {0}")]
    Database(String),
}

/// New, shadow `StartupError` — see module header.
#[derive(Debug, Clone, thiserror::Error)]
pub enum StartupError {
    #[error("cutover required")]
    CutoverRequired,
    #[error("database error: {0}")]
    Database(String),
}

/// IR v1 names `IdentityConvergenceReport` without a field list. Shaped from
/// `route_capture_handoff.triggers`' `ConvergenceVisit` row (visits works,
/// submits captured route evidence).
#[derive(Debug, Clone, Default)]
pub struct IdentityConvergenceReport {
    pub visited_work_count: u64,
    pub captured_route_count: u64,
}

#[cfg(feature = "test-helpers")]
static FAIL_NEXT_IDENTITY_CONVERGENCE_WORK: AtomicI64 = AtomicI64::new(0);

/// Inject one real per-Work convergence error before the production failure
/// branch, for cadence/backoff contract coverage.
#[cfg(feature = "test-helpers")]
pub fn fail_next_identity_convergence_for_tests(work_id: livrarr_domain::WorkId) {
    FAIL_NEXT_IDENTITY_CONVERGENCE_WORK.store(work_id, Ordering::SeqCst);
}

#[cfg(feature = "test-helpers")]
fn take_identity_convergence_failure(work_id: livrarr_domain::WorkId) -> bool {
    FAIL_NEXT_IDENTITY_CONVERGENCE_WORK
        .compare_exchange(work_id, 0, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

#[cfg(not(feature = "test-helpers"))]
fn take_identity_convergence_failure(_work_id: livrarr_domain::WorkId) -> bool {
    false
}

pub async fn run_identity_convergence_tick(
    state: crate::state::AppState,
    cancel: CancellationToken,
) -> Result<IdentityConvergenceReport, livrarr_jobs::JobError> {
    use chrono::Duration as ChronoDuration;
    use livrarr_db::{UserDb, WorkDb};
    use livrarr_domain::identity_layer::IdentityRoadService as _;
    use livrarr_domain::services::{ConvergeOutcome, WorkService};

    if cancel.is_cancelled() {
        return Err(livrarr_jobs::JobError::Failed {
            message: "identity convergence cancelled".to_string(),
            source: None,
        });
    }
    if !state.config.convergence.enabled {
        return Ok(IdentityConvergenceReport::default());
    }
    let threshold = state.config.convergence.attempt_threshold;
    let batch = state.config.convergence.batch_size;
    let cadence = ChronoDuration::seconds(state.config.convergence.interval_secs as i64);
    let users = state
        .db
        .list_users()
        .await
        .map_err(|error| livrarr_jobs::JobError::Failed {
            message: format!("identity convergence list users: {error}"),
            source: None,
        })?;
    let mut report = IdentityConvergenceReport::default();
    for user in users {
        if cancel.is_cancelled() {
            break;
        }
        let due = match state
            .db
            .list_convergence_due(user.id, chrono::Utc::now(), threshold, batch)
            .await
        {
            Ok(due) => due,
            Err(error) => {
                tracing::warn!(user_id = user.id, "convergence due read failed: {error}");
                continue;
            }
        };
        for work_id in due {
            if cancel.is_cancelled() {
                break;
            }
            report.visited_work_count += 1;
            let convergence = if take_identity_convergence_failure(work_id) {
                Err("injected identity convergence failure".to_string())
            } else {
                state
                    .work_service
                    .converge_work_with_handoff(user.id, work_id, threshold, None)
                    .await
                    .map_err(|error| error.to_string())
            };
            let mut pass = match convergence {
                Ok(pass) => pass,
                Err(error) => {
                    tracing::warn!(user_id = user.id, work_id, "convergence failed: {error}");
                    let _ = state
                        .db
                        .set_next_convergence_at(
                            user.id,
                            work_id,
                            Some(chrono::Utc::now() + cadence),
                        )
                        .await;
                    continue;
                }
            };

            // Fresh provider evidence is handed off before the attempt
            // checkpoint. Persisted routes never enter this seam.
            let found_fresh_route = pass
                .route_handoff
                .as_ref()
                .is_some_and(|handoff| !handoff.provider_identity.is_empty());
            if let Some(handoff) = pass.route_handoff.take() {
                report.captured_route_count += handoff.provider_identity.len() as u64;
                if let Err(error) = state
                    .identity_road
                    .apply_captured_route_handoff(
                        user.id,
                        work_id,
                        livrarr_domain::identity_layer::IdentityRoadOrigin::ConvergenceVisit,
                        handoff,
                    )
                    .await
                {
                    tracing::warn!(user_id = user.id, work_id, "route handoff failed: {error}");
                    let _ = state
                        .db
                        .set_next_convergence_at(
                            user.id,
                            work_id,
                            Some(chrono::Utc::now() + cadence),
                        )
                        .await;
                    continue;
                }
            }
            let bridge_attempt = if pass.provider_chase_attempted && !found_fresh_route {
                match state.db.read_captured_identity(user.id, work_id).await {
                    Ok(after)
                        if !after.active_routes.iter().any(|route| {
                            matches!(
                                route.kind,
                                livrarr_domain::identity_layer::RouteKind::OpenLibraryWork
                                    | livrarr_domain::identity_layer::RouteKind::GoodreadsWork
                                    | livrarr_domain::identity_layer::RouteKind::HardcoverWork
                            )
                        }) =>
                    {
                        match state
                            .db
                            .record_identity_convergence_attempt(
                                user.id,
                                work_id,
                                after.identity_generation,
                            )
                            .await
                        {
                            Ok(attempt) => Some(attempt),
                            Err(error) => {
                                tracing::warn!(
                                    user_id = user.id,
                                    work_id,
                                    "convergence attempt checkpoint failed: {error}"
                                );
                                let _ = state
                                    .db
                                    .set_next_convergence_at(
                                        user.id,
                                        work_id,
                                        Some(chrono::Utc::now() + cadence),
                                    )
                                    .await;
                                continue;
                            }
                        }
                    }
                    Ok(_) => None,
                    Err(error) => {
                        tracing::warn!(
                            user_id = user.id,
                            work_id,
                            "post-convergence identity read failed: {error}"
                        );
                        let _ = state
                            .db
                            .set_next_convergence_at(
                                user.id,
                                work_id,
                                Some(chrono::Utc::now() + cadence),
                            )
                            .await;
                        continue;
                    }
                }
            } else {
                None
            };
            let next = match bridge_attempt {
                Some(attempt) if attempt < threshold => Some(chrono::Utc::now() + cadence),
                Some(_) => None,
                None => match pass.outcome {
                    ConvergeOutcome::Completed | ConvergeOutcome::Terminal => None,
                    ConvergeOutcome::StillIncomplete => Some(chrono::Utc::now() + cadence),
                },
            };
            if let Err(error) = state
                .db
                .set_next_convergence_at(user.id, work_id, next)
                .await
            {
                tracing::warn!(
                    user_id = user.id,
                    work_id,
                    "convergence checkpoint failed: {error}"
                );
            }
        }
    }
    Ok(report)
}

/// List/show/resolve operate only under the local exclusive `CutoverOperator`
/// actor; `ResolveReview` parses a kind-matched `ReviewResolutionCommand`
/// before calling the shared road continuation.
pub async fn run_identity_cutover_command(
    command: IdentityCutoverCliCommand,
    data_dir: PathBuf,
    cancel: CancellationToken,
) -> Result<IdentityCutoverCliOutcome, IdentityCutoverCommandError> {
    if cancel.is_cancelled() {
        return Err(IdentityCutoverCommandError::Cancelled);
    }
    std::fs::create_dir_all(&data_dir)
        .map_err(|error| IdentityCutoverCommandError::Database(error.to_string()))?;
    let _exclusive_lock = ExclusiveCutoverLock::acquire(data_dir.clone())?;
    let pool = livrarr_db::pool::create_sqlite_pool(&data_dir)
        .await
        .map_err(|error| IdentityCutoverCommandError::Database(error.to_string()))?;
    livrarr_db::pool::run_migrations(&pool)
        .await
        .map_err(|error| IdentityCutoverCommandError::Database(error.to_string()))?;
    let db = livrarr_db::sqlite::SqliteDb::new(pool);
    let actor = ReviewActor::CutoverOperator {
        installation_id: "local-installation".to_string(),
        invocation_id: format!(
            "{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_micros()
        ),
    };

    match command {
        IdentityCutoverCliCommand::Rehearse { snapshot } => db
            .rehearse(
                livrarr_domain::identity_layer::SnapshotDatabase { path: snapshot },
                cancel,
            )
            .await
            .map(IdentityCutoverCliOutcome::Report)
            .map_err(map_migration_error),
        IdentityCutoverCliCommand::Apply { approved_report } => {
            let bytes = std::fs::read(approved_report)
                .map_err(|_| IdentityCutoverCommandError::InvalidActionFile)?;
            let report = serde_json::from_slice(&bytes)
                .map_err(|_| IdentityCutoverCommandError::InvalidActionFile)?;
            db.apply(report, cancel)
                .await
                .map(IdentityCutoverCliOutcome::Report)
                .map_err(map_migration_error)
        }
        IdentityCutoverCliCommand::ListReviews => {
            let rows = sqlx::query_as::<_, (i64, i64, String, i64)>(
                "SELECT id, user_id, kind, generation FROM identity_review_cards \
                 WHERE status = 'pending' ORDER BY id",
            )
            .fetch_all(db.pool())
            .await
            .map_err(|error| IdentityCutoverCommandError::Database(error.to_string()))?;
            let reviews = rows
                .into_iter()
                .map(|(card_id, user_id, kind, generation)| {
                    Ok(CutoverReviewSummary {
                        card_id,
                        user_id,
                        kind: parse_review_kind(&kind)?,
                        generation,
                    })
                })
                .collect::<Result<Vec<_>, IdentityCutoverCommandError>>()?;
            Ok(IdentityCutoverCliOutcome::ReviewList(reviews))
        }
        IdentityCutoverCliCommand::ShowReview { card_id } => {
            let pending = db
                .load_pending_review(actor, card_id)
                .await
                .map_err(map_repository_command_error)?;
            Ok(IdentityCutoverCliOutcome::ReviewDetail(
                CutoverReviewDetail {
                    card_id: pending.id,
                    kind: pending.kind,
                    generation: pending.generation,
                    evidence_json: serde_json::to_value(pending.payload).map_err(|error| {
                        IdentityCutoverCommandError::Database(error.to_string())
                    })?,
                },
            ))
        }
        IdentityCutoverCliCommand::ResolveReview {
            card_id,
            expected_generation,
            action_file,
        } => {
            let pending = db
                .load_pending_review(actor.clone(), card_id)
                .await
                .map_err(map_repository_command_error)?;
            let bytes = std::fs::read(action_file)
                .map_err(|_| IdentityCutoverCommandError::InvalidActionFile)?;
            let action: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|_| IdentityCutoverCommandError::InvalidActionFile)?;
            let resolution =
                command_from_action(pending.kind, card_id, expected_generation, action)?;
            let road = IdentityRoadServiceImpl {
                identity_engine: livrarr_identity::identity_layer::DeterministicIdentityEngine,
                identity_repository: db.clone(),
                edition_repository: db,
                author_link_workflow: CutoverAuthorWorkflow,
            };
            road.resolve_review(actor, resolution)
                .await
                .map(IdentityCutoverCliOutcome::ReviewResolved)
                .map_err(map_road_command_error)
        }
    }
}

struct ExclusiveCutoverLock(PathBuf);

impl ExclusiveCutoverLock {
    fn acquire(data_dir: PathBuf) -> Result<Self, IdentityCutoverCommandError> {
        livrarr_db::pool::acquire_pid_lock(&data_dir)
            .map_err(|error| IdentityCutoverCommandError::Database(error.to_string()))?;
        Ok(Self(data_dir))
    }
}

impl Drop for ExclusiveCutoverLock {
    fn drop(&mut self) {
        livrarr_db::pool::release_pid_lock(&self.0);
    }
}

fn parse_review_kind(value: &str) -> Result<ReviewKind, IdentityCutoverCommandError> {
    ReviewKind::from_storage_code(value).ok_or_else(|| {
        IdentityCutoverCommandError::Database(format!("unknown review kind {value}"))
    })
}

fn normalized_action(value: serde_json::Value) -> serde_json::Value {
    match &value {
        serde_json::Value::Object(map) if map.len() == 1 => {
            let (name, payload) = map.iter().next().expect("single action entry");
            if payload.is_null() {
                serde_json::Value::String(name.clone())
            } else {
                value
            }
        }
        _ => value,
    }
}

fn command_from_action(
    kind: ReviewKind,
    card_id: i64,
    expected_generation: i64,
    action: serde_json::Value,
) -> Result<ReviewResolutionCommand, IdentityCutoverCommandError> {
    let action = normalized_action(action);
    let invalid = |_| IdentityCutoverCommandError::InvalidActionFile;
    Ok(match kind {
        ReviewKind::IdentityConflict => ReviewResolutionCommand::IdentityConflict {
            card_id,
            expected_generation,
            action: serde_json::from_value(action).map_err(invalid)?,
        },
        ReviewKind::PendingRoute => ReviewResolutionCommand::PendingRoute {
            card_id,
            expected_generation,
            action: serde_json::from_value(action).map_err(invalid)?,
        },
        ReviewKind::GroupIdentity => ReviewResolutionCommand::GroupIdentity {
            card_id,
            expected_generation,
            action: serde_json::from_value(action).map_err(invalid)?,
        },
        ReviewKind::FieldResolution => ReviewResolutionCommand::FieldResolution {
            card_id,
            expected_generation,
            action: serde_json::from_value(action).map_err(invalid)?,
        },
        ReviewKind::EditionEvidence => ReviewResolutionCommand::EditionEvidence {
            card_id,
            expected_generation,
            action: serde_json::from_value(action).map_err(invalid)?,
        },
        ReviewKind::ImportIdentity => ReviewResolutionCommand::ImportIdentity {
            card_id,
            expected_generation,
            action: serde_json::from_value(action).map_err(invalid)?,
        },
        ReviewKind::MigrationRepair => ReviewResolutionCommand::MigrationRepair {
            card_id,
            expected_generation,
            action: serde_json::from_value(action).map_err(invalid)?,
        },
        ReviewKind::InvariantRepair => ReviewResolutionCommand::InvariantRepair {
            card_id,
            expected_generation,
            action: serde_json::from_value(action).map_err(invalid)?,
        },
        ReviewKind::ContributorOrder => serde_json::from_value(action).map_err(invalid)?,
    })
}

fn map_migration_error(error: IdentityMigrationError) -> IdentityCutoverCommandError {
    match error {
        IdentityMigrationError::NotSnapshot | IdentityMigrationError::SchemaMismatch => {
            IdentityCutoverCommandError::NotSnapshot
        }
        IdentityMigrationError::RehearsalMismatch | IdentityMigrationError::Collision => {
            IdentityCutoverCommandError::RehearsalMismatch
        }
        IdentityMigrationError::Cancelled => IdentityCutoverCommandError::Cancelled,
        IdentityMigrationError::Database(message) => IdentityCutoverCommandError::Database(message),
        IdentityMigrationError::InvalidFixture => IdentityCutoverCommandError::NotSnapshot,
    }
}

fn map_repository_command_error(
    error: livrarr_domain::identity_layer::IdentityRepositoryError,
) -> IdentityCutoverCommandError {
    match error {
        livrarr_domain::identity_layer::IdentityRepositoryError::NotFound => {
            IdentityCutoverCommandError::NotFound
        }
        livrarr_domain::identity_layer::IdentityRepositoryError::StaleGeneration => {
            IdentityCutoverCommandError::StaleGeneration
        }
        livrarr_domain::identity_layer::IdentityRepositoryError::ReviewKindMismatch => {
            IdentityCutoverCommandError::ReviewKindMismatch
        }
        livrarr_domain::identity_layer::IdentityRepositoryError::Cancelled => {
            IdentityCutoverCommandError::Cancelled
        }
        other => IdentityCutoverCommandError::Database(other.to_string()),
    }
}

fn map_road_command_error(
    error: livrarr_domain::identity_layer::IdentityRoadError,
) -> IdentityCutoverCommandError {
    match error {
        livrarr_domain::identity_layer::IdentityRoadError::NotFound => {
            IdentityCutoverCommandError::NotFound
        }
        livrarr_domain::identity_layer::IdentityRoadError::StaleGeneration => {
            IdentityCutoverCommandError::StaleGeneration
        }
        livrarr_domain::identity_layer::IdentityRoadError::ReviewKindMismatch => {
            IdentityCutoverCommandError::ReviewKindMismatch
        }
        livrarr_domain::identity_layer::IdentityRoadError::Cancelled => {
            IdentityCutoverCommandError::Cancelled
        }
        other => IdentityCutoverCommandError::Database(other.to_string()),
    }
}

#[derive(Clone, Copy)]
struct CutoverAuthorWorkflow;

impl AuthorLinkWorkflow for CutoverAuthorWorkflow {
    async fn enqueue(
        &self,
        _user_id: livrarr_domain::UserId,
        _author_id: livrarr_domain::AuthorId,
        _trigger: livrarr_domain::AuthorLinkTrigger,
    ) -> Result<(), livrarr_domain::AuthorLinkError> {
        Ok(())
    }
    async fn submit_evidence(
        &self,
        _user_id: livrarr_domain::UserId,
        _author_id: livrarr_domain::AuthorId,
        _evidence: livrarr_domain::AgreedAuthorRouteEvidence,
    ) -> Result<livrarr_domain::RouteWriteOutcome, livrarr_domain::AuthorLinkError> {
        Err(livrarr_domain::AuthorLinkError::NotFound)
    }
    async fn record_readarr_rejection(
        &self,
        _user_id: livrarr_domain::UserId,
        _author_id: livrarr_domain::AuthorId,
        _rejected: livrarr_domain::RejectedAuthorRouteEvidence,
    ) -> Result<livrarr_domain::AuthorLinkCandidate, livrarr_domain::AuthorLinkError> {
        Err(livrarr_domain::AuthorLinkError::NotFound)
    }
    async fn run_due(
        &self,
        _batch_size: u32,
        _cancel: CancellationToken,
    ) -> Result<livrarr_domain::AuthorSweepTickSummary, livrarr_domain::AuthorLinkError> {
        Err(livrarr_domain::AuthorLinkError::NotFound)
    }
}

/// Production-only wrapper called by `init_database` after migrations and
/// before repositories/HTTP/jobs; delegates to
/// `SqliteDb::ensure_identity_authority_ready`. `create_test_db` calls the
/// DB method directly, so there is no `livrarr-db -> livrarr-server` call.
/// An already-active database takes the `skip_legacy_writers` branch before
/// startup categories 9b, 9c, 9d, and 9e can execute.
pub async fn ensure_identity_authority_ready_before_serve(
    db: livrarr_db::sqlite::SqliteDb,
) -> Result<IdentityAuthorityReadiness, StartupError> {
    let readiness = db
        .ensure_identity_authority_ready()
        .await
        .map_err(|error| match error {
            IdentityMigrationError::SchemaMismatch
            | IdentityMigrationError::InvalidFixture
            | IdentityMigrationError::Collision
            | IdentityMigrationError::RehearsalMismatch
            | IdentityMigrationError::NotSnapshot => StartupError::CutoverRequired,
            IdentityMigrationError::Cancelled => {
                StartupError::Database("authority readiness cancelled".to_string())
            }
            IdentityMigrationError::Database(message) => StartupError::Database(message),
        })?;
    match readiness {
        IdentityAuthorityReadiness::Active | IdentityAuthorityReadiness::ActivatedFresh => {
            Ok(readiness)
        }
        IdentityAuthorityReadiness::CutoverRequired => Err(StartupError::CutoverRequired),
    }
}
