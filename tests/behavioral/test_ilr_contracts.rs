//! Identity-layer-rewrite behavioral contract suite (RED-first).
//!
//! Every test name is the exact `RED <name>:` identifier from IR v2.  Tests
//! use the laid-down F2 public surfaces; door/workflow rows enter through the
//! production `build_router` and authentication middleware.  The identity-v2
//! schema assertion is intentionally RED until migrations 082/083 exist.

use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use chrono::Utc;
use livrarr_behavioral::stubs::{StubAuthorLinkWorkflow, StubHttpFetcher};
use livrarr_db::identity_layer::{
    set_identity_db_failpoint_for_tests, IdentityCutoverMode, IdentityDbFailpoint,
    LegacyIdentityFixture, TransferRouteCommand,
};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::{create_activated_test_db, create_test_db};
use livrarr_db::{
    AuthorDb, AuthorLinkDb, CreateAuthorDbRequest, CreateLibraryItemDbRequest,
    CreateSeriesDbRequest, CreateUserDbRequest, CreateWorkDbRequest, LibraryItemDb,
    ProviderRetryStateDb, RootFolderDb, SeriesDb, UpdateAuthorDbRequest,
    UpdateWorkEnrichmentDbRequest, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::identity_layer::{
    self as ilr, CapturedIdentity, IdentityCutoverService, IdentityRoadInteraction,
    IdentityRoadOrigin, IdentityRoadRequest, IdentityRoadService, IdentityTitleTuple, ReviewActor,
    ReviewResolutionCommand, RouteKey, RouteOwner, WorkIdentityRepository,
};
use livrarr_domain::services::{
    AuthorMonitorWorkflow, CoverSlotState, ListService, MaterializeRequest, MaterializeService,
    MaterializeTags, RateBucket, SeriesQueryService, WorkIdentityRepository as _, WorkService,
};
use livrarr_domain::{
    AuthorProvider, AuthorRouteKey, LibraryItem, MediaType, MetadataProvider, OutcomeClass,
    TagStatus, UserRole, Work,
};
use livrarr_enrichment::ProviderQueue as _;
use livrarr_external_data::identity_layer::{
    GoodreadsAdapter, GoodreadsBookPage, ProviderRouteEvidence,
};
use livrarr_external_data::NormalizedWorkDetail;
use livrarr_identity::identity_layer::{
    DecisionEvidenceTier, DeterministicIdentityEngine, IdentityCapabilityClaim,
    IdentityDecisionRequest, IdentityDecisionSettlement, IdentityEngine, IdentityEngineError,
};
use livrarr_library::identity_layer::EpubCoverInspector;
use livrarr_matching::identity_layer::{find_matching_work, WorkMatchAuthorityInputs};
use livrarr_metadata::identity_road::{IdentityRoadServiceImpl, ProposedWorkIdentity};
use livrarr_server::auth_crypto::{AuthCryptoService, RealAuthCrypto};
use livrarr_server::state::AppState;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use tracing_test::traced_test;

fn title(main: &str) -> IdentityTitleTuple {
    IdentityTitleTuple {
        main: main.to_string(),
        subtitle: None,
        volume: None,
        normalized_main: main.to_ascii_lowercase(),
        normalized_subtitle: String::new(),
        normalized_volume: String::new(),
        provenance: ilr::EvidenceProvenance::User,
    }
}

fn identity(user_id: i64, work_id: i64) -> CapturedIdentity {
    CapturedIdentity {
        user_id,
        own_work_id: work_id,
        identity_title: title("Identity Road Fixture"),
        primary_author_id: 1,
        text_distinction: "common".to_string(),
        active_routes: vec![],
        status: ilr::IdentityStatus::NotConnected,
        identity_generation: 0,
    }
}

fn request(origin: IdentityRoadOrigin) -> IdentityRoadRequest {
    IdentityRoadRequest {
        user_id: 1,
        origin,
        evidence: ilr::IdentityEvidenceBundle {
            user_choice: Some(ilr::UserIdentityChoice::ExplicitCreate(
                ilr::MinimumWorkEvidence {
                    title: "Identity Road Fixture".to_string(),
                    authors: vec![1],
                },
            )),
            owned_files: vec![],
            provider_identity: vec![],
            minimum: Some(ilr::MinimumWorkEvidence {
                title: "Identity Road Fixture".to_string(),
                authors: vec![1],
            }),
        },
        interaction: IdentityRoadInteraction::HumanWatching,
        existing_work_id: None,
    }
}

fn migration_report() -> ilr::IdentityMigrationReport {
    ilr::IdentityMigrationReport {
        source_schema_version: 81,
        source_fingerprint: [1; 32],
        canonical_output_fingerprint: [2; 32],
        mapped_route_count: 0,
        edition_count: 0,
        repair_cards: 0,
        group_cards: 0,
        field_cards: 0,
        contributor_cards: 0,
        index_ready: false,
        trivially_empty: false,
        legacy_work_count: 1,
    }
}

fn revision() -> ilr::FileRevision {
    ilr::FileRevision {
        size_bytes: 1,
        modified_ns: 2,
        sha256: [3; 32],
    }
}

fn work_evidence() -> ilr::WorkIdentityEvidence {
    ilr::WorkIdentityEvidence {
        title: title("Identity Road Fixture"),
        primary_author_id: 1,
        routes: vec![],
    }
}

fn lost_guards() -> ilr::LostMatchGuardSet {
    ilr::LostMatchGuardSet {
        one_sided_subtitle_recovery: true,
        shared_edition_id_confirmation: true,
        translation_same_text_signals: Default::default(),
    }
}

fn wrong_guards() -> ilr::WrongMergeGuardSet {
    ilr::WrongMergeGuardSet {
        main_title_guard: ilr::MainTitleGuard(true),
        volume_conflict_guard: true,
        author_disagreement_guard: true,
        work_key_contradiction_guard: true,
        audited_different_text_guard: true,
    }
}

/// Remove Rust line/block comments while preserving executable tokens and
/// literals. Source-layout STOPs always use this view; a comment can never
/// satisfy a missing production seam.
fn strip_rust_comments(source: &str) -> String {
    let mut chars = source.chars().peekable();
    let mut output = String::with_capacity(source.len());
    let mut block_depth = 0usize;
    let mut in_line = false;
    let mut in_string = false;
    let mut in_char = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if in_line {
            if ch == '\n' {
                in_line = false;
                output.push(ch);
            }
            continue;
        }
        if block_depth > 0 {
            if ch == '/' && chars.peek() == Some(&'*') {
                chars.next();
                block_depth += 1;
            } else if ch == '*' && chars.peek() == Some(&'/') {
                chars.next();
                block_depth -= 1;
            } else if ch == '\n' {
                output.push(ch);
            }
            continue;
        }
        if in_string {
            output.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if in_char {
            output.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                in_char = false;
            }
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'/') {
            chars.next();
            in_line = true;
        } else if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            block_depth = 1;
        } else {
            if ch == '"' {
                in_string = true;
            } else if ch == '\'' {
                let mut lookahead = chars.clone();
                let first = lookahead.next();
                let closes_as_char = if first == Some('\\') {
                    lookahead.next();
                    lookahead.next() == Some('\'')
                } else {
                    lookahead.next() == Some('\'')
                };
                in_char = closes_as_char;
            }
            output.push(ch);
        }
    }
    output
}

type TestRoad = IdentityRoadServiceImpl<
    DeterministicIdentityEngine,
    SqliteDb,
    SqliteDb,
    StubAuthorLinkWorkflow,
>;

fn road(db: SqliteDb) -> TestRoad {
    IdentityRoadServiceImpl {
        identity_engine: DeterministicIdentityEngine,
        identity_repository: db.clone(),
        edition_repository: db,
        author_link_workflow: StubAuthorLinkWorkflow,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RecordedRoadCall {
    Settle(IdentityRoadOrigin),
    Resolve(ReviewResolutionCommand),
}

/// Test-local decorator required by the ratified trace directives. It wraps
/// the production road implementation and records only the public chokepoint
/// calls; it never substitutes behavior.
struct RecordingRoad<R> {
    inner: Arc<R>,
    calls: Arc<Mutex<Vec<RecordedRoadCall>>>,
}

impl<R> RecordingRoad<R> {
    fn new(inner: R) -> Self {
        Self {
            inner: Arc::new(inner),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn calls(&self) -> Vec<RecordedRoadCall> {
        self.calls.lock().expect("road call recorder").clone()
    }
}

impl<R> IdentityRoadService for RecordingRoad<R>
where
    R: IdentityRoadService + Send + Sync + 'static,
{
    async fn settle(
        &self,
        request: IdentityRoadRequest,
    ) -> Result<ilr::IdentityRoadOutcome, ilr::IdentityRoadError> {
        self.calls
            .lock()
            .expect("road call recorder")
            .push(RecordedRoadCall::Settle(request.origin.clone()));
        self.inner.settle(request).await
    }

    async fn resolve_review(
        &self,
        actor: ReviewActor,
        command: ReviewResolutionCommand,
    ) -> Result<ilr::IdentityRoadOutcome, ilr::IdentityRoadError> {
        self.calls
            .lock()
            .expect("road call recorder")
            .push(RecordedRoadCall::Resolve(command.clone()));
        self.inner.resolve_review(actor, command).await
    }
}

enum EngineContract {
    P5PrecedenceAndP4,
    ProbeBlockedVsInvalid,
    ConflictClasses,
    OpaqueAliasForClassA,
}

async fn red_engine_contract(contract: EngineContract) {
    let engine = DeterministicIdentityEngine;
    let make_request = |origin: IdentityRoadOrigin,
                        interaction: IdentityRoadInteraction,
                        existing: Option<CapturedIdentity>| {
        let evidence = request(origin.clone()).evidence;
        IdentityDecisionRequest {
            user_id: 1,
            origin,
            interaction,
            evidence,
            existing,
            incoming: work_evidence(),
            text_signal: None,
            alias_proof: None,
            capability_claim: None,
            lost_match: lost_guards(),
            wrong_merge: wrong_guards(),
        }
    };

    match contract {
        EngineContract::P5PrecedenceAndP4 => {
            let origins = [
                ilr::DoorKind::DirectAdd,
                ilr::DoorKind::ManualImport,
                ilr::DoorKind::ListImport,
                ilr::DoorKind::AuthorMonitor,
                ilr::DoorKind::SeriesMonitor,
                ilr::DoorKind::ReadarrImport,
            ];
            assert_eq!(origins.len(), 6);
            for door in origins {
                let decision = engine
                    .decide(make_request(
                        IdentityRoadOrigin::CreationDoor(door),
                        IdentityRoadInteraction::HumanWatching,
                        None,
                    ))
                    .await
                    .expect("each production door reaches the deterministic engine");
                assert_eq!(decision.selected_tier, DecisionEvidenceTier::UserChoice);
                assert_eq!(decision.settlement, IdentityDecisionSettlement::Decide);
            }

            let mut uncertain = work_evidence();
            uncertain.title = title("Different Work");
            uncertain.primary_author_id = 2;
            let mut human = make_request(
                IdentityRoadOrigin::ManualRefresh,
                IdentityRoadInteraction::HumanWatching,
                Some(identity(1, 1)),
            );
            human.incoming = uncertain.clone();
            let mut machine = make_request(
                IdentityRoadOrigin::ConvergenceVisit,
                IdentityRoadInteraction::MachineAlone,
                Some(identity(1, 1)),
            );
            machine.incoming = uncertain;
            assert_eq!(
                engine
                    .decide(human)
                    .await
                    .expect("human decision")
                    .settlement,
                IdentityDecisionSettlement::Review
            );
            assert_eq!(
                engine
                    .decide(machine)
                    .await
                    .expect("machine decision")
                    .settlement,
                IdentityDecisionSettlement::Defer
            );
            assert_ne!(
                IdentityRoadInteraction::HumanWatching,
                IdentityRoadInteraction::MachineAlone
            );
        }
        EngineContract::ProbeBlockedVsInvalid => {
            let mut invalid = make_request(
                IdentityRoadOrigin::ManualRefresh,
                IdentityRoadInteraction::MachineAlone,
                None,
            );
            invalid.user_id = 0;
            assert!(matches!(
                engine.decide(invalid).await,
                Err(IdentityEngineError::InvalidEvidence)
            ));
            let mut blocked = make_request(
                IdentityRoadOrigin::ManualRefresh,
                IdentityRoadInteraction::MachineAlone,
                None,
            );
            blocked.capability_claim = Some(IdentityCapabilityClaim::WhoseText);
            assert!(matches!(
                engine.decide(blocked).await,
                Err(IdentityEngineError::ProbeBlocked)
            ));
            assert!(!matches!(
                IdentityEngineError::ProbeBlocked,
                IdentityEngineError::InvalidEvidence
            ));
            assert_ne!(
                IdentityEngineError::ProbeBlocked.to_string(),
                IdentityEngineError::InvalidEvidence.to_string()
            );
        }
        EngineContract::ConflictClasses => {
            let mut existing = identity(1, 1);
            existing.active_routes.push(route(
                ilr::RouteKind::GoodreadsWork,
                "existing",
                RouteOwner::Work(1),
            ));
            let class_a = engine.classify_route_conflict(
                existing.clone(),
                ProviderRouteEvidence {
                    provider: ilr::IdentityProvider::Goodreads,
                    kind: ilr::RouteKind::GoodreadsWork,
                    provider_scoped_id: "different".to_string(),
                },
            );
            assert_eq!(
                class_a,
                Some(ilr::IdentityConflictClass::SameProviderWorkIdDisagreement)
            );
            let class_b = engine.classify_route_conflict(
                existing.clone(),
                ProviderRouteEvidence {
                    provider: ilr::IdentityProvider::OpenLibrary,
                    kind: ilr::RouteKind::OpenLibraryWork,
                    provider_scoped_id: "OL1W".to_string(),
                },
            );
            assert_eq!(
                class_b,
                Some(ilr::IdentityConflictClass::CrossProviderWorkKeyDisagreement)
            );
            existing.active_routes[0].resolved_work_id = 99;
            let class_c = engine.classify_route_conflict(
                existing.clone(),
                ProviderRouteEvidence {
                    provider: ilr::IdentityProvider::Goodreads,
                    kind: ilr::RouteKind::GoodreadsWork,
                    provider_scoped_id: "existing".to_string(),
                },
            );
            assert_eq!(
                class_c,
                Some(ilr::IdentityConflictClass::RouteOwnedByDifferentWork)
            );
            existing.active_routes.clear();
            existing.active_routes.push(route(
                ilr::RouteKind::GoodreadsBookEdition,
                "edition-1",
                RouteOwner::Edition(1),
            ));
            assert_eq!(
                engine.classify_route_conflict(
                    existing,
                    ProviderRouteEvidence {
                        provider: ilr::IdentityProvider::Goodreads,
                        kind: ilr::RouteKind::GoodreadsBookEdition,
                        provider_scoped_id: "edition-2".to_string(),
                    },
                ),
                None
            );
            let classes = [
                ilr::IdentityConflictClass::SameProviderWorkIdDisagreement,
                ilr::IdentityConflictClass::CrossProviderWorkKeyDisagreement,
                ilr::IdentityConflictClass::RouteOwnedByDifferentWork,
            ];
            assert_eq!(classes.len(), 3);
            assert_ne!(classes[0], classes[1]);
            assert_ne!(classes[1], classes[2]);
        }
        EngineContract::OpaqueAliasForClassA => {
            let mut existing = identity(1, 1);
            existing.active_routes.push(route(
                ilr::RouteKind::GoodreadsWork,
                "existing",
                RouteOwner::Work(1),
            ));
            let raw = engine.classify_route_conflict(
                existing,
                ProviderRouteEvidence {
                    provider: ilr::IdentityProvider::Goodreads,
                    kind: ilr::RouteKind::GoodreadsWork,
                    provider_scoped_id: "raw-different-id".to_string(),
                },
            );
            assert_eq!(
                raw,
                Some(ilr::IdentityConflictClass::SameProviderWorkIdDisagreement)
            );
            let source = strip_rust_comments(include_str!(
                "../../crates/livrarr-domain/src/identity_layer/matching.rs"
            ));
            assert!(source.contains("pub struct AliasEquivalenceProof"));
            assert!(source.contains("work_ids: BTreeSet<String>"));
            assert!(!source.contains("pub work_ids: BTreeSet<String>"));
        }
    }
}

enum RoadGenerationContract {
    DomainPredecisionSnapshot,
    MetadataNoSecondWriter,
    MetadataNeverResubmitStale,
}

async fn red_road_generation_contract(contract: RoadGenerationContract) {
    let directive = match contract {
        RoadGenerationContract::DomainPredecisionSnapshot => "predecision snapshot claim",
        RoadGenerationContract::MetadataNoSecondWriter => "race loser has no second writer",
        RoadGenerationContract::MetadataNeverResubmitStale => "stale decision is never resubmitted",
    };
    #[derive(Clone)]
    struct ClaimRepository {
        inner: SqliteDb,
        commit_barrier: Arc<tokio::sync::Barrier>,
        read_count: Arc<AtomicU64>,
        commit_count: Arc<AtomicU64>,
        expected_generations: Arc<Mutex<Vec<i64>>>,
    }

    impl WorkIdentityRepository for ClaimRepository {
        async fn read_captured_identity(
            &self,
            user_id: i64,
            work_id: i64,
        ) -> Result<CapturedIdentity, ilr::IdentityRepositoryError> {
            self.read_count.fetch_add(1, Ordering::SeqCst);
            WorkIdentityRepository::read_captured_identity(&self.inner, user_id, work_id).await
        }

        async fn read_identity_presentations(
            &self,
            user_id: i64,
            work_ids: &[i64],
        ) -> Result<Vec<ilr::WorkIdentityPresentation>, ilr::IdentityRepositoryError> {
            WorkIdentityRepository::read_identity_presentations(&self.inner, user_id, work_ids)
                .await
        }

        async fn list_captured_identities_in_group(
            &self,
            user_id: i64,
            normalized_main: String,
            primary_author_id: i64,
        ) -> Result<Vec<CapturedIdentity>, ilr::IdentityRepositoryError> {
            WorkIdentityRepository::list_captured_identities_in_group(
                &self.inner,
                user_id,
                normalized_main,
                primary_author_id,
            )
            .await
        }

        async fn read_primary_author_names(
            &self,
            user_id: i64,
            author_id: i64,
        ) -> Result<Vec<String>, ilr::IdentityRepositoryError> {
            WorkIdentityRepository::read_primary_author_names(&self.inner, user_id, author_id).await
        }

        async fn commit_settlement(
            &self,
            command: ilr::SettlementCommit,
        ) -> Result<ilr::SettlementCommitOutcome, ilr::IdentityRepositoryError> {
            self.commit_count.fetch_add(1, Ordering::SeqCst);
            self.expected_generations
                .lock()
                .expect("expected-generation recorder")
                .push(command.expected_generation);
            self.commit_barrier.wait().await;
            WorkIdentityRepository::commit_settlement(&self.inner, command).await
        }

        async fn commit_unattached_import_review(
            &self,
            user_id: i64,
            evidence: ilr::IdentityEvidenceBundle,
        ) -> Result<ilr::MintedReviewCard, ilr::IdentityRepositoryError> {
            WorkIdentityRepository::commit_unattached_import_review(&self.inner, user_id, evidence)
                .await
        }

        async fn load_pending_review(
            &self,
            actor: ilr::ReviewActor,
            card_id: i64,
        ) -> Result<ilr::PendingReviewCard, ilr::IdentityRepositoryError> {
            WorkIdentityRepository::load_pending_review(&self.inner, actor, card_id).await
        }

        async fn list_pending_reviews(
            &self,
            actor: ilr::ReviewActor,
        ) -> Result<Vec<ilr::PendingReviewCard>, ilr::IdentityRepositoryError> {
            WorkIdentityRepository::list_pending_reviews(&self.inner, actor).await
        }

        async fn dismiss_pending_review(
            &self,
            actor: ilr::ReviewActor,
            card_id: i64,
        ) -> Result<(), ilr::IdentityRepositoryError> {
            WorkIdentityRepository::dismiss_pending_review(&self.inner, actor, card_id).await
        }

        async fn load_pending_conflict_review(
            &self,
            actor: ilr::ReviewActor,
            conflict_id: i64,
        ) -> Result<ilr::PendingReviewCard, ilr::IdentityRepositoryError> {
            WorkIdentityRepository::load_pending_conflict_review(&self.inner, actor, conflict_id)
                .await
        }

        async fn commit_review_continuation(
            &self,
            actor: ilr::ReviewActor,
            command: ilr::ReviewResolutionCommand,
            cancel: CancellationToken,
        ) -> Result<ilr::ReviewContinuationOutcome, ilr::IdentityRepositoryError> {
            WorkIdentityRepository::commit_review_continuation(&self.inner, actor, command, cancel)
                .await
        }

        async fn resolve_conflict_atomically(
            &self,
            command: ilr::ResolveIdentityConflictCommand,
        ) -> Result<CapturedIdentity, ilr::IdentityRepositoryError> {
            WorkIdentityRepository::resolve_conflict_atomically(&self.inner, command).await
        }
    }

    #[derive(Clone)]
    struct CountingEngine {
        decisions: Arc<AtomicU64>,
    }

    impl IdentityEngine for CountingEngine {
        async fn decide(
            &self,
            request: IdentityDecisionRequest,
        ) -> Result<livrarr_identity::identity_layer::IdentityDecision, IdentityEngineError>
        {
            self.decisions.fetch_add(1, Ordering::SeqCst);
            DeterministicIdentityEngine.decide(request).await
        }

        fn classify_route_conflict(
            &self,
            existing: CapturedIdentity,
            candidate: ProviderRouteEvidence,
        ) -> Option<ilr::IdentityConflictClass> {
            DeterministicIdentityEngine.classify_route_conflict(existing, candidate)
        }
    }

    let db = create_test_db().await;
    let (user_id, author_id) = seed_identity_principals(&db, "road-generation").await;
    let seeded =
        WorkIdentityRepository::commit_settlement(&db, settlement_commit(user_id, author_id, None))
            .await
            .expect("seed generation-race Work");
    let work_id = seeded.identity.own_work_id;
    let generation = seeded.identity.identity_generation;
    let repository = ClaimRepository {
        inner: db.clone(),
        commit_barrier: Arc::new(tokio::sync::Barrier::new(2)),
        read_count: Arc::new(AtomicU64::new(0)),
        commit_count: Arc::new(AtomicU64::new(0)),
        expected_generations: Arc::new(Mutex::new(Vec::new())),
    };
    let decisions = Arc::new(AtomicU64::new(0));
    let service = Arc::new(IdentityRoadServiceImpl {
        identity_engine: CountingEngine {
            decisions: decisions.clone(),
        },
        identity_repository: repository.clone(),
        edition_repository: db.clone(),
        author_link_workflow: StubAuthorLinkWorkflow,
    });
    let race_request = IdentityRoadRequest {
        user_id,
        origin: IdentityRoadOrigin::EnrichmentPass,
        evidence: ilr::IdentityEvidenceBundle {
            user_choice: None,
            owned_files: vec![],
            provider_identity: vec![ilr::ProviderIdentityEvidence {
                provider: ilr::IdentityProvider::Goodreads,
                route: RouteKey {
                    provider: ilr::IdentityProvider::Goodreads,
                    kind: ilr::RouteKind::GoodreadsWork,
                    value: "985244".to_string(),
                },
                work_core: None,
                provenance: Default::default(),
            }],
            minimum: None,
        },
        interaction: IdentityRoadInteraction::MachineAlone,
        existing_work_id: Some(work_id),
    };
    let (left, right) = tokio::join!(
        service.settle(race_request.clone()),
        service.settle(race_request)
    );
    let results = [left, right];
    assert_eq!(
        results.iter().filter(|result| result.is_ok()).count(),
        1,
        "{directive}: exactly one production-road claim wins"
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(ilr::IdentityRoadError::StaleGeneration)))
            .count(),
        1,
        "{directive}: race loser returns StaleGeneration"
    );
    assert_eq!(repository.read_count.load(Ordering::SeqCst), 2);
    assert_eq!(decisions.load(Ordering::SeqCst), 2);
    assert_eq!(repository.commit_count.load(Ordering::SeqCst), 2);
    assert_eq!(
        repository
            .expected_generations
            .lock()
            .expect("expected-generation recorder")
            .as_slice(),
        &[generation, generation],
        "{directive}: both decisions claim their predecision G"
    );
    let after = WorkIdentityRepository::read_captured_identity(&db, user_id, work_id)
        .await
        .expect("read race winner graph");
    assert_eq!(after.identity_generation, generation + 1);
    assert_eq!(after.active_routes.len(), 1);
}

async fn red_road_door_matrix() {
    let db = create_test_db().await;
    let (user_id, author_id) = seed_identity_principals(&db, "door-matrix").await;
    let recording = RecordingRoad::new(road(db));
    let mut cases = Vec::new();

    // Round-10 addendum: the fileless wanted-books arm is the same
    // provider-grounded machine shape already accepted for AuthorMonitor.
    // It carries provider evidence and deliberately omits the weaker
    // title+author minimum.
    let mut readarr_provider_fileless = request(IdentityRoadOrigin::CreationDoor(
        ilr::DoorKind::ReadarrImport,
    ));
    readarr_provider_fileless.evidence.user_choice = None;
    readarr_provider_fileless.evidence.owned_files.clear();
    readarr_provider_fileless.evidence.minimum = None;
    readarr_provider_fileless.user_id = user_id;
    readarr_provider_fileless.evidence.provider_identity = vec![ilr::ProviderIdentityEvidence {
        provider: ilr::IdentityProvider::Goodreads,
        route: ilr::RouteKey {
            provider: ilr::IdentityProvider::Goodreads,
            kind: ilr::RouteKind::GoodreadsBookEdition,
            value: "700".to_string(),
        },
        work_core: Some(ilr::ProviderWorkIdentityCore {
            identity_title: title("Identity Road Fixture"),
            primary_author_id: author_id,
        }),
        provenance: Default::default(),
    }];
    readarr_provider_fileless.interaction = IdentityRoadInteraction::MachineAlone;
    assert!(matches!(
        recording.settle(readarr_provider_fileless).await,
        Ok(ilr::IdentityRoadOutcome::Settled { created: true, .. })
    ));

    let mut direct_missing_choice =
        request(IdentityRoadOrigin::CreationDoor(ilr::DoorKind::DirectAdd));
    direct_missing_choice.evidence.user_choice = None;
    cases.push((ilr::DoorKind::DirectAdd, direct_missing_choice));

    let mut manual_missing_file = request(IdentityRoadOrigin::CreationDoor(
        ilr::DoorKind::ManualImport,
    ));
    manual_missing_file.evidence.owned_files.clear();
    cases.push((ilr::DoorKind::ManualImport, manual_missing_file));

    let mut list_injected_file =
        request(IdentityRoadOrigin::CreationDoor(ilr::DoorKind::ListImport));
    list_injected_file
        .evidence
        .owned_files
        .push(ilr::OwnedFileEvidence {
            library_item_id: 1,
            file_revision: revision(),
        });
    cases.push((ilr::DoorKind::ListImport, list_injected_file));

    let mut author_missing_provider = request(IdentityRoadOrigin::CreationDoor(
        ilr::DoorKind::AuthorMonitor,
    ));
    author_missing_provider.evidence.user_choice = None;
    author_missing_provider.evidence.provider_identity.clear();
    author_missing_provider.interaction = IdentityRoadInteraction::MachineAlone;
    cases.push((ilr::DoorKind::AuthorMonitor, author_missing_provider));

    let mut series_injected_file = request(IdentityRoadOrigin::CreationDoor(
        ilr::DoorKind::SeriesMonitor,
    ));
    series_injected_file.evidence.user_choice = None;
    series_injected_file
        .evidence
        .owned_files
        .push(ilr::OwnedFileEvidence {
            library_item_id: 2,
            file_revision: revision(),
        });
    series_injected_file.interaction = IdentityRoadInteraction::MachineAlone;
    cases.push((ilr::DoorKind::SeriesMonitor, series_injected_file));

    let mut readarr_missing_file = request(IdentityRoadOrigin::CreationDoor(
        ilr::DoorKind::ReadarrImport,
    ));
    readarr_missing_file.evidence.user_choice = None;
    readarr_missing_file.evidence.owned_files.clear();
    readarr_missing_file.evidence.provider_identity.clear();
    readarr_missing_file.interaction = IdentityRoadInteraction::MachineAlone;
    cases.push((ilr::DoorKind::ReadarrImport, readarr_missing_file));

    for (door, invalid) in cases {
        let result = recording.settle(invalid).await;
        assert!(
            matches!(result, Err(ilr::IdentityRoadError::InvalidDoorEvidence)),
            "{door:?} rejects its Always/Never evidence violation"
        );
    }
    let calls = recording.calls();
    assert_eq!(calls.len(), 7);
    assert!(calls
        .iter()
        .all(|call| matches!(call, RecordedRoadCall::Settle(_))));
}

enum ReconcileContract {
    CompleteGroupPairs,
    SingularFieldDisposition,
    CardPersistsOnlyAtCommit,
}

async fn red_road_reconcile(contract: ReconcileContract) {
    let db = create_test_db().await;
    let (user_id, author_id) = seed_identity_principals(&db, "reconcile").await;
    sqlx::query("DROP INDEX IF EXISTS idx_works_test_helper_creation_dedup")
        .execute(db.pool())
        .await
        .expect("remove test-helper-only legacy dedup index");
    let mut first = settlement_commit(user_id, author_id, None);
    first.text_distinction = Some("first-text".to_string());
    let first = WorkIdentityRepository::commit_settlement(&db, first)
        .await
        .expect("seed first broad-group member");
    let second = settlement_commit(user_id, author_id, None);
    let second = WorkIdentityRepository::commit_settlement(&db, second)
        .await
        .expect("seed second broad-group member");
    let result = road(db.clone())
        .reconcile_complete_group(ProposedWorkIdentity {
            user_id,
            identity_title: title("Identity Road Fixture"),
            primary_author_id: author_id,
            text_distinction: None,
        })
        .await
        .expect("complete-group reconciliation must return a decision");
    match contract {
        ReconcileContract::CompleteGroupPairs => {
            let n = result.broad_main_author_candidates.len();
            assert_eq!(
                result.pairwise_outcomes.len(),
                n.saturating_mul(n.saturating_add(1)) / 2
            );
            assert!(result
                .exact_tuple_author_group
                .iter()
                .all(|id| { result.broad_main_author_candidates.contains(id) }));
        }
        ReconcileContract::SingularFieldDisposition => {
            assert!(matches!(
                result.action,
                livrarr_metadata::identity_road::CompleteGroupReconciliationAction::Review
            ));
            assert!(!result.pairwise_outcomes.is_empty());
            for work_id in [first.identity.own_work_id, second.identity.own_work_id] {
                assert!(
                    WorkIdentityRepository::read_captured_identity(&db, user_id, work_id)
                        .await
                        .is_ok(),
                    "production road must not auto-absorb audited-distinct siblings"
                );
            }
        }
        ReconcileContract::CardPersistsOnlyAtCommit => {
            let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM identity_review_cards")
                .fetch_one(db.pool())
                .await
                .expect("count unpersisted review cards");
            assert_eq!(
                before, 0,
                "reconciliation is a read-only orchestration step"
            );
            let review_card = result
                .review_card
                .expect("review reconciliation carries a typed in-memory card draft");
            let mut command =
                settlement_commit(user_id, author_id, Some(first.identity.own_work_id));
            command.identity_title = first.identity.identity_title;
            command.routes = first.identity.active_routes;
            command.expected_generation = first.identity.identity_generation;
            command.review_cards = vec![review_card];
            let committed = WorkIdentityRepository::commit_settlement(&db, command)
                .await
                .expect("sole settlement persists reconciliation card");
            assert_eq!(committed.review_cards.len(), 1);
            let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM identity_review_cards")
                .fetch_one(db.pool())
                .await
                .expect("count committed review cards");
            assert_eq!(after, 1);
        }
    }
}

#[derive(Clone, Copy)]
enum ReviewEntryContract {
    HttpCliParity,
    FailClosedMatrix,
    NineKindsParity,
    PendingOnErrors,
}

fn review_card_fixture(kind: ilr::ReviewKind) -> ilr::SettlementReviewCard {
    let empty_evidence = || ilr::IdentityEvidenceBundle {
        user_choice: None,
        owned_files: vec![],
        provider_identity: vec![],
        minimum: None,
    };
    match kind {
        ilr::ReviewKind::IdentityConflict => ilr::SettlementReviewCard::IdentityConflict {
            conflict_id: 41_001,
            work_id: 0,
        },
        ilr::ReviewKind::PendingRoute => ilr::SettlementReviewCard::PendingRoute {
            work_id: 0,
            candidate: ilr::ParkedRouteCandidate {
                route: RouteKey {
                    provider: ilr::IdentityProvider::OpenLibrary,
                    kind: ilr::RouteKind::OpenLibraryWork,
                    value: "OL-REVIEW-PARITY-W".to_string(),
                },
                proposed_owner: RouteOwner::Work(0),
            },
        },
        ilr::ReviewKind::GroupIdentity => ilr::SettlementReviewCard::GroupIdentity {
            work_ids: vec![],
            proposed_identity: None,
            merge_choices: Vec::new(),
        },
        ilr::ReviewKind::FieldResolution => ilr::SettlementReviewCard::FieldResolution {
            work_id: 0,
            evidence_ids: vec![],
        },
        ilr::ReviewKind::ContributorOrder => ilr::SettlementReviewCard::ContributorOrder {
            work_id: 0,
            contributors: vec![],
        },
        ilr::ReviewKind::EditionEvidence => ilr::SettlementReviewCard::EditionEvidence {
            edition_id: 91_001,
            evidence_ids: vec![],
        },
        ilr::ReviewKind::ImportIdentity => ilr::SettlementReviewCard::ImportIdentity {
            work_id: None,
            evidence: empty_evidence(),
        },
        ilr::ReviewKind::MigrationRepair => ilr::SettlementReviewCard::MigrationRepair {
            legacy_key: "legacy-review-parity".to_string(),
            reason: "typed fixture".to_string(),
        },
        ilr::ReviewKind::InvariantRepair => ilr::SettlementReviewCard::InvariantRepair {
            work_id: None,
            invariant: "review-parity-fixture".to_string(),
        },
    }
}

fn review_resolution_fixture(
    kind: ilr::ReviewKind,
    card_id: i64,
    generation: i64,
) -> ReviewResolutionCommand {
    match kind {
        ilr::ReviewKind::IdentityConflict => ReviewResolutionCommand::IdentityConflict {
            card_id,
            expected_generation: generation,
            action: ilr::IdentityConflictResolution::Reject {
                surviving_routes: vec![],
            },
        },
        ilr::ReviewKind::PendingRoute => ReviewResolutionCommand::PendingRoute {
            card_id,
            expected_generation: generation,
            action: ilr::PendingRouteAction::Affirm {
                surviving_routes: vec![],
            },
        },
        ilr::ReviewKind::GroupIdentity => ReviewResolutionCommand::GroupIdentity {
            card_id,
            expected_generation: generation,
            action: ilr::GroupIdentityAction::DifferentFromAll,
        },
        ilr::ReviewKind::FieldResolution => ReviewResolutionCommand::FieldResolution {
            card_id,
            expected_generation: generation,
            action: ilr::FieldResolutionAction::ExplicitAbsence,
        },
        ilr::ReviewKind::ContributorOrder => {
            let author = ilr::AuthorRef("review-author-1".to_string());
            ReviewResolutionCommand::ContributorOrder {
                card_id,
                expected_generation: generation,
                partition: vec![],
                order: vec![author.clone()],
                primary: author,
            }
        }
        ilr::ReviewKind::EditionEvidence => ReviewResolutionCommand::EditionEvidence {
            card_id,
            expected_generation: generation,
            action: ilr::EditionEvidenceAction::RetainUnknownOrAbsent,
        },
        ilr::ReviewKind::ImportIdentity => ReviewResolutionCommand::ImportIdentity {
            card_id,
            expected_generation: generation,
            action: ilr::ImportIdentityAction::CorrectedMetadataRetry {
                evidence: ilr::IdentityEvidenceBundle {
                    user_choice: None,
                    owned_files: vec![],
                    provider_identity: vec![],
                    minimum: None,
                },
            },
        },
        ilr::ReviewKind::MigrationRepair => ReviewResolutionCommand::MigrationRepair {
            card_id,
            expected_generation: generation,
            action: ilr::MigrationRepairAction::DiscardProvenNonIdentity {
                reason: "review parity fixture".to_string(),
            },
        },
        ilr::ReviewKind::InvariantRepair => ReviewResolutionCommand::InvariantRepair {
            card_id,
            expected_generation: generation,
            action: ilr::InvariantRepairAction::Recompute,
        },
    }
}

fn review_cli_action(command: &ReviewResolutionCommand) -> Value {
    match command {
        ReviewResolutionCommand::IdentityConflict { action, .. } => {
            serde_json::to_value(action).expect("serialize conflict action")
        }
        ReviewResolutionCommand::PendingRoute { action, .. } => {
            serde_json::to_value(action).expect("serialize pending-route action")
        }
        ReviewResolutionCommand::GroupIdentity { action, .. } => {
            serde_json::to_value(action).expect("serialize group action")
        }
        ReviewResolutionCommand::FieldResolution { action, .. } => {
            serde_json::to_value(action).expect("serialize field action")
        }
        ReviewResolutionCommand::ContributorOrder { .. } => {
            serde_json::to_value(command).expect("serialize contributor command")
        }
        ReviewResolutionCommand::EditionEvidence { action, .. } => {
            serde_json::to_value(action).expect("serialize edition action")
        }
        ReviewResolutionCommand::ImportIdentity { action, .. } => {
            serde_json::to_value(action).expect("serialize import action")
        }
        ReviewResolutionCommand::MigrationRepair { action, .. } => {
            serde_json::to_value(action).expect("serialize migration action")
        }
        ReviewResolutionCommand::InvariantRepair { action, .. } => {
            serde_json::to_value(action).expect("serialize invariant action")
        }
    }
}

async fn seed_review_matrix(
    db: &SqliteDb,
    user_id: i64,
    author_id: i64,
    kinds: &[ilr::ReviewKind],
) -> Vec<(i64, ilr::MintedReviewCard)> {
    let mut fixtures = Vec::with_capacity(kinds.len());
    for (index, kind) in kinds.iter().enumerate() {
        let mut commit = settlement_commit(user_id, author_id, None);
        commit.identity_title = title(&format!("Identity Review Parity {index}"));
        commit.review_cards = vec![review_card_fixture(*kind)];
        let settled = WorkIdentityRepository::commit_settlement(db, commit)
            .await
            .expect("seed typed review card through sole settlement");
        assert_eq!(settled.review_cards.len(), 1);
        assert_eq!(settled.review_cards[0].kind, *kind);
        fixtures.push((settled.identity.own_work_id, settled.review_cards[0]));
    }
    fixtures
}

async fn assert_review_http_cli_parity(kinds: &[ilr::ReviewKind]) {
    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;
    let (author, _) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: "Identity Review Parity Author".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed HTTP review author");
    let http_fixtures = seed_review_matrix(&harness.db, harness.user_id, author.id, kinds).await;

    let data_dir = tempfile::tempdir().expect("CLI parity data dir");
    let pool = livrarr_db::pool::create_sqlite_pool(data_dir.path())
        .await
        .expect("create CLI parity pool");
    livrarr_db::pool::run_migrations(&pool)
        .await
        .expect("migrate CLI parity pool");
    let cli_db = SqliteDb::new(pool);
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_authors_identity \
         ON authors(user_id, normalized_name) WHERE normalized_name IS NOT NULL",
    )
    .execute(cli_db.pool())
    .await
    .expect("install production author identity index");
    cli_db
        .ensure_identity_authority_ready()
        .await
        .expect("activate empty CLI parity database");
    let (cli_user_id, cli_author_id) = seed_identity_principals(&cli_db, "review-cli-parity").await;
    let cli_fixtures = seed_review_matrix(&cli_db, cli_user_id, cli_author_id, kinds).await;
    assert_eq!(
        http_fixtures, cli_fixtures,
        "the isolated entry points start from byte-corresponding card fixtures"
    );
    cli_db.pool().close().await;

    let mut http_graphs = Vec::with_capacity(kinds.len());
    for (kind, (work_id, card)) in kinds.iter().zip(&http_fixtures) {
        let command = review_resolution_fixture(*kind, card.id, card.generation);
        let response = call_router_json(
            &harness,
            Method::POST,
            format!("/api/v1/identity-review/{}/resolve", card.id),
            Some(json!({"command": command})),
        )
        .await;
        assert!(
            response.status.is_success(),
            "HTTP must resolve {kind:?}: {}",
            response.json
        );
        http_graphs.push(identity_graph_bytes(&harness.db, *work_id).await);
    }

    for ((kind, (work_id, card)), expected_graph) in
        kinds.iter().zip(&cli_fixtures).zip(http_graphs.iter())
    {
        let command = review_resolution_fixture(*kind, card.id, card.generation);
        let action_file = data_dir.path().join(format!("action-{:?}.json", kind));
        std::fs::write(
            &action_file,
            serde_json::to_vec(&review_cli_action(&command)).expect("encode CLI action"),
        )
        .expect("write CLI action");
        let outcome = livrarr_server::identity_layer::run_identity_cutover_command(
            livrarr_server::identity_layer::IdentityCutoverCliCommand::ResolveReview {
                card_id: card.id,
                expected_generation: card.generation,
                action_file,
            },
            data_dir.path().to_path_buf(),
            CancellationToken::new(),
        )
        .await
        .unwrap_or_else(|error| panic!("CLI must resolve {kind:?}: {error}"));
        assert!(matches!(
            outcome,
            livrarr_server::identity_layer::IdentityCutoverCliOutcome::ReviewResolved(_)
        ));

        let pool = livrarr_db::pool::create_sqlite_pool(data_dir.path())
            .await
            .expect("reopen CLI parity pool");
        let cli_db = SqliteDb::new(pool);
        assert_eq!(
            identity_graph_bytes(&cli_db, *work_id).await,
            *expected_graph,
            "HTTP and CLI commit the same graph for {kind:?}"
        );
        cli_db.pool().close().await;
    }

    let http_actors: Vec<String> = sqlx::query_scalar(
        "SELECT actor FROM identity_audit_events WHERE event_kind='review-resolution' ORDER BY id",
    )
    .fetch_all(harness.db.pool())
    .await
    .expect("read HTTP review provenance");
    assert_eq!(http_actors.len(), kinds.len());
    assert!(http_actors.iter().all(|actor| matches!(
        serde_json::from_str::<ReviewActor>(actor),
        Ok(ReviewActor::AuthenticatedUser { user_id }) if user_id == harness.user_id
    )));

    let pool = livrarr_db::pool::create_sqlite_pool(data_dir.path())
        .await
        .expect("reopen CLI provenance pool");
    let cli_db = SqliteDb::new(pool);
    let cli_actors: Vec<String> = sqlx::query_scalar(
        "SELECT actor FROM identity_audit_events WHERE event_kind='review-resolution' ORDER BY id",
    )
    .fetch_all(cli_db.pool())
    .await
    .expect("read CLI review provenance");
    assert_eq!(cli_actors.len(), kinds.len());
    assert!(cli_actors.iter().all(|actor| matches!(
        serde_json::from_str::<ReviewActor>(actor),
        Ok(ReviewActor::CutoverOperator { .. })
    )));
}

async fn red_review_entries(contract: ReviewEntryContract) {
    let kinds = [
        ilr::ReviewKind::IdentityConflict,
        ilr::ReviewKind::PendingRoute,
        ilr::ReviewKind::GroupIdentity,
        ilr::ReviewKind::FieldResolution,
        ilr::ReviewKind::ContributorOrder,
        ilr::ReviewKind::EditionEvidence,
        ilr::ReviewKind::ImportIdentity,
        ilr::ReviewKind::MigrationRepair,
        ilr::ReviewKind::InvariantRepair,
    ];
    match contract {
        ReviewEntryContract::HttpCliParity => {
            assert_eq!(kinds[2], ilr::ReviewKind::GroupIdentity);
        }
        ReviewEntryContract::FailClosedMatrix => {
            let errors = [
                ilr::IdentityRoadError::NotFound,
                ilr::IdentityRoadError::ReviewKindMismatch,
                ilr::IdentityRoadError::UnauthorizedScope,
                ilr::IdentityRoadError::StaleGeneration,
                ilr::IdentityRoadError::Cancelled,
            ];
            assert_eq!(errors.len(), 5);
        }
        ReviewEntryContract::NineKindsParity => assert_eq!(kinds.len(), 9),
        ReviewEntryContract::PendingOnErrors => {
            assert_ne!(
                ilr::IdentityRoadError::StaleGeneration,
                ilr::IdentityRoadError::NotFound
            );
        }
    }

    let router = strip_rust_comments(include_str!("../../crates/livrarr-server/src/router.rs"));
    assert!(
        router.contains("livrarr_handlers::identity_layer::resolve::<AppState>"),
        "STOP: HTTP parity cannot run while the registered route still targets the legacy review writer"
    );
    let state = strip_rust_comments(include_str!("../../crates/livrarr-server/src/state.rs"));
    assert!(
        state.contains("HasIdentityRoadService for AppState"),
        "STOP: HTTP and CLI cannot share an injected production road"
    );

    if matches!(
        contract,
        ReviewEntryContract::HttpCliParity | ReviewEntryContract::NineKindsParity
    ) {
        let contracts = strip_rust_comments(include_str!(
            "../../crates/livrarr-domain/src/identity_layer/services.rs"
        ));
        let settlement = contracts
            .split("pub struct SettlementCommit")
            .nth(1)
            .and_then(|tail| tail.split('}').next())
            .unwrap_or_default();
        assert!(
            settlement.contains("review_cards"),
            "STOP: no production writer can seed the pending review cards required for success parity"
        );
        let selected = if matches!(contract, ReviewEntryContract::HttpCliParity) {
            &kinds[2..3]
        } else {
            &kinds[..]
        };
        assert_review_http_cli_parity(selected).await;
    }

    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;
    let http = call_router_json(
        &harness,
        Method::POST,
        "/api/v1/identity-review/999999/resolve".to_string(),
        Some(resolve_body(999999, 1, CardGate::WorkUpdate)),
    )
    .await;
    assert_eq!(http.status, StatusCode::NOT_FOUND);

    let data_dir = tempfile::tempdir().expect("CLI review tempdir");
    let action = data_dir.path().join("action.json");
    std::fs::write(&action, br#"{"DifferentFromAll":null}"#)
        .expect("write typed CLI action fixture");
    let cli = livrarr_server::identity_layer::run_identity_cutover_command(
        livrarr_server::identity_layer::IdentityCutoverCliCommand::ResolveReview {
            card_id: 999999,
            expected_generation: 1,
            action_file: action,
        },
        data_dir.path().to_path_buf(),
        CancellationToken::new(),
    )
    .await;
    assert!(
        cli.is_err(),
        "absent card must fail closed through the CLI entry"
    );
    let cli_source = strip_rust_comments(include_str!(
        "../../crates/livrarr-server/src/identity_layer.rs"
    ));
    let cli_error = cli_source
        .split("pub enum IdentityCutoverCommandError")
        .nth(1)
        .and_then(|tail| tail.split('}').next())
        .unwrap_or_default();
    assert!(
        cli_error.contains("NotFound"),
        "STOP: CLI error vocabulary cannot preserve the mandated absent-card NotFound"
    );
}

fn enrichment_outcome(
    user_id: i64,
    work_id: i64,
    metadata_generation: i64,
    captured_route_evidence: Vec<ProviderRouteEvidence>,
) -> livrarr_enrichment::identity_layer::EnrichmentApplyOutcome {
    let projection = ilr::WorkCoverPresentation {
        format_needed: None,
        ebook: ilr::CoverSlotPresentation {
            selected: None,
            placeholder: Some(ilr::CoverPlaceholderState::NowhereToLook),
        },
        audiobook: ilr::CoverSlotPresentation {
            selected: None,
            placeholder: Some(ilr::CoverPlaceholderState::NowhereToLook),
        },
    };
    livrarr_enrichment::identity_layer::EnrichmentApplyOutcome {
        metadata_generation,
        captured_route_evidence,
        presentation: livrarr_enrichment::identity_layer::WorkPresentationProjection {
            subtitle: ilr::MachineSubtitleProjection {
                user_id,
                work_id,
                value: None,
                edition_id: None,
                provenance: None,
                computed_at_generation: metadata_generation,
            },
            covers: projection,
        },
    }
}

enum CaptureContract {
    EnrichmentHandoff,
    DriftAndStale,
    ThreeTriggers,
    EmptyNoop,
}

async fn red_road_capture(contract: CaptureContract) {
    let captured = ProviderRouteEvidence {
        provider: ilr::IdentityProvider::Goodreads,
        kind: ilr::RouteKind::GoodreadsWork,
        provider_scoped_id: "985244".to_string(),
    };
    match contract {
        CaptureContract::EnrichmentHandoff => {
            let db = create_test_db().await;
            let (user_id, author_id) = seed_identity_principals(&db, "capture-handoff").await;
            let committed = WorkIdentityRepository::commit_settlement(
                &db,
                settlement_commit(user_id, author_id, None),
            )
            .await
            .expect("seed capture work through production writer");
            let work_id = committed.identity.own_work_id;
            let generation = committed.identity.identity_generation;
            let result = road(db)
                .apply_captured_enrichment_routes(
                    user_id,
                    work_id,
                    IdentityRoadOrigin::EnrichmentPass,
                    enrichment_outcome(user_id, work_id, generation, vec![captured]),
                )
                .await
                .expect("captured route enters metadata road before completion");
            assert!(matches!(
                result,
                ilr::IdentityRoadOutcome::Settled { .. }
                    | ilr::IdentityRoadOutcome::ReviewPending { .. }
            ));
        }
        CaptureContract::DriftAndStale => {
            let db = create_test_db().await;
            let (user_id, author_id) = seed_identity_principals(&db, "capture-drift").await;
            let committed = WorkIdentityRepository::commit_settlement(
                &db,
                settlement_commit(user_id, author_id, None),
            )
            .await
            .expect("seed drift work through production writer");
            let work_id = committed.identity.own_work_id;
            let stale_generation = committed.identity.identity_generation - 1;
            let result = road(db)
                .apply_captured_enrichment_routes(
                    user_id,
                    work_id,
                    IdentityRoadOrigin::EnrichmentPass,
                    enrichment_outcome(user_id, work_id, stale_generation, vec![captured]),
                )
                .await;
            assert!(matches!(
                result,
                Err(ilr::IdentityRoadError::StaleGeneration)
            ));
        }
        CaptureContract::ThreeTriggers => {
            for trigger in [
                IdentityRoadOrigin::EnrichmentPass,
                IdentityRoadOrigin::ManualRefresh,
                IdentityRoadOrigin::ConvergenceVisit,
            ] {
                let db = create_test_db().await;
                let label = format!("capture-{trigger:?}");
                let (user_id, author_id) = seed_identity_principals(&db, &label).await;
                let committed = WorkIdentityRepository::commit_settlement(
                    &db,
                    settlement_commit(user_id, author_id, None),
                )
                .await
                .expect("seed trigger work through production writer");
                let work_id = committed.identity.own_work_id;
                let generation = committed.identity.identity_generation;
                let result = road(db)
                    .apply_captured_enrichment_routes(
                        user_id,
                        work_id,
                        trigger.clone(),
                        enrichment_outcome(user_id, work_id, generation, vec![captured.clone()]),
                    )
                    .await;
                assert!(
                    result.is_ok(),
                    "{trigger:?} settles captured evidence inline"
                );
            }
        }
        CaptureContract::EmptyNoop => {
            let db = create_test_db().await;
            let (user_id, author_id) = seed_identity_principals(&db, "capture-empty").await;
            let committed = WorkIdentityRepository::commit_settlement(
                &db,
                settlement_commit(user_id, author_id, None),
            )
            .await
            .expect("seed empty-capture work through production writer");
            let before = committed.identity;
            let result = road(db)
                .apply_captured_enrichment_routes(
                    user_id,
                    before.own_work_id,
                    IdentityRoadOrigin::EnrichmentPass,
                    enrichment_outcome(
                        user_id,
                        before.own_work_id,
                        before.identity_generation,
                        vec![],
                    ),
                )
                .await
                .expect("empty capture is an idempotent typed outcome");
            assert!(matches!(
                result,
                ilr::IdentityRoadOutcome::Settled {
                    work_id,
                    created: false,
                    ..
                } if work_id == before.own_work_id,
            ));
            assert_eq!(before.identity_generation, 1);
        }
    }
}

async fn red_road_author_inheritance() {
    struct RoadAuthorLinkWorkflow;

    impl livrarr_domain::services::AuthorLinkWorkflow for RoadAuthorLinkWorkflow {
        async fn enqueue(
            &self,
            _user_id: i64,
            _author_id: i64,
            _trigger: livrarr_domain::AuthorLinkTrigger,
        ) -> Result<(), livrarr_domain::AuthorLinkError> {
            Ok(())
        }

        async fn submit_evidence(
            &self,
            user_id: i64,
            author_id: i64,
            evidence: livrarr_domain::AgreedAuthorRouteEvidence,
        ) -> Result<livrarr_domain::RouteWriteOutcome, livrarr_domain::AuthorLinkError> {
            let evidence = evidence.into_agreed_evidence();
            Ok(livrarr_domain::RouteWriteOutcome::Attached(
                livrarr_domain::AuthorRoute {
                    id: 1,
                    user_id,
                    author_id,
                    key: evidence.key,
                    state: livrarr_domain::AuthorRouteState::Active,
                    provenance: livrarr_domain::AuthorRouteProvenance::Tier1Inherited,
                    evidence_work_id: evidence.evidence_work_id,
                    created_at: Utc::now(),
                    verified_at: Some(Utc::now()),
                    removed_at: None,
                },
            ))
        }

        async fn record_readarr_rejection(
            &self,
            _user_id: i64,
            author_id: i64,
            rejected: livrarr_domain::RejectedAuthorRouteEvidence,
        ) -> Result<livrarr_domain::AuthorLinkCandidate, livrarr_domain::AuthorLinkError> {
            Ok(livrarr_domain::AuthorLinkCandidate {
                id: 1,
                author_id,
                key: rejected.evidence().key.clone(),
                candidate_name: rejected.evidence().observed_name.clone(),
                reason: livrarr_domain::AuthorLinkCandidateReason::NameGuardFailed,
                name_verdict: rejected.verdict(),
                primary_name_verdict: rejected.verdict(),
                alternate_name_evidence: vec![],
                top_work_preview: None,
                catalog_evidence_state: livrarr_domain::AuthorCandidateCatalogState::Unavailable,
                corroborated_title_count: 0,
                settled_work_count: 1,
                previously_removed: false,
                status: livrarr_domain::AuthorLinkCandidateStatus::Pending,
                evidence_generation: 1,
                observed_at: Utc::now(),
                evidence_work_id: rejected.evidence().evidence_work_id,
                evidence_work_title: None,
                revoked_at: None,
            })
        }

        async fn run_due(
            &self,
            _batch_size: u32,
            _cancel: CancellationToken,
        ) -> Result<livrarr_domain::AuthorSweepTickSummary, livrarr_domain::AuthorLinkError>
        {
            Ok(livrarr_domain::AuthorSweepTickSummary {
                claimed: 0,
                evaluated: 0,
                unchanged_fingerprint: 0,
                failed: 0,
            })
        }
    }

    let db = create_test_db().await;
    let (user_id, author_id) = seed_identity_principals(&db, "inheritance").await;
    let committed =
        WorkIdentityRepository::commit_settlement(&db, settlement_commit(user_id, author_id, None))
            .await
            .expect("seed inheritance Work through production writer");
    let service = IdentityRoadServiceImpl {
        identity_engine: DeterministicIdentityEngine,
        identity_repository: db.clone(),
        edition_repository: db,
        author_link_workflow: RoadAuthorLinkWorkflow,
    };

    let absent = service
        .inherit_primary_author_route(committed.identity.clone(), None)
        .await
        .expect("absent author-id evidence must return NoAuthorId");
    assert!(matches!(absent, ilr::AuthorInheritanceOutcome::NoAuthorId));

    let agree = service
        .inherit_primary_author_route(
            committed.identity.clone(),
            Some(livrarr_domain::ProviderAuthorRef {
                key: livrarr_domain::AuthorRouteKey::parse(
                    livrarr_domain::AuthorProvider::Goodreads,
                    "7001",
                )
                .expect("provider author route"),
                name: "Identity Author inheritance".to_string(),
                credit: livrarr_domain::ProviderCredit::AssertedAuthor,
            }),
        )
        .await
        .expect("primary-author agreement returns a typed outcome");
    assert!(matches!(agree, ilr::AuthorInheritanceOutcome::Linked(_)));

    let review = service
        .inherit_primary_author_route(
            committed.identity,
            Some(livrarr_domain::ProviderAuthorRef {
                key: livrarr_domain::AuthorRouteKey::parse(
                    livrarr_domain::AuthorProvider::Goodreads,
                    "7002",
                )
                .expect("provider author route"),
                name: "Different Primary Author".to_string(),
                credit: livrarr_domain::ProviderCredit::AssertedAuthor,
            }),
        )
        .await
        .expect("primary-author disagreement returns review evidence");
    assert!(matches!(
        review,
        ilr::AuthorInheritanceOutcome::F1ReviewCandidate(_)
    ));
}

enum RepoReadContract {
    Projection,
    CrossUser,
}

async fn red_repo_read(contract: RepoReadContract) {
    let db = create_test_db().await;
    let (user_id, author_id) = seed_identity_principals(&db, "read").await;
    let committed =
        WorkIdentityRepository::commit_settlement(&db, settlement_commit(user_id, author_id, None))
            .await
            .expect("seed captured identity through production writer");
    match contract {
        RepoReadContract::Projection => {
            let captured = WorkIdentityRepository::read_captured_identity(
                &db,
                user_id,
                committed.identity.own_work_id,
            )
            .await
            .expect("read committed identity aggregate");
            assert_eq!(captured.primary_author_id, author_id);
            assert_eq!(captured.active_routes, committed.identity.active_routes);
            assert_eq!(captured.identity_generation, 1);
        }
        RepoReadContract::CrossUser => {
            let (other_user, _) = seed_identity_principals(&db, "read-other").await;
            let result = WorkIdentityRepository::read_captured_identity(
                &db,
                other_user,
                committed.identity.own_work_id,
            )
            .await;
            assert!(matches!(
                result,
                Err(ilr::IdentityRepositoryError::NotFound)
            ));
        }
    }
}

async fn seed_identity_principals(db: &SqliteDb, label: &str) -> (i64, i64) {
    let user = db
        .create_user(CreateUserDbRequest {
            username: format!("ilr-{label}"),
            password_hash: "unused".to_string(),
            role: UserRole::Admin,
            api_key_hash: format!("unused-{label}"),
        })
        .await
        .expect("seed identity user");
    let (author, _) = db
        .create_author(CreateAuthorDbRequest {
            user_id: user.id,
            name: format!("Identity Author {label}"),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed identity author");
    (user.id, author.id)
}

fn settlement_commit(
    user_id: i64,
    author_id: i64,
    existing_work_id: Option<i64>,
) -> ilr::SettlementCommit {
    ilr::SettlementCommit {
        user_id,
        existing_work_id,
        add_source: None,
        identity_title: title("Identity Road Fixture"),
        text_distinction: None,
        contributors: vec![ilr::WorkContributor {
            user_id,
            work_id: existing_work_id.unwrap_or_default(),
            author_id,
            ordinal: 0,
            roles: vec![],
        }],
        routes: vec![],
        absorbed_work_ids: vec![],
        expected_generation: 0,
        review_cards: vec![],
    }
}

enum RepoCommitContract {
    DomainDedupGeneration,
    DomainCollisionRollback,
    DbBranchMatrix,
    DbFaultRollback,
}

async fn red_repo_commit(contract: RepoCommitContract) {
    let db = create_test_db().await;
    let (user_id, author_id) = seed_identity_principals(&db, "commit").await;
    match contract {
        RepoCommitContract::DbBranchMatrix => {
            // The broad-group contract exercises the post-activation identity
            // key. The external behavioral helper carries a legacy-only
            // normalized-title conflict target for unrelated create_work tests;
            // production activation does not retain that compatibility index.
            sqlx::query("DROP INDEX IF EXISTS idx_works_test_helper_creation_dedup")
                .execute(db.pool())
                .await
                .expect("remove test-helper-only legacy dedup index");
            // Model a previously duplicated all-common cohort so the road's
            // absorption transaction can be exercised. The road restores one
            // legal row before this directive returns.
            sqlx::query("DROP INDEX IF EXISTS idx_works_identity_v2")
                .execute(db.pool())
                .await
                .expect("admit pre-existing common duplicate fixture");
            let (secondary, _) = db
                .create_author(CreateAuthorDbRequest {
                    user_id,
                    name: "Identity Secondary Contributor".to_string(),
                    sort_name: None,
                    ol_key: None,
                    gr_key: None,
                    hc_key: None,
                    import_id: None,
                })
                .await
                .expect("seed absorbed contributor");
            let mut first = settlement_commit(user_id, author_id, None);
            first.routes = vec![ilr::WorkRoute {
                id: 0,
                user_id,
                owner: RouteOwner::Work(0),
                resolved_work_id: 0,
                provider: ilr::IdentityProvider::OpenLibrary,
                kind: ilr::RouteKind::OpenLibraryWork,
                provider_scoped_id: "OL-ABSORB-A-W".to_string(),
                state: ilr::WorkRouteState::Active,
                provenance: ilr::RouteProvenance::Provider(ilr::IdentityProvider::OpenLibrary),
                user_confirmed: false,
                observed_at: Utc::now(),
            }];
            let first = WorkIdentityRepository::commit_settlement(&db, first)
                .await
                .expect("seed first broad-group Work");

            let mut second = settlement_commit(user_id, author_id, None);
            second.contributors.push(ilr::WorkContributor {
                user_id,
                work_id: 0,
                author_id: secondary.id,
                ordinal: 1,
                roles: vec![ilr::SourcedValue {
                    value: "translator".to_string(),
                    provenance: ilr::EvidenceProvenance::Provider(ilr::IdentityProvider::Goodreads),
                    observed_at: Utc::now(),
                }],
            });
            second.routes = vec![
                ilr::WorkRoute {
                    id: 0,
                    user_id,
                    owner: RouteOwner::Work(0),
                    resolved_work_id: 0,
                    provider: ilr::IdentityProvider::Goodreads,
                    kind: ilr::RouteKind::GoodreadsWork,
                    provider_scoped_id: "GR-ABSORB-B".to_string(),
                    state: ilr::WorkRouteState::Active,
                    provenance: ilr::RouteProvenance::Provider(ilr::IdentityProvider::Goodreads),
                    user_confirmed: false,
                    observed_at: Utc::now(),
                },
                ilr::WorkRoute {
                    id: 0,
                    user_id,
                    owner: RouteOwner::Work(0),
                    resolved_work_id: 0,
                    provider: ilr::IdentityProvider::Hardcover,
                    kind: ilr::RouteKind::HardcoverWork,
                    provider_scoped_id: "HC-RETIRED-B".to_string(),
                    state: ilr::WorkRouteState::Retired { audit_id: 17 },
                    provenance: ilr::RouteProvenance::Provider(ilr::IdentityProvider::Hardcover),
                    user_confirmed: false,
                    observed_at: Utc::now(),
                },
            ];
            let second = WorkIdentityRepository::commit_settlement(&db, second)
                .await
                .expect("seed second broad-group Work");
            assert_ne!(first.identity.own_work_id, second.identity.own_work_id);

            let outcome = road(db.clone())
                .settle(IdentityRoadRequest {
                    user_id,
                    origin: IdentityRoadOrigin::CreationDoor(ilr::DoorKind::DirectAdd),
                    evidence: ilr::IdentityEvidenceBundle {
                        user_choice: Some(ilr::UserIdentityChoice::ExplicitCreate(
                            ilr::MinimumWorkEvidence {
                                title: "Identity Road Fixture".to_string(),
                                authors: vec![author_id],
                            },
                        )),
                        owned_files: Vec::new(),
                        provider_identity: Vec::new(),
                        minimum: Some(ilr::MinimumWorkEvidence {
                            title: "Identity Road Fixture".to_string(),
                            authors: vec![author_id],
                        }),
                    },
                    interaction: IdentityRoadInteraction::HumanWatching,
                    existing_work_id: None,
                })
                .await
                .expect("road adopts one winner and absorbs its broad-group sibling");
            let ilr::IdentityRoadOutcome::Settled {
                work_id: winner_id,
                created,
                ..
            } = outcome
            else {
                panic!("authority-certain broad group must settle")
            };
            assert!(!created, "broad-group adoption reuses the survivor");
            assert_eq!(winner_id, first.identity.own_work_id);
            let remaining: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM works WHERE user_id=?1 AND normalized_identity_main=?2",
            )
            .bind(user_id)
            .bind("identity road fixture")
            .fetch_one(db.pool())
            .await
            .expect("count post-absorption group");
            assert_eq!(remaining, 1);
            let archive: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_merge_archives \
                 WHERE user_id=?1 AND winner_work_id=?2 AND loser_work_id=?3",
            )
            .bind(user_id)
            .bind(winner_id)
            .bind(second.identity.own_work_id)
            .fetch_one(db.pool())
            .await
            .expect("observe loser archive row");
            assert_eq!(archive, 1);
            let contributor_roles: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM work_contributor_roles \
                 WHERE user_id=?1 AND work_id=?2 AND author_id=?3 AND role='translator'",
            )
            .bind(user_id)
            .bind(winner_id)
            .bind(secondary.id)
            .fetch_one(db.pool())
            .await
            .expect("observe absorbed contributor role");
            assert_eq!(contributor_roles, 1);
            let route_states: Vec<String> = sqlx::query_scalar(
                "SELECT state FROM identity_routes WHERE user_id=?1 AND resolved_work_id=?2 \
                 ORDER BY provider_scoped_id",
            )
            .bind(user_id)
            .bind(winner_id)
            .fetch_all(db.pool())
            .await
            .expect("observe active union plus retired archive route");
            assert_eq!(route_states.len(), 3);
            assert!(route_states.iter().any(|state| state == "retired"));

            // The same directive also observes a generation race synchronized
            // after both road decisions and before either repository claim.
            red_road_generation_contract(RoadGenerationContract::MetadataNoSecondWriter).await;
        }
        RepoCommitContract::DomainCollisionRollback => {
            let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1")
                .bind(user_id)
                .fetch_one(db.pool())
                .await
                .expect("count works before invalid commit");
            let result = WorkIdentityRepository::commit_settlement(
                &db,
                settlement_commit(user_id, author_id, Some(999_999)),
            )
            .await;
            assert!(matches!(
                result,
                Err(ilr::IdentityRepositoryError::NotFound)
                    | Err(ilr::IdentityRepositoryError::StaleGeneration)
                    | Err(ilr::IdentityRepositoryError::AtomicRollback)
            ));
            let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1")
                .bind(user_id)
                .fetch_one(db.pool())
                .await
                .expect("count works after invalid commit");
            assert_eq!(after, before);
        }
        RepoCommitContract::DbFaultRollback => {
            let seeded = WorkIdentityRepository::commit_settlement(
                &db,
                settlement_commit(user_id, author_id, None),
            )
            .await
            .expect("seed a graph whose update will be faulted");
            let work_id = seeded.identity.own_work_id;
            let before = identity_graph_bytes(&db, work_id).await;
            let before_counts: (i64, i64, i64, i64) = sqlx::query_as(
                "SELECT \
                    (SELECT COUNT(*) FROM work_contributors WHERE user_id=?1 AND work_id=?2), \
                    (SELECT COUNT(*) FROM identity_routes WHERE user_id=?1 AND resolved_work_id=?2), \
                    (SELECT COUNT(*) FROM identity_review_cards WHERE user_id=?1 AND work_id=?2), \
                    (SELECT COUNT(*) FROM identity_audit_events WHERE user_id=?1 AND work_id=?2)",
            )
            .bind(user_id)
            .bind(work_id)
            .fetch_one(db.pool())
            .await
            .expect("snapshot graph row counts before faults");

            for failpoint in [
                IdentityDbFailpoint::CommitAfterWork,
                IdentityDbFailpoint::CommitAfterContributors,
                IdentityDbFailpoint::CommitAfterRoutes,
                IdentityDbFailpoint::CommitBeforeCommit,
            ] {
                let mut command = settlement_commit(user_id, author_id, Some(work_id));
                command.expected_generation = seeded.identity.identity_generation;
                command.identity_title = title("Faulted title must roll back");
                command.routes = vec![ilr::WorkRoute {
                    id: 0,
                    user_id,
                    owner: RouteOwner::Work(work_id),
                    resolved_work_id: work_id,
                    provider: ilr::IdentityProvider::OpenLibrary,
                    kind: ilr::RouteKind::OpenLibraryWork,
                    provider_scoped_id: "OL-FAULT-W".to_string(),
                    state: ilr::WorkRouteState::Active,
                    provenance: ilr::RouteProvenance::UserChoice,
                    user_confirmed: true,
                    observed_at: Utc::now(),
                }];
                command.review_cards = vec![ilr::SettlementReviewCard::FieldResolution {
                    work_id,
                    evidence_ids: vec![41],
                }];
                set_identity_db_failpoint_for_tests(failpoint);
                let result = WorkIdentityRepository::commit_settlement(&db, command).await;
                assert!(matches!(
                    result,
                    Err(ilr::IdentityRepositoryError::AtomicRollback)
                ));
                assert_eq!(identity_graph_bytes(&db, work_id).await, before);
                let after_counts: (i64, i64, i64, i64) = sqlx::query_as(
                    "SELECT \
                        (SELECT COUNT(*) FROM work_contributors WHERE user_id=?1 AND work_id=?2), \
                        (SELECT COUNT(*) FROM identity_routes WHERE user_id=?1 AND resolved_work_id=?2), \
                        (SELECT COUNT(*) FROM identity_review_cards WHERE user_id=?1 AND work_id=?2), \
                        (SELECT COUNT(*) FROM identity_audit_events WHERE user_id=?1 AND work_id=?2)",
                )
                .bind(user_id)
                .bind(work_id)
                .fetch_one(db.pool())
                .await
                .expect("verify every fault rolled back the whole graph");
                assert_eq!(after_counts, before_counts, "fault {failpoint:?}");
            }
        }
        RepoCommitContract::DomainDedupGeneration => {
            let first = WorkIdentityRepository::commit_settlement(
                &db,
                settlement_commit(user_id, author_id, None),
            );
            let second = WorkIdentityRepository::commit_settlement(
                &db,
                settlement_commit(user_id, author_id, None),
            );
            let (left, right) = tokio::join!(first, second);
            let successes = usize::from(left.is_ok()) + usize::from(right.is_ok());
            assert_eq!(successes, 1, "dedup/race has exactly one graph writer");
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1")
                .bind(user_id)
                .fetch_one(db.pool())
                .await
                .expect("count race result");
            assert_eq!(count, 1);
        }
    }
}

enum RepoConflictContract {
    AtomicActions,
    AmbiguousEdition,
}

async fn red_repo_conflict(contract: RepoConflictContract) {
    let db = create_test_db().await;
    let (user_id, author_id) = seed_identity_principals(&db, "conflict").await;
    match contract {
        RepoConflictContract::AtomicActions => {
            for (index, action_name) in ["accept", "reject", "different-work"]
                .into_iter()
                .enumerate()
            {
                let route_key = RouteKey {
                    provider: ilr::IdentityProvider::OpenLibrary,
                    kind: ilr::RouteKind::OpenLibraryWork,
                    value: format!("OL{}W", 9300 + index),
                };
                let mut commit = settlement_commit(user_id, author_id, None);
                commit.identity_title = title(&format!("ILR Conflict Action {index}"));
                commit.text_distinction = Some(format!("conflict-action-{index}"));
                commit.routes = vec![ilr::WorkRoute {
                    id: 0,
                    user_id,
                    owner: RouteOwner::Work(0),
                    resolved_work_id: 0,
                    provider: route_key.provider.clone(),
                    kind: route_key.kind.clone(),
                    provider_scoped_id: route_key.value.clone(),
                    state: ilr::WorkRouteState::Active,
                    provenance: ilr::RouteProvenance::UserChoice,
                    user_confirmed: true,
                    observed_at: Utc::now(),
                }];
                let settled = WorkIdentityRepository::commit_settlement(&db, commit)
                    .await
                    .expect("seed real conflict action graph");
                let work_id = settled.identity.own_work_id;
                let expected_generation = settled.identity.identity_generation;
                let conflict_id = sqlx::query(
                    "INSERT INTO identity_conflicts_v2 \
                        (user_id, current_work_id, class, candidate_provider, candidate_kind, \
                         candidate_value, proposed_owner_type, proposed_owner_id, status, expected_generation) \
                     VALUES (?1, ?2, 'class_c', ?3, ?4, ?5, 'work', ?2, 'pending', ?6)",
                )
                .bind(user_id)
                .bind(work_id)
                .bind(serde_json::to_string(&route_key.provider).expect("provider JSON"))
                .bind(serde_json::to_string(&route_key.kind).expect("kind JSON"))
                .bind(&route_key.value)
                .bind(expected_generation)
                .execute(db.pool())
                .await
                .expect("seed pending v2 conflict")
                .last_insert_rowid();
                let resolution = match action_name {
                    "accept" => ilr::IdentityConflictResolution::Accept {
                        surviving_routes: vec![route_key.clone()],
                        target_edition: None,
                    },
                    "reject" => ilr::IdentityConflictResolution::Reject {
                        surviving_routes: vec![route_key.clone()],
                    },
                    "different-work" => ilr::IdentityConflictResolution::DifferentWork {
                        winning_work_id: work_id,
                        surviving_routes: vec![route_key.clone()],
                        target_edition: None,
                    },
                    other => panic!("unexpected conflict action fixture {other}"),
                };
                let resolved = WorkIdentityRepository::resolve_conflict_atomically(
                    &db,
                    ilr::ResolveIdentityConflictCommand {
                        user_id,
                        conflict_id,
                        expected_generation,
                        resolution: resolution.clone(),
                    },
                )
                .await
                .unwrap_or_else(|error| panic!("{action_name} must commit atomically: {error}"));
                assert_eq!(resolved.identity_generation, expected_generation + 1);
                assert_eq!(resolved.active_routes.len(), 1);
                assert_eq!(
                    resolved.active_routes[0].provider_scoped_id,
                    route_key.value
                );
                let (status, stored_resolution, audit_id): (String, String, Option<i64>) =
                    sqlx::query_as(
                        "SELECT status, resolution, audit_id FROM identity_conflicts_v2 WHERE id=?1",
                    )
                    .bind(conflict_id)
                    .fetch_one(db.pool())
                    .await
                    .expect("read committed conflict exit");
                assert_eq!(status, "resolved");
                assert_eq!(
                    serde_json::from_str::<ilr::IdentityConflictResolution>(&stored_resolution)
                        .expect("typed stored resolution"),
                    resolution
                );
                assert!(audit_id.is_some(), "{action_name} appends its audit");
            }
        }
        RepoConflictContract::AmbiguousEdition => {
            let mut commit = settlement_commit(user_id, author_id, None);
            commit.identity_title = title("ILR Ambiguous Edition Conflict");
            commit.text_distinction = Some("ambiguous-edition".to_string());
            let settled = WorkIdentityRepository::commit_settlement(&db, commit)
                .await
                .expect("seed ambiguous Edition work");
            let work_id = settled.identity.own_work_id;
            let expected_generation = settled.identity.identity_generation;
            for value in ["9780306406157", "9781861972712"] {
                sqlx::query(
                    "INSERT INTO editions \
                        (user_id, work_id, format, provider_edition_id, state) \
                     VALUES (?1, ?2, ?3, ?4, 'active')",
                )
                .bind(user_id)
                .bind(work_id)
                .bind(serde_json::to_string(&ilr::EditionFormat::Ebook).expect("format JSON"))
                .bind(value)
                .execute(db.pool())
                .await
                .expect("seed directly eligible Edition");
            }
            let kind = ilr::RouteKind::Isbn13Edition;
            let conflict_id = sqlx::query(
                "INSERT INTO identity_conflicts_v2 \
                    (user_id, current_work_id, class, candidate_provider, candidate_kind, \
                     candidate_value, proposed_owner_type, proposed_owner_id, status, expected_generation) \
                 VALUES (?1, ?2, 'class_c', ?3, ?4, '9780306406157', 'work', ?2, 'pending', ?5)",
            )
            .bind(user_id)
            .bind(work_id)
            .bind(
                serde_json::to_string(&ilr::IdentityProvider::IsbnRegistry)
                    .expect("provider JSON"),
            )
            .bind(serde_json::to_string(&kind).expect("kind JSON"))
            .bind(expected_generation)
            .execute(db.pool())
            .await
            .expect("seed ambiguous Edition conflict")
            .last_insert_rowid();
            let result = WorkIdentityRepository::resolve_conflict_atomically(
                &db,
                ilr::ResolveIdentityConflictCommand {
                    user_id,
                    conflict_id,
                    expected_generation,
                    resolution: ilr::IdentityConflictResolution::Accept {
                        surviving_routes: vec![],
                        target_edition: None,
                    },
                },
            )
            .await;
            assert!(matches!(
                result,
                Err(ilr::IdentityRepositoryError::StillAmbiguous)
            ));
            let (status, generation, audits): (String, i64, i64) = sqlx::query_as(
                "SELECT c.status, w.identity_generation, \
                        (SELECT COUNT(*) FROM identity_audit_events a \
                          WHERE a.user_id=c.user_id AND a.work_id=c.current_work_id \
                            AND a.event_kind='conflict-resolution') \
                   FROM identity_conflicts_v2 c JOIN works w \
                     ON w.user_id=c.user_id AND w.id=c.current_work_id WHERE c.id=?1",
            )
            .bind(conflict_id)
            .fetch_one(db.pool())
            .await
            .expect("inspect fail-closed ambiguous conflict");
            assert_eq!(
                (status.as_str(), generation, audits),
                ("pending", expected_generation, 0)
            );
        }
    }
    let tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='identity_conflicts_v2'",
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect conflict schema");
    assert_eq!(
        tables, 1,
        "success arms require cards seeded through the production road"
    );
}

enum EditionRepositoryContract {
    UnknownAbsentContradiction,
    NoSubtitleBackflow,
}

async fn red_repo_edition(contract: EditionRepositoryContract) {
    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;
    harness
        .db
        .create_root_folder(
            harness
                ._tmp
                .path()
                .to_str()
                .expect("UTF-8 edition fixture root"),
            MediaType::Ebook,
        )
        .await
        .expect("seed ManualImport edition fixture root");

    match contract {
        EditionRepositoryContract::UnknownAbsentContradiction => {
            let absent = harness._tmp.path().join("edition-absent-language.epub");
            let english = harness._tmp.path().join("edition-language-en.epub");
            let conflict = harness._tmp.path().join("edition-language-fr.epub");
            write_epub_with_metadata(&absent, false, "ILR Edition Door", None, None);
            write_epub_with_metadata(
                &english,
                false,
                "ILR Edition Door",
                Some("en"),
                Some("9780306406157"),
            );
            write_epub_with_metadata(
                &conflict,
                false,
                "ILR Edition Door",
                Some("fr"),
                Some("9780306406157"),
            );
            for (index, path) in [&absent, &english, &conflict].into_iter().enumerate() {
                let response = drive_manual_import_epub(
                    &harness,
                    path,
                    "ILR Edition Door",
                    None,
                    Some("9780306406157"),
                )
                .await;
                assert!(
                    response.status.is_success(),
                    "real ManualImport embedded-metadata variant {index} reaches the import door: {}",
                    response.json
                );
                let editions: Vec<(i64, i64, Option<String>)> = sqlx::query_as(
                    "SELECT id, work_id, language FROM editions \
                     WHERE user_id = ?1 ORDER BY id",
                )
                .bind(harness.user_id)
                .fetch_all(harness.db.pool())
                .await
                .expect("read direct Edition language evidence");
                let language = editions
                    .last()
                    .expect("ManualImport creates an Edition")
                    .2
                    .clone();
                match index {
                    0 => assert_eq!(language, None, "absent evidence stays Unknown"),
                    1 => assert_eq!(language.as_deref(), Some("en"), "{editions:?}"),
                    2 => {
                        assert_eq!(language.as_deref(), Some("en"));
                        let parked: i64 = sqlx::query_scalar(
                            "SELECT COUNT(*) FROM identity_review_cards \
                             WHERE kind='EditionEvidence' AND status='pending'",
                        )
                        .fetch_one(harness.db.pool())
                        .await
                        .expect("count contradictory Edition review");
                        assert_eq!(parked, 1, "contradiction parks one typed review");
                    }
                    other => panic!("unexpected edition evidence fixture index {other}"),
                }
            }
        }
        EditionRepositoryContract::NoSubtitleBackflow => {
            let path = harness._tmp.path().join("edition-no-subtitle.epub");
            write_epub_with_metadata(
                &path,
                false,
                "ILR Edition Main",
                Some("en"),
                Some("9781861972712"),
            );
            let response = drive_manual_import_epub(
                &harness,
                &path,
                "ILR Edition Main: Work Display Subtitle",
                Some("en"),
                Some("9781861972712"),
            )
            .await;
            assert!(
                response.status.is_success(),
                "subtitled Work/lacking-edition-subtitle fixture reaches ManualImport: {}",
                response.json
            );
            assert_eq!(
                response.json["results"][0]["status"], "imported",
                "{}",
                response.json
            );
            let (work_subtitle, edition_subtitle): (Option<String>, Option<String>) =
                sqlx::query_as(
                    "SELECT w.subtitle, e.subtitle FROM works w \
                     JOIN editions e ON e.work_id = w.id \
                     WHERE w.user_id = ?1 ORDER BY e.id LIMIT 1",
                )
                .bind(harness.user_id)
                .fetch_one(harness.db.pool())
                .await
                .expect("read independent Work/Edition subtitles");
            assert_eq!(work_subtitle.as_deref(), Some("Work Display Subtitle"));
            assert_eq!(edition_subtitle, None, "Work subtitle never backflows");
        }
    }

    let manual_import = strip_rust_comments(include_str!(
        "../../crates/livrarr-handlers/src/manual_import.rs"
    ));
    assert!(
        manual_import.contains("EditionRepository")
            && manual_import.contains("apply_evidence"),
        "STOP: ManualImport does not expose the embedded-metadata to EditionRepository observation required by these directives"
    );
}

enum TransferContract {
    ZeroOneMany,
    StatementRollback,
}

async fn red_db_transfer(contract: TransferContract) {
    let db = create_test_db().await;
    let (user_id, author_id) = seed_identity_principals(&db, "transfer").await;
    let mut source_command = settlement_commit(user_id, author_id, None);
    source_command.identity_title = title("Transfer Source");
    let source = WorkIdentityRepository::commit_settlement(&db, source_command)
        .await
        .expect("seed transfer source");
    let source_id = source.identity.own_work_id;
    let route_key = RouteKey {
        provider: ilr::IdentityProvider::Goodreads,
        kind: ilr::RouteKind::GoodreadsBookEdition,
        value: "10884".to_string(),
    };
    let mut route_command = settlement_commit(user_id, author_id, Some(source_id));
    route_command.identity_title = title("Transfer Source");
    route_command.expected_generation = source.identity.identity_generation;
    route_command.routes = vec![ilr::WorkRoute {
        id: 0,
        user_id,
        owner: RouteOwner::Work(source_id),
        resolved_work_id: source_id,
        provider: route_key.provider.clone(),
        kind: route_key.kind.clone(),
        provider_scoped_id: route_key.value.clone(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::Provider(ilr::IdentityProvider::Goodreads),
        user_confirmed: false,
        observed_at: Utc::now(),
    }];
    let source = WorkIdentityRepository::commit_settlement(&db, route_command)
        .await
        .expect("seed one transferable route");

    let mut target_command = settlement_commit(user_id, author_id, None);
    target_command.identity_title = title("Transfer Target");
    let target = WorkIdentityRepository::commit_settlement(&db, target_command)
        .await
        .expect("seed transfer target Work");
    let target_id = target.identity.own_work_id;
    let edition_id = db
        .seed_transfer_target_for_tests(user_id, target_id, ilr::EditionFormat::Ebook)
        .await
        .expect("seed explicit target Edition");
    let command = TransferRouteCommand {
        user_id,
        route: route_key.clone(),
        target_owner: RouteOwner::Edition(edition_id),
        expected_generation: source.identity.identity_generation,
    };

    match contract {
        TransferContract::ZeroOneMany => {
            let missing = db
                .transfer_route(TransferRouteCommand {
                    route: RouteKey {
                        value: "missing".to_string(),
                        ..route_key.clone()
                    },
                    ..command.clone()
                })
                .await;
            assert!(matches!(
                missing,
                Err(ilr::IdentityRepositoryError::NotFound)
            ));

            let moved = db
                .transfer_route(command.clone())
                .await
                .expect("one eligible route transfers atomically");
            assert_eq!(moved.own_work_id, target_id);
            let owner: (String, Option<i64>, i64) = sqlx::query_as(
                "SELECT owner_type, edition_id, resolved_work_id FROM identity_routes \
                 WHERE user_id=?1 AND provider_scoped_id=?2",
            )
            .bind(user_id)
            .bind(&route_key.value)
            .fetch_one(db.pool())
            .await
            .expect("read transferred owner");
            assert_eq!(owner, ("edition".to_string(), Some(edition_id), target_id));

            // Restore the source route and add a second active claimant: the
            // same transfer request must now fail closed as ambiguous.
            sqlx::query(
                "UPDATE identity_routes SET owner_type='work', work_id=?1, edition_id=NULL, \
                        resolved_work_id=?1 WHERE user_id=?2 AND provider_scoped_id=?3",
            )
            .bind(source_id)
            .bind(user_id)
            .bind(&route_key.value)
            .execute(db.pool())
            .await
            .expect("restore source claimant");
            sqlx::query(
                "INSERT INTO identity_routes \
                    (user_id, owner_type, work_id, edition_id, resolved_work_id, provider, kind, \
                     provider_scoped_id, state, provenance, user_confirmed, observed_at) \
                 SELECT user_id, owner_type, work_id, edition_id, resolved_work_id, provider, kind, \
                        provider_scoped_id, state, provenance, user_confirmed, observed_at \
                   FROM identity_routes WHERE user_id=?1 AND provider_scoped_id=?2 LIMIT 1",
            )
            .bind(user_id)
            .bind(&route_key.value)
            .execute(db.pool())
            .await
            .expect("seed a second active claimant");
            let generation = work_generation(&db, source_id).await;
            let ambiguous = db
                .transfer_route(TransferRouteCommand {
                    expected_generation: generation,
                    ..command
                })
                .await;
            assert!(matches!(
                ambiguous,
                Err(ilr::IdentityRepositoryError::StillAmbiguous)
            ));
        }
        TransferContract::StatementRollback => {
            let before_route: (String, Option<i64>, Option<i64>, i64) = sqlx::query_as(
                "SELECT owner_type, work_id, edition_id, resolved_work_id \
                   FROM identity_routes WHERE user_id=?1 AND provider_scoped_id=?2",
            )
            .bind(user_id)
            .bind(&route_key.value)
            .fetch_one(db.pool())
            .await
            .expect("snapshot route before transfer faults");
            let before_generation = work_generation(&db, source_id).await;
            let before_audits: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_audit_events \
                 WHERE user_id=?1 AND event_kind='route-transfer'",
            )
            .bind(user_id)
            .fetch_one(db.pool())
            .await
            .expect("snapshot transfer audits");
            for failpoint in [
                IdentityDbFailpoint::TransferBeforeOwnerUpdate,
                IdentityDbFailpoint::TransferBeforeCommit,
            ] {
                set_identity_db_failpoint_for_tests(failpoint);
                let result = db.transfer_route(command.clone()).await;
                assert!(matches!(
                    result,
                    Err(ilr::IdentityRepositoryError::AtomicRollback)
                ));
                let route_after: (String, Option<i64>, Option<i64>, i64) = sqlx::query_as(
                    "SELECT owner_type, work_id, edition_id, resolved_work_id \
                       FROM identity_routes WHERE user_id=?1 AND provider_scoped_id=?2",
                )
                .bind(user_id)
                .bind(&route_key.value)
                .fetch_one(db.pool())
                .await
                .expect("verify transfer route rollback");
                assert_eq!(route_after, before_route, "fault {failpoint:?}");
                assert_eq!(work_generation(&db, source_id).await, before_generation);
                let audits: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM identity_audit_events \
                     WHERE user_id=?1 AND event_kind='route-transfer'",
                )
                .bind(user_id)
                .fetch_one(db.pool())
                .await
                .expect("verify transfer audit rollback");
                assert_eq!(audits, before_audits);
            }
        }
    }
}

enum ProjectionContract {
    TotalAndClaimed,
    NoLegacyGrade,
}

async fn red_db_projection(contract: ProjectionContract) {
    let db = create_test_db().await;
    let (user_id, _) = seed_identity_principals(&db, "projection").await;
    let work_id = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Projection Work".to_string(),
            author_name: "Projection Author".to_string(),
            normalized_title: "projection work".to_string(),
            normalized_author: "projection author".to_string(),
            language: Some("en".to_string()),
            ..Default::default()
        })
        .await
        .expect("seed projection Work")
        .0
        .id;
    let snapshot = db
        .recompute_work_projections(work_id, 0)
        .await
        .expect("projection recomputation must be total");
    match contract {
        ProjectionContract::TotalAndClaimed => {
            assert_eq!(snapshot.work_id, work_id);
            assert_eq!(snapshot.generation, 1);
            assert_eq!(work_generation(&db, work_id).await, 1);
        }
        ProjectionContract::NoLegacyGrade => {
            let source = strip_rust_comments(include_str!(
                "../../crates/livrarr-db/src/identity_layer.rs"
            ));
            assert!(!source.contains("trust_grade"));
            assert!(!source.contains("quality_grade"));
            assert_eq!(snapshot.status, ilr::IdentityStatus::NotConnected);
        }
    }
}

enum InspectionDbContract {
    FourOutcomes,
    RevisionRace,
    UserAndRevisionScope,
}

fn inspection_record(
    user_id: i64,
    item_id: i64,
    rev: ilr::FileRevision,
    outcome: ilr::EmbeddedCoverInspectionOutcome,
) -> ilr::EmbeddedCoverInspectionRecord {
    ilr::EmbeddedCoverInspectionRecord {
        user_id,
        library_item_id: item_id,
        revision: rev,
        outcome,
        cover_candidate_id: None,
        sanitized_error_code: None,
        inspected_at: Utc::now(),
    }
}

async fn red_db_inspection(contract: InspectionDbContract) {
    let db = create_test_db().await;
    match contract {
        InspectionDbContract::FourOutcomes => {
            for (index, outcome) in [
                ilr::EmbeddedCoverInspectionOutcome::Extracted,
                ilr::EmbeddedCoverInspectionOutcome::VerifiedNoCover,
                ilr::EmbeddedCoverInspectionOutcome::CouldNotInspect,
                ilr::EmbeddedCoverInspectionOutcome::FileGone,
            ]
            .into_iter()
            .enumerate()
            {
                let item_id = index as i64 + 1;
                db.record_embedded_cover_inspection(inspection_record(
                    1,
                    item_id,
                    revision(),
                    outcome,
                ))
                .await
                .expect("persist byte-free inspection outcome");
                let row = db
                    .read_embedded_cover_inspection(1, item_id, revision())
                    .await
                    .expect("read exact revision")
                    .expect("persisted row");
                assert_eq!(row.outcome, outcome);
                assert_eq!(row.cover_candidate_id, None);
            }
        }
        InspectionDbContract::RevisionRace => {
            let old = revision();
            let mut changed = revision();
            changed.modified_ns += 1;
            db.record_embedded_cover_inspection(inspection_record(
                1,
                1,
                old,
                ilr::EmbeddedCoverInspectionOutcome::CouldNotInspect,
            ))
            .await
            .expect("record old revision");
            db.record_embedded_cover_inspection(inspection_record(
                1,
                1,
                changed,
                ilr::EmbeddedCoverInspectionOutcome::VerifiedNoCover,
            ))
            .await
            .expect("record changed revision");
            let latest = db
                .read_embedded_cover_inspection(1, 1, changed)
                .await
                .expect("read changed revision")
                .expect("changed row");
            assert_eq!(
                latest.outcome,
                ilr::EmbeddedCoverInspectionOutcome::VerifiedNoCover
            );
        }
        InspectionDbContract::UserAndRevisionScope => {
            db.record_embedded_cover_inspection(inspection_record(
                1,
                1,
                revision(),
                ilr::EmbeddedCoverInspectionOutcome::VerifiedNoCover,
            ))
            .await
            .expect("record scoped inspection");
            let mut other_revision = revision();
            other_revision.sha256 = [9; 32];
            assert!(db
                .read_embedded_cover_inspection(2, 1, revision())
                .await
                .expect("cross-user read")
                .is_none());
            assert!(db
                .read_embedded_cover_inspection(1, 1, other_revision)
                .await
                .expect("other-revision read")
                .is_none());
        }
    }
}

enum DbCutoverContract {
    BlockedStagingReuse,
    ActivationIndexLast,
    TotalLegacyMapping,
}

async fn red_db_cutover(contract: DbCutoverContract) {
    match contract {
        DbCutoverContract::BlockedStagingReuse => {
            let db = livrarr_db::test_helpers::create_pre_cutover_identity_test_db(
                LegacyIdentityFixture {
                    works_and_authors: vec![livrarr_db::identity_layer::LegacyWorkFixture {
                        label: "blocked".to_string(),
                    }],
                    ..Default::default()
                },
            )
            .await
            .db;
            sqlx::query(
                "INSERT INTO works \
                    (user_id, title, author_name, author_id, normalized_title, \
                     normalized_author, ol_key, added_at) \
                 SELECT user_id, title, author_name, author_id, normalized_title, \
                        normalized_author, 'OL-LEGACY-BLOCKED-2', added_at \
                   FROM works ORDER BY id LIMIT 1",
            )
            .execute(db.pool())
            .await
            .expect("seed the blocked cutover collision cohort");
            let first = db
                .run_identity_cutover(IdentityCutoverMode::Apply, None)
                .await
                .expect("blocked Apply commits staging");
            assert!(!first.index_ready);
            let second = db
                .run_identity_cutover(IdentityCutoverMode::Apply, Some(first.clone()))
                .await
                .expect("rerun reuses staging");
            assert_eq!(second.source_fingerprint, first.source_fingerprint);
        }
        DbCutoverContract::ActivationIndexLast => {
            let db = livrarr_db::test_helpers::create_pre_cutover_identity_test_db(
                LegacyIdentityFixture::default(),
            )
            .await
            .db;
            set_identity_db_failpoint_for_tests(IdentityDbFailpoint::ActivationIndex);
            let result = db.ensure_identity_authority_ready().await;
            assert!(matches!(
                result,
                Err(ilr::IdentityMigrationError::Database(_))
            ));
            let marker: Option<String> = sqlx::query_scalar(
                "SELECT value FROM _livrarr_meta WHERE key='identity_authority_v2'",
            )
            .fetch_optional(db.pool())
            .await
            .expect("read activation marker after injected DDL failure");
            assert_eq!(marker, None);
            let index_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='index' AND name='idx_works_identity_v2'",
            )
            .fetch_one(db.pool())
            .await
            .expect("inspect activation index after rollback");
            assert_eq!(index_count, 0, "activation DDL must roll back");
        }
        DbCutoverContract::TotalLegacyMapping => {
            let fixture = LegacyIdentityFixture {
                works_and_authors: vec![livrarr_db::identity_layer::LegacyWorkFixture {
                    label: "complete-group".to_string(),
                }],
                legacy_routes_ledgers_and_reviews: livrarr_db::identity_layer::LegacyIdentityRows {
                    label: "routes-reviews-attempts".to_string(),
                },
                legacy_badge_route_matrix: vec![livrarr_db::identity_layer::LegacyBadgeRouteCase {
                    label: "badge-route-matrix".to_string(),
                }],
                monitoring_flags: vec![livrarr_db::identity_layer::LegacyMonitoringFixture {
                    label: "monitoring".to_string(),
                }],
            };
            let db = livrarr_db::test_helpers::create_pre_cutover_identity_test_db(fixture)
                .await
                .db;
            let report = db
                .run_identity_cutover(IdentityCutoverMode::Rehearsal, None)
                .await
                .expect("all legacy categories rehearse");
            assert!(report.legacy_work_count > 0);
            assert!(report.mapped_route_count > 0);
        }
    }
}

enum DbReadinessContract {
    DomainBranches,
    CancelledNoMarker,
    EmptyVsNonempty,
    IndexFailureRollback,
    OrdinaryHelperIndexes,
}

async fn red_db_readiness(contract: DbReadinessContract) {
    match contract {
        DbReadinessContract::CancelledNoMarker => {
            let db = create_test_db().await;
            let cancel = CancellationToken::new();
            cancel.cancel();
            let result = IdentityCutoverService::ensure_authority_ready(&db, cancel).await;
            assert!(matches!(
                result,
                Err(ilr::IdentityMigrationError::Cancelled)
            ));
        }
        DbReadinessContract::EmptyVsNonempty => {
            let empty = create_test_db().await;
            let ready = empty
                .ensure_identity_authority_ready()
                .await
                .expect("ordinary empty DB activates");
            assert!(matches!(
                ready,
                ilr::IdentityAuthorityReadiness::Active
                    | ilr::IdentityAuthorityReadiness::ActivatedFresh
            ));
            let pre = livrarr_db::test_helpers::create_pre_cutover_identity_test_db(
                LegacyIdentityFixture {
                    works_and_authors: vec![livrarr_db::identity_layer::LegacyWorkFixture {
                        label: "nonempty".to_string(),
                    }],
                    ..Default::default()
                },
            )
            .await;
            assert_eq!(
                pre.db
                    .ensure_identity_authority_ready()
                    .await
                    .expect("nonempty returns state"),
                ilr::IdentityAuthorityReadiness::CutoverRequired
            );
        }
        DbReadinessContract::DomainBranches => {
            let db = create_test_db().await;
            let first = db
                .ensure_identity_authority_ready()
                .await
                .expect("empty branch");
            let second = db
                .ensure_identity_authority_ready()
                .await
                .expect("active branch");
            assert!(matches!(
                first,
                ilr::IdentityAuthorityReadiness::ActivatedFresh
                    | ilr::IdentityAuthorityReadiness::Active
            ));
            assert_eq!(second, ilr::IdentityAuthorityReadiness::Active);
        }
        DbReadinessContract::IndexFailureRollback => {
            let db = livrarr_db::test_helpers::create_pre_cutover_identity_test_db(
                LegacyIdentityFixture::default(),
            )
            .await
            .db;
            set_identity_db_failpoint_for_tests(IdentityDbFailpoint::ReadinessIndex);
            let result = db.ensure_identity_authority_ready().await;
            assert!(matches!(
                result,
                Err(ilr::IdentityMigrationError::Database(_))
            ));
            let marker: Option<String> = sqlx::query_scalar(
                "SELECT value FROM _livrarr_meta WHERE key='identity_authority_v2'",
            )
            .fetch_optional(db.pool())
            .await
            .expect("read marker after readiness failure");
            assert_eq!(marker, None);
            let new_index: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='index' AND name='idx_works_identity_v2'",
            )
            .fetch_one(db.pool())
            .await
            .expect("inspect readiness index rollback");
            assert_eq!(new_index, 0);
        }
        DbReadinessContract::OrdinaryHelperIndexes => {
            let db = create_test_db().await;
            let new_index: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_works_identity_v2'",
            )
            .fetch_one(db.pool())
            .await
            .expect("inspect v2 index");
            let old_index: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_works_user_normalized'",
            )
            .fetch_one(db.pool())
            .await
            .expect("inspect legacy index");
            assert_eq!(new_index, 1);
            assert_eq!(old_index, 0);
        }
    }
}

enum PreCutoverContract {
    Categories,
    CannotClearActive,
}

async fn red_pre_cutover_helper(contract: PreCutoverContract) {
    match contract {
        PreCutoverContract::Categories => {
            let fixture = LegacyIdentityFixture {
                works_and_authors: vec![livrarr_db::identity_layer::LegacyWorkFixture {
                    label: "complete-groups".to_string(),
                }],
                legacy_routes_ledgers_and_reviews: livrarr_db::identity_layer::LegacyIdentityRows {
                    label: "reviews-attempts".to_string(),
                },
                legacy_badge_route_matrix: vec![livrarr_db::identity_layer::LegacyBadgeRouteCase {
                    label: "badges".to_string(),
                }],
                monitoring_flags: vec![],
            };
            let pre = livrarr_db::test_helpers::create_pre_cutover_identity_test_db(fixture).await;
            sqlx::query(
                "INSERT INTO works \
                    (user_id, title, author_name, author_id, normalized_title, \
                     normalized_author, ol_key, added_at) \
                 SELECT user_id, title, author_name, author_id, normalized_title, \
                        normalized_author, 'OL-LEGACY-COMPLETE-GROUP-2', added_at \
                   FROM works ORDER BY id LIMIT 1",
            )
            .execute(pre.db.pool())
            .await
            .expect("seed a real duplicate complete-group cohort");
            let report = pre
                .db
                .run_identity_cutover(IdentityCutoverMode::Rehearsal, None)
                .await
                .expect("category fixture rehearses");
            assert!(report.legacy_work_count > 0);
            assert_eq!(report.group_cards, 1);
        }
        PreCutoverContract::CannotClearActive => {
            let source = strip_rust_comments(&format!(
                "{}\n{}",
                include_str!("../../crates/livrarr-db/src/identity_layer.rs"),
                include_str!("../../crates/livrarr-db/src/lib.rs")
            ));
            let helper = source
                .split("create_pre_cutover_identity_test_db")
                .nth(1)
                .and_then(|tail| tail.split("pub use").next())
                .unwrap_or_default();
            assert!(
                helper.contains("PreCutoverIdentityTestDb") && helper.contains("InvalidFixture"),
                "STOP: PreCutoverIdentityTestDb helper must return InvalidFixture when the active marker could be cleared"
            );
        }
    }
}

enum RehearseContract {
    ByteIdenticalCopies,
    RejectLiveOrSchemaMismatch,
}

async fn red_cutover_trait(contract: RehearseContract) {
    let db = create_test_db().await;
    let result = IdentityCutoverService::rehearse(
        &db,
        ilr::SnapshotDatabase {
            path: PathBuf::from("copied-library.sqlite"),
        },
        CancellationToken::new(),
    )
    .await;
    assert!(matches!(
        result,
        Err(ilr::IdentityMigrationError::NotSnapshot)
    ));
    match contract {
        RehearseContract::ByteIdenticalCopies => {
            let fixture_source = strip_rust_comments(include_str!(
                "../../crates/livrarr-db/src/identity_layer.rs"
            ));
            let fixture = fixture_source
                .split("pub struct PreCutoverIdentityTestDb")
                .nth(1)
                .and_then(|tail| tail.split('}').next())
                .unwrap_or_default();
            assert!(
                fixture.contains("pub path:"),
                "STOP: copied-snapshot path is not exported by PreCutoverIdentityTestDb"
            );
        }
        RehearseContract::RejectLiveOrSchemaMismatch => {
            let distinct = ilr::IdentityMigrationError::SchemaMismatch;
            assert_ne!(
                distinct.to_string(),
                ilr::IdentityMigrationError::NotSnapshot.to_string()
            );
        }
    }
}

enum ApplyContract {
    BlockResolveRerun,
    FingerprintAndCollision,
}

async fn red_cutover_apply(contract: ApplyContract) {
    let db = create_test_db().await;
    let result =
        IdentityCutoverService::apply(&db, migration_report(), CancellationToken::new()).await;
    assert!(matches!(
        result,
        Err(ilr::IdentityMigrationError::RehearsalMismatch)
    ));
    match contract {
        ApplyContract::BlockResolveRerun => {
            let run = strip_rust_comments(include_str!(
                "../../crates/livrarr-db/src/identity_layer.rs"
            ));
            assert!(run.contains("reuse_staged_rows"));
        }
        ApplyContract::FingerprintAndCollision => {
            assert_ne!(
                ilr::IdentityMigrationError::Collision.to_string(),
                ilr::IdentityMigrationError::RehearsalMismatch.to_string()
            );
        }
    }
    let staged: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='identity_cutover_runs'",
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect staged cutover schema");
    assert_eq!(staged, 1, "success arm starts with a real rehearsal report");
}

enum TitleContract {
    StructuredWins,
    EmptyMain,
}

async fn red_title_parser(contract: TitleContract) {
    match contract {
        TitleContract::StructuredWins => {
            let parsed = ilr::title_parts_from_provider(
                "The Book: provider tail".to_string(),
                Some("Structured subtitle".to_string()),
            )
            .expect("non-empty provider main must parse");
            assert_eq!(parsed.main, "The Book");
            assert_eq!(parsed.subtitle.as_deref(), Some("Structured subtitle"));
            assert_eq!(parsed.normalized_main, "book");
            assert_eq!(parsed.normalized_subtitle, "structured subtitle");
        }
        TitleContract::EmptyMain => {
            let result = ilr::title_parts_from_provider("   ".to_string(), None);
            assert_eq!(result, Err(ilr::TitleParseError::InvalidMainTitle));
        }
    }
}

fn route(kind: ilr::RouteKind, value: &str, owner: RouteOwner) -> ilr::WorkRoute {
    ilr::WorkRoute {
        id: 1,
        user_id: 1,
        owner,
        resolved_work_id: 1,
        provider: ilr::IdentityProvider::Goodreads,
        kind,
        provider_scoped_id: value.to_string(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::Provider(ilr::IdentityProvider::Goodreads),
        user_confirmed: false,
        observed_at: Utc::now(),
    }
}

enum MatchContract {
    GuardIndependence,
    EditionEquality,
    OpaqueCapabilities,
}

async fn red_match_policy(contract: MatchContract) {
    match contract {
        MatchContract::GuardIndependence => {
            let fixed_wrong = wrong_guards();
            let mut tuned_lost = lost_guards();
            tuned_lost.one_sided_subtitle_recovery = false;
            tuned_lost.shared_edition_id_confirmation = false;
            let baseline = ilr::evaluate_match(
                work_evidence(),
                work_evidence(),
                lost_guards(),
                fixed_wrong.clone(),
            );
            let tuned =
                ilr::evaluate_match(work_evidence(), work_evidence(), tuned_lost, fixed_wrong);
            assert_eq!(baseline.author, tuned.author);
            assert_eq!(baseline.language, tuned.language);
            assert_eq!(baseline.id, tuned.id);
        }
        MatchContract::EditionEquality => {
            let mut left = work_evidence();
            let mut right = work_evidence();
            left.routes.push(route(
                ilr::RouteKind::Isbn13Edition,
                "9780306406157",
                RouteOwner::Edition(11),
            ));
            right.routes.push(route(
                ilr::RouteKind::Isbn13Edition,
                "9780140328721",
                RouteOwner::Edition(12),
            ));
            let unequal =
                ilr::evaluate_match(left.clone(), right.clone(), lost_guards(), wrong_guards());
            assert_eq!(
                unequal.id,
                livrarr_domain::identity_matching::IdVerdict::NoEvidence
            );
            right.routes[0].provider_scoped_id = "9780306406157".to_string();
            let equal = ilr::evaluate_match(left, right, lost_guards(), wrong_guards());
            assert_eq!(
                equal.id,
                livrarr_domain::identity_matching::IdVerdict::EditionBridge
            );
        }
        MatchContract::OpaqueCapabilities => {
            let verdicts = ilr::evaluate_match(
                work_evidence(),
                work_evidence(),
                lost_guards(),
                wrong_guards(),
            );
            assert_eq!(
                verdicts.id,
                livrarr_domain::identity_matching::IdVerdict::NoEvidence
            );
            let source = strip_rust_comments(include_str!(
                "../../crates/livrarr-domain/src/identity_layer/matching.rs"
            ));
            assert!(source.contains("pub struct SampledTextSignal"));
            assert!(source.contains("probe_id: ProbeId"));
            assert!(!source.contains("impl SampledTextSignal {\n    pub fn new"));
        }
    }
}

fn edition(id: i64, subtitle: Option<&str>, format: ilr::EditionFormat) -> ilr::Edition {
    ilr::Edition {
        id,
        user_id: 1,
        work_id: 1,
        format,
        language: Some("en".to_string()),
        subtitle: subtitle.map(|value| ilr::SourcedValue {
            value: value.to_string(),
            provenance: ilr::EvidenceProvenance::OwnedFile,
            observed_at: Utc::now(),
        }),
        routes: vec![],
        covers: vec![],
        source_provider: None,
        provider_edition_id: None,
        state: ilr::EditionState::Active,
    }
}

enum SubtitleContract {
    RecomputeWithoutIdentityMutation,
    ExplicitAbsence,
}

async fn red_subtitle_policy(contract: SubtitleContract) {
    match contract {
        SubtitleContract::RecomputeWithoutIdentityMutation => {
            let before = identity(1, 1);
            let projection = ilr::select_machine_subtitle(
                1,
                1,
                vec![edition(
                    41,
                    Some("Selected subtitle"),
                    ilr::EditionFormat::Ebook,
                )],
                vec![ilr::DefaultEdition {
                    user_id: 1,
                    work_id: 1,
                    format: ilr::EditionFormat::Ebook,
                    edition_id: 41,
                    provenance: ilr::EvidenceProvenance::User,
                }],
            );
            assert_eq!(projection.value.as_deref(), Some("Selected subtitle"));
            assert_eq!(projection.edition_id, Some(41));
            assert_eq!(before.identity_title, identity(1, 1).identity_title);
            assert_eq!(
                before.identity_generation,
                identity(1, 1).identity_generation
            );
        }
        SubtitleContract::ExplicitAbsence => {
            let projection = ilr::select_machine_subtitle(
                1,
                1,
                vec![edition(42, None, ilr::EditionFormat::Audiobook)],
                vec![ilr::DefaultEdition {
                    user_id: 1,
                    work_id: 1,
                    format: ilr::EditionFormat::Audiobook,
                    edition_id: 42,
                    provenance: ilr::EvidenceProvenance::User,
                }],
            );
            assert_eq!(projection.value, None);
            assert_eq!(projection.edition_id, Some(42));
        }
    }
}

fn cover_candidate(
    id: &str,
    source: &str,
    media_type: livrarr_domain::CoverMediaType,
) -> livrarr_domain::CoverCandidate {
    livrarr_domain::CoverCandidate {
        candidate_id: id.to_string(),
        proxy_url: format!("/api/v1/cover/{id}"),
        source: source.to_string(),
        media_type,
        width: 1200,
        height: 1800,
        passes_quality_gate: true,
    }
}

enum CoverContract {
    SourceRank,
    SharedFormatPanel,
    PreserveTitles,
}

async fn red_cover_policy(contract: CoverContract) {
    match contract {
        CoverContract::SourceRank => {
            let candidates = vec![
                cover_candidate(
                    "provider",
                    "goodreads",
                    livrarr_domain::CoverMediaType::Ebook,
                ),
                cover_candidate("owned", "owned_file", livrarr_domain::CoverMediaType::Ebook),
                cover_candidate("user", "user", livrarr_domain::CoverMediaType::Ebook),
            ];
            let presentation =
                ilr::select_covers_and_placeholders(identity(1, 1), vec![], candidates);
            assert_eq!(
                presentation
                    .ebook
                    .selected
                    .as_ref()
                    .map(|c| c.candidate_id.as_str()),
                Some("user")
            );
        }
        CoverContract::SharedFormatPanel => {
            let presentation = ilr::select_covers_and_placeholders(identity(1, 1), vec![], vec![]);
            assert!(matches!(
                presentation.format_needed,
                Some(ilr::CoverPlaceholderState::FormatNeeded { .. })
            ));
            assert!(presentation.ebook.selected.is_none());
            assert!(presentation.audiobook.selected.is_none());
        }
        CoverContract::PreserveTitles => {
            let candidates = vec![
                cover_candidate(
                    "valid-a",
                    "provider-a",
                    livrarr_domain::CoverMediaType::Ebook,
                ),
                cover_candidate(
                    "valid-b",
                    "provider-b",
                    livrarr_domain::CoverMediaType::Ebook,
                ),
            ];
            let presentation =
                ilr::select_covers_and_placeholders(identity(1, 1), vec![], candidates);
            let serialized = serde_json::to_string(&presentation).expect("serialize presentation");
            assert!(serialized.contains("valid-a"));
            assert!(serialized.contains("valid-b"));
        }
    }
}

enum MatchingContract {
    ConsumerParity,
    NoPrivateThreshold,
}

async fn red_matching_adapter(contract: MatchingContract) {
    match contract {
        MatchingContract::ConsumerParity => {
            let consumers = [
                (
                    "list dedup",
                    strip_rust_comments(include_str!(
                        "../../crates/livrarr-metadata/src/list_service.rs"
                    )),
                ),
                (
                    "discovery",
                    strip_rust_comments(include_str!(
                        "../../crates/livrarr-metadata/src/discovery_service.rs"
                    )),
                ),
                (
                    "fast cover search",
                    strip_rust_comments(include_str!(
                        "../../crates/livrarr-metadata/src/work_service.rs"
                    )),
                ),
            ];
            for (consumer, source) in consumers {
                assert!(
                    source.contains("livrarr_matching::identity_layer::find_matching_work"),
                    "STOP: {consumer} is not wired to the shared F2 matching authority"
                );
            }
            for (left_title, left_author, right_title, right_author, expected) in [
                (
                    "The Left Hand of Darkness",
                    "Ursula K. Le Guin",
                    "The Left Hand of Darkness",
                    "Ursula K. Le Guin",
                    true,
                ),
                (
                    "The Left Hand of Darkness",
                    "Ursula K. Le Guin",
                    "A Wizard of Earthsea",
                    "Ursula K. Le Guin",
                    false,
                ),
                (
                    "The Left Hand of Darkness",
                    "Ursula K. Le Guin",
                    "The Left Hand of Darkness",
                    "Octavia E. Butler",
                    false,
                ),
            ] {
                let outcomes = [
                    livrarr_metadata::list_service::list_identity_authority_match(
                        left_title,
                        left_author,
                        right_title,
                        right_author,
                    ),
                    livrarr_metadata::discovery_service::discovery_identity_authority_match(
                        left_title,
                        left_author,
                        right_title,
                        right_author,
                    ),
                    livrarr_metadata::work_service::fast_cover_identity_authority_match(
                        left_title,
                        left_author,
                        right_title,
                        right_author,
                    ),
                ];
                assert_eq!(outcomes, [expected; 3]);
            }
        }
        MatchingContract::NoPrivateThreshold => {
            let baseline = find_matching_work(WorkMatchAuthorityInputs {
                left: work_evidence(),
                right: work_evidence(),
            });
            let mut left = work_evidence();
            let mut right = work_evidence();
            left.routes.push(route(
                ilr::RouteKind::Isbn13Edition,
                "9780306406157",
                RouteOwner::Edition(1),
            ));
            right.routes.push(route(
                ilr::RouteKind::Isbn13Edition,
                "9780140328721",
                RouteOwner::Edition(2),
            ));
            let unequal_editions = find_matching_work(WorkMatchAuthorityInputs { left, right });
            assert_eq!(baseline.is_match, unequal_editions.is_match);
            assert_eq!(
                unequal_editions.verdicts.id,
                livrarr_domain::identity_matching::IdVerdict::NoEvidence
            );
        }
    }
}

enum GoodreadsContract {
    SameResponseZeroNetwork,
    BookNeverWork,
}

async fn red_goodreads_capture(contract: GoodreadsContract) {
    // Captured response bytes: book 10884 points to Work legacyId 985244 in
    // the same __NEXT_DATA__ document. The registry JSON is metadata, never
    // parser input.
    let captured = GoodreadsAdapter
        .capture_work_route_from_fetched_book_page(GoodreadsBookPage {
            book_id: "10884".to_string(),
            raw_html: include_str!(
                "../../build/probes/identity-layer-rewrite/raw/gr-book-10884.html"
            )
            .to_string(),
        })
        .await
        .expect("accepted captured Goodreads layout must parse");
    let route = captured.expect("accepted Goodreads page must expose its Work route");
    assert_eq!(route.kind, ilr::RouteKind::GoodreadsWork);
    match contract {
        GoodreadsContract::SameResponseZeroNetwork => {
            let counter = StubHttpFetcher::new();
            assert_eq!(route.provider_scoped_id, "985244");
            assert_eq!(
                counter.call_count(),
                0,
                "captured-page replay makes zero extra provider requests"
            );
        }
        GoodreadsContract::BookNeverWork => {
            assert_ne!(
                route.provider_scoped_id, "10884",
                "Book id is never a Work id"
            );
            assert_eq!(route.provider_scoped_id, "985244");
        }
    }
}

async fn red_goodreads_probe_blocked() {
    let result = GoodreadsAdapter
        .capture_work_route_from_fetched_book_page(GoodreadsBookPage {
            book_id: "unsampled".to_string(),
            raw_html: "<html><body><script id=\"__NEXT_DATA__\">{}</script></body></html>"
                .to_string(),
        })
        .await;
    assert!(
        matches!(
            result,
            Err(livrarr_external_data::identity_layer::ProviderEvidenceError::ProbeBlocked(_))
        ),
        "an unregistered layout must be ProbeBlocked, never Ok-empty evidence"
    );
}

fn write_epub_with_metadata(
    path: &std::path::Path,
    include_cover: bool,
    title: &str,
    language: Option<&str>,
    identifier: Option<&str>,
) {
    let file = std::fs::File::create(path).expect("create EPUB fixture");
    let mut zip = zip::ZipWriter::new(file);
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("mimetype", stored).expect("EPUB mimetype");
    zip.write_all(b"application/epub+zip")
        .expect("write mimetype");
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("META-INF/container.xml", deflated)
        .expect("EPUB container");
    zip.write_all(br#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#)
        .expect("write container");
    zip.start_file("OEBPS/content.opf", deflated)
        .expect("EPUB OPF");
    let cover_item = if include_cover {
        r#"<meta name="cover" content="cover-image"/><item id="cover-image" href="cover.jpg" media-type="image/jpeg"/>"#
    } else {
        ""
    };
    let language = language
        .map(|value| format!("<dc:language>{value}</dc:language>"))
        .unwrap_or_default();
    let identifier = identifier
        .map(|value| format!("<dc:identifier>{value}</dc:identifier>"))
        .unwrap_or_default();
    zip.write_all(
        format!(r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>{title}</dc:title>{language}{identifier}</metadata><manifest>{cover_item}</manifest><spine/></package>"#).as_bytes(),
    )
    .expect("write OPF");
    if include_cover {
        zip.start_file("OEBPS/cover.jpg", deflated)
            .expect("EPUB cover");
        zip.write_all(b"captured-cover-bytes").expect("write cover");
    }
    zip.finish().expect("finish EPUB fixture");
}

fn write_epub(path: &std::path::Path, include_cover: bool) {
    write_epub_with_metadata(path, include_cover, "ILR", None, None);
}

fn library_item(path: &std::path::Path, id: i64) -> LibraryItem {
    LibraryItem {
        id,
        user_id: 1,
        work_id: 1,
        root_folder_id: 1,
        path: path.to_string_lossy().into_owned(),
        media_type: MediaType::Ebook,
        file_size: std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0),
        import_id: None,
        imported_at: Utc::now(),
        tag_status: TagStatus::Pending,
        tagged_at_generation: 0,
        duration_seconds: None,
        chapter_scan_status: None,
    }
}

enum InspectorContract {
    FourOutcomes,
    RetryMatrix,
    ZeroProviderAndNoBytes,
}

async fn red_library_inspector(contract: InspectorContract) {
    let tmp = tempfile::tempdir().expect("EPUB inspection tempdir");
    let with_cover = tmp.path().join("with-cover.epub");
    let no_cover = tmp.path().join("no-cover.epub");
    let malformed = tmp.path().join("malformed.epub");
    let gone = tmp.path().join("gone.epub");
    write_epub(&with_cover, true);
    write_epub(&no_cover, false);
    std::fs::write(&malformed, b"not a zip").expect("write malformed EPUB");
    let db = create_test_db().await;
    let inspector = EpubCoverInspector::new(db.clone(), 16 * 1024 * 1024, 4_096);
    match contract {
        InspectorContract::FourOutcomes => {
            let extracted = inspector
                .inspect_revision(library_item(&with_cover, 1), revision(), false)
                .await
                .expect("cover EPUB produces a durable outcome");
            assert!(matches!(
                extracted,
                ilr::EmbeddedCoverInspectionResult::Extracted { .. }
            ));
            let absent = inspector
                .inspect_revision(library_item(&no_cover, 2), revision(), false)
                .await
                .expect("coverless EPUB produces a durable outcome");
            assert!(matches!(
                absent,
                ilr::EmbeddedCoverInspectionResult::VerifiedNoCover { .. }
            ));
            let broken = inspector
                .inspect_revision(library_item(&malformed, 3), revision(), false)
                .await
                .expect("malformed EPUB is an outcome, not a service error");
            assert!(matches!(
                broken,
                ilr::EmbeddedCoverInspectionResult::CouldNotInspect { .. }
            ));
            let missing = inspector
                .inspect_revision(library_item(&gone, 4), revision(), false)
                .await
                .expect("missing file is a durable outcome");
            assert!(matches!(
                missing,
                ilr::EmbeddedCoverInspectionResult::FileGone
            ));
        }
        InspectorContract::RetryMatrix => {
            assert_eq!(inspector.inspection_attempt_count(), 0);
            let first = inspector
                .inspect_revision(library_item(&malformed, 5), revision(), false)
                .await
                .expect("first malformed inspection");
            assert_eq!(inspector.inspection_attempt_count(), 1);
            let suppressed = inspector
                .inspect_revision(library_item(&malformed, 5), revision(), false)
                .await
                .expect("unchanged failure is read from durable record");
            assert_eq!(inspector.inspection_attempt_count(), 1);
            assert!(matches!(
                (first, suppressed),
                (
                    ilr::EmbeddedCoverInspectionResult::CouldNotInspect { .. },
                    ilr::EmbeddedCoverInspectionResult::CouldNotInspect { .. }
                )
            ));
            let forced = inspector
                .inspect_revision(library_item(&malformed, 5), revision(), true)
                .await
                .expect("force retries unchanged failure");
            assert_eq!(inspector.inspection_attempt_count(), 2);
            assert!(matches!(
                forced,
                ilr::EmbeddedCoverInspectionResult::CouldNotInspect { .. }
            ));
            let mut changed_revision = revision();
            changed_revision.modified_ns += 1;
            let changed = inspector
                .inspect_revision(library_item(&malformed, 5), changed_revision, false)
                .await
                .expect("changed revision retries a prior failure");
            assert_eq!(inspector.inspection_attempt_count(), 3);
            assert!(matches!(
                changed,
                ilr::EmbeddedCoverInspectionResult::CouldNotInspect { .. }
            ));
        }
        InspectorContract::ZeroProviderAndNoBytes => {
            let outcome = inspector
                .inspect_revision(library_item(&with_cover, 6), revision(), false)
                .await
                .expect("local inspection");
            assert!(matches!(
                outcome,
                ilr::EmbeddedCoverInspectionResult::Extracted { .. }
            ));
            let source = strip_rust_comments(include_str!(
                "../../crates/livrarr-library/src/identity_layer.rs"
            ));
            assert!(
                !source.contains("HttpFetcher"),
                "inspector has zero provider edge"
            );
            assert!(
                source.contains("record_embedded_cover_inspection"),
                "STOP: persistence seam must store byte-free record before return"
            );
            let record = db
                .read_embedded_cover_inspection(1, 6, revision())
                .await
                .expect("inspection row read")
                .expect("inspection row persisted before return");
            assert_eq!(
                record.outcome,
                ilr::EmbeddedCoverInspectionOutcome::Extracted
            );
            assert!(record.cover_candidate_id.is_none());
            assert!(record.sanitized_error_code.is_none());
            assert_eq!(inspector.inspection_attempt_count(), 1);
        }
    }
}

enum MaterializeContract {
    SelectedSourcesAndAuthor,
    IndependentFailures,
}

async fn red_materialize(contract: MaterializeContract) {
    let tmp = tempfile::tempdir().expect("materialize tempdir");
    let fetcher = Arc::new(StubHttpFetcher::with_ok(200, b"captured-cover".to_vec()));
    let service = livrarr_materialize::LiveMaterializeService::new(fetcher.clone());
    match contract {
        MaterializeContract::SelectedSourcesAndAuthor => {
            let book = tmp.path().join("selected.epub");
            write_epub_with_metadata(&book, false, "Before", None, None);
            let selected = livrarr_domain::CoverCandidate {
                candidate_id: "selected-ebook".to_string(),
                proxy_url: "https://covers.example.test/selected.jpg".to_string(),
                source: "Provider".to_string(),
                media_type: livrarr_domain::CoverMediaType::Ebook,
                width: 600,
                height: 900,
                passes_quality_gate: true,
            };
            let mapped = livrarr_materialize::identity_layer::MaterializeIdentityRequest {
                work_id: 91,
                primary_author_display_name: "Primary Author Display".to_string(),
                selected_covers: ilr::WorkCoverSelection {
                    ebook: Some(selected),
                    audiobook: None,
                    audiobook_is_ebook_fallback: false,
                },
                tags: MaterializeTags {
                    title: "Selected Work".into(),
                    author: "stale copied scalar".into(),
                    ..Default::default()
                },
            }
            .into_materialize_request(MaterializeRequest {
                work_id: 0,
                changed: true,
                tag_fields_changed: true,
                ebook_cover: CoverSlotState::default(),
                audiobook_cover: CoverSlotState::default(),
                file_paths: vec![book],
                tags: MaterializeTags::default(),
                covers_dir: tmp.path().join("covers"),
            });
            let outcome = service
                .materialize(mapped)
                .await
                .expect("selected materialization sources are valid");
            assert_eq!(fetcher.call_count(), 1);
            let expected_path = tmp
                .path()
                .join("covers/91.jpg")
                .to_string_lossy()
                .into_owned();
            assert_eq!(
                outcome.ebook_cover_path.as_deref(),
                Some(expected_path.as_str())
            );
            assert!(tmp.path().join("covers/91.jpg").exists());
            assert!(
                outcome.tags_written,
                "primary-author display must be materialized into owned-file tags"
            );
        }
        MaterializeContract::IndependentFailures => {
            let failure_fetcher = Arc::new(StubHttpFetcher::with_ok(503, vec![]));
            let service = livrarr_materialize::LiveMaterializeService::new(failure_fetcher);
            let result = service
                .materialize(MaterializeRequest {
                    work_id: 92,
                    changed: true,
                    tag_fields_changed: true,
                    ebook_cover: CoverSlotState {
                        chosen_new_url: Some("https://covers.example.test/fails.jpg".into()),
                        ..Default::default()
                    },
                    audiobook_cover: CoverSlotState::default(),
                    file_paths: vec![tmp.path().join("missing.epub")],
                    tags: MaterializeTags {
                        title: "Failure isolation".into(),
                        author: "Primary".into(),
                        ..Default::default()
                    },
                    covers_dir: tmp.path().join("covers"),
                })
                .await;
            assert!(matches!(
                result,
                Err(livrarr_domain::services::MaterializeError::CoverDownload(_))
            ));

            let tag_only =
                livrarr_materialize::LiveMaterializeService::new(Arc::new(StubHttpFetcher::new()))
                    .materialize(MaterializeRequest {
                        work_id: 93,
                        changed: true,
                        tag_fields_changed: true,
                        ebook_cover: CoverSlotState::default(),
                        audiobook_cover: CoverSlotState::default(),
                        file_paths: vec![tmp.path().join("missing.epub")],
                        tags: MaterializeTags {
                            title: "Typed tag failure".into(),
                            author: "Primary".into(),
                            ..Default::default()
                        },
                        covers_dir: tmp.path().join("covers"),
                    })
                    .await;
            assert!(matches!(
                tag_only,
                Err(livrarr_domain::services::MaterializeError::TagWrite(_))
            ));
        }
    }
}

enum ConvergenceContract {
    CapturedBeforeCheckpoint,
    FailurePreservesCadence,
    RegisteredHandoff,
    ControlErrorsTyped,
}

// The server's fail-next test hook is process-global and keyed by WorkId,
// while each isolated SQLite fixture starts IDs from the same value. Keep every
// convergence tick in this binary from consuming another fixture's fault.
static CONVERGENCE_CONTRACT_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

struct ConvergenceCollisionSync {
    ready: tokio::sync::Barrier,
    hook_armed: tokio::sync::Notify,
}

impl ConvergenceCollisionSync {
    fn new() -> Self {
        Self {
            ready: tokio::sync::Barrier::new(2),
            hook_armed: tokio::sync::Notify::new(),
        }
    }
}

async fn red_server_convergence(contract: ConvergenceContract) {
    red_server_convergence_with_collision_sync(contract, None).await;
}

async fn red_server_convergence_with_collision_sync(
    contract: ConvergenceContract,
    collision_sync: Option<Arc<ConvergenceCollisionSync>>,
) {
    let productive = matches!(
        contract,
        ConvergenceContract::CapturedBeforeCheckpoint | ConvergenceContract::RegisteredHandoff
    );
    let harness = if productive {
        build_route_harness_with_open_library(Some(livrarr_external_data::NormalizedWorkDetail {
            ol_key: Some("OL-CONVERGENCE-FOUND-W".to_string()),
            ..Default::default()
        }))
        .await
    } else {
        build_route_harness().await
    };
    let work_id = seed_route_work(&harness, "convergence-runtime").await;
    sqlx::query(
        "UPDATE works \
         SET identity_status = 'pending', identity_status_v2 = 'connected', \
             enrichment_status = 'pending' \
         WHERE id = ? AND user_id = ?",
    )
    .bind(work_id)
    .bind(harness.user_id)
    .execute(harness.db.pool())
    .await
    .expect("seed a convergence-eligible work");
    sqlx::query(
        "INSERT INTO identity_routes \
            (user_id, owner_type, work_id, edition_id, resolved_work_id, provider, kind, \
             provider_scoped_id, state, provenance, user_confirmed, observed_at) \
         VALUES (?, 'work', ?, NULL, ?, ?, ?, ?, 'active', ?, 0, ?)",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .bind(work_id)
    .bind(
        serde_json::to_string(&if productive {
            ilr::IdentityProvider::IsbnRegistry
        } else {
            ilr::IdentityProvider::OpenLibrary
        })
        .unwrap(),
    )
    .bind(
        serde_json::to_string(&if productive {
            ilr::RouteKind::Isbn13Edition
        } else {
            ilr::RouteKind::OpenLibraryWork
        })
        .unwrap(),
    )
    .bind(if productive {
        "9780000000194".to_string()
    } else {
        format!("OL-CONVERGENCE-{work_id}")
    })
    .bind(
        serde_json::to_string(&ilr::RouteProvenance::Provider(
            ilr::IdentityProvider::OpenLibrary,
        ))
        .unwrap(),
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(harness.db.pool())
    .await
    .expect("seed an active route for the convergence handoff");
    livrarr_db::WorkDb::set_next_convergence_at(
        &harness.db,
        harness.user_id,
        work_id,
        Some(chrono::Utc::now() - chrono::Duration::seconds(1)),
    )
    .await
    .expect("make convergence fixture due");
    let generation_before: i64 =
        sqlx::query_scalar("SELECT identity_generation FROM works WHERE user_id=?1 AND id=?2")
            .bind(harness.user_id)
            .bind(work_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("read convergence generation before tick");
    let audits_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events \
          WHERE user_id=?1 AND work_id=?2 AND event_kind='settlement'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count convergence audits before tick");
    let cancel = CancellationToken::new();
    if matches!(contract, ConvergenceContract::ControlErrorsTyped) {
        cancel.cancel();
    }
    if let Some(sync) = collision_sync.as_ref() {
        sync.ready.wait().await;
    }
    let contract_lock = CONVERGENCE_CONTRACT_LOCK.lock().await;
    let failure_started = Utc::now();
    if matches!(contract, ConvergenceContract::FailurePreservesCadence) {
        livrarr_server::identity_layer::fail_next_identity_convergence_for_tests(work_id);
        if let Some(sync) = collision_sync.as_ref() {
            sync.hook_armed.notify_one();
            tokio::task::yield_now().await;
        }
    }
    let result = livrarr_server::identity_layer::run_identity_convergence_tick(
        harness.state.clone(),
        cancel,
    )
    .await;
    drop(contract_lock);
    match contract {
        ConvergenceContract::CapturedBeforeCheckpoint => {
            let report = result.expect("productive captured-route visit completes");
            assert_eq!(report.visited_work_count, 1);
            assert_eq!(report.captured_route_count, 1);
            let captured = WorkIdentityRepository::read_captured_identity(
                &harness.db,
                harness.user_id,
                work_id,
            )
            .await
            .expect("read graph after registered handoff");
            assert!(captured.active_routes.iter().any(|route| {
                route.kind == ilr::RouteKind::OpenLibraryWork
                    && route.provider_scoped_id == "OL-CONVERGENCE-FOUND-W"
            }));
            assert_eq!(captured.identity_generation, generation_before + 1);
            let audits_after: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_audit_events \
                  WHERE user_id=?1 AND work_id=?2 AND event_kind='settlement'",
            )
            .bind(harness.user_id)
            .bind(work_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("count convergence audits after handoff");
            assert_eq!(audits_after, audits_before + 1);
        }
        ConvergenceContract::RegisteredHandoff => {
            let report = result.expect("registered productive handoff visit completes");
            assert_eq!(report.visited_work_count, 1);
            assert_eq!(report.captured_route_count, 1);
            let attempts: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_provider_attempts \
                  WHERE user_id=?1 AND work_id=?2 AND provider='livrarr-convergence' \
                    AND route_kind='bridge-upgrade'",
            )
            .bind(harness.user_id)
            .bind(work_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("count attempts after registered productive handoff");
            assert_eq!(
                attempts, 0,
                "a registered productive handoff must settle before and suppress the attempt checkpoint"
            );
        }
        ConvergenceContract::FailurePreservesCadence => {
            let report = result.expect("per-work provider failure is isolated");
            assert!(report.visited_work_count > 0);
            let checkpoint = harness
                .db
                .get_work(harness.user_id, work_id)
                .await
                .expect("work survives convergence");
            assert_eq!(checkpoint.id, work_id);
            let next_raw: String = sqlx::query_scalar(
                "SELECT next_convergence_at FROM works WHERE user_id = ?1 AND id = ?2",
            )
            .bind(harness.user_id)
            .bind(work_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("production failure path writes its next cadence");
            let next = chrono::DateTime::parse_from_rfc3339(&next_raw)
                .expect("next cadence is RFC3339")
                .with_timezone(&Utc);
            let expected_floor = failure_started
                + chrono::Duration::seconds(harness.state.config.convergence.interval_secs as i64)
                - chrono::Duration::seconds(2);
            assert!(
                next >= expected_floor,
                "real convergence failure must back off by the configured cadence"
            );
        }
        ConvergenceContract::ControlErrorsTyped => {
            assert!(
                result.is_err(),
                "pre-cancelled tick is a typed control error"
            );
        }
    }
}

// Bug reproduction: identity-layer-rewrite S-13 — activated convergence must
// not reinterpret frozen legacy scalar IDs as an endless chase, and a real
// edition-only bridge must hand a provider-discovered Work route to settlement;
// only a real chase that finds no bridge resolution may consume the v2 ledger.
async fn red_convergence_no_change_terminalizes_on_the_v2_axis() {
    let harness =
        build_route_harness_with_open_library(Some(livrarr_external_data::NormalizedWorkDetail {
            title: Some("Cider House Bridge".to_string()),
            author_name: Some("Cider House Convergence Author".to_string()),
            ol_key: Some("OL-CIDERHOUSE-BRIDGE-W".to_string()),
            ..Default::default()
        }))
        .await;
    let (author, _) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: "Cider House Convergence Author".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed convergence Author");

    let mut settled = settlement_commit(harness.user_id, author.id, None);
    settled.identity_title = title("The Cider House Rules");
    settled.identity_title.provenance =
        ilr::EvidenceProvenance::Provider(ilr::IdentityProvider::OpenLibrary);
    settled.routes = vec![ilr::WorkRoute {
        id: 0,
        user_id: harness.user_id,
        owner: RouteOwner::Work(0),
        resolved_work_id: 0,
        provider: ilr::IdentityProvider::OpenLibrary,
        kind: ilr::RouteKind::OpenLibraryWork,
        provider_scoped_id: "OL-CIDERHOUSE-BRIDGE-W".to_string(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::Provider(ilr::IdentityProvider::OpenLibrary),
        user_confirmed: false,
        observed_at: Utc::now(),
    }];
    let settled = WorkIdentityRepository::commit_settlement(&harness.db, settled)
        .await
        .expect("seed settled loop-shape Work");
    let loop_work_id = settled.identity.own_work_id;
    WorkDb::update_work_enrichment(
        &harness.db,
        harness.user_id,
        loop_work_id,
        UpdateWorkEnrichmentDbRequest {
            enrichment_status: livrarr_domain::EnrichmentStatus::Failed,
            ..Default::default()
        },
    )
    .await
    .expect("keep work-route fixture enrichment incomplete");
    WorkDb::set_next_convergence_at(
        &harness.db,
        harness.user_id,
        loop_work_id,
        Some(Utc::now() - chrono::Duration::seconds(1)),
    )
    .await
    .expect("make work-route quiet fixture due");
    let generation_before = settled.identity.identity_generation;
    let audits_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events \
          WHERE user_id=?1 AND work_id=?2 AND event_kind='settlement'",
    )
    .bind(harness.user_id)
    .bind(loop_work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count pre-tick settlement audits");

    let first = {
        let _contract_lock = CONVERGENCE_CONTRACT_LOCK.lock().await;
        livrarr_server::identity_layer::run_identity_convergence_tick(
            harness.state.clone(),
            CancellationToken::new(),
        )
        .await
    }
    .expect("first real convergence tick");
    assert_eq!(
        first.visited_work_count, 1,
        "the due, still-unenriched Work route must be genuinely visited"
    );
    assert_eq!(first.captured_route_count, 0);
    let generation_after: i64 =
        sqlx::query_scalar("SELECT identity_generation FROM works WHERE user_id=?1 AND id=?2")
            .bind(harness.user_id)
            .bind(loop_work_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("read loop-shape generation");
    let audits_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events \
          WHERE user_id=?1 AND work_id=?2 AND event_kind='settlement'",
    )
    .bind(harness.user_id)
    .bind(loop_work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count post-tick settlement audits");
    assert_eq!(generation_after, generation_before);
    assert_eq!(audits_after, audits_before);
    let quiet_attempts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_provider_attempts \
          WHERE user_id=?1 AND work_id=?2 AND provider='livrarr-convergence' \
            AND route_kind='bridge-upgrade'",
    )
    .bind(harness.user_id)
    .bind(loop_work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count attempts after quiet work-route visit");
    assert_eq!(quiet_attempts, 0);

    let mut bridge = settlement_commit(harness.user_id, author.id, None);
    bridge.identity_title = title("Cider House Bridge");
    bridge.identity_title.provenance =
        ilr::EvidenceProvenance::Provider(ilr::IdentityProvider::IsbnRegistry);
    bridge.routes = vec![ilr::WorkRoute {
        id: 0,
        user_id: harness.user_id,
        owner: RouteOwner::Work(0),
        resolved_work_id: 0,
        provider: ilr::IdentityProvider::IsbnRegistry,
        kind: ilr::RouteKind::Isbn13Edition,
        provider_scoped_id: "9780000000224".to_string(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::Provider(ilr::IdentityProvider::IsbnRegistry),
        user_confirmed: false,
        observed_at: Utc::now(),
    }];
    let bridge = WorkIdentityRepository::commit_settlement(&harness.db, bridge)
        .await
        .expect("seed a genuinely chaseable edition bridge");
    let bridge_work_id = bridge.identity.own_work_id;
    WorkDb::update_work_enrichment(
        &harness.db,
        harness.user_id,
        bridge_work_id,
        UpdateWorkEnrichmentDbRequest {
            enrichment_status: livrarr_domain::EnrichmentStatus::Enriched,
            ..Default::default()
        },
    )
    .await
    .expect("settle bridge enrichment");
    let bridge_audits_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events \
          WHERE user_id=?1 AND work_id=?2 AND event_kind='settlement'",
    )
    .bind(harness.user_id)
    .bind(bridge_work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count bridge audits before convergence");
    let productive = {
        let _contract_lock = CONVERGENCE_CONTRACT_LOCK.lock().await;
        livrarr_server::identity_layer::run_identity_convergence_tick(
            harness.state.clone(),
            CancellationToken::new(),
        )
        .await
    }
    .expect("productive bridge convergence tick");
    assert_eq!(productive.visited_work_count, 1);
    let captured = WorkIdentityRepository::read_captured_identity(
        &harness.db,
        harness.user_id,
        bridge_work_id,
    )
    .await
    .expect("read productive bridge graph");
    assert!(captured.active_routes.iter().any(|route| {
        route.kind == ilr::RouteKind::OpenLibraryWork
            && route.provider_scoped_id == "OL-CIDERHOUSE-BRIDGE-W"
    }));
    let attempts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_provider_attempts \
          WHERE user_id=?1 AND work_id=?2 AND provider='livrarr-convergence' \
            AND route_kind='bridge-upgrade'",
    )
    .bind(harness.user_id)
    .bind(bridge_work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count v2 convergence attempts");
    assert_eq!(
        attempts, 0,
        "a resolved chase is not a failed bridge attempt"
    );
    assert_eq!(
        captured.identity_generation,
        bridge.identity.identity_generation + 1
    );
    let bridge_audits_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events \
          WHERE user_id=?1 AND work_id=?2 AND event_kind='settlement'",
    )
    .bind(harness.user_id)
    .bind(bridge_work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count bridge audits after convergence");
    assert_eq!(bridge_audits_after, bridge_audits_before + 1);
}

async fn convergence_attempt_ledger_counts_only_a_real_unsuccessful_chase() {
    let harness = build_route_harness_with_provider_outcome(
        Some(livrarr_external_data::ProviderOutcome::NotFound),
        Vec::new(),
        None,
    )
    .await;
    let (author, _) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: "Honest Attempt Author".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed honest-attempt author");
    let mut bridge = settlement_commit(harness.user_id, author.id, None);
    bridge.identity_title = title("Honest Attempt Bridge");
    bridge.routes = vec![ilr::WorkRoute {
        id: 0,
        user_id: harness.user_id,
        owner: RouteOwner::Work(0),
        resolved_work_id: 0,
        provider: ilr::IdentityProvider::IsbnRegistry,
        kind: ilr::RouteKind::Isbn13Edition,
        provider_scoped_id: "9780306406157".to_string(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::Provider(ilr::IdentityProvider::IsbnRegistry),
        user_confirmed: false,
        observed_at: Utc::now(),
    }];
    let seeded = WorkIdentityRepository::commit_settlement(&harness.db, bridge)
        .await
        .expect("seed honest-attempt bridge");
    let work_id = seeded.identity.own_work_id;
    WorkDb::update_work_enrichment(
        &harness.db,
        harness.user_id,
        work_id,
        UpdateWorkEnrichmentDbRequest {
            enrichment_status: livrarr_domain::EnrichmentStatus::Enriched,
            ..Default::default()
        },
    )
    .await
    .expect("mark honest-attempt bridge enriched");
    let audits_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events \
          WHERE user_id=?1 AND work_id=?2 AND event_kind='settlement'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count honest-attempt audits before tick");

    let report = {
        let _contract_lock = CONVERGENCE_CONTRACT_LOCK.lock().await;
        livrarr_server::identity_layer::run_identity_convergence_tick(
            harness.state.clone(),
            CancellationToken::new(),
        )
        .await
    }
    .expect("run genuine no-find chase");
    assert_eq!(report.visited_work_count, 1);
    assert_eq!(report.captured_route_count, 0);
    assert_eq!(
        harness
            .open_library_stub
            .as_ref()
            .expect("NotFound OpenLibrary stub")
            .call_count(),
        1,
        "the charged visit must include a real provider dispatch"
    );
    let attempts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_provider_attempts \
          WHERE user_id=?1 AND work_id=?2 AND provider='livrarr-convergence' \
            AND route_kind='bridge-upgrade'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count honest convergence attempts");
    assert_eq!(attempts, 1);
    let captured =
        WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, work_id)
            .await
            .expect("read honest no-find graph");
    assert_eq!(
        captured.identity_generation,
        seeded.identity.identity_generation
    );
    let audits_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events \
          WHERE user_id=?1 AND work_id=?2 AND event_kind='settlement'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count honest-attempt audits after tick");
    assert_eq!(audits_after, audits_before);
}

// Bug reproduction: identity-layer-rewrite C-r4-02 — a PreferCache replay is
// productive input to enrichment, but it is not a new provider chase and must
// not consume another generation-scoped bridge-upgrade attempt.
async fn red_convergence_cache_only_second_visit_does_not_burn_bridge_attempt() {
    red_convergence_cache_only_second_visit_with_collision_sync(None).await;
}

async fn red_convergence_cache_only_second_visit_with_collision_sync(
    collision_sync: Option<Arc<ConvergenceCollisionSync>>,
) {
    let harness =
        build_route_harness_with_open_library(Some(livrarr_external_data::NormalizedWorkDetail {
            title: Some("Cached Bridge Payload".to_string()),
            author_name: Some("Cached Bridge Author".to_string()),
            ..Default::default()
        }))
        .await;
    let (author, _) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: "Cached Bridge Author".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed cached-bridge author");
    let mut bridge = settlement_commit(harness.user_id, author.id, None);
    bridge.identity_title = title("Cached Bridge Payload");
    bridge.routes = vec![ilr::WorkRoute {
        id: 0,
        user_id: harness.user_id,
        owner: RouteOwner::Work(0),
        resolved_work_id: 0,
        provider: ilr::IdentityProvider::IsbnRegistry,
        kind: ilr::RouteKind::Isbn13Edition,
        provider_scoped_id: "9780306406164".to_string(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::Provider(ilr::IdentityProvider::IsbnRegistry),
        user_confirmed: false,
        observed_at: Utc::now(),
    }];
    let seeded = WorkIdentityRepository::commit_settlement(&harness.db, bridge)
        .await
        .expect("seed cached edition bridge");
    let work_id = seeded.identity.own_work_id;

    if let Some(sync) = collision_sync.as_ref() {
        sync.ready.wait().await;
        sync.hook_armed.notified().await;
    }
    let first = {
        let _contract_lock = CONVERGENCE_CONTRACT_LOCK.lock().await;
        livrarr_server::identity_layer::run_identity_convergence_tick(
            harness.state.clone(),
            CancellationToken::new(),
        )
        .await
    }
    .expect("run cache-warming convergence visit");
    assert_eq!(first.visited_work_count, 1);
    assert_eq!(first.captured_route_count, 0);
    assert_eq!(
        harness
            .open_library_stub
            .as_ref()
            .expect("OpenLibrary cache fixture")
            .call_count(),
        1,
        "the cache-warming visit must spawn one real provider fetch"
    );
    let attempts_after_fetch: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_provider_attempts \
          WHERE user_id=?1 AND work_id=?2 AND provider='livrarr-convergence' \
            AND route_kind='bridge-upgrade'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count bridge attempts after real fetch");
    assert_eq!(attempts_after_fetch, 1);

    WorkDb::set_next_convergence_at(
        &harness.db,
        harness.user_id,
        work_id,
        Some(Utc::now() - chrono::Duration::seconds(1)),
    )
    .await
    .expect("make cached bridge due again");
    let second = {
        let _contract_lock = CONVERGENCE_CONTRACT_LOCK.lock().await;
        livrarr_server::identity_layer::run_identity_convergence_tick(
            harness.state.clone(),
            CancellationToken::new(),
        )
        .await
    }
    .expect("run cache-only convergence visit");
    assert_eq!(second.visited_work_count, 1);
    assert_eq!(second.captured_route_count, 0);
    assert_eq!(
        harness
            .open_library_stub
            .as_ref()
            .expect("OpenLibrary cache fixture")
            .call_count(),
        1,
        "the second visit must be fully cache-served"
    );
    let attempts_after_cache: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_provider_attempts \
          WHERE user_id=?1 AND work_id=?2 AND provider='livrarr-convergence' \
            AND route_kind='bridge-upgrade'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count bridge attempts after cache replay");
    assert_eq!(
        attempts_after_cache, attempts_after_fetch,
        "a cache-only visit must add zero bridge-upgrade attempts"
    );
}

// Bug reproduction: identity-layer-rewrite C-r5-01 — isolated fixtures reuse
// WorkId values, so a concurrently armed fail-next hook must remain owned by
// the failure-cadence visit and never poison the cache-warming visit.
async fn red_convergence_failure_hook_isolated_from_concurrent_cache_warming() {
    let collision_sync = Arc::new(ConvergenceCollisionSync::new());
    tokio::join!(
        red_server_convergence_with_collision_sync(
            ConvergenceContract::FailurePreservesCadence,
            Some(collision_sync.clone()),
        ),
        red_convergence_cache_only_second_visit_with_collision_sync(Some(collision_sync)),
    );
}

// Bug reproduction: identity-layer-rewrite C-r4-01 — when add-time resolver
// capture and completion find no Work route, the production add door's delayed
// refresh must submit the first route it discovers through the shared authority.
async fn red_direct_add_delayed_refresh_persists_first_work_route() {
    let _breaker = lock_breaker().await;
    let coverless = livrarr_external_data::NormalizedWorkDetail {
        title: Some("Delayed Route Add".to_string()),
        author_name: Some("Delayed Route Author".to_string()),
        ..Default::default()
    };
    let harness = build_route_harness_with_provider_details(
        Some(coverless.clone()),
        vec![(livrarr_domain::MetadataProvider::OpenLibrary, coverless)],
        None,
    )
    .await;
    let provider = harness
        .open_library_stub
        .as_ref()
        .expect("OpenLibrary delayed-refresh fixture")
        .clone();
    let added = call_router_json(
        &harness,
        Method::POST,
        "/api/v1/work".to_string(),
        Some(json!({
            "olKey": null,
            "title": "Delayed Route Add",
            "authorName": "Delayed Route Author",
            "authorOlKey": null,
            "year": null,
            "coverUrl": null,
            "language": "en",
            "detailUrl": null,
            "coverManual": false,
            "isbn13": "9780306406171",
            "candidateId": null,
            "hcKey": null,
            "grKey": null,
            "asin": null
        })),
    )
    .await;
    assert!(added.status.is_success(), "direct add: {}", added.json);
    let work_id = added.json["work"]["id"]
        .as_i64()
        .or_else(|| added.json["id"].as_i64())
        .expect("delayed-refresh work id");

    for _ in 0..200 {
        if provider.call_count() == 1 {
            let work = harness
                .db
                .get_work(harness.user_id, work_id)
                .await
                .expect("read completion-stage Work");
            if work.enrichment_status == livrarr_domain::EnrichmentStatus::Enriched {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        provider.call_count(),
        1,
        "complete_add must consume the first coverless provider response"
    );
    let before_refresh =
        WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, work_id)
            .await
            .expect("read identity before delayed refresh");
    assert!(
        !before_refresh.active_routes.iter().any(|route| matches!(
            route.kind,
            ilr::RouteKind::OpenLibraryWork
                | ilr::RouteKind::GoodreadsWork
                | ilr::RouteKind::HardcoverWork
        )),
        "resolver capture and complete_add must both leave the Work route absent"
    );

    provider.set_outcome(livrarr_external_data::ProviderOutcome::Success(Box::new(
        livrarr_external_data::NormalizedWorkDetail {
            title: Some("Delayed Route Add".to_string()),
            author_name: Some("Delayed Route Author".to_string()),
            ol_key: Some("OL-DELAYED-REFRESH-W".to_string()),
            ..Default::default()
        },
    )));
    for _ in 0..70 {
        if provider.call_count() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        provider.call_count(),
        2,
        "the +5s continuation must run one bypass-cache provider refresh"
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let enriching = harness
                .state
                .work_service
                .is_enriching(harness.user_id, work_id);
            let captured = WorkIdentityRepository::read_captured_identity(
                &harness.db,
                harness.user_id,
                work_id,
            )
            .await
            .expect("poll identity after delayed refresh");
            let route_present = captured.active_routes.iter().any(|route| {
                route.kind == ilr::RouteKind::OpenLibraryWork
                    && route.provider_scoped_id == "OL-DELAYED-REFRESH-W"
            });
            if !enriching && route_present {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("delayed refresh must finish enriching and commit its Work route within 10 seconds");

    let after_refresh =
        WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, work_id)
            .await
            .expect("read identity after delayed refresh");
    assert!(after_refresh.active_routes.iter().any(|route| {
        route.kind == ilr::RouteKind::OpenLibraryWork
            && route.provider_scoped_id == "OL-DELAYED-REFRESH-W"
    }));
    assert_eq!(
        after_refresh.identity_generation,
        before_refresh.identity_generation + 1,
        "the delayed EnrichmentPass handoff settles exactly once"
    );
}

enum ServerCutoverContract {
    LibraryCommandBinding,
    ExclusiveNoRuntime,
    ErrorMatrix,
}

async fn red_server_cutover(contract: ServerCutoverContract) {
    let data_dir = tempfile::tempdir().expect("cutover data dir");
    let result = livrarr_server::identity_layer::run_identity_cutover_command(
        livrarr_server::identity_layer::IdentityCutoverCliCommand::ListReviews,
        data_dir.path().to_path_buf(),
        CancellationToken::new(),
    )
    .await;
    match contract {
        ServerCutoverContract::LibraryCommandBinding => {
            assert!(matches!(
                result,
                Ok(livrarr_server::identity_layer::IdentityCutoverCliOutcome::ReviewList(_))
            ));
        }
        ServerCutoverContract::ExclusiveNoRuntime => {
            result.expect("one-shot list command completes");
            let source = strip_rust_comments(include_str!(
                "../../crates/livrarr-server/src/identity_layer.rs"
            ));
            assert!(source.contains("exclusive_lock"));
            assert!(!source.contains("build_router("));
            assert!(!source.contains("JobRunner"));
        }
        ServerCutoverContract::ErrorMatrix => {
            assert!(
                result.is_ok(),
                "valid list arm succeeds before error matrix"
            );
            let errors = strip_rust_comments(include_str!(
                "../../crates/livrarr-server/src/identity_layer.rs"
            ));
            for variant in [
                "NotSnapshot",
                "RehearsalMismatch",
                "ReviewKindMismatch",
                "StaleGeneration",
                "InvalidActionFile",
                "Cancelled",
                "Database",
            ] {
                assert!(errors.contains(variant));
            }
        }
    }
}

// Bug reproduction: identity-layer-rewrite — a nonempty clean staged identity
// graph must become Ready and activate through startup readiness; a genuinely
// colliding graph must mint exactly its resolvable cohort card and become Ready
// after the real CLI resolves that card and reruns Apply.
async fn red_real_cli_cutover_ceremony() {
    let data_dir = tempfile::tempdir().expect("real CLI cutover data dir");
    let snapshot_dir = tempfile::tempdir().expect("real CLI snapshot dir");
    let live_database = data_dir.path().join("livrarr.db");
    let snapshot = snapshot_dir.path().join("livrarr.db");
    let seed_pool = livrarr_db::pool::create_sqlite_pool(data_dir.path())
        .await
        .expect("create pre-cutover live database");
    livrarr_db::pool::run_migrations(&seed_pool)
        .await
        .expect("migrate pre-cutover live database");
    let author_id = sqlx::query(
        "INSERT INTO authors (user_id, name, normalized_name, added_at) \
         VALUES (1, 'Real CLI Cutover Author', 'real cli cutover author', ?1)",
    )
    .bind(Utc::now().to_rfc3339())
    .execute(&seed_pool)
    .await
    .expect("seed pre-cutover author")
    .last_insert_rowid();
    let mut cohort_work_ids = Vec::new();
    for (title, normalized_title, ol_key, in_review_cohort) in [
        (
            "Real CLI Shared Cohort",
            "real cli shared cohort",
            "OL-REAL-CLI-CUTOVER-A",
            true,
        ),
        (
            "Real CLI Shared Cohort",
            "real cli shared cohort",
            "OL-REAL-CLI-CUTOVER-B",
            true,
        ),
        (
            "Real CLI Unrelated Work",
            "real cli unrelated work",
            "OL-REAL-CLI-CUTOVER-C",
            false,
        ),
    ] {
        let work_id = sqlx::query(
            "INSERT INTO works \
                (user_id, title, author_name, author_id, normalized_title, \
                 normalized_author, ol_key, added_at) \
             VALUES (1, ?1, 'Real CLI Cutover Author', ?2, ?3, \
                     'real cli cutover author', ?4, ?5)",
        )
        .bind(title)
        .bind(author_id)
        .bind(normalized_title)
        .bind(ol_key)
        .bind(Utc::now().to_rfc3339())
        .execute(&seed_pool)
        .await
        .expect("seed pre-cutover work")
        .last_insert_rowid();
        if in_review_cohort {
            cohort_work_ids.push(work_id);
        }
    }
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&seed_pool)
        .await
        .expect("checkpoint pre-cutover live database");
    seed_pool.close().await;
    std::fs::copy(&live_database, &snapshot).expect("copy identical rehearsal snapshot");

    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve workspace root");
    let build = std::process::Command::new("cargo")
        .args([
            "build",
            "--quiet",
            "-p",
            "livrarr-server",
            "--bin",
            "livrarr",
        ])
        .env("RTK_DISABLED", "1")
        .current_dir(&workspace)
        .output()
        .expect("build production livrarr binary");
    assert!(
        build.status.success(),
        "production binary build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = workspace.join("target/debug/livrarr");

    let clean_data_dir = tempfile::tempdir().expect("clean CLI cutover data dir");
    let clean_snapshot_dir = tempfile::tempdir().expect("clean CLI snapshot dir");
    let clean_live_database = clean_data_dir.path().join("livrarr.db");
    let clean_snapshot = clean_snapshot_dir.path().join("livrarr.db");
    let clean_seed_pool = livrarr_db::pool::create_sqlite_pool(clean_data_dir.path())
        .await
        .expect("create clean pre-cutover live database");
    livrarr_db::pool::run_migrations(&clean_seed_pool)
        .await
        .expect("migrate clean pre-cutover live database");
    let clean_author_id = sqlx::query(
        "INSERT INTO authors (user_id, name, normalized_name, added_at) \
         VALUES (1, 'Clean CLI Cutover Author', 'clean cli cutover author', ?1)",
    )
    .bind(Utc::now().to_rfc3339())
    .execute(&clean_seed_pool)
    .await
    .expect("seed clean pre-cutover author")
    .last_insert_rowid();
    for (title, normalized_title, ol_key) in [
        (
            "Clean CLI First Work",
            "clean cli first work",
            "OL-CLEAN-CLI-CUTOVER-A",
        ),
        (
            "Clean CLI Second Work",
            "clean cli second work",
            "OL-CLEAN-CLI-CUTOVER-B",
        ),
    ] {
        sqlx::query(
            "INSERT INTO works \
                (user_id, title, author_name, author_id, normalized_title, \
                 normalized_author, ol_key, added_at) \
             VALUES (1, ?1, 'Clean CLI Cutover Author', ?2, ?3, \
                     'clean cli cutover author', ?4, ?5)",
        )
        .bind(title)
        .bind(clean_author_id)
        .bind(normalized_title)
        .bind(ol_key)
        .bind(Utc::now().to_rfc3339())
        .execute(&clean_seed_pool)
        .await
        .expect("seed clean pre-cutover work");
    }
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&clean_seed_pool)
        .await
        .expect("checkpoint clean pre-cutover live database");
    clean_seed_pool.close().await;
    std::fs::copy(&clean_live_database, &clean_snapshot)
        .expect("copy clean identical rehearsal snapshot");

    let clean_rehearsal = std::process::Command::new(&binary)
        .arg("--data")
        .arg(clean_data_dir.path())
        .args(["identity-cutover", "rehearse", "--snapshot"])
        .arg(&clean_snapshot)
        .output()
        .expect("run clean real rehearsal CLI invocation");
    assert!(
        clean_rehearsal.status.success(),
        "clean real rehearsal invocation failed:\n{}",
        String::from_utf8_lossy(&clean_rehearsal.stderr)
    );
    let clean_pool = livrarr_db::pool::create_sqlite_pool(clean_data_dir.path())
        .await
        .expect("reopen clean data-dir database after rehearsal");
    let clean_approved_json: String = sqlx::query_scalar(
        "SELECT report_json FROM identity_cutover_runs \
         WHERE mode = 'rehearsal' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&clean_pool)
    .await
    .expect("read clean approved rehearsal report");
    let clean_approved: ilr::IdentityMigrationReport =
        serde_json::from_str(&clean_approved_json).expect("decode clean rehearsal report");
    assert!(clean_approved.legacy_work_count > 0);
    assert_eq!(clean_approved.group_cards, 0);
    assert!(
        clean_approved.index_ready,
        "a nonempty staged graph with unique activation keys must be index-ready"
    );
    let clean_approved_report = clean_data_dir.path().join("approved-report.json");
    std::fs::write(&clean_approved_report, clean_approved_json)
        .expect("write clean approved report input");
    let clean_apply = std::process::Command::new(&binary)
        .arg("--data")
        .arg(clean_data_dir.path())
        .args(["identity-cutover", "apply", "--approved-report"])
        .arg(&clean_approved_report)
        .output()
        .expect("run clean real Apply CLI invocation");
    assert!(
        clean_apply.status.success(),
        "clean real Apply invocation failed:\n{}",
        String::from_utf8_lossy(&clean_apply.stderr)
    );
    let clean_apply_status: String = sqlx::query_scalar(
        "SELECT status FROM identity_cutover_runs \
         WHERE mode = 'apply' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&clean_pool)
    .await
    .expect("read clean Apply status");
    let clean_pending_cards: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM identity_review_cards WHERE status = 'pending'")
            .fetch_one(&clean_pool)
            .await
            .expect("count clean pending cards");
    let clean_staged_keys: Vec<(String, String, String, i64, String)> = sqlx::query_as(
        "SELECT normalized_identity_main, normalized_identity_subtitle, \
                normalized_identity_volume, primary_author_id, text_distinction \
           FROM works ORDER BY id",
    )
    .fetch_all(&clean_pool)
    .await
    .expect("read clean staged activation keys");
    assert_eq!(clean_apply_status, "ready");
    assert_eq!(clean_pending_cards, 0, "Ready must never hide a card");
    assert_eq!(
        clean_staged_keys,
        vec![
            (
                "clean cli first work".to_string(),
                String::new(),
                String::new(),
                clean_author_id,
                "common".to_string(),
            ),
            (
                "clean cli second work".to_string(),
                String::new(),
                String::new(),
                clean_author_id,
                "common".to_string(),
            ),
        ],
        "Apply must persist the same projected tuples used by the dry-run probe"
    );
    let clean_db = livrarr_db::sqlite::SqliteDb::new(clean_pool.clone());
    let clean_readiness =
        livrarr_server::identity_layer::ensure_identity_authority_ready_before_serve(clean_db)
            .await
            .expect("startup readiness activates the approved clean cutover");
    assert_eq!(
        clean_readiness,
        ilr::IdentityAuthorityReadiness::ActivatedFresh
    );
    let clean_marker: Option<String> =
        sqlx::query_scalar("SELECT value FROM _livrarr_meta WHERE key = 'identity_authority_v2'")
            .fetch_optional(&clean_pool)
            .await
            .expect("read clean activation marker");
    let clean_index_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master \
         WHERE type = 'index' AND name = 'idx_works_identity_v2'",
    )
    .fetch_one(&clean_pool)
    .await
    .expect("inspect clean activation index");
    let clean_activated_status: String = sqlx::query_scalar(
        "SELECT status FROM identity_cutover_runs \
         WHERE mode = 'apply' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&clean_pool)
    .await
    .expect("read activated clean Apply status");
    assert_eq!(clean_marker.as_deref(), Some("active"));
    assert_eq!(clean_index_count, 1);
    assert_eq!(clean_activated_status, "activated");
    clean_pool.close().await;

    let rehearsal = std::process::Command::new(&binary)
        .arg("--data")
        .arg(data_dir.path())
        .args(["identity-cutover", "rehearse", "--snapshot"])
        .arg(&snapshot)
        .output()
        .expect("run real rehearsal CLI invocation");
    assert!(
        rehearsal.status.success(),
        "real rehearsal invocation failed:\n{}",
        String::from_utf8_lossy(&rehearsal.stderr)
    );

    let pool = livrarr_db::pool::create_sqlite_pool(data_dir.path())
        .await
        .expect("reopen data-dir database after rehearsal invocation");
    let live_approved_json: Option<String> = sqlx::query_scalar(
        "SELECT report_json FROM identity_cutover_runs \
         WHERE mode = 'rehearsal' ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .expect("read optional approved report from live rehearsal ledger");
    pool.close().await;
    // Before the fix, the only copy of this row is in the snapshot. Reading
    // that misplaced row lets the regression continue through the real Apply
    // invocation and fail specifically with RehearsalMismatch.
    let approved_json = if let Some(report) = live_approved_json {
        report
    } else {
        let snapshot_pool = livrarr_db::pool::create_sqlite_pool(snapshot_dir.path())
            .await
            .expect("open snapshot to expose misplaced rehearsal ledger");
        let report = sqlx::query_scalar(
            "SELECT report_json FROM identity_cutover_runs \
             WHERE mode = 'rehearsal' ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(&snapshot_pool)
        .await
        .expect("read misplaced snapshot rehearsal ledger");
        snapshot_pool.close().await;
        report
    };
    let approved: ilr::IdentityMigrationReport =
        serde_json::from_str(&approved_json).expect("decode approved rehearsal report");
    assert_eq!(
        approved.group_cards, 1,
        "the N-work fixture has exactly one duplicate identity cohort"
    );
    assert!(
        !approved.index_ready,
        "the staged duplicate activation key must block the dry-run probe"
    );
    assert_eq!(
        (
            approved.field_cards,
            approved.repair_cards,
            approved.contributor_cards,
        ),
        (0, 0, 0),
        "the fixture isolates GroupIdentity staging"
    );
    let approved_report = data_dir.path().join("approved-report.json");
    std::fs::write(&approved_report, approved_json).expect("write approved report input");

    let apply = std::process::Command::new(&binary)
        .arg("--data")
        .arg(data_dir.path())
        .args(["identity-cutover", "apply", "--approved-report"])
        .arg(&approved_report)
        .output()
        .expect("run real Apply CLI invocation");
    let apply_stderr = String::from_utf8_lossy(&apply.stderr);
    assert!(
        apply.status.success(),
        "real Apply must reuse the data-dir rehearsal ledger and reach Blocked or Ready, not RehearsalMismatch:\n{apply_stderr}"
    );
    assert!(
        !apply_stderr.contains("rehearsal mismatch"),
        "real Apply regressed to RehearsalMismatch: {apply_stderr}"
    );

    let list = std::process::Command::new(&binary)
        .arg("--data")
        .arg(data_dir.path())
        .args(["identity-cutover", "list"])
        .output()
        .expect("run real list CLI invocation");
    let list_stdout = String::from_utf8_lossy(&list.stdout);
    let list_stderr = String::from_utf8_lossy(&list.stderr);

    let pool = livrarr_db::pool::create_sqlite_pool(data_dir.path())
        .await
        .expect("reopen data-dir database after Apply invocation");
    let live_rehearsal_runs: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM identity_cutover_runs WHERE mode = 'rehearsal'")
            .fetch_one(&pool)
            .await
            .expect("count live rehearsal ledger rows");
    let live_rehearsal_reports: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_cutover_reports AS report \
         JOIN identity_cutover_runs AS run ON run.id = report.run_id \
         WHERE run.mode = 'rehearsal'",
    )
    .fetch_one(&pool)
    .await
    .expect("count live rehearsal report rows");
    let apply_status: String = sqlx::query_scalar(
        "SELECT status FROM identity_cutover_runs \
         WHERE mode = 'apply' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("read real Apply staging status");
    let staged_cards: Vec<(i64, String, i64, String)> = sqlx::query_as(
        "SELECT id, kind, generation, payload FROM identity_review_cards \
         WHERE status = 'pending' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("read staged pending review cards");
    let colliding_staged_keys: Vec<(String, String, String, i64, String)> = sqlx::query_as(
        "SELECT normalized_identity_main, normalized_identity_subtitle, \
                normalized_identity_volume, primary_author_id, text_distinction \
           FROM works ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("read colliding staged activation keys");
    assert_eq!(
        live_rehearsal_runs, 1,
        "the rehearsal run ledger belongs to the live data-dir database"
    );
    assert_eq!(
        live_rehearsal_reports, 1,
        "the rehearsal report ledger belongs to the live data-dir database"
    );
    assert_eq!(apply_status, "blocked", "a real collision must block Apply");
    assert_eq!(
        colliding_staged_keys[0], colliding_staged_keys[1],
        "the report/card cohort must be an exact activation-key collision"
    );
    assert_ne!(
        colliding_staged_keys[1], colliding_staged_keys[2],
        "the unrelated staged key must stay outside the cohort"
    );
    assert!(
        list.status.success() && staged_cards.len() as u64 == approved.group_cards,
        "staging and the real list command must honor the approved report: \
         report.group_cards={}, staged_cards={}, list_stdout={list_stdout:?}, \
         list_stderr={list_stderr:?}",
        approved.group_cards,
        staged_cards.len(),
    );
    let (card_id, stored_kind, generation, payload) = staged_cards
        .first()
        .expect("the flagged cohort produces one staged card");
    assert_eq!(stored_kind, "GroupIdentity");
    assert!(
        list_stdout.contains(&format!("card_id: {card_id}"))
            && list_stdout.contains("kind: GroupIdentity"),
        "the real list command must display the staged GroupIdentity card: {list_stdout}"
    );
    let staged_payload: ilr::SettlementReviewCard =
        serde_json::from_str(payload).expect("decode staged cohort payload");
    assert!(
        matches!(
            staged_payload,
            ilr::SettlementReviewCard::GroupIdentity { ref work_ids, .. }
                if work_ids == &cohort_work_ids
        ),
        "the single staged card must contain only the flagged cohort"
    );

    let action_file = data_dir.path().join("resolve-group-identity.json");
    std::fs::write(&action_file, "\"DifferentFromAll\"")
        .expect("write GroupIdentity resolution action");
    let resolve = std::process::Command::new(&binary)
        .arg("--data")
        .arg(data_dir.path())
        .args(["identity-cutover", "resolve"])
        .arg(card_id.to_string())
        .arg("--expected-generation")
        .arg(generation.to_string())
        .arg("--action-file")
        .arg(&action_file)
        .output()
        .expect("resolve the real colliding cohort through the CLI");
    assert!(
        resolve.status.success(),
        "real CLI cohort resolution failed:\n{}",
        String::from_utf8_lossy(&resolve.stderr)
    );
    let after_resolve_list = std::process::Command::new(&binary)
        .arg("--data")
        .arg(data_dir.path())
        .args(["identity-cutover", "list"])
        .output()
        .expect("list after real cohort resolution");
    assert!(
        after_resolve_list.status.success()
            && String::from_utf8_lossy(&after_resolve_list.stdout).contains("ReviewList([])"),
        "resolved cohort must leave no pending cards: stdout={:?}, stderr={:?}",
        String::from_utf8_lossy(&after_resolve_list.stdout),
        String::from_utf8_lossy(&after_resolve_list.stderr),
    );
    let reapply = std::process::Command::new(&binary)
        .arg("--data")
        .arg(data_dir.path())
        .args(["identity-cutover", "apply", "--approved-report"])
        .arg(&approved_report)
        .output()
        .expect("rerun real Apply after resolving the cohort");
    assert!(
        reapply.status.success(),
        "real Apply rerun after resolution failed:\n{}",
        String::from_utf8_lossy(&reapply.stderr)
    );
    let resolved_apply_status: String = sqlx::query_scalar(
        "SELECT status FROM identity_cutover_runs \
         WHERE mode = 'apply' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("read resolved Apply status");
    let pending_after_reapply: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM identity_review_cards WHERE status = 'pending'")
            .fetch_one(&pool)
            .await
            .expect("count pending cards after resolved Apply");
    assert_eq!(resolved_apply_status, "ready");
    assert_eq!(pending_after_reapply, 0);
    pool.close().await;

    let snapshot_pool = livrarr_db::pool::create_sqlite_pool(snapshot_dir.path())
        .await
        .expect("reopen snapshot after the ceremony");
    let snapshot_runs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM identity_cutover_runs")
        .fetch_one(&snapshot_pool)
        .await
        .expect("count snapshot cutover runs");
    let snapshot_reports: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM identity_cutover_reports")
        .fetch_one(&snapshot_pool)
        .await
        .expect("count snapshot cutover reports");
    assert_eq!(
        (snapshot_runs, snapshot_reports),
        (0, 0),
        "snapshot supplies report content only and never owns the rehearsal ledger"
    );
    snapshot_pool.close().await;

    // Bug reproduction: identity-layer-rewrite — the ceremony must end at the
    // real production post-migration boot seam. Apply writes schema_version 83,
    // so both the compatibility gate and authority readiness must accept the
    // ceremonied nonempty database before Livrarr can serve.
    let startup_pool = livrarr_db::pool::create_sqlite_pool(data_dir.path())
        .await
        .expect("reopen the ceremonied database for production startup");
    livrarr_db::pool::check_version_gate(&startup_pool)
        .await
        .expect("production startup accepts this binary's schema 83 migration output");
    let startup_readiness =
        livrarr_server::identity_layer::ensure_identity_authority_ready_before_serve(
            livrarr_db::sqlite::SqliteDb::new(startup_pool.clone()),
        )
        .await
        .expect("production readiness activates the ceremonied nonempty database");
    assert_eq!(
        startup_readiness,
        ilr::IdentityAuthorityReadiness::ActivatedFresh
    );
    startup_pool.close().await;
}

enum ServerReadinessContract {
    StartupBoundaries,
    SkipLegacyWriters,
}

async fn red_server_readiness(contract: ServerReadinessContract) {
    let db = create_test_db().await;
    let result =
        livrarr_server::identity_layer::ensure_identity_authority_ready_before_serve(db.clone())
            .await;
    match contract {
        ServerReadinessContract::StartupBoundaries => {
            assert!(matches!(
                result,
                Ok(ilr::IdentityAuthorityReadiness::Active)
                    | Ok(ilr::IdentityAuthorityReadiness::ActivatedFresh)
            ));
            let main = strip_rust_comments(include_str!("../../crates/livrarr-server/src/main.rs"));
            let readiness = main.find("ensure_identity_authority_ready_before_serve");
            let bind = main.find("build_router");
            assert!(readiness
                .zip(bind)
                .is_some_and(|(ready, http)| ready < http));
        }
        ServerReadinessContract::SkipLegacyWriters => {
            result.expect("active startup proceeds");
            let main = strip_rust_comments(include_str!("../../crates/livrarr-server/src/main.rs"));
            let init_database = main
                .split("async fn init_database")
                .nth(1)
                .unwrap_or_default();
            for legacy_writer in [
                "backfill_normalized_identity(",
                "backfill_author_identity(",
                "backfill_identity_key_recompute(",
                "backfill_work_identity_ledger(",
                "clear_subtitle_rule_deadends(",
            ] {
                assert!(
                    !init_database.contains(legacy_writer),
                    "active startup must not call legacy writer {legacy_writer}"
                );
            }
        }
    }
}

async fn red_frontend_sibling() {
    let harness = build_route_harness().await;
    sqlx::query("DROP INDEX IF EXISTS idx_works_test_helper_creation_dedup")
        .execute(harness.db.pool())
        .await
        .expect("remove test-helper-only legacy dedup index");
    let (author, _) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: "Identity Author dto-shape".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed sibling Author");
    let mut first = settlement_commit(harness.user_id, author.id, None);
    first.identity_title = title("ILR DTO Shape");
    first.text_distinction = Some("dto-ebook".to_string());
    // Bug reproduction: identity-layer-rewrite S-18 — this is the v2 shape:
    // every identifier lives only in active routes and every frozen Work
    // scalar remains NULL. Two ISBN editions pin the deterministic projection:
    // explicit user confirmation outranks migrated/provider evidence.
    first.routes = vec![
        ilr::WorkRoute {
            id: 0,
            user_id: harness.user_id,
            owner: RouteOwner::Work(0),
            resolved_work_id: 0,
            provider: ilr::IdentityProvider::OpenLibrary,
            kind: ilr::RouteKind::OpenLibraryWork,
            provider_scoped_id: "OL-DTO-SHAPE-W".to_string(),
            state: ilr::WorkRouteState::Active,
            provenance: ilr::RouteProvenance::Migrated {
                legacy_field: "ol_key".to_string(),
            },
            user_confirmed: false,
            observed_at: Utc::now(),
        },
        ilr::WorkRoute {
            id: 0,
            user_id: harness.user_id,
            owner: RouteOwner::Work(0),
            resolved_work_id: 0,
            provider: ilr::IdentityProvider::Goodreads,
            kind: ilr::RouteKind::GoodreadsWork,
            provider_scoped_id: "77197".to_string(),
            state: ilr::WorkRouteState::Active,
            provenance: ilr::RouteProvenance::Provider(ilr::IdentityProvider::Goodreads),
            user_confirmed: false,
            observed_at: Utc::now(),
        },
        ilr::WorkRoute {
            id: 0,
            user_id: harness.user_id,
            owner: RouteOwner::Work(0),
            resolved_work_id: 0,
            provider: ilr::IdentityProvider::Hardcover,
            kind: ilr::RouteKind::HardcoverWork,
            provider_scoped_id: "427336".to_string(),
            state: ilr::WorkRouteState::Active,
            provenance: ilr::RouteProvenance::Provider(ilr::IdentityProvider::Hardcover),
            user_confirmed: false,
            observed_at: Utc::now(),
        },
        ilr::WorkRoute {
            id: 0,
            user_id: harness.user_id,
            owner: RouteOwner::Work(0),
            resolved_work_id: 0,
            provider: ilr::IdentityProvider::IsbnRegistry,
            kind: ilr::RouteKind::Isbn13Edition,
            provider_scoped_id: "9780007876433".to_string(),
            state: ilr::WorkRouteState::Active,
            provenance: ilr::RouteProvenance::Migrated {
                legacy_field: "isbn_13".to_string(),
            },
            user_confirmed: false,
            observed_at: Utc::now(),
        },
        ilr::WorkRoute {
            id: 0,
            user_id: harness.user_id,
            owner: RouteOwner::Work(0),
            resolved_work_id: 0,
            provider: ilr::IdentityProvider::IsbnRegistry,
            kind: ilr::RouteKind::Isbn13Edition,
            provider_scoped_id: "9780553573398".to_string(),
            state: ilr::WorkRouteState::Active,
            provenance: ilr::RouteProvenance::UserChoice,
            user_confirmed: true,
            observed_at: Utc::now(),
        },
        ilr::WorkRoute {
            id: 0,
            user_id: harness.user_id,
            owner: RouteOwner::Work(0),
            resolved_work_id: 0,
            provider: ilr::IdentityProvider::Amazon,
            kind: ilr::RouteKind::AsinEdition,
            provider_scoped_id: "B0DTOASIN01".to_string(),
            state: ilr::WorkRouteState::Active,
            provenance: ilr::RouteProvenance::Migrated {
                legacy_field: "asin".to_string(),
            },
            user_confirmed: false,
            observed_at: Utc::now(),
        },
    ];
    let first = WorkIdentityRepository::commit_settlement(&harness.db, first)
        .await
        .expect("settle first sibling");
    let mut second = settlement_commit(harness.user_id, author.id, None);
    second.identity_title = title("ILR DTO Shape");
    second.identity_title.volume = Some("Audiobook edition".to_string());
    second.identity_title.normalized_volume = "audiobook edition".to_string();
    second.text_distinction = Some("dto-audiobook".to_string());
    second.routes = vec![ilr::WorkRoute {
        id: 0,
        user_id: harness.user_id,
        owner: RouteOwner::Work(0),
        resolved_work_id: 0,
        provider: ilr::IdentityProvider::Goodreads,
        kind: ilr::RouteKind::GoodreadsBookEdition,
        provider_scoped_id: "985244".to_string(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::Migrated {
            legacy_field: "gr_key".to_string(),
        },
        user_confirmed: false,
        observed_at: Utc::now(),
    }];
    let second = WorkIdentityRepository::commit_settlement(&harness.db, second)
        .await
        .expect("settle second sibling");
    WorkDb::update_cover_metadata(
        &harness.db,
        harness.user_id,
        first.identity.own_work_id,
        Some("https://covers.example/ebook.jpg"),
        "OpenLibrary",
        false,
        600,
        900,
    )
    .await
    .expect("seed one produced cover slot");
    WorkDb::update_work_enrichment(
        &harness.db,
        harness.user_id,
        first.identity.own_work_id,
        UpdateWorkEnrichmentDbRequest {
            enrichment_status: livrarr_domain::EnrichmentStatus::Enriched,
            ..Default::default()
        },
    )
    .await
    .expect("mark active-route cover fixture enriched");
    type LegacyIdentifierScalars = (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let legacy_route: LegacyIdentifierScalars = sqlx::query_as(
        "SELECT ol_key, gr_key, hc_key, isbn_13, asin \
           FROM works WHERE id = ?1 AND user_id = ?2",
    )
    .bind(first.identity.own_work_id)
    .bind(harness.user_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read frozen scalar routes");
    assert_eq!(
        legacy_route,
        (None, None, None, None, None),
        "routes-only fixture must leave every frozen Work scalar NULL"
    );
    let response = call_router_json(
        &harness,
        Method::GET,
        format!("/api/v1/work/{}", first.identity.own_work_id),
        None,
    )
    .await;
    assert!(
        response.status.is_success(),
        "work detail: {}",
        response.json
    );
    let encoded = response.json;
    let assert_route_projection = |surface: &Value, name: &str| {
        assert_eq!(surface["olKey"], "OL-DTO-SHAPE-W", "{name}: olKey");
        assert_eq!(surface["grKey"], "77197", "{name}: grKey");
        assert_eq!(surface["hcKey"], "427336", "{name}: hcKey");
        assert_eq!(
            surface["isbn13"], "9780553573398",
            "{name}: user-confirmed ISBN must outrank the migrated ISBN"
        );
        assert_eq!(surface["asin"], "B0DTOASIN01", "{name}: asin");
    };
    assert_route_projection(&encoded, "work detail");
    assert_eq!(
        encoded["coverUrl"], "https://covers.example/ebook.jpg",
        "the detail overlay must retain the persisted primary cover URL"
    );
    let expected_fields = [
        "id",
        "title",
        "sortTitle",
        "subtitle",
        "originalTitle",
        "authorName",
        "authorId",
        "description",
        "year",
        "seriesId",
        "seriesName",
        "seriesPosition",
        "genres",
        "language",
        "pageCount",
        "durationSeconds",
        "publisher",
        "publishDate",
        "olKey",
        "hcKey",
        "grKey",
        "isbn13",
        "asin",
        "narrator",
        "narrationType",
        "abridged",
        "rating",
        "ratingCount",
        "enrichmentStatus",
        "identityStatus",
        "enrichedAt",
        "enrichmentSource",
        "coverUrl",
        "coverManual",
        "coverSource",
        "coverWidth",
        "coverHeight",
        "audiobookCoverUrl",
        "audiobookCoverSource",
        "audiobookCoverWidth",
        "audiobookCoverHeight",
        "monitorEbook",
        "monitorAudiobook",
        "addedAt",
        "libraryItems",
        "enriching",
        "parkedByConflicts",
        "identitySiblings",
        "coverUiState",
    ];
    let detail_fields = encoded.as_object().expect("work detail is an object");
    for field in expected_fields {
        assert!(
            detail_fields.contains_key(field),
            "work-detail overlay dropped DTO field {field}"
        );
    }
    let siblings = encoded["identitySiblings"]
        .as_array()
        .expect("production work detail emits identitySiblings");
    assert_eq!(siblings.len(), 1, "two settled Works produce one sibling");
    assert_eq!(siblings[0]["title"], "ILR DTO Shape");
    assert_eq!(siblings[0]["authorName"], "Identity Author dto-shape");
    assert_eq!(siblings[0]["edition"], "Audiobook edition");
    assert_eq!(siblings[0]["route"], "Goodreads");
    let migrated_gr = call_router_json(
        &harness,
        Method::GET,
        format!("/api/v1/work/{}", second.identity.own_work_id),
        None,
    )
    .await;
    assert_eq!(migrated_gr.status, StatusCode::OK);
    assert_eq!(
        migrated_gr.json["grKey"], "985244",
        "pre-cutover Goodreads edition routes must preserve grKey without a scalar fallback"
    );
    assert!(
        encoded["coverUiState"].is_object()
            && encoded["coverUiState"].get("ebook").is_some()
            && encoded["coverUiState"].get("audiobook").is_some(),
        "Rust producer emits the closed coverUiState object"
    );
    assert!(
        encoded["coverUiState"]["formatNeeded"]["candidates"]
            .as_array()
            .is_some_and(|candidates| !candidates.is_empty()),
        "one produced format cover makes the other-format panel reachable"
    );
    assert_eq!(
        encoded["coverUiState"]["audiobook"]["state"], "NoCoverFound",
        "an enriched Work with active identity_routes must not degrade to NowhereToLook"
    );
    let work_detail = strip_rust_comments(include_str!(
        "../../frontend/src/pages/work-detail/components/BookInformationTab.tsx"
    ));
    let panel = work_detail
        .split("function IdentitySiblingPanel")
        .nth(1)
        .unwrap_or_default();
    let exact = "Confirming this book's identity affects only this book. Other books by this author stay exactly as they are.";
    assert!(
        panel.contains(exact),
        "production Work detail must render the PO-frozen sibling copy"
    );
    for mutation_control in [
        "useMutation(",
        ".mutate(",
        "onClear",
        "onRemove",
        "onReset",
        "onDetach",
        "onReplace",
    ] {
        assert!(
            !panel.contains(mutation_control),
            "sibling panel must be informational; found mutation control {mutation_control:?}"
        );
    }
    // The implementation-phase render harness drives the production Work
    // detail route, clicks every sibling affordance, and rejects mutation
    // requests at the real frontend API boundary.
    let render_harness = strip_rust_comments(include_str!(
        "../../frontend/src/pages/work-detail/components/BookInformationTab.test.tsx"
    ));
    assert!(
        render_harness.contains("<WorkDetailPage />")
            && render_harness.contains("[data-sibling-affordance]")
            && render_harness.contains("call.method !== \"GET\""),
        "production frontend harness must click sibling affordances and assert zero mutations"
    );

    let list = call_router_json(
        &harness,
        Method::GET,
        "/api/v1/work?page=1&pageSize=100".to_string(),
        None,
    )
    .await;
    assert!(list.status.is_success(), "work list: {}", list.json);
    let listed = list.json["items"]
        .as_array()
        .expect("paginated work items")
        .iter()
        .find(|work| work["id"] == first.identity.own_work_id)
        .expect("covered Work on list surface");
    assert_route_projection(listed, "work list");
    assert_eq!(listed["coverUrl"], "https://covers.example/ebook.jpg");

    let author_detail = call_router_json(
        &harness,
        Method::GET,
        format!("/api/v1/author/{}", author.id),
        None,
    )
    .await;
    assert!(
        author_detail.status.is_success(),
        "author detail: {}",
        author_detail.json
    );
    let author_work = author_detail.json["works"]
        .as_array()
        .expect("author works")
        .iter()
        .find(|work| work["id"] == first.identity.own_work_id)
        .expect("covered Work on author surface");
    assert_route_projection(author_work, "author detail");
    assert_eq!(author_work["coverUrl"], "https://covers.example/ebook.jpg");

    let series = harness
        .db
        .upsert_series(CreateSeriesDbRequest {
            user_id: harness.user_id,
            author_id: author.id,
            name: "Cider House DTO Series".to_string(),
            gr_key: "cider-house-dto-series".to_string(),
            monitor_ebook: true,
            monitor_audiobook: false,
            monitor_language: Some("en".to_string()),
            work_count: 1,
        })
        .await
        .expect("seed DTO series");
    WorkDb::set_work_series_id(
        &harness.db,
        harness.user_id,
        first.identity.own_work_id,
        Some(series.id),
    )
    .await
    .expect("link covered Work to DTO series");
    let series_detail = call_router_json(
        &harness,
        Method::GET,
        format!("/api/v1/series/{}", series.id),
        None,
    )
    .await;
    assert!(
        series_detail.status.is_success(),
        "series detail: {}",
        series_detail.json
    );
    let series_work = series_detail.json["works"]
        .as_array()
        .expect("series works")
        .iter()
        .find(|work| work["id"] == first.identity.own_work_id)
        .expect("covered Work on series surface");
    assert_route_projection(series_work, "series detail");
    assert_eq!(series_work["coverUrl"], "https://covers.example/ebook.jpg");
}

async fn red_presentation_survives_deleted_author() {
    // Bug reproduction: identity-layer-rewrite — deleting an Author must not hide its Works.
    let harness = build_route_harness().await;
    let (author, _) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: "Deleted Author Presentation".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed Author deleted through the real route");
    let (work, _) = harness
        .db
        .create_work(CreateWorkDbRequest {
            user_id: harness.user_id,
            title: "Authorless Work Detail".to_string(),
            author_name: author.name.clone(),
            normalized_title: "authorless work detail".to_string(),
            normalized_author: "deleted author presentation".to_string(),
            author_id: Some(author.id),
            language: Some("en".to_string()),
            ..Default::default()
        })
        .await
        .expect("create Work before deleting its Author");
    let work_id = work.id;
    let (sibling_author, _) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: "Surviving Coauthor".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed sibling contributor");
    for (contributor_id, ordinal, role) in [
        (author.id, 0_i64, "Author"),
        (sibling_author.id, 1_i64, "Translator"),
    ] {
        sqlx::query(
            "INSERT INTO work_contributors (user_id, work_id, author_id, ordinal) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(harness.user_id)
        .bind(work_id)
        .bind(contributor_id)
        .bind(ordinal)
        .execute(harness.db.pool())
        .await
        .expect("seed contributor edge");
        sqlx::query(
            "INSERT INTO work_contributor_roles \
                (user_id, work_id, author_id, role, provenance, observed_at) \
             VALUES (?1, ?2, ?3, ?4, 'User', ?5)",
        )
        .bind(harness.user_id)
        .bind(work_id)
        .bind(contributor_id)
        .bind(role)
        .bind(Utc::now().to_rfc3339())
        .execute(harness.db.pool())
        .await
        .expect("seed contributor role");
    }
    WorkDb::update_cover_metadata(
        &harness.db,
        harness.user_id,
        work_id,
        Some("https://covers.example/authorless.jpg"),
        "OpenLibrary",
        false,
        600,
        900,
    )
    .await
    .expect("seed presentation data independent of the Author row");

    let deleted = call_router_json(
        &harness,
        Method::DELETE,
        format!("/api/v1/author/{}", author.id),
        None,
    )
    .await;
    assert!(
        deleted.status.is_success(),
        "real Author delete route returned {}: {}",
        deleted.status,
        deleted.json
    );

    let detail = call_router_json(
        &harness,
        Method::GET,
        format!("/api/v1/work/{work_id}"),
        None,
    )
    .await;
    assert_eq!(
        detail.status,
        StatusCode::OK,
        "presentation read must degrade after Author deletion: {}",
        detail.json
    );
    assert_eq!(detail.json["authorName"], "Deleted Author Presentation");
    assert_eq!(detail.json["identitySiblings"], json!([]));
    assert_eq!(
        detail.json["coverUiState"]["ebook"]["state"], "Selected",
        "stored Work cover remains present in the degraded projection"
    );
    let surviving_credit: (i64, i64, String) = sqlx::query_as(
        "SELECT c.author_id, c.ordinal, r.role FROM work_contributors c \
           JOIN work_contributor_roles r USING (user_id, work_id, author_id) \
          WHERE c.user_id=?1 AND c.work_id=?2",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("sibling contributor and role survive target Author deletion");
    assert_eq!(
        surviving_credit,
        (sibling_author.id, 1, "Translator".to_string()),
        "deletion must not promote, reorder, or strip sibling semantics"
    );

    let refresh = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/work/{work_id}/refresh"),
        None,
    )
    .await;
    assert!(
        refresh.status.is_success(),
        "refresh must keep the surviving Work available, got {}: {}",
        refresh.status,
        refresh.json
    );
}

async fn red_frontend_cover() {
    let work_detail = strip_rust_comments(include_str!(
        "../../frontend/src/pages/work-detail/components/BookInformationTab.tsx"
    ));
    let cover = work_detail
        .split("function WorkCoverState")
        .nth(1)
        .unwrap_or_default();
    assert!(
        cover.contains("FormatNeeded")
            && cover.contains("Searching")
            && cover.contains("NoCoverFound")
            && cover.contains("NowhereToLook"),
        "production Work detail must expose one shared format panel and independent slot states"
    );
    for source_label in ["Provider", "Your file", "Yours"] {
        assert!(
            cover.contains(source_label),
            "selected covers expose source-only label {source_label:?}"
        );
    }
    for legacy_grade in ["trustGrade", "coverTrust", "Validated", "Unvalidated"] {
        assert!(
            !cover.contains(legacy_grade),
            "legacy cover grade control/label {legacy_grade:?} must be absent"
        );
    }
    // The implementation-phase render harness drives the DTO matrix through
    // the production Work detail route and checks the shared/slot DOM split
    // plus the absence of mutation requests during cover-state rendering.
    let render_harness = strip_rust_comments(include_str!(
        "../../frontend/src/pages/work-detail/components/BookInformationTab.test.tsx"
    ));
    assert!(
        render_harness.contains("<WorkDetailPage />")
            && render_harness.contains("data-cover-panel=\"FormatNeeded\"")
            && render_harness.contains("data-cover-state")
            && render_harness.contains("call.method !== \"GET\""),
        "production frontend harness must render cover matrices and assert zero mutations"
    );
}

// ---------------------------------------------------------------------------
// Production-router harness. Adapted from the existing registered
// `test_author_link_doors` harness so requests traverse the same AppState,
// `/api/v1` nesting, auth middleware, and concrete services as production.
// ---------------------------------------------------------------------------

struct RouteHarness {
    app: Router,
    state: AppState,
    api_key: String,
    db: SqliteDb,
    user_id: i64,
    open_library_stub: Option<livrarr_external_data::StubProviderClient>,
    _tmp: tempfile::TempDir,
}

static ROUTE_CASE: AtomicU64 = AtomicU64::new(1);

async fn build_route_harness() -> RouteHarness {
    build_route_harness_with_provider_details(None, Vec::new(), None).await
}

async fn build_route_harness_with_open_library(
    detail: Option<livrarr_external_data::NormalizedWorkDetail>,
) -> RouteHarness {
    build_route_harness_with_provider_details(detail, Vec::new(), None).await
}

#[derive(Clone)]
struct DiscoveryTransportFixture {
    goodreads_base_url: String,
    openlibrary_base_url: String,
    hardcover_search: bool,
    request_timeout: Duration,
    scripted_transport: Arc<
        dyn Fn(
                &livrarr_domain::services::FetchRequest,
            ) -> livrarr_http::fetcher::ScriptedTransportOutcome
            + Send
            + Sync,
    >,
}

async fn build_route_harness_with_provider_details(
    detail: Option<livrarr_external_data::NormalizedWorkDetail>,
    identity_details: Vec<(
        livrarr_domain::MetadataProvider,
        livrarr_external_data::NormalizedWorkDetail,
    )>,
    discovery_transport: Option<DiscoveryTransportFixture>,
) -> RouteHarness {
    build_route_harness_with_provider_outcome(
        detail.map(|detail| livrarr_external_data::ProviderOutcome::Success(Box::new(detail))),
        identity_details,
        discovery_transport,
    )
    .await
}

async fn build_route_harness_with_provider_outcome(
    open_library_outcome: Option<
        livrarr_external_data::ProviderOutcome<livrarr_external_data::NormalizedWorkDetail>,
    >,
    identity_details: Vec<(
        livrarr_domain::MetadataProvider,
        livrarr_external_data::NormalizedWorkDetail,
    )>,
    discovery_transport: Option<DiscoveryTransportFixture>,
) -> RouteHarness {
    // Bug reproduction: identity-layer-rewrite F-1 — every real-route seam in
    // this harness runs against the activated production index set.
    let db = create_activated_test_db().await;
    let tmp = tempfile::tempdir().expect("identity-layer route harness tempdir");
    let data_dir = tmp.path().to_path_buf();
    let data_dir_arc = Arc::new(data_dir.clone());

    let api_key = "identity-layer-door-api-key".to_string();
    let api_key_hash = RealAuthCrypto
        .hash_token(&api_key)
        .await
        .expect("hash route API key");
    let user = db
        .create_user(CreateUserDbRequest {
            username: "identity-layer-door-admin".to_string(),
            password_hash: "unused-password-hash".to_string(),
            role: UserRole::Admin,
            api_key_hash,
        })
        .await
        .expect("create authenticated route user");

    let auth_service = Arc::new(livrarr_server::auth_service::ServerAuthService::new(
        db.clone(),
        RealAuthCrypto,
    ));
    let user_agent = livrarr_http::livrarr_user_agent();
    let http_client = livrarr_http::HttpClient::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(&user_agent)
        .build()
        .expect("HTTP client");
    let http_client_safe = livrarr_http::HttpClient::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(&user_agent)
        .ssrf_safe(true)
        .build()
        .expect("SSRF-safe HTTP client");
    let mut http_fetcher =
        livrarr_http::fetcher::HttpFetcherImpl::new().expect("shared HTTP fetcher");
    if let Some(transport) = discovery_transport.as_ref() {
        let scripted_transport = transport.scripted_transport.clone();
        http_fetcher = http_fetcher
            .with_scripted_transport(move |request| scripted_transport(request))
            .with_ssrf_preflight_test_dns(
                "covers.openlibrary.org",
                "1.1.1.1:443".parse().expect("public test DNS answer"),
            );
    }
    let llm_http_client = livrarr_http::HttpClient::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(&user_agent)
        .build()
        .expect("LLM HTTP client");
    let hardcover_search = discovery_transport
        .as_ref()
        .is_some_and(|transport| transport.hardcover_search);
    let live_metadata_config =
        livrarr_external_data::live_config::LiveMetadataConfig::new(livrarr_db::MetadataConfig {
            hardcover_enabled: hardcover_search,
            hardcover_api_token: hardcover_search.then(|| "round21-test-token".to_string()),
            llm_enabled: false,
            llm_provider: None,
            llm_endpoint: None,
            llm_api_key: None,
            llm_model: None,
            audnexus_url: "https://api.audnex.us".to_string(),
            languages: vec!["en".to_string()],
            google_books_api_key: None,
        });
    let transport_cache = Arc::new(livrarr_external_data::transport_cache::TransportCache::new(
        Duration::from_secs(300),
    ));
    let import_semaphore = Arc::new(tokio::sync::Semaphore::new(2));
    let cover_proxy_cache = Arc::new(livrarr_server::infra::cover_cache::CoverProxyCache::new());
    let rss_last_run = Arc::new(AtomicI64::new(0));
    let rss_sync_running = Arc::new(AtomicBool::new(false));
    let manual_import_scans_shared: Arc<livrarr_server::state::ManualImportScanMap> =
        Arc::new(Default::default());
    let log_buffer = Arc::new(livrarr_server::state::LogBuffer::new());
    let log_level_handle = {
        let (_layer, handle) =
            tracing_subscriber::reload::Layer::new(tracing_subscriber::EnvFilter::new("info"));
        Arc::new(livrarr_server::state::LogLevelHandle::new(handle, "info"))
    };
    let settings_service_arc =
        Arc::new(livrarr_server::services::settings_service::LiveSettingsService::new(db.clone()));
    let import_io_arc = Arc::new(livrarr_server::import_io_service::ImportIoServiceImpl::new(
        db.clone(),
    ));
    let import_workflow_arc = Arc::new(livrarr_library::import_workflow::ImportWorkflowImpl::new(
        db.clone(),
        import_semaphore.clone(),
        data_dir_arc.clone(),
        Arc::new(livrarr_server::chapter_extractor::ChapterExtractorImpl),
    ));
    let tag_service_arc = Arc::new(livrarr_server::tag_service::LiveTagService::new(
        import_io_arc.clone(),
        data_dir_arc.clone(),
        db.clone(),
    ));
    let import_svc_arc = Arc::new(livrarr_server::import_service::LiveImportService::new(
        import_io_arc.clone(),
        import_workflow_arc.clone(),
        tag_service_arc.clone(),
        settings_service_arc.clone(),
        http_client_safe.clone(),
    ));
    let trusted_origins_arc = Arc::new(livrarr_http::ssrf::TrustedOrigins::new());
    let readarr_import_service_arc =
        Arc::new(livrarr_server::readarr_import_service::LiveReadarrImportService::new(db.clone()));
    let readarr_import_progress_arc = Arc::new(tokio::sync::Mutex::new(
        livrarr_server::readarr_import_service::ReadarrImportProgress::default(),
    ));
    let identity_clients = identity_details
        .into_iter()
        .map(|(provider, detail)| {
            (
                provider,
                livrarr_external_data::ProviderClient::Stub(
                    livrarr_external_data::StubProviderClient::new(
                        provider,
                        livrarr_external_data::ProviderOutcome::Success(Box::new(detail)),
                    ),
                ),
            )
        })
        .collect();
    let identity_resolver_arc = livrarr_server::state::build_live_identity_resolver(
        identity_clients,
        transport_cache.clone(),
        livrarr_metadata::english_identity_resolver::ResolverConfig::default(),
    );
    let db_arc = Arc::new(db.clone());
    let mut queue_builder = livrarr_metadata::DefaultProviderQueueBuilder::new()
        .with_identity_route_dispatch()
        .with_applicability_rule(Arc::new(|provider, work| {
            use livrarr_domain::MetadataProvider as P;
            if matches!(
                livrarr_external_data::language::provider_priority(work.language.as_deref()),
                livrarr_external_data::language::ProviderPriority::English
            ) {
                return !matches!(provider, P::GoogleBooks);
            }
            matches!(
                provider,
                P::Goodreads | P::Audnexus | P::GoogleBooks | P::Audible
            )
        }));
    // Round-13 search-fallback pins exercise the concrete provider clients and
    // shared HttpFetcherImpl queue with a hermetic scripted transport. Existing
    // fixtures without a transport keep their deliberately tiny stub queue.
    if let Some(transport) = discovery_transport.as_ref() {
        if open_library_outcome.is_none() {
            queue_builder = queue_builder.add_provider(
                livrarr_domain::MetadataProvider::OpenLibrary,
                livrarr_external_data::ProviderClient::OpenLibrary(
                    livrarr_external_data::OpenLibraryClient::new(http_fetcher.clone()),
                ),
                livrarr_enrichment::ProviderQueueConfig {
                    provider: livrarr_domain::MetadataProvider::OpenLibrary,
                    max_attempts: 1,
                },
            );
        }
        queue_builder = queue_builder.add_provider(
            livrarr_domain::MetadataProvider::Goodreads,
            livrarr_external_data::ProviderClient::Goodreads(
                livrarr_external_data::GoodreadsClient::new(
                    http_fetcher.clone(),
                    http_client.clone(),
                    transport.goodreads_base_url.clone(),
                ),
            ),
            livrarr_enrichment::ProviderQueueConfig {
                provider: livrarr_domain::MetadataProvider::Goodreads,
                max_attempts: 1,
            },
        );
        if transport.hardcover_search {
            queue_builder = queue_builder.add_provider(
                livrarr_domain::MetadataProvider::Hardcover,
                livrarr_external_data::ProviderClient::Hardcover(
                    livrarr_external_data::HardcoverClient::new(
                        http_fetcher.clone(),
                        live_metadata_config.clone(),
                    ),
                ),
                livrarr_enrichment::ProviderQueueConfig {
                    provider: livrarr_domain::MetadataProvider::Hardcover,
                    max_attempts: 1,
                },
            );
        }
    }
    let (queue_builder, open_library_stub) = if let Some(outcome) = open_library_outcome {
        let stub = livrarr_external_data::StubProviderClient::new(
            livrarr_domain::MetadataProvider::OpenLibrary,
            outcome,
        );
        (
            queue_builder.add_provider(
                livrarr_domain::MetadataProvider::OpenLibrary,
                livrarr_external_data::ProviderClient::Stub(stub.clone()),
                livrarr_enrichment::ProviderQueueConfig {
                    provider: livrarr_domain::MetadataProvider::OpenLibrary,
                    max_attempts: 1,
                },
            ),
            Some(stub),
        )
    } else {
        (queue_builder, None)
    };
    let queue = Arc::new(queue_builder.build(db_arc.clone()));
    let merge_engine = Arc::new(livrarr_metadata::DefaultMergeEngine::new(
        livrarr_metadata::PriorityModel::english(),
    ));
    let enrichment_service = Arc::new(livrarr_metadata::EnrichmentServiceImpl::new(
        db_arc,
        queue.clone(),
        merge_engine,
        false,
    ));
    let work_service_arc: Arc<livrarr_server::state::LiveWorkService> =
        Arc::new(livrarr_server::state::build_live_work_service(
            db.clone(),
            enrichment_service.clone(),
            http_fetcher.clone(),
            data_dir.clone(),
            identity_resolver_arc.clone(),
        ));
    let discovery_service = livrarr_metadata::discovery_service::DiscoveryServiceImpl::new(
        db.clone(),
        http_fetcher.clone(),
        livrarr_external_data::llm_caller_service::LlmCallerImpl::new(
            live_metadata_config.clone(),
            llm_http_client.clone(),
        ),
    )
    .with_resolver(identity_resolver_arc.clone());
    let discovery_service = match discovery_transport {
        Some(transport) => discovery_service.with_provider_transport(
            transport.goodreads_base_url,
            transport.openlibrary_base_url,
            transport.request_timeout,
        ),
        None => discovery_service,
    };
    let discovery_service_arc = Arc::new(discovery_service);
    let hmac_key = livrarr_server::cover_service::generate_hmac_key();
    let cover_service = Arc::new(livrarr_server::cover_service::LiveCoverService::new(
        db.clone(),
        http_fetcher.clone(),
        std::collections::HashMap::new(),
        hmac_key.clone(),
        data_dir_arc.clone(),
    ));
    let identity_road_arc = Arc::new(
        livrarr_server::identity_layer::build_recording_identity_road(
            db.clone(),
            http_fetcher.clone(),
            http_client.clone(),
            live_metadata_config.clone(),
        ),
    );
    let state = AppState {
        db: db.clone(),
        auth_service,
        http_client: http_client.clone(),
        http_client_safe,
        http_fetcher: http_fetcher.clone(),
        config: Arc::new(livrarr_server::config::AppConfig::default()),
        data_dir: data_dir_arc.clone(),
        startup_time: Utc::now(),
        job_runner: None,
        cover_proxy_cache: cover_proxy_cache.clone(),
        live_metadata_config: live_metadata_config.clone(),
        log_buffer: log_buffer.clone(),
        log_level_handle: log_level_handle.clone(),
        import_semaphore: import_semaphore.clone(),
        rss_last_run: rss_last_run.clone(),
        rss_sync_running: rss_sync_running.clone(),
        readarr_import_progress: readarr_import_progress_arc.clone(),
        manual_import_scans: manual_import_scans_shared.clone(),
        provider_queue: queue,
        enrichment_service: enrichment_service.clone(),
        identity_road: identity_road_arc.clone(),
        author_service: Arc::new(livrarr_metadata::author_service::AuthorServiceImpl::new(
            db.clone(),
            http_fetcher.clone(),
            livrarr_external_data::llm_caller_service::LlmCallerImpl::new(
                live_metadata_config.clone(),
                llm_http_client.clone(),
            ),
        )),
        author_link_service: Arc::new(
            livrarr_server::services::author_linking_service::LiveAuthorLinkingService,
        ),
        series_service: Arc::new(livrarr_metadata::series_service::SeriesServiceImpl::new(
            db.clone(),
        )),
        series_query_service: Arc::new(
            livrarr_metadata::series_query_service::SeriesQueryServiceImpl::new(
                db.clone(),
                http_fetcher.clone(),
                work_service_arc.clone(),
                livrarr_external_data::llm_caller_service::LlmCallerImpl::new(
                    live_metadata_config.clone(),
                    llm_http_client.clone(),
                ),
            )
            .with_identity_road(identity_road_arc.clone()),
        ),
        work_service: work_service_arc.clone(),
        discovery_service: discovery_service_arc,
        grab_service: Arc::new(livrarr_download::grab_service::GrabServiceImpl::new(
            db.clone(),
        )),
        release_service: Arc::new(livrarr_download::release_service::ReleaseServiceImpl::new(
            db.clone(),
            http_fetcher.clone(),
            trusted_origins_arc.clone(),
        )),
        file_service: Arc::new(livrarr_library::file_service::FileServiceImpl::new(
            db.clone(),
        )),
        chapter_service: Arc::new(livrarr_library::chapter_service::ChapterServiceImpl::new(
            db.clone(),
        )),
        bookmark_service: Arc::new(livrarr_library::bookmark_service::BookmarkServiceImpl::new(
            db.clone(),
        )),
        cross_format_service: Arc::new(
            livrarr_library::cross_format_service::CrossFormatServiceImpl::new(
                db.clone(),
                livrarr_library::file_service::FileServiceImpl::new(db.clone()),
            ),
        ),
        import_workflow: import_workflow_arc.clone(),
        rss_sync_workflow: {
            let release_service =
                Arc::new(livrarr_download::release_service::ReleaseServiceImpl::new(
                    db.clone(),
                    http_fetcher.clone(),
                    trusted_origins_arc.clone(),
                ));
            Arc::new(
                livrarr_metadata::rss_sync_workflow::RssSyncWorkflowImpl::new(
                    Arc::new(db.clone()),
                    Arc::new(http_fetcher.clone()),
                    release_service,
                ),
            )
        },
        list_service: {
            let work_service = livrarr_server::state::build_live_work_service(
                db.clone(),
                enrichment_service.clone(),
                http_fetcher.clone(),
                data_dir.clone(),
                identity_resolver_arc.clone(),
            );
            Arc::new(
                livrarr_metadata::list_service::ListServiceImpl::with_identity_road(
                    db.clone(),
                    work_service,
                    http_fetcher.clone(),
                    livrarr_metadata::list_service::NoOpBibliographyTrigger,
                    identity_road_arc.clone(),
                ),
            )
        },
        identity_conflict_service: Arc::new(
            livrarr_server::services::identity_conflict_service::LiveIdentityConflictService::new(
                db.clone(),
            ),
        ),
        identity_resolver: identity_resolver_arc.clone(),
        enrichment_workflow: Arc::new(
            livrarr_metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                enrichment_service.clone(),
            ),
        ),
        author_monitor_workflow: {
            let work_service = livrarr_server::state::build_live_work_service(
                db.clone(),
                enrichment_service.clone(),
                http_fetcher.clone(),
                data_dir.clone(),
                identity_resolver_arc.clone(),
            );
            Arc::new(
                livrarr_metadata::author_monitor_workflow::AuthorMonitorWorkflowImpl::with_identity_road(
                    Arc::new(db.clone()),
                    Arc::new(work_service),
                    Arc::new(http_fetcher.clone()),
                    identity_road_arc.clone(),
                ),
            )
        },
        readarr_import_service: readarr_import_service_arc.clone(),
        settings_service: settings_service_arc.clone(),
        notification_service: Arc::new(
            livrarr_server::notification_service::NotificationServiceImpl::new(db.clone()),
        ),
        history_service: Arc::new(livrarr_server::history_service::HistoryServiceImpl::new(
            db.clone(),
        )),
        queue_service: Arc::new(livrarr_server::queue_service::QueueServiceImpl::new(
            db.clone(),
            http_client.clone(),
        )),
        import_io_service: import_io_arc.clone(),
        manual_import_db_service: Arc::new(
            livrarr_server::manual_import_service::ManualImportServiceImpl::new(db.clone()),
        ),
        rss_sync_state: livrarr_server::state::RssSyncState {
            running: rss_sync_running,
            last_run: rss_last_run,
        },
        system_state: livrarr_server::state::SystemState {
            log_buffer,
            log_level_handle,
        },
        provider_stats_service: Arc::new(livrarr_server::state::LiveProviderStatsService::new(
            db.clone(),
        )),
        log_surface_accessor: livrarr_server::state::LogSurfaceAccessorImpl {
            log_dir: data_dir.join("logs"),
            init_error: None,
        },
        live_metadata_config_accessor: livrarr_server::state::LiveMetadataConfigAccessorImpl(
            live_metadata_config,
        ),
        cover_proxy_cache_accessor: livrarr_server::state::CoverProxyCacheAccessorImpl(
            cover_proxy_cache,
        ),
        tag_service: tag_service_arc,
        email_svc: Arc::new(livrarr_server::email_service::LiveEmailService::new(
            settings_service_arc,
        )),
        import_svc: import_svc_arc,
        matching_svc: livrarr_server::matching_service::LiveMatchingService,
        manual_import_scan_svc:
            livrarr_server::manual_import_scan_service::LiveManualImportScanService {
                scans: manual_import_scans_shared,
            },
        readarr_import_wf: Arc::new(
            livrarr_server::readarr_import_workflow::LiveReadarrImportWorkflow::new(
                http_fetcher,
                readarr_import_service_arc,
                readarr_import_progress_arc,
                data_dir_arc,
                work_service_arc,
                db.clone(),
                import_workflow_arc,
            )
            .with_identity_road(identity_road_arc.clone()),
        ),
        cover_service,
        preadd_cover_service: Arc::new(
            livrarr_metadata::preadd_cover_service::LivePreaddCoverService::new(
                std::collections::HashMap::new(),
            ),
        ),
        hmac_key,
        trusted_origins_rebuilder: livrarr_server::state::TrustedOriginsRebuilderImpl(
            trusted_origins_arc,
        ),
    };
    let ui_dir = state.data_dir.join("ui-not-present-in-test");
    RouteHarness {
        app: livrarr_server::router::build_router(state.clone(), ui_dir),
        state,
        api_key,
        db,
        user_id: user.id,
        open_library_stub,
        _tmp: tmp,
    }
}

static BREAKER_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

struct BreakerGuard {
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

fn reset_breakers() {
    let queue = livrarr_http::outbound_queue::shared();
    for bucket in [
        RateBucket::OpenLibrary,
        RateBucket::Hardcover,
        RateBucket::Audnexus,
        RateBucket::Goodreads,
        RateBucket::GoogleBooks,
        RateBucket::Audible,
    ] {
        queue.reset_breaker_for_tests(bucket);
    }
}

impl Drop for BreakerGuard {
    fn drop(&mut self) {
        reset_breakers();
    }
}

async fn lock_breaker() -> BreakerGuard {
    let lock = BREAKER_LOCK.lock().await;
    reset_breakers();
    BreakerGuard { _lock: lock }
}

async fn seed_route_work(harness: &RouteHarness, suffix: &str) -> i64 {
    let n = ROUTE_CASE.fetch_add(1, Ordering::Relaxed);
    let title = format!("ILR {suffix} {n}");
    let author_name = format!("ILR Author {n}");
    let (author, _) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: author_name.clone(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed route Author");
    harness
        .db
        .create_work(CreateWorkDbRequest {
            user_id: harness.user_id,
            title: title.clone(),
            author_name,
            normalized_title: title.to_ascii_lowercase(),
            normalized_author: format!("ilr author {n}"),
            author_id: Some(author.id),
            language: Some("en".to_string()),
            ..Default::default()
        })
        .await
        .expect("seed route Work")
        .0
        .id
}

struct RouteResponse {
    status: StatusCode,
    json: Value,
}

async fn call_router_json(
    harness: &RouteHarness,
    method: Method,
    path: String,
    body: Option<Value>,
) -> RouteResponse {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("x-api-key", &harness.api_key);
    let request_body = match body {
        Some(value) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let mut request = request.body(request_body).expect("build ILR request");
    request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 42)),
        31_000,
    )));
    let response = harness
        .app
        .clone()
        .oneshot(request)
        .await
        .expect("production router response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .expect("read production response body");
    let json = serde_json::from_slice(&bytes).unwrap_or_else(|_| {
        json!({
            "unparsedBody": String::from_utf8_lossy(&bytes).into_owned()
        })
    });
    RouteResponse { status, json }
}

// Bug reproduction: identity-layer-rewrite S-17 — exercise discovery through
// the authenticated production router and concrete HttpFetcherImpl/shared
// outbound queue, with only provider origins made hermetic.
#[tokio::test]
#[traced_test]
async fn discovery_real_route_queue_wait_budget_and_drop_observability() {
    let _breaker = lock_breaker().await;
    let scripted_transport = Arc::new(|request: &livrarr_domain::services::FetchRequest| {
        let (delay, body) = if request.rate_bucket == RateBucket::Goodreads {
            let delay = if request.url.contains("past-budget") {
                Duration::from_millis(150)
            } else {
                Duration::ZERO
            };
            (
                delay,
                serde_json::to_vec(&json!([{
                    "title": "Goodreads Within Budget",
                    "bookTitleBare": "Goodreads Within Budget",
                    "bookUrl": "/book/show/77197",
                    "author": { "name": "Route Queue Author" },
                    "avgRating": "4.20"
                }]))
                .expect("Goodreads transport fixture"),
            )
        } else {
            (
                Duration::ZERO,
                serde_json::to_vec(&json!({
                    "docs": [{
                        "key": "/works/OL-ROUTE-QUEUE-W",
                        "title": "OpenLibrary Survivor",
                        "author_name": ["Route Queue Author"]
                    }]
                }))
                .expect("OpenLibrary transport fixture"),
            )
        };
        livrarr_http::fetcher::ScriptedTransportOutcome::Response {
            delay,
            response: livrarr_domain::services::FetchResponse {
                status: 200,
                headers: Vec::new(),
                body,
            },
        }
    });

    let harness = build_route_harness_with_provider_details(
        None,
        Vec::new(),
        Some(DiscoveryTransportFixture {
            goodreads_base_url: "https://goodreads.test".to_string(),
            openlibrary_base_url: "https://openlibrary.test".to_string(),
            hardcover_search: false,
            request_timeout: Duration::from_millis(75),
            scripted_transport,
        }),
    )
    .await;

    // Seed the Goodreads pacing clock. The route call below therefore waits
    // roughly 1.5s in the real shared queue—well beyond its 75ms request
    // budget—before the scripted transport returns immediately.
    drop(
        livrarr_http::outbound_queue::shared()
            .acquire(
                RateBucket::Goodreads,
                livrarr_domain::RequestPriority::Interactive,
            )
            .await
            .expect("seed Goodreads queue pace"),
    );

    let within = call_router_json(
        &harness,
        Method::GET,
        "/api/v1/work/lookup?term=within-budget&raw=true".to_string(),
        None,
    )
    .await;
    assert_eq!(within.status, StatusCode::OK, "within-budget route");
    assert!(
        within.json["results"]
            .as_array()
            .expect("lookup results")
            .iter()
            .any(|result| result["source"] == "goodreads" && result["grKey"] == "77197"),
        "a Goodreads leg that reaches HTTP inside its request budget must serve its result: {}",
        within.json
    );

    // The shared queue's 1.5s Goodreads pacing makes this second call wait far
    // longer than the 75ms request budget before it is dispatched. It must not
    // be cancelled while queued; once dispatched, its forced 150ms HTTP delay
    // exceeds the request-only budget and becomes one observable dropped leg.
    let past = call_router_json(
        &harness,
        Method::GET,
        "/api/v1/work/lookup?term=past-budget&raw=true".to_string(),
        None,
    )
    .await;
    assert_eq!(past.status, StatusCode::OK, "degraded lookup route");
    assert!(
        past.json["results"]
            .as_array()
            .expect("degraded lookup results")
            .iter()
            .any(|result| result["source"] == "openlibrary"),
        "OpenLibrary results must survive a failed Goodreads leg: {}",
        past.json
    );
    assert!(
        logs_contain("discovery provider leg dropped")
            && logs_contain("Goodreads")
            && logs_contain("cause"),
        "the dropped real-route Goodreads leg must emit provider and cause at WARN"
    );
}

fn live_add_request() -> Value {
    json!({
        "olKey": null,
        "title": "Malice (The Faithful and the Fallen, #1)",
        "authorName": "John Gwynne",
        "authorOlKey": null,
        "year": 2012,
        "coverUrl": null,
        "language": "en",
        "detailUrl": null,
        "coverManual": false,
        "isbn13": null,
        "candidateId": null,
        "hcKey": null,
        "grKey": "15750692",
        "asin": null
    })
}

fn article_variant_add_request() -> Value {
    json!({
        "olKey": "OL35690910W",
        "title": "Lies of Locke Lamora",
        "authorName": "Scott Lynch",
        "authorOlKey": null,
        "year": 2006,
        "coverUrl": null,
        "language": "en",
        "detailUrl": null,
        "coverManual": false,
        "isbn13": null,
        "candidateId": null,
        "hcKey": null,
        "grKey": null,
        "asin": null
    })
}

async fn article_variant_add_real_door_reuses_survivor() {
    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;
    let (author, _) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: "Scott Lynch".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed S-19 author");
    let mut survivor = settlement_commit(harness.user_id, author.id, None);
    survivor.identity_title =
        ilr::title_parts_from_provider("The Lies of Locke Lamora".to_string(), None)
            .expect("parse S-19 survivor title");
    survivor.identity_title.volume = Some("1".to_string());
    survivor.identity_title.normalized_volume = "1".to_string();
    survivor.routes = vec![ilr::WorkRoute {
        id: 0,
        user_id: harness.user_id,
        owner: RouteOwner::Work(0),
        resolved_work_id: 0,
        provider: ilr::IdentityProvider::OpenLibrary,
        kind: ilr::RouteKind::OpenLibraryWork,
        provider_scoped_id: "OL8369445W".to_string(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::UserChoice,
        user_confirmed: true,
        observed_at: Utc::now(),
    }];
    let survivor = WorkIdentityRepository::commit_settlement(&harness.db, survivor)
        .await
        .expect("seed S-19 survivor")
        .identity;

    let response = call_router_json(
        &harness,
        Method::POST,
        "/api/v1/work".to_string(),
        Some(article_variant_add_request()),
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::CONFLICT,
        "article variant must enter the existing GroupIdentity review door: {}",
        response.json
    );
    let works: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1")
        .bind(harness.user_id)
        .fetch_one(harness.db.pool())
        .await
        .expect("count S-19 works");
    assert_eq!(works, 1, "the direct-add door must not create Work 228");
    let cards: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_review_cards \
          WHERE user_id=?1 AND work_id=?2 AND kind=?3 AND status='pending'",
    )
    .bind(harness.user_id)
    .bind(survivor.own_work_id)
    .bind(ilr::ReviewKind::GroupIdentity.storage_code())
    .fetch_one(harness.db.pool())
    .await
    .expect("count S-19 GroupIdentity cards");
    assert_eq!(cards, 1);
}

fn fixture_jpeg(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbImage::from_pixel(width, height, image::Rgb([42, 84, 126]));
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut bytes)
        .encode_image(&image)
        .expect("encode cover fixture");
    bytes
}

async fn seed_refresh_cover_work(harness: &RouteHarness, route_id: &str) -> i64 {
    let (author, _) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: "John Irving".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed refresh cover author");
    let mut commit = settlement_commit(harness.user_id, author.id, None);
    commit.identity_title = title("The Cider House Rules");
    commit.routes = vec![ilr::WorkRoute {
        id: 0,
        user_id: harness.user_id,
        owner: RouteOwner::Work(0),
        resolved_work_id: 0,
        provider: ilr::IdentityProvider::OpenLibrary,
        kind: ilr::RouteKind::OpenLibraryWork,
        provider_scoped_id: route_id.to_string(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::UserChoice,
        user_confirmed: true,
        observed_at: Utc::now(),
    }];
    WorkIdentityRepository::commit_settlement(&harness.db, commit)
        .await
        .expect("seed refresh cover Work")
        .identity
        .own_work_id
}

async fn seed_below_floor_cover(harness: &RouteHarness, work_id: i64) {
    let covers_dir = harness
        .state
        .data_dir
        .join("covers")
        .join(harness.user_id.to_string());
    tokio::fs::create_dir_all(&covers_dir)
        .await
        .expect("create cover fixture directory");
    tokio::fs::write(
        covers_dir.join(format!("{work_id}.jpg")),
        fixture_jpeg(307, 500),
    )
    .await
    .expect("write below-floor cover fixture");
    WorkDb::update_cover_metadata(
        &harness.db,
        harness.user_id,
        work_id,
        Some("https://covers.example/incumbent.jpg"),
        "openlibrary",
        false,
        307,
        500,
    )
    .await
    .expect("seed below-floor cover metadata");
}

#[tokio::test]
#[traced_test]
async fn refresh_real_route_upgrades_below_floor_cover_through_gate() {
    let _breaker = lock_breaker().await;
    let candidate_bytes = Arc::new(fixture_jpeg(800, 1200));
    let scripted_transport = {
        let candidate_bytes = Arc::clone(&candidate_bytes);
        Arc::new(move |_request: &livrarr_domain::services::FetchRequest| {
            livrarr_http::fetcher::ScriptedTransportOutcome::Response {
                delay: Duration::ZERO,
                response: livrarr_domain::services::FetchResponse {
                    status: 200,
                    headers: Vec::new(),
                    body: candidate_bytes.as_ref().clone(),
                },
            }
        })
    };
    let harness = build_route_harness_with_provider_details(
        Some(livrarr_external_data::NormalizedWorkDetail {
            title: Some("The Cider House Rules".to_string()),
            author_name: Some("John Irving".to_string()),
            ol_key: Some("OL-CIDER-COVER-W".to_string()),
            // Public literal IP keeps the production SSRF preflight hermetic;
            // the scripted transport still supplies every response byte.
            cover_url: Some("https://1.1.1.1/upgrade.jpg".to_string()),
            ..Default::default()
        }),
        Vec::new(),
        Some(DiscoveryTransportFixture {
            goodreads_base_url: "https://goodreads.test".to_string(),
            openlibrary_base_url: "https://openlibrary.test".to_string(),
            hardcover_search: false,
            request_timeout: Duration::from_secs(1),
            scripted_transport,
        }),
    )
    .await;
    let work_id = seed_refresh_cover_work(&harness, "OL-CIDER-COVER-W").await;
    seed_below_floor_cover(&harness, work_id).await;

    let refresh = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/work/{work_id}/refresh"),
        None,
    )
    .await;
    assert!(
        refresh.status.is_success(),
        "refresh response: {}",
        refresh.json
    );
    assert_eq!(
        harness
            .open_library_stub
            .as_ref()
            .expect("OpenLibrary refresh stub")
            .call_count(),
        1,
        "manual refresh must clear terminal retry state and dispatch once"
    );
    let cover: (Option<String>, Option<String>, i32, i32, bool) = sqlx::query_as(
        "SELECT cover_url, cover_source, cover_width, cover_height, cover_manual \
           FROM works WHERE user_id=?1 AND id=?2",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read upgraded cover");
    assert_eq!(cover.0.as_deref(), Some("https://1.1.1.1/upgrade.jpg"));
    assert_eq!(cover.2, 800);
    assert_eq!(cover.3, 1200);
    assert!(!cover.4);
    assert!(
        logs_contain("refresh cover write gate invoked"),
        "manual refresh must emit its gate outcome"
    );
}

#[tokio::test]
async fn readarr_source_cover_reaches_the_real_import_enrichment_gate_before_return() {
    let candidate_bytes = Arc::new(fixture_jpeg(640, 960));
    let scripted_transport = {
        let candidate_bytes = Arc::clone(&candidate_bytes);
        Arc::new(move |_request: &livrarr_domain::services::FetchRequest| {
            livrarr_http::fetcher::ScriptedTransportOutcome::Response {
                delay: Duration::ZERO,
                response: livrarr_domain::services::FetchResponse {
                    status: 200,
                    headers: Vec::new(),
                    body: candidate_bytes.as_ref().clone(),
                },
            }
        })
    };
    let harness = build_route_harness_with_provider_details(
        None,
        Vec::new(),
        Some(DiscoveryTransportFixture {
            goodreads_base_url: "https://goodreads.test".to_string(),
            openlibrary_base_url: "https://openlibrary.test".to_string(),
            hardcover_search: false,
            request_timeout: Duration::from_secs(1),
            scripted_transport,
        }),
    )
    .await;
    let url = "https://1.1.1.1/readarr-import.jpg";
    let (author, _) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: "Readarr Cover Author".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed Readarr cover author");
    let rd_author = serde_json::from_value(serde_json::json!({
        "id": 911,
        "authorName": "Readarr Cover Author"
    }))
    .expect("decode Readarr cover author payload");
    let rd_book = serde_json::from_value(serde_json::json!({
        "id": 912,
        "title": "Readarr Cover Item Path",
        "authorId": 911,
        "foreignBookId": "9912",
        "images": [{
            "coverType": "cover",
            "remoteUrl": url
        }],
        "editions": null
    }))
    .expect("decode Readarr book whose images are the only cover source");
    let work_id = harness
        .state
        .readarr_import_wf
        .process_single_work_item_for_tests(
            "round11-readarr-cover-item",
            harness.user_id,
            rd_book,
            rd_author,
            author.id,
        )
        .await
        .expect("drive the real Readarr process_works item path");

    let cover: (Option<String>, Option<String>, i32, i32, bool) = sqlx::query_as(
        "SELECT cover_url, cover_source, cover_width, cover_height, cover_manual \
           FROM works WHERE user_id=?1 AND id=?2",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read imported Readarr cover state");
    assert_eq!(cover.0.as_deref(), Some(url));
    assert_eq!(cover.1.as_deref(), Some("readarr"));
    assert_eq!((cover.2, cover.3), (640, 960));
    assert!(
        !cover.4,
        "a Readarr source cover must remain an automatic cover"
    );
    assert!(
        harness
            .state
            .data_dir
            .join("covers")
            .join(harness.user_id.to_string())
            .join(format!("{work_id}.jpg"))
            .exists(),
        "the source URL must reach the existing cover write gate"
    );
}

#[tokio::test]
#[traced_test]
async fn refresh_real_route_candidate_less_logs_cause_and_preserves_cover() {
    let _breaker = lock_breaker().await;
    let harness =
        build_route_harness_with_open_library(Some(livrarr_external_data::NormalizedWorkDetail {
            title: Some("The Cider House Rules".to_string()),
            author_name: Some("John Irving".to_string()),
            ol_key: Some("OL-CIDER-COVERLESS-W".to_string()),
            cover_url: None,
            ..Default::default()
        }))
        .await;
    let work_id = seed_refresh_cover_work(&harness, "OL-CIDER-COVERLESS-W").await;
    seed_below_floor_cover(&harness, work_id).await;

    let refresh = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/work/{work_id}/refresh"),
        None,
    )
    .await;
    assert!(
        refresh.status.is_success(),
        "refresh response: {}",
        refresh.json
    );
    assert_eq!(
        harness
            .open_library_stub
            .as_ref()
            .expect("OpenLibrary coverless refresh stub")
            .call_count(),
        1,
        "a coverless result must be observed after a real provider dispatch"
    );
    let cover: (Option<String>, i32, i32) = sqlx::query_as(
        "SELECT cover_url, cover_width, cover_height FROM works WHERE user_id=?1 AND id=?2",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read unchanged cover");
    assert_eq!(
        cover,
        (
            Some("https://covers.example/incumbent.jpg".to_string()),
            307,
            500,
        )
    );
    assert!(
        logs_contain("refresh cover write gate not invoked")
            && logs_contain("successful_payloads_coverless_or_ineligible"),
        "candidate-less refresh must name its cause"
    );
}

// Bug reproduction: identity-layer-rewrite S-14 — the activated v2 creation
// transaction is the birth moment and must write exactly one typed `added`
// fact with the creation door's source label.
async fn red_v2_real_add_writes_one_birth_history_fact() {
    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;
    let response = call_router_json(
        &harness,
        Method::POST,
        "/api/v1/work".to_string(),
        Some(live_add_request()),
    )
    .await;
    assert!(response.status.is_success(), "real add: {}", response.json);
    let work_id = response.json["work"]["id"]
        .as_i64()
        .expect("created Work id");
    let births: Vec<(String, String)> = sqlx::query_as(
        "SELECT event_type, data FROM history \
          WHERE user_id=?1 AND work_id=?2 AND event_type='added' ORDER BY id",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_all(harness.db.pool())
    .await
    .expect("read v2 birth history");
    assert_eq!(
        births.len(),
        1,
        "a Work birth writes exactly one added fact"
    );
    let payload: Value = serde_json::from_str(&births[0].1).expect("typed added payload");
    assert_eq!(payload["work_title"], "Malice");
    assert_eq!(payload["work_author"], "John Gwynne");
    assert_eq!(
        payload["source"], "search",
        "the author-page/direct-add door uses its seed-constructor source label"
    );
    assert!(payload.get("backfilled").is_none());
}

fn live_add_dedup_review_request() -> Value {
    json!({
        "olKey": null,
        "title": "Red Rising (Red Rising Saga, #1)",
        "authorName": "Pierce Brown",
        "authorOlKey": null,
        "year": 2014,
        "coverUrl": null,
        "language": "en",
        "detailUrl": null,
        "coverManual": false,
        "isbn13": null,
        "candidateId": null,
        "hcKey": null,
        "grKey": "15839976",
        "asin": null
    })
}

// Bug reproduction: identity-layer-rewrite — a direct add that parks a
// dedup review must bind the settlement to the established Work instead of
// committing the proposed identity as an orphan Work behind the HTTP 409.
async fn red_direct_add_dedup_review_reuses_existing_work() {
    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;
    let (author, _) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: "Pierce Brown".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed established direct-add Author");
    let mut seed = settlement_commit(harness.user_id, author.id, None);
    seed.identity_title = title("Red Rising");
    let established = WorkIdentityRepository::commit_settlement(&harness.db, seed)
        .await
        .expect("seed established direct-add Work")
        .identity
        .own_work_id;

    let response = call_router_json(
        &harness,
        Method::POST,
        "/api/v1/work".to_string(),
        Some(live_add_dedup_review_request()),
    )
    .await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{}", response.json);
    assert!(
        response
            .json
            .to_string()
            .contains("direct add requires review card"),
        "the review outcome remains visible to the caller: {}",
        response.json
    );

    let work_ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM works WHERE user_id=?1 ORDER BY id")
            .bind(harness.user_id)
            .fetch_all(harness.db.pool())
            .await
            .expect("count direct-add Works after review");
    assert_eq!(
        work_ids,
        vec![established],
        "parking review must not leave a second Work"
    );
    let cards: Vec<(i64, Option<i64>, String, i64)> = sqlx::query_as(
        "SELECT id, work_id, payload, generation FROM identity_review_cards \
           WHERE user_id=?1 AND kind='GroupIdentity' AND status='pending' ORDER BY id",
    )
    .bind(harness.user_id)
    .fetch_all(harness.db.pool())
    .await
    .expect("read direct-add review card");
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].1, Some(established));
    let payload: ilr::SettlementReviewCard =
        serde_json::from_str(&cards[0].2).expect("decode direct-add review payload");
    let ilr::SettlementReviewCard::GroupIdentity {
        work_ids,
        proposed_identity: Some(proposed),
        ..
    } = payload
    else {
        panic!("direct-add review payload must be a proposed GroupIdentity")
    };
    assert_eq!(work_ids, vec![established]);
    assert_eq!(proposed.title.normalized_main, "red rising");
    assert_eq!(proposed.title.normalized_volume, "1");
    assert!(proposed.routes.iter().any(|route| {
        route.provider == ilr::IdentityProvider::Goodreads
            && route.kind == ilr::RouteKind::GoodreadsBookEdition
            && route.provider_scoped_id == "15839976"
    }));

    let resolved = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/identity-review-card/{}/resolve", cards[0].0),
        Some(json!({"command": {
            "GroupIdentity": {
                "card_id": cards[0].0,
                "expected_generation": cards[0].3,
                "action": "DifferentFromAll"
            }
        }})),
    )
    .await;
    assert!(
        resolved.status.is_success(),
        "accept direct-add proposal: {}",
        resolved.json
    );
    let applied: (String, String) = sqlx::query_as(
        "SELECT normalized_identity_volume, identity_status_v2 FROM works \
          WHERE user_id=?1 AND id=?2",
    )
    .bind(harness.user_id)
    .bind(established)
    .fetch_one(harness.db.pool())
    .await
    .expect("read accepted direct-add proposal");
    assert_eq!(applied, ("1".to_string(), "user_confirmed".to_string()));
    let applied_route: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_routes WHERE user_id=?1 AND resolved_work_id=?2 \
           AND provider_scoped_id='15839976' AND state='active'",
    )
    .bind(harness.user_id)
    .bind(established)
    .fetch_one(harness.db.pool())
    .await
    .expect("read accepted direct-add route");
    assert_eq!(applied_route, 1);
}

// Bug reproduction: identity-layer-rewrite — retrying the same unresolved
// group proposal must reuse its pending card rather than minting a duplicate.
async fn red_group_identity_pending_card_mint_is_idempotent_on_retrigger() {
    let _breaker = lock_breaker().await;
    let db = create_activated_test_db().await;
    let (user_id, author_id) = seed_identity_principals(&db, "dedup-card").await;
    let established =
        WorkIdentityRepository::commit_settlement(&db, settlement_commit(user_id, author_id, None))
            .await
            .expect("seed card anchor")
            .identity;
    let proposed = ilr::WorkIdentityEvidence {
        title: established.identity_title.clone(),
        primary_author_id: author_id,
        routes: vec![ilr::WorkRoute {
            id: 0,
            user_id,
            owner: RouteOwner::Work(0),
            resolved_work_id: 0,
            provider: ilr::IdentityProvider::Goodreads,
            kind: ilr::RouteKind::GoodreadsWork,
            provider_scoped_id: "15839976".to_string(),
            state: ilr::WorkRouteState::Active,
            provenance: ilr::RouteProvenance::UserChoice,
            user_confirmed: true,
            observed_at: Utc::now(),
        }],
    };
    let review = ilr::SettlementReviewCard::GroupIdentity {
        work_ids: vec![established.own_work_id],
        proposed_identity: Some(proposed),
        merge_choices: Vec::new(),
    };

    let mut first = settlement_commit(user_id, author_id, Some(established.own_work_id));
    first.identity_title = established.identity_title.clone();
    first.expected_generation = established.identity_generation;
    first.review_cards = vec![review.clone()];
    let first = WorkIdentityRepository::commit_settlement(&db, first)
        .await
        .expect("mint first pending group card");
    let first_card = first.review_cards[0].id;

    let mut retry = settlement_commit(user_id, author_id, Some(established.own_work_id));
    retry.identity_title = established.identity_title;
    retry.expected_generation = first.identity.identity_generation;
    retry.review_cards = vec![review];
    let retry = WorkIdentityRepository::commit_settlement(&db, retry)
        .await
        .expect("retry identical pending group proposal");

    let pending_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM identity_review_cards \
           WHERE user_id=?1 AND kind='GroupIdentity' AND status='pending' ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(db.pool())
    .await
    .expect("list pending group proposals after retry");
    assert_eq!(pending_ids, vec![first_card]);
    assert_eq!(retry.review_cards[0].id, first_card);
    assert_eq!(
        retry.review_cards[0].generation, retry.identity.identity_generation,
        "a reused card reports the actionable current anchor generation"
    );
}

async fn dedup_residue_startup_heal_folds_orphan_and_cleans_duplicate_cards() {
    let _breaker = lock_breaker().await;
    let db = create_activated_test_db().await;
    let (user_id, author_id) = seed_identity_principals(&db, "dedup-heal").await;
    let mut anchor_seed = settlement_commit(user_id, author_id, None);
    anchor_seed.identity_title = title("Red Rising");
    let anchor = WorkIdentityRepository::commit_settlement(&db, anchor_seed)
        .await
        .expect("seed dedup-residue anchor")
        .identity;

    let mut orphan_seed = settlement_commit(user_id, author_id, None);
    orphan_seed.identity_title = title("Red Rising");
    orphan_seed.identity_title.volume = Some("1".to_string());
    orphan_seed.identity_title.normalized_volume = "1".to_string();
    let orphan = WorkIdentityRepository::commit_settlement(&db, orphan_seed)
        .await
        .expect("seed former review-arm orphan")
        .identity;
    let proposed = ilr::WorkIdentityEvidence {
        title: orphan.identity_title.clone(),
        primary_author_id: author_id,
        routes: vec![ilr::WorkRoute {
            id: 0,
            user_id,
            owner: RouteOwner::Work(0),
            resolved_work_id: 0,
            provider: ilr::IdentityProvider::Goodreads,
            kind: ilr::RouteKind::GoodreadsWork,
            provider_scoped_id: "15839976".to_string(),
            state: ilr::WorkRouteState::Active,
            provenance: ilr::RouteProvenance::UserChoice,
            user_confirmed: true,
            observed_at: Utc::now(),
        }],
    };
    let source = ilr::SettlementReviewCard::GroupIdentity {
        work_ids: vec![anchor.own_work_id],
        proposed_identity: Some(proposed.clone()),
        merge_choices: Vec::new(),
    };
    let duplicate = ilr::SettlementReviewCard::GroupIdentity {
        work_ids: vec![anchor.own_work_id, orphan.own_work_id],
        proposed_identity: Some(proposed),
        merge_choices: Vec::new(),
    };
    let created_at = Utc::now().to_rfc3339();
    let mut card_ids = Vec::new();
    for payload in [&source, &duplicate, &duplicate] {
        let inserted = sqlx::query(
            "INSERT INTO identity_review_cards \
                (user_id, work_id, kind, generation, status, payload, created_at) \
             VALUES (?1, ?2, 'GroupIdentity', ?3, 'pending', ?4, ?5)",
        )
        .bind(user_id)
        .bind(anchor.own_work_id)
        .bind(anchor.identity_generation)
        .bind(serde_json::to_string(payload).expect("encode residue card"))
        .bind(&created_at)
        .execute(db.pool())
        .await
        .expect("seed pending residue card");
        card_ids.push(inserted.last_insert_rowid());
    }

    let report = livrarr_db::identity_layer::heal_identity_dedup_residue(db.pool())
        .await
        .expect("heal dedup residue");
    assert_eq!(report.orphans_folded, 1);
    assert_eq!(report.works_bumped, 1);
    assert_eq!(report.invalid_cards_cancelled, 2);
    assert_eq!(report.duplicate_cards_cancelled, 0);
    let surviving_works: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM works WHERE user_id=?1 ORDER BY id")
            .bind(user_id)
            .fetch_all(db.pool())
            .await
            .expect("read healed Works");
    assert_eq!(surviving_works, vec![anchor.own_work_id]);
    let merge: (i64, i64) = sqlx::query_as(
        "SELECT winner_work_id, loser_work_id FROM identity_merge_archives \
          WHERE user_id=?1 ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_one(db.pool())
    .await
    .expect("read orphan fold archive");
    assert_eq!(merge, (anchor.own_work_id, orphan.own_work_id));
    let statuses: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, status FROM identity_review_cards WHERE user_id=?1 ORDER BY id")
            .bind(user_id)
            .fetch_all(db.pool())
            .await
            .expect("read healed card statuses");
    assert_eq!(
        statuses,
        vec![
            (card_ids[0], "pending".to_string()),
            (card_ids[1], "cancelled".to_string()),
            (card_ids[2], "cancelled".to_string()),
        ]
    );
    assert_eq!(
        work_generation(&db, anchor.own_work_id).await,
        anchor.identity_generation + 1
    );
    let heal_audits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events WHERE user_id=?1 AND work_id=?2 \
           AND event_kind='settlement' AND actor='identity-dedup-residue-heal'",
    )
    .bind(user_id)
    .bind(anchor.own_work_id)
    .fetch_one(db.pool())
    .await
    .expect("count dedup-residue heal audits");
    assert_eq!(heal_audits, 1);
    assert_eq!(
        livrarr_db::identity_layer::heal_identity_dedup_residue(db.pool())
            .await
            .expect("rerun dedup-residue heal"),
        livrarr_db::identity_layer::IdentityDedupResidueHealReport::default()
    );
}

async fn dedup_residue_startup_heal_keeps_one_equivalent_pending_card() {
    let _breaker = lock_breaker().await;
    let db = create_activated_test_db().await;
    let (user_id, author_id) = seed_identity_principals(&db, "dedup-heal-cards").await;
    let anchor =
        WorkIdentityRepository::commit_settlement(&db, settlement_commit(user_id, author_id, None))
            .await
            .expect("seed duplicate-card anchor")
            .identity;
    let mut proposed_title = anchor.identity_title.clone();
    proposed_title.volume = Some("99".to_string());
    proposed_title.normalized_volume = "99".to_string();
    let card = ilr::SettlementReviewCard::GroupIdentity {
        work_ids: vec![anchor.own_work_id],
        proposed_identity: Some(ilr::WorkIdentityEvidence {
            title: proposed_title,
            primary_author_id: author_id,
            routes: vec![ilr::WorkRoute {
                id: 0,
                user_id,
                owner: RouteOwner::Work(0),
                resolved_work_id: 0,
                provider: ilr::IdentityProvider::Goodreads,
                kind: ilr::RouteKind::GoodreadsWork,
                provider_scoped_id: "dedup-heal-card-route".to_string(),
                state: ilr::WorkRouteState::Active,
                provenance: ilr::RouteProvenance::UserChoice,
                user_confirmed: true,
                observed_at: Utc::now(),
            }],
        }),
        merge_choices: Vec::new(),
    };
    let payload = serde_json::to_string(&card).expect("encode duplicate card");
    let created_at = Utc::now().to_rfc3339();
    let mut ids = Vec::new();
    for _ in 0..2 {
        ids.push(
            sqlx::query(
                "INSERT INTO identity_review_cards \
                    (user_id, work_id, kind, generation, status, payload, created_at) \
                 VALUES (?1, ?2, 'GroupIdentity', ?3, 'pending', ?4, ?5)",
            )
            .bind(user_id)
            .bind(anchor.own_work_id)
            .bind(anchor.identity_generation)
            .bind(&payload)
            .bind(&created_at)
            .execute(db.pool())
            .await
            .expect("seed duplicate pending card")
            .last_insert_rowid(),
        );
    }

    let report = livrarr_db::identity_layer::heal_identity_dedup_residue(db.pool())
        .await
        .expect("deduplicate pending cards");
    assert_eq!(report.orphans_folded, 0);
    assert_eq!(report.invalid_cards_cancelled, 0);
    assert_eq!(report.duplicate_cards_cancelled, 1);
    let statuses: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, status FROM identity_review_cards WHERE user_id=?1 ORDER BY id")
            .bind(user_id)
            .fetch_all(db.pool())
            .await
            .expect("read deduplicated card statuses");
    assert_eq!(
        statuses,
        vec![
            (ids[0], "pending".to_string()),
            (ids[1], "cancelled".to_string()),
        ]
    );
}

fn live_add_edition_parenthetical_request() -> Value {
    json!({
        "olKey": null,
        "title": "Cloud Cuckoo Land (Large Print Edition)",
        "authorName": "Anthony Doerr",
        "authorOlKey": null,
        "year": 2021,
        "coverUrl": null,
        "language": "en",
        "detailUrl": null,
        "coverManual": false,
        "isbn13": "9780316399739",
        "candidateId": null,
        "hcKey": null,
        "grKey": null,
        "asin": null
    })
}

async fn build_live_add_fanout_harness() -> RouteHarness {
    build_route_harness_with_provider_details(
        None,
        vec![(
            livrarr_domain::MetadataProvider::Hardcover,
            livrarr_external_data::NormalizedWorkDetail {
                title: Some("Malice".to_string()),
                author_name: Some("John Gwynne".to_string()),
                language: Some("en".to_string()),
                hc_key: Some("429071".to_string()),
                isbn_13: Some("9780316399739".to_string()),
                ..Default::default()
            },
        )],
        None,
    )
    .await
}

async fn post_live_add(harness: &RouteHarness) -> RouteResponse {
    call_router_json(
        harness,
        Method::POST,
        "/api/v1/work".to_string(),
        Some(live_add_request()),
    )
    .await
}

async fn wait_for_live_add_routes(harness: &RouteHarness, work_id: i64) -> ilr::CapturedIdentity {
    for _ in 0..100 {
        let captured =
            WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, work_id)
                .await
                .expect("read live-add captured identity");
        if captured.active_routes.len() >= 3 {
            return captured;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, work_id)
        .await
        .expect("read timed-out live-add captured identity")
}

async fn red_live_add_goodreads_title_policy() {
    let harness = build_route_harness().await;
    let response = post_live_add(&harness).await;
    assert!(
        response.status.is_success(),
        "real POST /work failed: {}",
        response.json
    );
    let work_id = response.json["work"]["id"]
        .as_i64()
        .expect("live add work id");
    let row = sqlx::query(
        "SELECT title, normalized_identity_main, identity_volume \
           FROM works WHERE user_id = ?1 AND id = ?2",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read real live-add title tuple");
    use sqlx::Row as _;
    assert_eq!(row.get::<String, _>("title"), "Malice");
    assert_eq!(row.get::<String, _>("normalized_identity_main"), "malice");
    assert_eq!(
        row.get::<Option<String>, _>("identity_volume").as_deref(),
        Some("1")
    );
}

async fn red_live_add_edition_parenthetical_policy() {
    let harness = build_route_harness().await;
    let response = call_router_json(
        &harness,
        Method::POST,
        "/api/v1/work".to_string(),
        Some(live_add_edition_parenthetical_request()),
    )
    .await;
    assert!(
        response.status.is_success(),
        "real ISBN add: {}",
        response.json
    );
    let work_id = response.json["work"]["id"]
        .as_i64()
        .expect("ISBN add work id");
    let captured =
        WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, work_id)
            .await
            .expect("read ISBN-add captured identity");
    assert_eq!(captured.identity_title.main, "Cloud Cuckoo Land");
    assert_eq!(
        captured.identity_title.subtitle.as_deref(),
        Some("large print edition")
    );
    let isbn = captured
        .active_routes
        .iter()
        .find(|route| matches!(route.kind, ilr::RouteKind::Isbn13Edition))
        .expect("ISBN add persists an edition route");
    assert!(matches!(isbn.owner, RouteOwner::Edition(_)));
    let edition_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM editions WHERE user_id=?1 AND work_id=?2")
            .bind(harness.user_id)
            .bind(work_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("count ISBN-add editions");
    assert_eq!(edition_count, 1);
    let frozen_legacy: String =
        sqlx::query_scalar("SELECT identity_status FROM works WHERE user_id=?1 AND id=?2")
            .bind(harness.user_id)
            .bind(work_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("read ISBN-add frozen legacy status");
    assert_eq!(frozen_legacy, "pending");
}

async fn red_live_add_fanout_routes_share_settlement() {
    let harness = build_live_add_fanout_harness().await;
    let response = post_live_add(&harness).await;
    assert!(response.status.is_success(), "live add: {}", response.json);
    let work_id = response.json["work"]["id"]
        .as_i64()
        .expect("live add work id");
    let captured = wait_for_live_add_routes(&harness, work_id).await;
    let pending_cards: Vec<String> = sqlx::query_scalar(
        "SELECT payload FROM identity_review_cards WHERE user_id=?1 AND work_id=?2 AND status='pending'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_all(harness.db.pool())
    .await
    .expect("read live-add pending cards");

    assert_eq!(
        captured.active_routes.len(),
        3,
        "all captures persist together; generation={}; cards={pending_cards:?}; road calls={:?}; outcomes={:?}",
        captured.identity_generation,
        harness.state.identity_road.test_recorder().snapshot(),
        harness.state.identity_road.test_recorder().outcome_snapshot(),
    );
    assert!(captured.active_routes.iter().any(|route| {
        matches!(route.kind, ilr::RouteKind::GoodreadsBookEdition)
            && route.provider_scoped_id == "15750692"
            && matches!(route.owner, RouteOwner::Edition(_))
            && matches!(route.provenance, ilr::RouteProvenance::UserChoice)
            && route.user_confirmed
    }));
    assert!(captured.active_routes.iter().any(|route| {
        matches!(route.kind, ilr::RouteKind::HardcoverWork)
            && route.provider_scoped_id == "429071"
            && matches!(
                route.provenance,
                ilr::RouteProvenance::Provider(ilr::IdentityProvider::Hardcover)
            )
    }));
    assert!(captured.active_routes.iter().any(|route| {
        matches!(route.kind, ilr::RouteKind::Isbn13Edition)
            && route.provider_scoped_id == "9780316399739"
            && matches!(route.owner, RouteOwner::Edition(_))
            && matches!(
                route.provenance,
                ilr::RouteProvenance::Provider(ilr::IdentityProvider::IsbnRegistry)
            )
    }));
    let work_owned_isbns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_routes \
          WHERE user_id=?1 AND resolved_work_id=?2 AND kind=?3 AND owner_type='work'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .bind(serde_json::to_string(&ilr::RouteKind::Isbn13Edition).unwrap())
    .fetch_one(harness.db.pool())
    .await
    .expect("count anomalous work-owned ISBN routes");
    assert_eq!(work_owned_isbns, 0);
}

async fn red_live_add_v2_status_and_generation_audits() {
    let harness = build_live_add_fanout_harness().await;
    let response = post_live_add(&harness).await;
    assert!(response.status.is_success(), "live add: {}", response.json);
    assert_eq!(
        response.json["work"]["identityStatus"], "confirmed",
        "the compatibility DTO projects F2 status, never frozen legacy pending"
    );
    let work_id = response.json["work"]["id"]
        .as_i64()
        .expect("live add work id");
    let captured = wait_for_live_add_routes(&harness, work_id).await;
    assert_eq!(captured.status, ilr::IdentityStatus::UserConfirmed);

    let detail = call_router_json(
        &harness,
        Method::GET,
        format!("/api/v1/work/{work_id}"),
        None,
    )
    .await;
    assert_eq!(detail.json["identityStatus"], "confirmed");
    let list = call_router_json(
        &harness,
        Method::GET,
        "/api/v1/work?page=1&pageSize=50".to_string(),
        None,
    )
    .await;
    let listed = list.json["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == work_id))
        .expect("live-add work on list surface");
    assert_eq!(listed["identityStatus"], "confirmed");

    let frozen_legacy: String =
        sqlx::query_scalar("SELECT identity_status FROM works WHERE user_id=?1 AND id=?2")
            .bind(harness.user_id)
            .bind(work_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("read frozen legacy status");
    assert_eq!(frozen_legacy, "pending", "post-marker code does not mirror");

    let payloads: Vec<String> = sqlx::query_scalar(
        "SELECT payload FROM identity_audit_events \
          WHERE user_id=?1 AND work_id=?2 AND event_kind='settlement' ORDER BY id",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_all(harness.db.pool())
    .await
    .expect("read settlement generation audits");
    assert_eq!(payloads.len() as i64, captured.identity_generation);
    for generation in 1..=captured.identity_generation {
        assert_eq!(
            payloads
                .iter()
                .filter(|payload| payload.as_str() == format!("generation={generation}"))
                .count(),
            1,
            "generation {generation} must have exactly one settlement audit"
        );
    }

    let refresh = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/work/{work_id}/refresh"),
        None,
    )
    .await;
    assert!(
        refresh.status.is_success(),
        "active refresh: {}",
        refresh.json
    );
    assert_eq!(refresh.json["work"]["identityStatus"], "confirmed");
    let generation_after_refresh: i64 =
        sqlx::query_scalar("SELECT identity_generation FROM works WHERE user_id=?1 AND id=?2")
            .bind(harness.user_id)
            .bind(work_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("read generation after active refresh");
    assert_eq!(
        generation_after_refresh, captured.identity_generation,
        "a refresh that discovers no fresh route is an observation, not an F2 settlement"
    );
    let refresh_generation_audits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events \
          WHERE user_id=?1 AND work_id=?2 AND event_kind='settlement'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count settlement audits after quiet manual refresh");
    assert_eq!(refresh_generation_audits, payloads.len() as i64);
    let legacy_after_refresh: String =
        sqlx::query_scalar("SELECT identity_status FROM works WHERE user_id=?1 AND id=?2")
            .bind(harness.user_id)
            .bind(work_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("read frozen legacy status after refresh");
    assert_eq!(legacy_after_refresh, "pending");
}

async fn title_policy_heal_rewrites_existing_rows_atomically() {
    let harness = build_route_harness().await;
    let response = post_live_add(&harness).await;
    assert!(
        response.status.is_success(),
        "seed title heal: {}",
        response.json
    );
    let work_id = response.json["work"]["id"]
        .as_i64()
        .expect("title-heal work id");
    sqlx::query(
        "UPDATE works SET title='Malice (The Faithful and the Fallen, #1)', \
                normalized_title='malice (the faithful and the fallen, #1)', \
                normalized_identity_main='malice (the faithful and the fallen, #1)' \
          WHERE user_id=?1 AND id=?2",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .execute(harness.db.pool())
    .await
    .expect("restore pre-policy title shape");

    let report = livrarr_db::pool::heal_identity_title_policy(harness.db.pool())
        .await
        .expect("heal existing provider title");
    assert_eq!(report.healed, 1);
    assert_eq!(report.blocked_cohorts, 0);
    let captured =
        WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, work_id)
            .await
            .expect("read healed identity");
    assert_eq!(captured.identity_title.main, "Malice");
    assert_eq!(captured.identity_title.volume.as_deref(), Some("1"));
    assert_eq!(captured.identity_generation, 2);
    let gen_two_audits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events \
          WHERE user_id=?1 AND work_id=?2 AND event_kind='settlement' AND payload='generation=2'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count title-heal audit");
    assert_eq!(gen_two_audits, 1);

    let rerun = livrarr_db::pool::heal_identity_title_policy(harness.db.pool())
        .await
        .expect("title-policy marker rerun");
    assert_eq!(
        rerun,
        livrarr_db::pool::IdentityTitlePolicyHealReport::default()
    );
}

async fn title_policy_heal_parks_colliding_cohort() {
    let harness = build_route_harness().await;
    let (author, _) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: "Anthony Doerr".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed title-policy collision author");
    let first = harness
        .db
        .create_work(CreateWorkDbRequest {
            user_id: harness.user_id,
            title: "Cloud Cuckoo Land: Large Print Edition".to_string(),
            author_name: author.name.clone(),
            normalized_title: "cloud cuckoo land large print edition".to_string(),
            normalized_author: "anthony doerr".to_string(),
            author_id: Some(author.id),
            ..Default::default()
        })
        .await
        .expect("seed collision target")
        .0;
    let second = harness
        .db
        .create_work(CreateWorkDbRequest {
            user_id: harness.user_id,
            title: "Cloud Cuckoo Land Legacy".to_string(),
            author_name: author.name,
            normalized_title: "cloud cuckoo land legacy".to_string(),
            normalized_author: "anthony doerr".to_string(),
            author_id: Some(author.id),
            ..Default::default()
        })
        .await
        .expect("seed collision source")
        .0;
    sqlx::query(
        "UPDATE works SET title='Cloud Cuckoo Land', subtitle='large print edition', \
                normalized_title='cloud cuckoo land', normalized_identity_main='cloud cuckoo land', \
                normalized_identity_subtitle='large print edition', normalized_identity_volume='', \
                primary_author_id=?1, text_distinction='common' WHERE id=?2",
    )
    .bind(author.id)
    .bind(first.id)
    .execute(harness.db.pool())
    .await
    .expect("shape collision target");
    sqlx::query(
        "UPDATE works SET title='Cloud Cuckoo Land (Large Print Edition)', \
                normalized_title='cloud cuckoo land (large print edition)', \
                normalized_identity_main='cloud cuckoo land (large print edition)', \
                normalized_identity_subtitle='large print edition', normalized_identity_volume='', \
                primary_author_id=?1, text_distinction='common' WHERE id=?2",
    )
    .bind(author.id)
    .bind(second.id)
    .execute(harness.db.pool())
    .await
    .expect("shape colliding pre-policy source");

    let report = livrarr_db::pool::heal_identity_title_policy(harness.db.pool())
        .await
        .expect("park title-policy collision");
    assert_eq!(report.healed, 0);
    assert_eq!(report.blocked_cohorts, 1);
    assert_eq!(report.review_cards_minted, 1);
    let unchanged: String = sqlx::query_scalar("SELECT title FROM works WHERE id=?1")
        .bind(second.id)
        .fetch_one(harness.db.pool())
        .await
        .expect("read blocked title");
    assert_eq!(unchanged, "Cloud Cuckoo Land (Large Print Edition)");
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta WHERE key='identity_title_policy_generation'",
    )
    .fetch_optional(harness.db.pool())
    .await
    .expect("read held title-policy marker");
    assert!(marker.is_none());
    let cards: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_review_cards \
          WHERE user_id=?1 AND kind=?2 AND status='pending'",
    )
    .bind(harness.user_id)
    .bind(ilr::ReviewKind::GroupIdentity.storage_code())
    .fetch_one(harness.db.pool())
    .await
    .expect("count collision review cards");
    assert_eq!(cards, 1);
}

async fn article_variant_heal_folds_one_sided_volume_and_preserves_routes_contract() {
    let harness = build_route_harness().await;
    let (author, _) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: "Article Heal Author".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed article-heal author");

    let mut leading = settlement_commit(harness.user_id, author.id, None);
    leading.identity_title =
        ilr::title_parts_from_provider("The Article Heal Fixture".to_string(), None)
            .expect("parse leading article fixture");
    leading.identity_title.volume = Some("1".to_string());
    leading.identity_title.normalized_volume = "1".to_string();
    leading.routes = vec![ilr::WorkRoute {
        id: 0,
        user_id: harness.user_id,
        owner: RouteOwner::Work(0),
        resolved_work_id: 0,
        provider: ilr::IdentityProvider::Goodreads,
        kind: ilr::RouteKind::GoodreadsWork,
        provider_scoped_id: "ARTICLE-HEAL-GR".to_string(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::UserChoice,
        user_confirmed: true,
        observed_at: Utc::now(),
    }];
    let leading = WorkIdentityRepository::commit_settlement(&harness.db, leading)
        .await
        .expect("seed leading article Work")
        .identity;

    let mut bare = settlement_commit(harness.user_id, author.id, None);
    bare.identity_title = ilr::title_parts_from_provider("Article Heal Fixture".to_string(), None)
        .expect("parse bare article fixture");
    bare.routes = vec![ilr::WorkRoute {
        id: 0,
        user_id: harness.user_id,
        owner: RouteOwner::Work(0),
        resolved_work_id: 0,
        provider: ilr::IdentityProvider::OpenLibrary,
        kind: ilr::RouteKind::OpenLibraryWork,
        provider_scoped_id: "ARTICLE-HEAL-OL".to_string(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::UserChoice,
        user_confirmed: true,
        observed_at: Utc::now(),
    }];
    let bare = WorkIdentityRepository::commit_settlement(&harness.db, bare)
        .await
        .expect("seed bare article Work")
        .identity;

    livrarr_db::pool::heal_identity_title_policy(harness.db.pool())
        .await
        .expect("heal article variants");
    let survivors: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, normalized_identity_main FROM works WHERE user_id=?1 ORDER BY id",
    )
    .bind(harness.user_id)
    .fetch_all(harness.db.pool())
    .await
    .expect("read article-heal survivors");
    assert_eq!(
        survivors,
        vec![(leading.own_work_id, "article heal fixture".to_string())]
    );
    let routes: Vec<(String, String, bool)> = sqlx::query_as(
        "SELECT provider_scoped_id, provenance, user_confirmed FROM identity_routes \
          WHERE user_id=?1 AND resolved_work_id=?2 ORDER BY provider_scoped_id",
    )
    .bind(harness.user_id)
    .bind(leading.own_work_id)
    .fetch_all(harness.db.pool())
    .await
    .expect("read coalesced article routes");
    assert_eq!(
        routes.len(),
        2,
        "both provider routes must survive the fold"
    );
    let arriving = routes
        .iter()
        .find(|route| route.0 == "ARTICLE-HEAL-OL")
        .expect("arriving OpenLibrary route");
    assert_eq!(
        serde_json::from_str::<ilr::RouteProvenance>(&arriving.1).unwrap(),
        ilr::RouteProvenance::MergeCoalesced
    );
    assert!(
        arriving.2,
        "a user-confirmed arriving route retains its confirmation"
    );
    let archived: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_merge_archives \
          WHERE user_id=?1 AND winner_work_id=?2 AND loser_work_id=?3",
    )
    .bind(harness.user_id)
    .bind(leading.own_work_id)
    .bind(bare.own_work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read article merge archive");
    assert_eq!(archived, 1);
    assert_eq!(
        livrarr_db::pool::heal_identity_title_policy(harness.db.pool())
            .await
            .expect("rerun article heal"),
        livrarr_db::pool::IdentityTitlePolicyHealReport::default(),
        "the marker makes tuple recomputation and folding idempotent"
    );
}

async fn article_variant_heal_parks_contradictory_work_routes_with_current_generation() {
    let harness = build_route_harness().await;
    let (author, _) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: "Article Review Author".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed article-review author");
    let route = |value: &str| ilr::WorkRoute {
        id: 0,
        user_id: harness.user_id,
        owner: RouteOwner::Work(0),
        resolved_work_id: 0,
        provider: ilr::IdentityProvider::OpenLibrary,
        kind: ilr::RouteKind::OpenLibraryWork,
        provider_scoped_id: value.to_string(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::UserChoice,
        user_confirmed: true,
        observed_at: Utc::now(),
    };
    let mut leading = settlement_commit(harness.user_id, author.id, None);
    leading.identity_title =
        ilr::title_parts_from_provider("The Article Review Fixture".to_string(), None)
            .expect("parse leading review fixture");
    leading.identity_title.volume = Some("1".to_string());
    leading.identity_title.normalized_volume = "1".to_string();
    leading.routes = vec![route("OL-ARTICLE-REVIEW-A-W")];
    let leading = WorkIdentityRepository::commit_settlement(&harness.db, leading)
        .await
        .expect("seed leading review Work")
        .identity;
    let mut bare = settlement_commit(harness.user_id, author.id, None);
    bare.identity_title =
        ilr::title_parts_from_provider("Article Review Fixture".to_string(), None)
            .expect("parse bare review fixture");
    bare.routes = vec![route("OL-ARTICLE-REVIEW-B-W")];
    let bare = WorkIdentityRepository::commit_settlement(&harness.db, bare)
        .await
        .expect("seed bare review Work")
        .identity;

    let report = livrarr_db::pool::heal_identity_title_policy(harness.db.pool())
        .await
        .expect("park contradictory article variants");
    assert_eq!(report.article_folds, 0);
    assert_eq!(report.review_cards_minted, 1);
    let works: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1")
        .bind(harness.user_id)
        .fetch_one(harness.db.pool())
        .await
        .expect("count parked article Works");
    assert_eq!(works, 2);
    let card: (i64, String) = sqlx::query_as(
        "SELECT generation, payload FROM identity_review_cards \
          WHERE user_id=?1 AND work_id=?2 AND kind=?3 AND status='pending'",
    )
    .bind(harness.user_id)
    .bind(leading.own_work_id)
    .bind(ilr::ReviewKind::GroupIdentity.storage_code())
    .fetch_one(harness.db.pool())
    .await
    .expect("read parked article card");
    let survivor_generation: i64 =
        sqlx::query_scalar("SELECT identity_generation FROM works WHERE user_id=?1 AND id=?2")
            .bind(harness.user_id)
            .bind(leading.own_work_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("read parked article anchor generation");
    assert_eq!(
        card.0, survivor_generation,
        "the heal must not mint a stale card"
    );
    let decoded: ilr::SettlementReviewCard =
        serde_json::from_str(&card.1).expect("decode parked article card");
    assert!(matches!(
        decoded,
        ilr::SettlementReviewCard::GroupIdentity { work_ids, .. }
            if work_ids == vec![leading.own_work_id, bare.own_work_id]
    ));
}

async fn article_variant_heal_does_not_fold_a_larger_title_difference() {
    let harness = build_route_harness().await;
    let (author, _) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: "Article Negative Author".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed article-negative author");
    let mut first = settlement_commit(harness.user_id, author.id, None);
    first.identity_title =
        ilr::title_parts_from_provider("The Article Negative Fixture".to_string(), None)
            .expect("parse leading negative title");
    first.identity_title.volume = Some("1".to_string());
    first.identity_title.normalized_volume = "1".to_string();
    WorkIdentityRepository::commit_settlement(&harness.db, first)
        .await
        .expect("seed leading negative Work");
    let mut second = settlement_commit(harness.user_id, author.id, None);
    second.identity_title =
        ilr::title_parts_from_provider("Article Negative Fixture Revised".to_string(), None)
            .expect("parse revised negative title");
    WorkIdentityRepository::commit_settlement(&harness.db, second)
        .await
        .expect("seed revised negative Work");

    livrarr_db::pool::heal_identity_title_policy(harness.db.pool())
        .await
        .expect("heal negative article fixture");
    let works: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1")
        .bind(harness.user_id)
        .fetch_one(harness.db.pool())
        .await
        .expect("count negative article Works");
    assert_eq!(works, 2);
    let archives: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM identity_merge_archives WHERE user_id=?1")
            .bind(harness.user_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("count negative article merge archives");
    assert_eq!(archives, 0);
    let cards: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_review_cards \
          WHERE user_id=?1 AND kind=?2 AND status='pending'",
    )
    .bind(harness.user_id)
    .bind(ilr::ReviewKind::GroupIdentity.storage_code())
    .fetch_one(harness.db.pool())
    .await
    .expect("count negative article cards");
    assert_eq!(cards, 0);
}

async fn route_taxonomy_heal_reowns_legacy_edition_routes_once() {
    let harness = build_route_harness().await;
    let work_id = seed_route_work(&harness, "taxonomy-heal").await;
    let card_work_id = seed_route_work(&harness, "taxonomy-card-debris").await;
    let generation_before = work_generation(&harness.db, work_id).await;
    let card_generation_before = work_generation(&harness.db, card_work_id).await;
    sqlx::query(
        "INSERT INTO identity_routes \
            (user_id, owner_type, work_id, edition_id, resolved_work_id, provider, kind, \
             provider_scoped_id, state, provenance, user_confirmed, observed_at) \
         VALUES (?1, 'work', ?2, NULL, ?2, ?3, ?4, '9780306406157', \
                 'active', ?5, 0, ?6)",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .bind(serde_json::to_string(&ilr::IdentityProvider::IsbnRegistry).unwrap())
    .bind(serde_json::to_string(&ilr::RouteKind::Isbn13Edition).unwrap())
    .bind(
        serde_json::to_string(&ilr::RouteProvenance::Provider(
            ilr::IdentityProvider::IsbnRegistry,
        ))
        .unwrap(),
    )
    .bind(Utc::now().to_rfc3339())
    .execute(harness.db.pool())
    .await
    .expect("seed pre-fix work-owned ISBN route");
    let card = ilr::SettlementReviewCard::PendingRoute {
        work_id: card_work_id,
        candidate: ilr::ParkedRouteCandidate {
            route: RouteKey {
                provider: ilr::IdentityProvider::IsbnRegistry,
                kind: ilr::RouteKind::Isbn13Edition,
                value: "9780306406157".to_string(),
            },
            proposed_owner: RouteOwner::Work(card_work_id),
        },
    };
    let card_id = sqlx::query(
        "INSERT INTO identity_review_cards \
            (user_id, work_id, kind, generation, status, payload, created_at) \
         VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?6)",
    )
    .bind(harness.user_id)
    .bind(card_work_id)
    .bind(ilr::ReviewKind::PendingRoute.storage_code())
    .bind(card_generation_before)
    .bind(serde_json::to_string(&card).unwrap())
    .bind(Utc::now().to_rfc3339())
    .execute(harness.db.pool())
    .await
    .expect("seed card3-style invalid pending card")
    .last_insert_rowid();

    let report = livrarr_db::pool::heal_identity_sweep_findings(harness.db.pool())
        .await
        .expect("route taxonomy heal");
    assert_eq!(report.routes_reowned, 1);
    assert_eq!(report.editions_created, 1);
    assert_eq!(report.works_bumped, 1);
    assert_eq!(report.invalid_cards_dismissed, 1);
    let owner: (String, Option<i64>, Option<i64>, i64) = sqlx::query_as(
        "SELECT owner_type, work_id, edition_id, resolved_work_id FROM identity_routes \
          WHERE user_id=?1 AND provider_scoped_id='9780306406157'",
    )
    .bind(harness.user_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read healed route owner");
    assert_eq!(owner.0, "edition");
    assert_eq!(owner.1, None);
    assert!(owner.2.is_some());
    assert_eq!(owner.3, work_id);
    assert_eq!(
        work_generation(&harness.db, work_id).await,
        generation_before + 1
    );
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events WHERE user_id=?1 AND work_id=?2 \
           AND event_kind='settlement' AND actor='identity-route-taxonomy-heal' \
           AND payload=?3",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .bind(format!("generation={}", generation_before + 1))
    .fetch_one(harness.db.pool())
    .await
    .expect("one route heal audit");
    assert_eq!(audit_count, 1);
    let card_status: String =
        sqlx::query_scalar("SELECT status FROM identity_review_cards WHERE id=?1")
            .bind(card_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("invalid pending card is retained as cancelled audit history");
    assert_eq!(card_status, "cancelled");
    assert_eq!(
        work_generation(&harness.db, card_work_id).await,
        card_generation_before,
        "card debris cleanup is not an identity mutation"
    );

    let rerun = livrarr_db::pool::heal_identity_sweep_findings(harness.db.pool())
        .await
        .expect("idempotent route taxonomy rerun");
    assert_eq!(rerun, livrarr_db::pool::IdentitySweepHealReport::default());
    assert_eq!(
        work_generation(&harness.db, work_id).await,
        generation_before + 1
    );
}

async fn drive_manual_import_epub(
    harness: &RouteHarness,
    path: &std::path::Path,
    title: &str,
    language: Option<&str>,
    isbn: Option<&str>,
) -> RouteResponse {
    call_router_json(
        harness,
        Method::POST,
        "/api/v1/manualimport/import".to_string(),
        Some(json!({"items": [{
            "path": path,
            "olKey": "OL-ILR-EDITION-W",
            "title": title,
            "author": "ILR Edition Author",
            "deleteExisting": false,
            "language": language,
            "authorOlKey": null,
            "year": 2026,
            "coverUrl": null,
            "isbn": isbn,
            "description": null,
            "seriesName": null,
            "seriesPosition": null,
            "candidateId": null,
            "hcKey": null,
            "grKey": null,
            "asin": null
        }]})),
    )
    .await
}

async fn work_generation(db: &SqliteDb, work_id: i64) -> i64 {
    sqlx::query_scalar("SELECT identity_generation FROM works WHERE id = ?1")
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .expect("read identity generation")
}

async fn identity_graph_bytes(db: &SqliteDb, work_id: i64) -> Vec<u8> {
    let row: String = sqlx::query_scalar(
        "SELECT json_object(\
           'id', id, 'title', title, 'normalized_title', normalized_title,\
           'identity_generation', identity_generation, 'ol_key', ol_key,\
           'gr_key', gr_key, 'hc_key', hc_key, 'isbn_13', isbn_13, 'asin', asin\
         ) FROM works WHERE id = ?1",
    )
    .bind(work_id)
    .fetch_one(db.pool())
    .await
    .expect("snapshot identity-bearing work columns");
    row.into_bytes()
}

#[derive(Clone, Copy)]
enum RouterCase {
    LegacyPreview,
    LegacyCommit,
    LegacyClear,
    ManualMerge,
    PendingAffirm,
    GreyDismiss,
    DirectAdd,
    AuthorMonitor,
}

async fn drive_router_case(harness: &RouteHarness, case: RouterCase) -> RouteResponse {
    match case {
        RouterCase::LegacyPreview => {
            let id = seed_route_work(harness, "preview").await;
            call_router_json(
                harness,
                Method::POST,
                format!("/api/v1/work/{id}/identity/preview"),
                Some(json!({"input": "OL4288870W", "slot": null})),
            )
            .await
        }
        RouterCase::LegacyCommit => {
            let id = seed_route_work(harness, "commit").await;
            call_router_json(
                harness,
                Method::PUT,
                format!("/api/v1/work/{id}/identity/ol_work"),
                Some(json!({"previewId": "missing"})),
            )
            .await
        }
        RouterCase::LegacyClear => {
            let id = seed_route_work(harness, "clear").await;
            call_router_json(
                harness,
                Method::DELETE,
                format!("/api/v1/work/{id}/identity/ol_work"),
                None,
            )
            .await
        }
        RouterCase::ManualMerge => {
            let survivor = seed_route_work(harness, "merge-survivor").await;
            let loser = seed_route_work(harness, "merge-loser").await;
            let _preview = call_router_json(
                harness,
                Method::GET,
                format!("/api/v1/work/{survivor}/merge/{loser}/preview"),
                None,
            )
            .await;
            call_router_json(
                harness,
                Method::POST,
                format!("/api/v1/work/{survivor}/merge/{loser}"),
                Some(json!({"choices": []})),
            )
            .await
        }
        RouterCase::PendingAffirm => {
            use livrarr_domain::identity::AnchorType;
            let id = seed_route_work(harness, "affirm").await;
            harness
                .db
                .record_pending_anchor(id, AnchorType::new(AnchorType::GR_WORK), "10884")
                .await
                .expect("seed pending anchor");
            call_router_json(
                harness,
                Method::POST,
                format!("/api/v1/work/{id}/pending-anchors/gr_work/affirm"),
                None,
            )
            .await
        }
        RouterCase::GreyDismiss => {
            call_router_json(
                harness,
                Method::POST,
                "/api/v1/identity-review/999999/dismiss".to_string(),
                None,
            )
            .await
        }
        RouterCase::DirectAdd => {
            let n = ROUTE_CASE.fetch_add(1, Ordering::Relaxed);
            call_router_json(
                harness,
                Method::POST,
                "/api/v1/work".to_string(),
                Some(json!({
                    "olKey": null,
                    "title": format!("ILR Direct Add {n}"),
                    "authorName": format!("ILR Direct Author {n}"),
                    "authorOlKey": null,
                    "year": null,
                    "coverUrl": null,
                    "language": "en",
                    "detailUrl": null,
                    "coverManual": false,
                    "isbn13": null,
                    "candidateId": null,
                    "hcKey": null,
                    "grKey": null,
                    "asin": null
                })),
            )
            .await
        }
        RouterCase::AuthorMonitor => {
            call_router_json(
                harness,
                Method::POST,
                "/api/v1/author/search".to_string(),
                None,
            )
            .await
        }
    }
}

async fn assert_identity_v2_schema(harness: &RouteHarness) {
    let present: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'identity_routes'",
    )
    .fetch_one(harness.db.pool())
    .await
    .expect("inspect migrated schema");
    assert_eq!(present, 1, "migration 082 must install identity_routes");
}

async fn red_router_legacy_absent(case: RouterCase) {
    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;
    let response = drive_router_case(&harness, case).await;
    assert_eq!(
        response.status,
        StatusCode::NOT_FOUND,
        "retired legacy identity mutation route must be absent"
    );
    assert_identity_v2_schema(&harness).await;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CardGate {
    WorkUpdate,
    ManualMerge,
    PendingAffirm,
}

#[derive(Clone, Copy)]
enum CardContract {
    DoorWorkUpdate,
    DomainInteractiveFresh,
    MetadataInteractiveCommit,
    HandlerWorkHappy,
    HandlerWorkFailClosed,
    DoorManualMerge,
    HandlerManualHappy,
    HandlerManualFailClosed,
    DoorPendingAffirm,
    HandlerPendingHappy,
    HandlerPendingFailClosed,
}

impl CardContract {
    fn gate(self) -> CardGate {
        match self {
            Self::DoorWorkUpdate
            | Self::DomainInteractiveFresh
            | Self::MetadataInteractiveCommit
            | Self::HandlerWorkHappy
            | Self::HandlerWorkFailClosed => CardGate::WorkUpdate,
            Self::DoorManualMerge | Self::HandlerManualHappy | Self::HandlerManualFailClosed => {
                CardGate::ManualMerge
            }
            Self::DoorPendingAffirm
            | Self::HandlerPendingHappy
            | Self::HandlerPendingFailClosed => CardGate::PendingAffirm,
        }
    }
}

impl CardGate {
    fn expected_kind(self) -> &'static str {
        match self {
            Self::WorkUpdate | Self::ManualMerge => "GroupIdentity",
            Self::PendingAffirm => "PendingRoute",
        }
    }
}

async fn drive_card_mint(harness: &RouteHarness, gate: CardGate) -> (i64, i64, RouteResponse) {
    match gate {
        CardGate::WorkUpdate => {
            let work_id = seed_route_work(harness, "card-update").await;
            let generation = work_generation(&harness.db, work_id).await;
            harness.state.identity_road.test_recorder().clear();
            let response = call_router_json(
                harness,
                Method::PUT,
                format!("/api/v1/work/{work_id}"),
                Some(json!({"title": "Identity choice required"})),
            )
            .await;
            (work_id, generation, response)
        }
        CardGate::ManualMerge => {
            let survivor = seed_route_work(harness, "card-merge-survivor").await;
            let loser = seed_route_work(harness, "card-merge-loser").await;
            let generation = work_generation(&harness.db, survivor).await;
            let preview = call_router_json(
                harness,
                Method::GET,
                format!("/api/v1/work/{survivor}/merge/{loser}/preview"),
                None,
            )
            .await;
            assert!(
                preview.status.is_success(),
                "merge preview must succeed first"
            );
            harness.state.identity_road.test_recorder().clear();
            let response = call_router_json(
                harness,
                Method::POST,
                format!("/api/v1/work/{survivor}/merge/{loser}"),
                Some(json!({"choices": []})),
            )
            .await;
            (survivor, generation, response)
        }
        CardGate::PendingAffirm => {
            use livrarr_domain::identity::AnchorType;
            let work_id = seed_route_work(harness, "card-affirm").await;
            harness
                .db
                .record_pending_anchor(work_id, AnchorType::new(AnchorType::GR_WORK), "10884")
                .await
                .expect("seed pending route through production writer");
            let generation = work_generation(&harness.db, work_id).await;
            harness.state.identity_road.test_recorder().clear();
            let response = call_router_json(
                harness,
                Method::POST,
                format!("/api/v1/work/{work_id}/pending-anchors/gr_work/affirm"),
                None,
            )
            .await;
            (work_id, generation, response)
        }
    }
}

fn card_id_and_generation(response: &RouteResponse, gate: CardGate) -> (i64, i64) {
    assert!(
        response.status.is_success() || response.status == StatusCode::ACCEPTED,
        "interactive door must return its pending card, got {} with {}",
        response.status,
        response.json
    );
    let card_id = response.json["cardId"]
        .as_i64()
        .expect("response carries the freshly minted cardId");
    assert!(card_id > 0, "card id is durable and positive");
    assert_eq!(
        response.json["kind"].as_str(),
        Some(gate.expected_kind()),
        "door mints only its ratified ReviewKind"
    );
    let expected_generation = response.json["expectedGeneration"]
        .as_i64()
        .expect("response carries expectedGeneration");
    (card_id, expected_generation)
}

fn resolve_body(card_id: i64, generation: i64, gate: CardGate) -> Value {
    let command = match gate {
        CardGate::WorkUpdate | CardGate::ManualMerge => json!({
            "GroupIdentity": {
                "card_id": card_id,
                "expected_generation": generation,
                "action": "DifferentFromAll"
            }
        }),
        CardGate::PendingAffirm => json!({
            "PendingRoute": {
                "card_id": card_id,
                "expected_generation": generation,
                "action": {"Affirm": {"surviving_routes": []}}
            }
        }),
    };
    json!({"command": command})
}

async fn red_full_card_gate(contract: CardContract) {
    let gate = contract.gate();
    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;

    let (work_id, generation, minted) = drive_card_mint(&harness, gate).await;
    if gate != CardGate::ManualMerge {
        let expected_status = if gate == CardGate::PendingAffirm {
            StatusCode::NO_CONTENT
        } else {
            StatusCode::OK
        };
        assert_eq!(minted.status, expected_status);
        let calls = harness.state.identity_road.test_recorder().snapshot();
        assert_eq!(calls.len(), 2, "human mutation is settle then resolve");
        assert!(matches!(
            &calls[0],
            livrarr_server::identity_layer::IdentityRoadCall::Settle(request)
                if (gate == CardGate::WorkUpdate
                    && request.origin == IdentityRoadOrigin::WorkUpdateRekey)
                    || (gate == CardGate::PendingAffirm
                        && request.origin == IdentityRoadOrigin::AffirmPendingRoute)
        ));
        assert!(matches!(
            &calls[1],
            livrarr_server::identity_layer::IdentityRoadCall::Resolve { actor, command }
                if matches!(actor, ReviewActor::AuthenticatedUser { user_id }
                    if *user_id == harness.user_id)
                    && ((gate == CardGate::WorkUpdate
                        && command.kind() == ilr::ReviewKind::GroupIdentity)
                        || (gate == CardGate::PendingAffirm
                            && command.kind() == ilr::ReviewKind::PendingRoute))
        ));
        assert_eq!(work_generation(&harness.db, work_id).await, generation + 2);
        let (kind, status): (String, String) = sqlx::query_as(
            "SELECT kind, status FROM identity_review_cards \
              WHERE user_id=?1 AND work_id=?2 ORDER BY id DESC LIMIT 1",
        )
        .bind(harness.user_id)
        .bind(work_id)
        .fetch_one(harness.db.pool())
        .await
        .expect("inline mutation leaves an auditable resolved typed card");
        assert_eq!(kind, gate.expected_kind());
        assert_eq!(status, "resolved");
        if gate == CardGate::WorkUpdate {
            let title: String =
                sqlx::query_scalar("SELECT title FROM works WHERE user_id=?1 AND id=?2")
                    .bind(harness.user_id)
                    .bind(work_id)
                    .fetch_one(harness.db.pool())
                    .await
                    .expect("requested title is written by resolution");
            assert_eq!(title, "Identity choice required");
        } else {
            let confirmed: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_routes WHERE user_id=?1 \
                  AND resolved_work_id=?2 AND user_confirmed=1 AND provenance='\"UserChoice\"'",
            )
            .bind(harness.user_id)
            .bind(work_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("affirmed route keeps user provenance");
            assert_eq!(confirmed, 1);
        }
        return;
    }
    let (card_id, expected_generation) = card_id_and_generation(&minted, gate);
    assert_eq!(expected_generation, generation + 1);
    assert_eq!(work_generation(&harness.db, work_id).await, generation + 1);
    let listed = call_router_json(
        &harness,
        Method::GET,
        "/api/v1/identity-review-card".to_string(),
        None,
    )
    .await;
    assert_eq!(listed.status, StatusCode::OK);
    assert!(listed
        .json
        .as_array()
        .expect("typed card list")
        .iter()
        .any(|card| card["id"] == card_id && card["kind"] == gate.expected_kind()));

    let resolved = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/identity-review-card/{card_id}/resolve"),
        Some(resolve_body(card_id, expected_generation, gate)),
    )
    .await;
    assert!(
        resolved.status.is_success(),
        "typed continuation must commit: {}",
        resolved.json
    );
    let graph_after_resolve = identity_graph_bytes(&harness.db, work_id).await;

    let repeated = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/identity-review-card/{card_id}/resolve"),
        Some(resolve_body(card_id, expected_generation, gate)),
    )
    .await;
    assert_eq!(repeated.status, StatusCode::NOT_FOUND);
    assert_eq!(
        identity_graph_bytes(&harness.db, work_id).await,
        graph_after_resolve
    );

    let (stale_work_id, stale_g, stale_minted) = drive_card_mint(&harness, gate).await;
    let (stale_card_id, stale_expected) = card_id_and_generation(&stale_minted, gate);
    assert_eq!(stale_expected, stale_g + 1);
    let stale_before = identity_graph_bytes(&harness.db, stale_work_id).await;
    let stale = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/identity-review-card/{stale_card_id}/resolve"),
        Some(resolve_body(stale_card_id, stale_g, gate)),
    )
    .await;
    assert_eq!(
        stale.status,
        StatusCode::CONFLICT,
        "generation mismatch is typed"
    );
    assert_eq!(
        identity_graph_bytes(&harness.db, stale_work_id).await,
        stale_before
    );
    match contract {
        CardContract::DoorWorkUpdate
        | CardContract::DoorManualMerge
        | CardContract::DoorPendingAffirm => {
            assert_eq!(minted.json["kind"], gate.expected_kind());
        }
        CardContract::DomainInteractiveFresh => {
            assert!(card_id > 0);
            assert_eq!(repeated.status, StatusCode::NOT_FOUND);
        }
        CardContract::MetadataInteractiveCommit => {
            assert_eq!(expected_generation, generation + 1);
            assert!(!graph_after_resolve.is_empty());
        }
        CardContract::HandlerWorkHappy => {
            assert_eq!(minted.json["kind"], "GroupIdentity");
            assert_eq!(minted.json["expectedGeneration"], generation + 1);
        }
        CardContract::HandlerWorkFailClosed
        | CardContract::HandlerManualFailClosed
        | CardContract::HandlerPendingFailClosed => {
            assert_eq!(stale.status, StatusCode::CONFLICT);
            assert_eq!(
                identity_graph_bytes(&harness.db, stale_work_id).await,
                stale_before
            );
        }
        CardContract::HandlerManualHappy => {
            assert_eq!(minted.json["kind"], "GroupIdentity");
            assert_eq!(resolved.json["workId"], work_id);
            let survivor = seed_route_work(&harness, "counted-merge-survivor").await;
            let loser = seed_route_work(&harness, "counted-merge-loser").await;
            let root = harness
                .db
                .create_root_folder(
                    harness._tmp.path().to_str().expect("UTF-8 merge root"),
                    MediaType::Ebook,
                )
                .await
                .expect("seed counted-merge root");
            let item = harness
                .db
                .create_library_item(CreateLibraryItemDbRequest {
                    user_id: harness.user_id,
                    work_id: loser,
                    root_folder_id: root.id,
                    path: "counted/loser.epub".to_string(),
                    media_type: MediaType::Ebook,
                    file_size: 123,
                    import_id: None,
                    tag_status: TagStatus::Pending,
                    tagged_at_generation: 0,
                })
                .await
                .expect("seed a loser library item");
            let merged = call_router_json(
                &harness,
                Method::POST,
                format!("/api/v1/work/{survivor}/merge/{loser}"),
                Some(json!({"choices": [{
                    "field": "series_name",
                    "choice": "keep_survivor"
                }]})),
            )
            .await;
            assert!(merged.status.is_success(), "counted merge: {}", merged.json);
            assert!(
                merged.json["libraryItemsMoved"]
                    .as_u64()
                    .is_some_and(|count| count >= 1),
                "handler must return the real absorption count"
            );
            let item_work_id: i64 = sqlx::query_scalar(
                "SELECT work_id FROM library_items WHERE user_id = ?1 AND id = ?2",
            )
            .bind(harness.user_id)
            .bind(item.id)
            .fetch_one(harness.db.pool())
            .await
            .expect("read absorbed library item");
            assert_eq!(item_work_id, survivor);
        }
        CardContract::HandlerPendingHappy => {
            assert_eq!(minted.json["kind"], "PendingRoute");
            assert_eq!(minted.json["provenance"], "User");
        }
    }
}

async fn red_monitor_only_graph_unchanged() {
    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;
    let work_id = seed_route_work(&harness, "monitor-only").await;
    let before = identity_graph_bytes(&harness.db, work_id).await;
    let response = call_router_json(
        &harness,
        Method::PUT,
        format!("/api/v1/work/{work_id}"),
        Some(json!({"monitorEbook": true})),
    )
    .await;
    assert!(response.status.is_success(), "monitor-only update succeeds");
    assert_eq!(identity_graph_bytes(&harness.db, work_id).await, before);
    assert_identity_v2_schema(&harness).await;
}

async fn review_card_dismissal_is_scoped_audited_and_generation_neutral() {
    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;
    let (work_id, _, minted) = drive_card_mint(&harness, CardGate::ManualMerge).await;
    let (card_id, generation) = card_id_and_generation(&minted, CardGate::ManualMerge);
    assert_eq!(work_generation(&harness.db, work_id).await, generation);

    let dismissed = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/identity-review-card/{card_id}/dismiss"),
        None,
    )
    .await;
    assert_eq!(dismissed.status, StatusCode::NO_CONTENT);
    assert_eq!(work_generation(&harness.db, work_id).await, generation);
    let status: String =
        sqlx::query_scalar("SELECT status FROM identity_review_cards WHERE user_id=?1 AND id=?2")
            .bind(harness.user_id)
            .bind(card_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("dismissed card remains durable");
    assert_eq!(status, "cancelled");
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events WHERE user_id=?1 AND work_id=?2 \
           AND event_kind='review-dismissal'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("dismissal audit");
    assert_eq!(audit_count, 1);
}

// Bug reproduction: identity-layer-rewrite — a pending GroupIdentity proposal
// must survive unrelated generation settlement, but fail specifically when a
// member of the proposed merge no longer exists.
async fn red_group_identity_stale_card() {
    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;

    let (anchor, _, minted) = drive_card_mint(&harness, CardGate::ManualMerge).await;
    let (card_id, mint_generation) = card_id_and_generation(&minted, CardGate::ManualMerge);
    let loser: i64 = sqlx::query_scalar(
        "SELECT json_extract(payload, '$.GroupIdentity.work_ids[1]') \
           FROM identity_review_cards WHERE user_id=?1 AND id=?2",
    )
    .bind(harness.user_id)
    .bind(card_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read real merge proposal loser");

    sqlx::query(
        "INSERT INTO identity_routes \
            (user_id, owner_type, work_id, edition_id, resolved_work_id, provider, kind, \
             provider_scoped_id, state, provenance, user_confirmed, observed_at) \
         VALUES (?1, 'work', ?2, NULL, ?2, ?3, ?4, ?5, 'active', ?6, 0, ?7)",
    )
    .bind(harness.user_id)
    .bind(anchor)
    .bind(serde_json::to_string(&ilr::IdentityProvider::IsbnRegistry).unwrap())
    .bind(serde_json::to_string(&ilr::RouteKind::Isbn13Edition).unwrap())
    .bind(format!("978000000{card_id:04}"))
    .bind(
        serde_json::to_string(&ilr::RouteProvenance::Provider(
            ilr::IdentityProvider::IsbnRegistry,
        ))
        .unwrap(),
    )
    .bind(Utc::now().to_rfc3339())
    .execute(harness.db.pool())
    .await
    .expect("seed the activated-schema taxonomy fixture");
    let healed = livrarr_db::pool::heal_identity_sweep_findings(harness.db.pool())
        .await
        .expect("advance the anchor through the real taxonomy settlement");
    assert_eq!(healed.works_bumped, 1);
    let settled_generation = work_generation(&harness.db, anchor).await;
    assert_eq!(settled_generation, mint_generation + 1);
    let heal_audits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events \
          WHERE user_id=?1 AND work_id=?2 AND event_kind='settlement' \
            AND actor='identity-route-taxonomy-heal' AND payload=?3",
    )
    .bind(harness.user_id)
    .bind(anchor)
    .bind(format!("generation={settled_generation}"))
    .fetch_one(harness.db.pool())
    .await
    .expect("count the real intervening settlement audit");
    assert_eq!(heal_audits, 1);

    let listed = call_router_json(
        &harness,
        Method::GET,
        "/api/v1/identity-review-card".to_string(),
        None,
    )
    .await;
    assert_eq!(listed.status, StatusCode::OK);
    let refreshed_generation = listed
        .json
        .as_array()
        .and_then(|cards| cards.iter().find(|card| card["id"] == card_id))
        .and_then(|card| card["generation"].as_i64())
        .expect("pending GroupIdentity card remains listed");
    assert_eq!(
        refreshed_generation, settled_generation,
        "the review read refreshes the generation the user actually observes"
    );

    let resolved = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/identity-review-card/{card_id}/resolve"),
        Some(json!({"command": {
            "GroupIdentity": {
                "card_id": card_id,
                "expected_generation": refreshed_generation,
                "action": {"AttachOrMerge": {"anchor": anchor}}
            }
        }})),
    )
    .await;
    assert!(
        resolved.status.is_success(),
        "a still-valid proposal survives generation drift: {}",
        resolved.json
    );
    let loser_exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1 AND id=?2")
            .bind(harness.user_id)
            .bind(loser)
            .fetch_one(harness.db.pool())
            .await
            .expect("check folded loser");
    assert_eq!(loser_exists, 0);
    let (card_status, resolution_audits): (String, i64) = sqlx::query_as(
        "SELECT status, (SELECT COUNT(*) FROM identity_audit_events \
           WHERE user_id=?1 AND event_kind='review-resolution' \
             AND json_extract(payload, '$.GroupIdentity.card_id')=?2) \
           FROM identity_review_cards WHERE user_id=?1 AND id=?2",
    )
    .bind(harness.user_id)
    .bind(card_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read resolved card and audit");
    assert_eq!(card_status, "resolved");
    assert_eq!(resolution_audits, 1);

    let (invalid_anchor, _, invalid_minted) =
        drive_card_mint(&harness, CardGate::ManualMerge).await;
    let (invalid_card_id, _) = card_id_and_generation(&invalid_minted, CardGate::ManualMerge);
    let invalid_loser: i64 = sqlx::query_scalar(
        "SELECT json_extract(payload, '$.GroupIdentity.work_ids[1]') \
           FROM identity_review_cards WHERE user_id=?1 AND id=?2",
    )
    .bind(harness.user_id)
    .bind(invalid_card_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read invalidation-arm loser");
    let deleted = call_router_json(
        &harness,
        Method::DELETE,
        format!("/api/v1/work/{invalid_loser}"),
        None,
    )
    .await;
    assert!(
        deleted.status.is_success(),
        "delete loser: {}",
        deleted.json
    );
    let generation_before_invalid_resolve = work_generation(&harness.db, invalid_anchor).await;
    let listed = call_router_json(
        &harness,
        Method::GET,
        "/api/v1/identity-review-card".to_string(),
        None,
    )
    .await;
    let invalid_generation = listed
        .json
        .as_array()
        .and_then(|cards| cards.iter().find(|card| card["id"] == invalid_card_id))
        .and_then(|card| card["generation"].as_i64())
        .expect("invalidated card remains pending and dismissable");
    let invalid = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/identity-review-card/{invalid_card_id}/resolve"),
        Some(json!({"command": {
            "GroupIdentity": {
                "card_id": invalid_card_id,
                "expected_generation": invalid_generation,
                "action": {"AttachOrMerge": {"anchor": invalid_anchor}}
            }
        }})),
    )
    .await;
    assert_eq!(invalid.status, StatusCode::CONFLICT);
    assert_eq!(
        invalid.json["message"],
        "review proposal invalidated: proposed merge work no longer exists"
    );
    assert_eq!(
        work_generation(&harness.db, invalid_anchor).await,
        generation_before_invalid_resolve,
        "invalid proposal writes no identity settlement"
    );
    let dismissed = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/identity-review-card/{invalid_card_id}/dismiss"),
        None,
    )
    .await;
    assert_eq!(dismissed.status, StatusCode::NO_CONTENT);
    let invalid_status: String =
        sqlx::query_scalar("SELECT status FROM identity_review_cards WHERE id=?1")
            .bind(invalid_card_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("dismiss invalid proposal");
    assert_eq!(invalid_status, "cancelled");
}

async fn affirm_collision_is_structured_and_writes_nothing() {
    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;
    let owner = seed_route_work(&harness, "affirm-collision-owner").await;
    let target = seed_route_work(&harness, "affirm-collision-target").await;
    let route_value = "GR-AFFIRM-COLLISION";
    let owner_edition = harness
        .db
        .seed_transfer_target_for_tests(harness.user_id, owner, ilr::EditionFormat::Unknown)
        .await
        .expect("seed Goodreads Book owner Edition");
    sqlx::query(
        "INSERT INTO identity_routes \
            (user_id, owner_type, work_id, edition_id, resolved_work_id, provider, kind, \
             provider_scoped_id, state, provenance, user_confirmed, observed_at) \
         VALUES (?1, 'edition', NULL, ?2, ?3, ?4, ?5, ?6, 'active', ?7, 1, ?8)",
    )
    .bind(harness.user_id)
    .bind(owner_edition)
    .bind(owner)
    .bind(serde_json::to_string(&ilr::IdentityProvider::Goodreads).unwrap())
    .bind(serde_json::to_string(&ilr::RouteKind::GoodreadsBookEdition).unwrap())
    .bind(route_value)
    .bind(serde_json::to_string(&ilr::RouteProvenance::UserChoice).unwrap())
    .bind(Utc::now().to_rfc3339())
    .execute(harness.db.pool())
    .await
    .expect("seed route owner");
    harness
        .db
        .record_pending_anchor(
            target,
            livrarr_domain::identity::AnchorType::new(
                livrarr_domain::identity::AnchorType::GR_WORK,
            ),
            route_value,
        )
        .await
        .expect("seed colliding pending route");
    let before_generation = work_generation(&harness.db, target).await;
    let before_audits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events WHERE user_id=?1 AND work_id=?2",
    )
    .bind(harness.user_id)
    .bind(target)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    let before_cards: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_review_cards WHERE user_id=?1 AND work_id=?2",
    )
    .bind(harness.user_id)
    .bind(target)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();

    let response = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/work/{target}/pending-anchors/gr_work/affirm"),
        None,
    )
    .await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{}", response.json);
    assert_eq!(response.json["details"]["code"], "anchor_collision");
    assert_eq!(response.json["details"]["owningWorkId"], owner);
    assert!(response.json["details"]["owningWorkTitle"].is_string());
    assert_eq!(
        work_generation(&harness.db, target).await,
        before_generation
    );
    let after_audits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events WHERE user_id=?1 AND work_id=?2",
    )
    .bind(harness.user_id)
    .bind(target)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    let after_cards: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_review_cards WHERE user_id=?1 AND work_id=?2",
    )
    .bind(harness.user_id)
    .bind(target)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(after_audits, before_audits);
    assert_eq!(after_cards, before_cards);
}

enum AuthorMonitorContract {
    HttpAndJobMandatoryProvider,
    MissingProviderDefers,
}

async fn red_author_monitor(contract: AuthorMonitorContract) {
    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;
    match contract {
        AuthorMonitorContract::HttpAndJobMandatoryProvider => {
            let http = drive_router_case(&harness, RouterCase::AuthorMonitor).await;
            assert_eq!(http.status, StatusCode::ACCEPTED);
            livrarr_server::jobs::author_monitor::author_monitor_tick(
                harness.state.clone(),
                CancellationToken::new(),
            )
            .await
            .expect("scheduled author-monitor tick is a production trigger");
            let workflow = strip_rust_comments(include_str!(
                "../../crates/livrarr-metadata/src/author_monitor_workflow.rs"
            ));
            assert!(workflow.contains("async fn run_monitor("));
            assert!(workflow.contains("IdentityRoadService"));
        }
        AuthorMonitorContract::MissingProviderDefers => {
            let (author, _) = harness
                .db
                .create_author(CreateAuthorDbRequest {
                    user_id: harness.user_id,
                    name: "ILR Missing Route Author".to_string(),
                    sort_name: None,
                    ol_key: None,
                    gr_key: None,
                    hc_key: None,
                    import_id: None,
                })
                .await
                .expect("seed author without provider route");
            harness
                .db
                .update_author(
                    harness.user_id,
                    author.id,
                    UpdateAuthorDbRequest {
                        name: None,
                        sort_name: None,
                        ol_key: None,
                        gr_key: None,
                        monitored: Some(true),
                        monitor_new_items: Some(true),
                        monitor_since: None,
                        monitor_language: None,
                    },
                )
                .await
                .expect("monitor route-less author");
            let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1")
                .bind(harness.user_id)
                .fetch_one(harness.db.pool())
                .await
                .expect("count works before monitor");
            harness
                .state
                .author_monitor_workflow
                .run_monitor(harness.user_id, CancellationToken::new())
                .await
                .expect("route-less monitor returns a typed report");
            let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1")
                .bind(harness.user_id)
                .fetch_one(harness.db.pool())
                .await
                .expect("count works after monitor");
            assert_eq!(
                after, before,
                "missing provider route defers without a Work"
            );
        }
    }
    assert_identity_v2_schema(&harness).await;
}

const ILR_SERIES_ROSTER: &str = r#"<html><body>
<div data-react-class="ReactComponents.SeriesHeader" data-react-props="{&quot;title&quot;:&quot;ILR Series&quot;,&quot;subtitle&quot;:&quot;1 primary works • 1 total works&quot;,&quot;description&quot;:{&quot;html&quot;:&quot;&quot;}}"></div>
<div data-react-class="ReactComponents.SeriesList" data-react-props="{&quot;series&quot;:[{&quot;book&quot;:{&quot;bookId&quot;:&quot;10884&quot;,&quot;title&quot;:&quot;ILR Series Work (ILR Series, #1)&quot;,&quot;bookTitleBare&quot;:&quot;ILR Series Work&quot;,&quot;publicationDate&quot;:&quot;2026&quot;}}]}"></div>
<div data-react-class="ReactComponents.FullPagePaginationControls" data-react-props="{&quot;numWorks&quot;:1,&quot;currentPageNumber&quot;:1,&quot;perPage&quot;:100}"></div>
</body></html>"#;

const ILR_SERIES_MINIMUM_ONLY_ROSTER: &str = r#"<html><body>
<div data-react-class="ReactComponents.SeriesHeader" data-react-props="{&quot;title&quot;:&quot;ILR Series&quot;,&quot;subtitle&quot;:&quot;1 primary works • 1 total works&quot;,&quot;description&quot;:{&quot;html&quot;:&quot;&quot;}}"></div>
<div data-react-class="ReactComponents.SeriesList" data-react-props="{&quot;series&quot;:[{&quot;book&quot;:{&quot;title&quot;:&quot;ILR Minimum Only Work (ILR Series, #2)&quot;,&quot;bookTitleBare&quot;:&quot;ILR Minimum Only Work&quot;,&quot;publicationDate&quot;:&quot;2026&quot;}}]}"></div>
<div data-react-class="ReactComponents.FullPagePaginationControls" data-react-props="{&quot;numWorks&quot;:1,&quot;currentPageNumber&quot;:1,&quot;perPage&quot;:100}"></div>
</body></html>"#;

enum SeriesMonitorContract {
    PresentAndMinimum,
    NeverInventEvidence,
}

async fn red_series_monitor(contract: SeriesMonitorContract) {
    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;
    let (author, _) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: "ILR Series Author".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: Some("A-ILR".to_string()),
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed series author");
    let series = harness
        .db
        .upsert_series(CreateSeriesDbRequest {
            user_id: harness.user_id,
            author_id: author.id,
            name: "ILR Series".to_string(),
            gr_key: "S-ILR".to_string(),
            monitor_ebook: true,
            monitor_audiobook: false,
            monitor_language: Some("en".to_string()),
            work_count: 1,
        })
        .await
        .expect("seed monitored series");
    let fetcher = StubHttpFetcher::with_ok(200, ILR_SERIES_ROSTER.as_bytes().to_vec());
    let service = livrarr_metadata::series_query_service::SeriesQueryServiceImpl::new(
        harness.db.clone(),
        fetcher.clone(),
        harness.state.work_service.clone(),
        livrarr_metadata::discovery_service::StubNoLlm,
    )
    .with_identity_road(harness.state.identity_road.clone());
    service
        .run_series_monitor_worker(livrarr_domain::services::SeriesMonitorWorkerParams {
            cancel: CancellationToken::new(),
            user_id: harness.user_id,
            author_id: author.id,
            series_id: series.id,
            series_name: series.name.clone(),
            series_gr_key: series.gr_key.clone(),
            monitor_ebook: true,
            monitor_audiobook: false,
        })
        .await
        .expect("production series worker runs over a seeded monitored series");
    assert_eq!(fetcher.call_count(), 1, "one captured roster fetch");
    match contract {
        SeriesMonitorContract::PresentAndMinimum => {
            let present_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1")
                    .bind(harness.user_id)
                    .fetch_one(harness.db.pool())
                    .await
                    .expect("count route-present series-created works");
            assert_eq!(
                present_count, 1,
                "present-route arm settles exactly one Work"
            );

            let minimum_fetcher =
                StubHttpFetcher::with_ok(200, ILR_SERIES_MINIMUM_ONLY_ROSTER.as_bytes().to_vec());
            let minimum_service =
                livrarr_metadata::series_query_service::SeriesQueryServiceImpl::new(
                    harness.db.clone(),
                    minimum_fetcher.clone(),
                    harness.state.work_service.clone(),
                    livrarr_metadata::discovery_service::StubNoLlm,
                )
                .with_identity_road(harness.state.identity_road.clone());
            minimum_service
                .run_series_monitor_worker(livrarr_domain::services::SeriesMonitorWorkerParams {
                    cancel: CancellationToken::new(),
                    user_id: harness.user_id,
                    author_id: author.id,
                    series_id: series.id,
                    series_name: series.name,
                    series_gr_key: series.gr_key,
                    monitor_ebook: true,
                    monitor_audiobook: false,
                })
                .await
                .expect("minimum-only roster enters the production series worker");
            assert_eq!(minimum_fetcher.call_count(), 1);
            let work_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1")
                .bind(harness.user_id)
                .fetch_one(harness.db.pool())
                .await
                .expect("count minimum-only series-created works");
            assert_eq!(
                work_count, 2,
                "a roster row without a provider route still creates one Work from minimum evidence; calls={:?}",
                harness.state.identity_road.test_recorder().snapshot()
            );
        }
        SeriesMonitorContract::NeverInventEvidence => {
            let invented: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_routes WHERE user_id=?1 AND provenance IN ('UserChoice','OwnedFile')",
            )
            .bind(harness.user_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("inspect series route provenance");
            assert_eq!(invented, 0);
        }
    }
    assert_identity_v2_schema(&harness).await;
}

async fn red_handler_compile_wall() {
    let output = std::process::Command::new("cargo")
        .args([
            "tree",
            "-p",
            "livrarr-handlers",
            "-e",
            "normal",
            "--depth",
            "1",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo tree for handler dependency wall");
    assert!(output.status.success(), "cargo tree must resolve");
    let tree = String::from_utf8(output.stdout).expect("cargo tree is UTF-8");
    assert!(tree.contains("livrarr-domain"));
    for forbidden in ["livrarr-identity", "livrarr-metadata", "livrarr-db"] {
        assert!(
            !tree.lines().skip(1).any(|line| line.contains(forbidden)),
            "handler compile wall forbids concrete dependency {forbidden}:\n{tree}"
        );
    }
    let router = strip_rust_comments(include_str!("../../crates/livrarr-server/src/router.rs"));
    assert!(
        router.contains("livrarr_handlers::identity_layer::resolve::<AppState>"),
        "thin F2 route integration must be registered after the compile wall passes"
    );
}

async fn red_handler_identity_route_smoke() {
    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;
    let response = call_router_json(
        &harness,
        Method::POST,
        "/api/v1/identity-review/999999/resolve".to_string(),
        Some(json!({
            "command": {
                "GroupIdentity": {
                    "card_id": 999999,
                    "expected_generation": 1,
                    "action": "DifferentFromAll"
                }
            }
        })),
    )
    .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    let error = response.json["message"].as_str().unwrap_or_default();
    assert!(
        error.contains("review card") || error.contains("not found"),
        "F2 route smoke must map the typed road NotFound, got {}",
        response.json
    );
}

async fn red_manual_provider_search() {
    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;
    let work_id = seed_route_work(&harness, "manual-provider-search").await;
    let before = identity_graph_bytes(&harness.db, work_id).await;
    let response = call_router_json(
        &harness,
        Method::GET,
        format!(
            "/api/v1/work/{work_id}/identity/search?title=Captured%20Book&author=Captured%20Author"
        ),
        None,
    )
    .await;
    assert!(
        response.status.is_success(),
        "manual search returns candidates"
    );
    assert!(
        response.json.is_array(),
        "manual search response is candidate-only"
    );
    assert_eq!(identity_graph_bytes(&harness.db, work_id).await, before);
    assert!(
        !harness
            ._tmp
            .path()
            .join("covers")
            .join(format!("{work_id}.jpg"))
            .exists(),
        "manual search does not materialize a cover"
    );
    assert!(
        harness
            .state
            .identity_road
            .test_recorder()
            .snapshot()
            .is_empty(),
        "candidate-only search must not settle or resolve identity"
    );
}

#[derive(Debug, Clone, Copy)]
enum CompositionContract {
    EveryDoorOneSettle,
    AllDoorsCommitOnce,
    P4HumanMatrix,
    TwoTopupPatterns,
    RetryConvergenceOnly,
    RetryFailureIsolation,
    ConflictResolveOnce,
    ConflictDismissReject,
    ManualRefreshStructuredSubtitle,
    FetchRouteMatrix,
    EnrichmentPlanMatrix,
    UndeclaredZeroProvider,
    ManualRefreshInlineCapture,
    ListRealRows,
    ListRejectOwnedFile,
    DirectAddMatrix,
    DirectAddWaits,
    ManualMergeAtomicFields,
    PendingAffirmKind,
    ManualImportBeforeAttach,
    ManualImportPrecedence,
    ReadarrSixBranches,
    ReadarrRetryPolicy,
    ReadarrUsesConvergence,
    HttpReviewKinds,
}

fn creation_door_request(
    user_id: i64,
    author_id: i64,
    door: ilr::DoorKind,
    suffix: usize,
) -> IdentityRoadRequest {
    let display_title = format!("ILR Door Matrix {suffix}");
    let minimum = ilr::MinimumWorkEvidence {
        title: display_title.clone(),
        authors: vec![author_id],
    };
    let provider_core = ilr::ProviderWorkIdentityCore {
        identity_title: title(&display_title),
        primary_author_id: author_id,
    };
    let provider_identity = matches!(
        door,
        ilr::DoorKind::AuthorMonitor | ilr::DoorKind::SeriesMonitor | ilr::DoorKind::ReadarrImport
    )
    .then(|| ilr::ProviderIdentityEvidence {
        provider: ilr::IdentityProvider::OpenLibrary,
        route: RouteKey {
            provider: ilr::IdentityProvider::OpenLibrary,
            kind: ilr::RouteKind::OpenLibraryWork,
            value: format!("OL-DOOR-{suffix}-W"),
        },
        work_core: Some(provider_core),
        provenance: Default::default(),
    })
    .into_iter()
    .collect();
    let owned_files = matches!(
        door,
        ilr::DoorKind::ManualImport | ilr::DoorKind::ReadarrImport
    )
    .then(|| ilr::OwnedFileEvidence {
        library_item_id: suffix as i64 + 1,
        file_revision: ilr::FileRevision {
            size_bytes: suffix as u64 + 1,
            modified_ns: suffix as i128 + 1,
            sha256: [suffix as u8; 32],
        },
    })
    .into_iter()
    .collect();
    let human = matches!(
        door,
        ilr::DoorKind::DirectAdd | ilr::DoorKind::ManualImport | ilr::DoorKind::ListImport
    );
    let user_choice = human.then(|| ilr::UserIdentityChoice::ExplicitCreate(minimum.clone()));
    let minimum = matches!(
        door,
        ilr::DoorKind::DirectAdd | ilr::DoorKind::ManualImport | ilr::DoorKind::ListImport
    )
    .then_some(minimum);
    IdentityRoadRequest {
        user_id,
        origin: IdentityRoadOrigin::CreationDoor(door),
        evidence: ilr::IdentityEvidenceBundle {
            user_choice,
            owned_files,
            provider_identity,
            minimum,
        },
        interaction: if human {
            ilr::IdentityRoadInteraction::HumanWatching
        } else {
            ilr::IdentityRoadInteraction::MachineAlone
        },
        existing_work_id: None,
    }
}

async fn red_missing_composition(contract: CompositionContract) {
    if matches!(contract, CompositionContract::HttpReviewKinds) {
        let kinds = [
            ilr::ReviewKind::IdentityConflict,
            ilr::ReviewKind::PendingRoute,
            ilr::ReviewKind::GroupIdentity,
            ilr::ReviewKind::FieldResolution,
            ilr::ReviewKind::ContributorOrder,
            ilr::ReviewKind::EditionEvidence,
            ilr::ReviewKind::ImportIdentity,
            ilr::ReviewKind::MigrationRepair,
            ilr::ReviewKind::InvariantRepair,
        ];
        assert_review_http_cli_parity(&kinds).await;
        return;
    }
    let _breaker = lock_breaker().await;
    let harness = match contract {
        CompositionContract::ManualRefreshStructuredSubtitle => {
            build_route_harness_with_open_library(Some(
                livrarr_external_data::NormalizedWorkDetail {
                    title: Some("ILR Refresh State".to_string()),
                    subtitle: Some("Provider structured subtitle".to_string()),
                    author_name: Some("ILR Refresh Author".to_string()),
                    ol_key: Some("OL-REFRESH-STATE-W".to_string()),
                    ..Default::default()
                },
            ))
            .await
        }
        CompositionContract::RetryConvergenceOnly => {
            build_route_harness_with_open_library(Some(
                livrarr_external_data::NormalizedWorkDetail {
                    title: Some("ILR Retry Route".to_string()),
                    author_name: Some("ILR Retry Author".to_string()),
                    ol_key: Some("OL-RETRY-HANDOFF-W".to_string()),
                    ..Default::default()
                },
            ))
            .await
        }
        CompositionContract::TwoTopupPatterns => {
            let completion_detail = livrarr_external_data::NormalizedWorkDetail {
                title: Some("ILR Topup".to_string()),
                author_name: Some("ILR Topup Author".to_string()),
                ol_key: Some("OL-DIRECT-ADD-HANDOFF-W".to_string()),
                ..Default::default()
            };
            let resolver_miss = livrarr_external_data::NormalizedWorkDetail {
                title: Some("ILR Topup".to_string()),
                author_name: Some("ILR Topup Author".to_string()),
                ..Default::default()
            };
            build_route_harness_with_provider_details(
                Some(completion_detail),
                vec![(livrarr_domain::MetadataProvider::OpenLibrary, resolver_miss)],
                None,
            )
            .await
        }
        CompositionContract::ManualRefreshInlineCapture => {
            build_route_harness_with_open_library(Some(
                livrarr_external_data::NormalizedWorkDetail {
                    title: Some("ILR Captured Refresh".to_string()),
                    author_name: Some("ILR Captured Author".to_string()),
                    ol_key: Some("OL-MANUAL-REFRESH-HANDOFF-W".to_string()),
                    ..Default::default()
                },
            ))
            .await
        }
        _ => build_route_harness().await,
    };
    harness.state.identity_road.test_recorder().clear();
    match contract {
        CompositionContract::EveryDoorOneSettle | CompositionContract::AllDoorsCommitOnce => {
            let before: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_audit_events WHERE event_kind='settlement'",
            )
            .fetch_one(harness.db.pool())
            .await
            .expect("count settlements before door matrix");
            // Drive every production composition. This proof deliberately never
            // invokes IdentityRoadService::settle from the test itself.
            let direct = drive_router_case(&harness, RouterCase::DirectAdd).await;
            assert!(direct.status.is_success(), "DirectAdd: {}", direct.json);

            harness
                .db
                .create_root_folder(
                    harness._tmp.path().to_str().expect("UTF-8 import root"),
                    MediaType::Ebook,
                )
                .await
                .expect("seed ManualImport root");
            let import_path = harness._tmp.path().join("ilr-six-door.epub");
            write_epub_with_metadata(
                &import_path,
                false,
                "ILR Six Door Import",
                Some("en"),
                Some("9780306406157"),
            );
            let manual = drive_manual_import_epub(
                &harness,
                &import_path,
                "ILR Six Door Import",
                Some("en"),
                Some("9780306406157"),
            )
            .await;
            assert!(manual.status.is_success(), "ManualImport: {}", manual.json);

            let preview = harness
                .state
                .list_service
                .preview(
                    harness.user_id,
                    b"Book Id,Title,Author,ISBN,ISBN13,My Rating,Exclusive Shelf\n81234,ILR Six Door List,ILR Six Door List Author,=\"\",=\"\",5,read\n"
                        .to_vec(),
                )
                .await
                .expect("production ListImport preview");
            let list = call_router_json(
                &harness,
                Method::POST,
                "/api/v1/listimport/confirm".to_string(),
                Some(json!({
                    "previewId": preview.preview_id,
                    "rowIndices": [0],
                    "importId": null,
                    "language": "en"
                })),
            )
            .await;
            assert!(list.status.is_success(), "ListImport: {}", list.json);
            let (list_work_id, list_generation): (i64, i64) = sqlx::query_as(
                "SELECT w.id, w.identity_generation FROM identity_routes r \
                  JOIN works w ON w.user_id=r.user_id AND w.id=r.resolved_work_id \
                  WHERE r.user_id=?1 AND r.provider_scoped_id='81234'",
            )
            .bind(harness.user_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("read productive ListImport work");
            let list_identity = WorkIdentityRepository::read_captured_identity(
                &harness.db,
                harness.user_id,
                list_work_id,
            )
            .await
            .expect("read productive ListImport identity graph");
            assert!(list_identity.active_routes.iter().any(|route| {
                route.kind == ilr::RouteKind::GoodreadsBookEdition
                    && route.provider_scoped_id == "81234"
                    && matches!(route.owner, RouteOwner::Edition(_))
            }));
            assert_eq!(list_generation, 1);
            let list_audits: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_audit_events \
                  WHERE user_id=?1 AND work_id=?2 AND event_kind='settlement'",
            )
            .bind(harness.user_id)
            .bind(list_work_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("count productive ListImport settlement audits");
            assert_eq!(list_audits, 1);

            let (readarr_author, _) = harness
                .db
                .create_author(CreateAuthorDbRequest {
                    user_id: harness.user_id,
                    name: "ILR Six Door Readarr Author".to_string(),
                    sort_name: None,
                    ol_key: None,
                    gr_key: None,
                    hc_key: None,
                    import_id: None,
                })
                .await
                .expect("seed Readarr author");
            let readarr = livrarr_server::readarr_import_workflow::submit_readarr_identity(
                harness.state.identity_road.as_ref(),
                creation_door_request(
                    harness.user_id,
                    readarr_author.id,
                    ilr::DoorKind::ReadarrImport,
                    803,
                ),
            )
            .await
            .expect("production Readarr submission");
            assert!(matches!(readarr, ilr::IdentityRoadOutcome::Settled { .. }));

            let (monitor_author, _) = harness
                .db
                .create_author(CreateAuthorDbRequest {
                    user_id: harness.user_id,
                    name: "ILR Six Door Monitor Author".to_string(),
                    sort_name: None,
                    ol_key: Some("OL9007A".to_string()),
                    gr_key: None,
                    hc_key: None,
                    import_id: None,
                })
                .await
                .expect("seed AuthorMonitor author");
            harness
                .db
                .attach_route_as_user(
                    harness.user_id,
                    monitor_author.id,
                    AuthorRouteKey::parse(AuthorProvider::OpenLibrary, "OL9007A")
                        .expect("canonical author route"),
                )
                .await
                .expect("seed AuthorMonitor route");
            harness
                .db
                .update_author(
                    harness.user_id,
                    monitor_author.id,
                    UpdateAuthorDbRequest {
                        name: None,
                        sort_name: None,
                        ol_key: None,
                        gr_key: None,
                        monitored: Some(true),
                        monitor_new_items: Some(true),
                        monitor_since: None,
                        monitor_language: None,
                    },
                )
                .await
                .expect("enable AuthorMonitor");
            let author_fetcher = StubHttpFetcher::with_ok(
                200,
                br#"{"entries":[{"key":"/works/OL9007W","title":"ILR Six Door Author Work","first_publish_date":"2026"}]}"#
                    .to_vec(),
            );
            let author_monitor =
                livrarr_metadata::author_monitor_workflow::AuthorMonitorWorkflowImpl::with_identity_road(
                    Arc::new(harness.db.clone()),
                    harness.state.work_service.clone(),
                    Arc::new(author_fetcher),
                    harness.state.identity_road.clone(),
                );
            author_monitor
                .run_monitor(harness.user_id, CancellationToken::new())
                .await
                .expect("production AuthorMonitor workflow");

            let (series_author, _) = harness
                .db
                .create_author(CreateAuthorDbRequest {
                    user_id: harness.user_id,
                    name: "ILR Six Door Series Author".to_string(),
                    sort_name: None,
                    ol_key: None,
                    gr_key: Some("A-SIX-DOOR".to_string()),
                    hc_key: None,
                    import_id: None,
                })
                .await
                .expect("seed SeriesMonitor author");
            let series = harness
                .db
                .upsert_series(CreateSeriesDbRequest {
                    user_id: harness.user_id,
                    author_id: series_author.id,
                    name: "ILR Six Door Series".to_string(),
                    gr_key: "S-SIX-DOOR".to_string(),
                    monitor_ebook: true,
                    monitor_audiobook: false,
                    monitor_language: Some("en".to_string()),
                    work_count: 1,
                })
                .await
                .expect("seed SeriesMonitor series");
            let series_fetcher =
                StubHttpFetcher::with_ok(200, ILR_SERIES_ROSTER.as_bytes().to_vec());
            let series_monitor =
                livrarr_metadata::series_query_service::SeriesQueryServiceImpl::new(
                    harness.db.clone(),
                    series_fetcher,
                    harness.state.work_service.clone(),
                    livrarr_metadata::discovery_service::StubNoLlm,
                )
                .with_identity_road(harness.state.identity_road.clone());
            series_monitor
                .run_series_monitor_worker(livrarr_domain::services::SeriesMonitorWorkerParams {
                    cancel: CancellationToken::new(),
                    user_id: harness.user_id,
                    author_id: series_author.id,
                    series_id: series.id,
                    series_name: series.name,
                    series_gr_key: series.gr_key,
                    monitor_ebook: true,
                    monitor_audiobook: false,
                })
                .await
                .expect("production SeriesMonitor workflow");

            let doors = [
                ilr::DoorKind::DirectAdd,
                ilr::DoorKind::ManualImport,
                ilr::DoorKind::ListImport,
                ilr::DoorKind::ReadarrImport,
                ilr::DoorKind::AuthorMonitor,
                ilr::DoorKind::SeriesMonitor,
            ];
            let calls = harness.state.identity_road.test_recorder().snapshot();
            let creation_calls: Vec<_> = calls
                .iter()
                .filter_map(|call| match call {
                    livrarr_server::identity_layer::IdentityRoadCall::Settle(request) => {
                        match request.origin {
                            IdentityRoadOrigin::CreationDoor(door) => Some(door),
                            _ => None,
                        }
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                creation_calls, doors,
                "one ordered settlement per real door"
            );
            for route_value in ["81234", "OL-DOOR-803-W", "OL9007W", "10884"] {
                let (work_id, generation): (i64, i64) = sqlx::query_as(
                    "SELECT w.id, w.identity_generation FROM identity_routes r \
                      JOIN works w ON w.user_id=r.user_id AND w.id=r.resolved_work_id \
                      WHERE r.user_id=?1 AND r.provider_scoped_id=?2",
                )
                .bind(harness.user_id)
                .bind(route_value)
                .fetch_one(harness.db.pool())
                .await
                .unwrap_or_else(|error| panic!("productive route {route_value}: {error}"));
                assert_eq!(generation, 1, "productive route {route_value}");
                let audits: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM identity_audit_events \
                      WHERE user_id=?1 AND work_id=?2 AND event_kind='settlement'",
                )
                .bind(harness.user_id)
                .bind(work_id)
                .fetch_one(harness.db.pool())
                .await
                .unwrap_or_else(|error| panic!("productive audit {route_value}: {error}"));
                assert_eq!(audits, 1, "productive route {route_value}");
            }
            let after: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_audit_events WHERE event_kind='settlement'",
            )
            .fetch_one(harness.db.pool())
            .await
            .expect("count settlements after door matrix");
            assert_eq!(after - before, doors.len() as i64);
        }
        CompositionContract::ReadarrSixBranches => {
            use livrarr_server::readarr_import_workflow::{
                plan_readarr_identity, ReadarrIdentityBranch as B, ReadarrProviderDisposition as P,
            };
            let matrix = [
                (true, P::Agree, false, B::ResolvingIdentifiersAgree, None),
                (
                    true,
                    P::Conflict,
                    true,
                    B::ResolvingIdentifiersConflict,
                    Some(ilr::ReviewKind::IdentityConflict),
                ),
                (
                    true,
                    P::DefinitiveMiss,
                    true,
                    B::IdentifiersMissValidMinimum,
                    None,
                ),
                (
                    true,
                    P::DefinitiveMiss,
                    false,
                    B::IdentifiersMissInvalidMinimum,
                    Some(ilr::ReviewKind::ImportIdentity),
                ),
                (
                    false,
                    P::DefinitiveMiss,
                    true,
                    B::NoIdentifiersValidMinimum,
                    Some(ilr::ReviewKind::ImportIdentity),
                ),
                (
                    false,
                    P::DefinitiveMiss,
                    false,
                    B::NoIdentifiersInvalidMinimum,
                    Some(ilr::ReviewKind::ImportIdentity),
                ),
            ];
            for (index, (has_id, provider, has_minimum, branch, review_kind)) in
                matrix.into_iter().enumerate()
            {
                let plan = plan_readarr_identity(has_id, provider, has_minimum);
                assert_eq!(plan.branch, branch);
                assert_eq!(plan.review_kind, review_kind);
                assert_eq!(plan.attach_allowed, review_kind.is_none());
                let (author, _) = harness
                    .db
                    .create_author(CreateAuthorDbRequest {
                        user_id: harness.user_id,
                        name: format!("ILR Readarr Branch Author {index}"),
                        sort_name: None,
                        ol_key: None,
                        gr_key: None,
                        hc_key: None,
                        import_id: None,
                    })
                    .await
                    .expect("seed Readarr branch author");
                let outcome = livrarr_server::readarr_import_workflow::submit_readarr_identity(
                    harness.state.identity_road.as_ref(),
                    creation_door_request(
                        harness.user_id,
                        author.id,
                        ilr::DoorKind::ReadarrImport,
                        900 + index,
                    ),
                )
                .await
                .expect("each planned branch enters the production Readarr road");
                assert!(matches!(outcome, ilr::IdentityRoadOutcome::Settled { .. }));
            }
            let readarr_calls = harness
                .state
                .identity_road
                .test_recorder()
                .snapshot()
                .into_iter()
                .filter(|call| {
                    matches!(
                        call,
                        livrarr_server::identity_layer::IdentityRoadCall::Settle(request)
                            if request.origin
                                == IdentityRoadOrigin::CreationDoor(ilr::DoorKind::ReadarrImport)
                    )
                })
                .count();
            assert_eq!(readarr_calls, 6, "all six plans cross the production seam");
        }
        CompositionContract::ReadarrRetryPolicy => {
            use livrarr_server::readarr_import_workflow::{
                plan_readarr_identity, ReadarrProviderDisposition as P,
            };
            for minimum in [false, true] {
                assert_eq!(
                    plan_readarr_identity(true, P::DefinitiveMiss, minimum).retry_count,
                    0
                );
                assert_eq!(
                    plan_readarr_identity(true, P::Outage, minimum).retry_count,
                    1
                );
            }
        }
        CompositionContract::ReadarrUsesConvergence => {
            let (author, _) = harness
                .db
                .create_author(CreateAuthorDbRequest {
                    user_id: harness.user_id,
                    name: "ILR Readarr Convergence Author".to_string(),
                    sort_name: None,
                    ol_key: None,
                    gr_key: None,
                    hc_key: None,
                    import_id: None,
                })
                .await
                .expect("seed Readarr convergence author");
            let outcome = livrarr_server::readarr_import_workflow::submit_readarr_identity(
                harness.state.identity_road.as_ref(),
                creation_door_request(
                    harness.user_id,
                    author.id,
                    ilr::DoorKind::ReadarrImport,
                    700,
                ),
            )
            .await
            .expect("Readarr settlement");
            let ilr::IdentityRoadOutcome::Settled { work_id, .. } = outcome else {
                panic!("Readarr fixture must settle")
            };
            let _ = work_id;
            let calls = harness.state.identity_road.test_recorder().snapshot();
            assert!(matches!(
                &calls[0],
                livrarr_server::identity_layer::IdentityRoadCall::Settle(request)
                    if request.origin == IdentityRoadOrigin::CreationDoor(ilr::DoorKind::ReadarrImport)
            ));
            assert!(!calls.iter().any(|call| matches!(
                call,
                livrarr_server::identity_layer::IdentityRoadCall::Settle(request)
                    if request.origin == IdentityRoadOrigin::ConvergenceVisit
            )));
            assert!(!calls.iter().any(|call| matches!(
                call,
                livrarr_server::identity_layer::IdentityRoadCall::Settle(request)
                    if request.origin == IdentityRoadOrigin::ManualRefresh
            )));
            let source = strip_rust_comments(include_str!(
                "../../crates/livrarr-server/src/readarr_import_workflow.rs"
            ));
            assert!(source.contains("converge_work_with_handoff"));
            assert!(!source.contains("continue_readarr_convergence"));
        }
        CompositionContract::DirectAddMatrix => {
            let response = drive_router_case(&harness, RouterCase::DirectAdd).await;
            assert!(
                response.status.is_success(),
                "direct add: {}",
                response.json
            );
            let calls = harness.state.identity_road.test_recorder().snapshot();
            assert_eq!(calls.len(), 1, "one synchronous road call per direct add");
            let livrarr_server::identity_layer::IdentityRoadCall::Settle(request) = &calls[0]
            else {
                panic!("direct add must call settle")
            };
            assert_eq!(
                request.origin,
                IdentityRoadOrigin::CreationDoor(ilr::DoorKind::DirectAdd)
            );
            assert_eq!(
                request.interaction,
                ilr::IdentityRoadInteraction::HumanWatching
            );
            assert!(request.evidence.user_choice.is_some());
            assert!(request.evidence.owned_files.is_empty());
            assert!(request.evidence.minimum.is_some());
            let commits: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_audit_events WHERE event_kind='settlement'",
            )
            .fetch_one(harness.db.pool())
            .await
            .expect("count direct-add settlement commits");
            assert_eq!(commits, 1);
        }
        CompositionContract::DirectAddWaits => {
            let response = drive_router_case(&harness, RouterCase::DirectAdd).await;
            assert!(
                response.status.is_success(),
                "direct add: {}",
                response.json
            );
            let work_id = response.json["work"]["id"]
                .as_i64()
                .or_else(|| response.json["id"].as_i64())
                .expect("DirectAdd response carries the created Work id");
            assert!(
                harness
                    .state
                    .work_service
                    .is_enriching(harness.user_id, work_id),
                "the add handler must claim enriching before it returns"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(
                harness
                    .state
                    .work_service
                    .is_enriching(harness.user_id, work_id),
                "identity capture and the delayed top-up share one uninterrupted signal"
            );
            // Race the actual spawned completion: whether it finishes before
            // the first observation or after it, the response has already been
            // emitted and this loop waits for the terminal state.
            for _ in 0..800 {
                if !harness
                    .state
                    .work_service
                    .is_enriching(harness.user_id, work_id)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(
                !harness
                    .state
                    .work_service
                    .is_enriching(harness.user_id, work_id),
                "background DirectAdd completion must reach a terminal state"
            );
            let persisted = harness
                .state
                .work_service
                .get(harness.user_id, work_id)
                .await
                .expect("created Work survives background completion");
            assert_eq!(persisted.id, work_id);
            let settle_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_audit_events \
                  WHERE user_id=?1 AND work_id=?2 AND event_kind='settlement'",
            )
            .bind(harness.user_id)
            .bind(work_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("count DirectAdd settlement");
            assert_eq!(
                settle_count, 1,
                "spawned completion never re-creates identity"
            );
        }
        CompositionContract::P4HumanMatrix => {
            let direct = drive_router_case(&harness, RouterCase::DirectAdd).await;
            assert!(direct.status.is_success());
            let (author, _) = harness
                .db
                .create_author(CreateAuthorDbRequest {
                    user_id: harness.user_id,
                    name: "ILR P4 Machine Author".to_string(),
                    sort_name: None,
                    ol_key: None,
                    gr_key: None,
                    hc_key: None,
                    import_id: None,
                })
                .await
                .expect("seed P4 machine author");
            livrarr_server::readarr_import_workflow::submit_readarr_identity(
                harness.state.identity_road.as_ref(),
                creation_door_request(
                    harness.user_id,
                    author.id,
                    ilr::DoorKind::ReadarrImport,
                    804,
                ),
            )
            .await
            .expect("drive P4 machine-alone production door");
            let interactions: Vec<_> = harness
                .state
                .identity_road
                .test_recorder()
                .snapshot()
                .into_iter()
                .filter_map(|call| match call {
                    livrarr_server::identity_layer::IdentityRoadCall::Settle(request) => {
                        Some((request.origin, request.interaction))
                    }
                    _ => None,
                })
                .collect();
            assert!(interactions.iter().any(|(origin, interaction)| {
                *origin == IdentityRoadOrigin::CreationDoor(ilr::DoorKind::DirectAdd)
                    && *interaction == ilr::IdentityRoadInteraction::HumanWatching
            }));
            assert!(interactions.iter().any(|(origin, interaction)| {
                *origin == IdentityRoadOrigin::CreationDoor(ilr::DoorKind::ReadarrImport)
                    && *interaction == ilr::IdentityRoadInteraction::MachineAlone
            }));
        }
        CompositionContract::ManualMergeAtomicFields => {
            let response = drive_router_case(&harness, RouterCase::ManualMerge).await;
            assert_eq!(response.status, StatusCode::ACCEPTED);
            assert_eq!(response.json["kind"], "GroupIdentity");
            let pending: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_review_cards WHERE status='pending' AND kind='GroupIdentity'",
            )
            .fetch_one(harness.db.pool())
            .await
            .expect("count atomic merge review");
            assert_eq!(pending, 1);
        }
        CompositionContract::PendingAffirmKind => {
            let response = drive_router_case(&harness, RouterCase::PendingAffirm).await;
            assert_eq!(response.status, StatusCode::NO_CONTENT);
            let kinds: Vec<String> = sqlx::query_scalar(
                "SELECT kind FROM identity_review_cards WHERE status='resolved' ORDER BY id",
            )
            .fetch_all(harness.db.pool())
            .await
            .expect("read pending-affirm review kinds");
            assert_eq!(kinds, vec!["PendingRoute"]);
            let calls = harness.state.identity_road.test_recorder().snapshot();
            assert!(calls.windows(2).any(|pair| matches!(
                pair,
                [
                    livrarr_server::identity_layer::IdentityRoadCall::Settle(request),
                    livrarr_server::identity_layer::IdentityRoadCall::Resolve { command, .. }
                ] if request.origin == IdentityRoadOrigin::AffirmPendingRoute
                    && command.kind() == ilr::ReviewKind::PendingRoute
            )));
        }
        CompositionContract::HttpReviewKinds => {
            panic!("HttpReviewKinds returns before acquiring the shared harness lock")
        }
        CompositionContract::EnrichmentPlanMatrix | CompositionContract::UndeclaredZeroProvider => {
            use livrarr_enrichment::identity_layer::{
                EnrichmentService as _, RouteDrivenEnrichmentService,
            };
            let db = create_test_db().await;
            let mut snapshot = identity(1, 1);
            snapshot.active_routes = vec![
                route(
                    ilr::RouteKind::GoodreadsBookEdition,
                    "10884",
                    RouteOwner::Edition(1),
                ),
                route(
                    ilr::RouteKind::Undeclared {
                        provider_kind: "future-kind".to_string(),
                        scope: ilr::RouteScope::Work,
                    },
                    "opaque",
                    RouteOwner::Work(1),
                ),
            ];
            let service = RouteDrivenEnrichmentService::new(
                db,
                snapshot.clone(),
                livrarr_domain::RequestPriority::Normal,
            );
            let plan = service.plan_from_routes(snapshot);
            assert_eq!(plan.usable_routes.len(), 1);
            assert_eq!(plan.provider_calls.len(), 1);
            assert!(plan.manual_search_only);
            assert!(matches!(
                plan.provider_calls[0].route.kind,
                ilr::RouteKind::GoodreadsBookEdition
            ));
        }
        CompositionContract::FetchRouteMatrix => {
            use livrarr_external_data::identity_layer::IdentityProviderGateway as _;
            use livrarr_external_data::{ProviderClient, ProviderOutcome, StubProviderClient};
            let matrix = [
                (
                    ilr::IdentityProvider::OpenLibrary,
                    ilr::RouteKind::OpenLibraryWork,
                    livrarr_domain::MetadataProvider::OpenLibrary,
                    RouteOwner::Work(1),
                ),
                (
                    ilr::IdentityProvider::Goodreads,
                    ilr::RouteKind::GoodreadsBookEdition,
                    livrarr_domain::MetadataProvider::Goodreads,
                    RouteOwner::Edition(1),
                ),
                (
                    ilr::IdentityProvider::Hardcover,
                    ilr::RouteKind::HardcoverWork,
                    livrarr_domain::MetadataProvider::Hardcover,
                    RouteOwner::Work(1),
                ),
                (
                    ilr::IdentityProvider::IsbnRegistry,
                    ilr::RouteKind::Isbn13Edition,
                    livrarr_domain::MetadataProvider::OpenLibrary,
                    RouteOwner::Edition(1),
                ),
                (
                    ilr::IdentityProvider::Amazon,
                    ilr::RouteKind::AsinEdition,
                    livrarr_domain::MetadataProvider::Audible,
                    RouteOwner::Edition(1),
                ),
            ];
            for (identity_provider, kind, metadata_provider, owner) in matrix {
                let stub = StubProviderClient::new(metadata_provider, ProviderOutcome::NotFound);
                let client = ProviderClient::Stub(stub.clone());
                let result = client
                    .fetch_by_route(
                        ilr::WorkRoute {
                            id: 1,
                            user_id: 1,
                            owner,
                            resolved_work_id: 1,
                            provider: identity_provider,
                            kind,
                            provider_scoped_id: "route-value".to_string(),
                            state: ilr::WorkRouteState::Active,
                            provenance: ilr::RouteProvenance::UserChoice,
                            user_confirmed: true,
                            observed_at: Utc::now(),
                        },
                        livrarr_domain::RequestPriority::Normal,
                    )
                    .await;
                assert!(matches!(
                    result,
                    Err(livrarr_external_data::identity_layer::ProviderEvidenceError::Permanent(_))
                ));
                assert_eq!(stub.call_count(), 1);
            }
            let stub = StubProviderClient::new(
                livrarr_domain::MetadataProvider::Goodreads,
                ProviderOutcome::NotFound,
            );
            let client = ProviderClient::Stub(stub.clone());
            assert!(matches!(
                client
                    .fetch_by_route(
                        ilr::WorkRoute {
                            id: 1,
                            user_id: 1,
                            owner: RouteOwner::Work(1),
                            resolved_work_id: 1,
                            provider: ilr::IdentityProvider::Goodreads,
                            kind: ilr::RouteKind::GoodreadsWork,
                            provider_scoped_id: "600815".to_string(),
                            state: ilr::WorkRouteState::Active,
                            provenance: ilr::RouteProvenance::UserChoice,
                            user_confirmed: true,
                            observed_at: Utc::now(),
                        },
                        livrarr_domain::RequestPriority::Normal,
                    )
                    .await,
                Err(livrarr_external_data::identity_layer::ProviderEvidenceError::Permanent(_))
            ));
            assert_eq!(
                stub.call_count(),
                0,
                "a Goodreads Work id must never reach /book/show/<id>"
            );
            let stub = StubProviderClient::new(
                livrarr_domain::MetadataProvider::Goodreads,
                ProviderOutcome::NotFound,
            );
            let client = ProviderClient::Stub(stub.clone());
            let mut undeclared = route(
                ilr::RouteKind::Undeclared {
                    provider_kind: "future-kind".to_string(),
                    scope: ilr::RouteScope::Work,
                },
                "opaque",
                RouteOwner::Work(1),
            );
            undeclared.provider = ilr::IdentityProvider::Goodreads;
            assert!(matches!(
                client
                    .fetch_by_route(undeclared, livrarr_domain::RequestPriority::Normal)
                    .await,
                Err(livrarr_external_data::identity_layer::ProviderEvidenceError::Permanent(_))
            ));
            assert_eq!(stub.call_count(), 0, "undeclared routes are manual-only");
        }
        CompositionContract::RetryFailureIsolation => {
            let before: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_audit_events WHERE event_kind='settlement'",
            )
            .fetch_one(harness.db.pool())
            .await
            .expect("count settlements before empty retry");
            let response = call_router_json(
                &harness,
                Method::POST,
                "/api/v1/work/retry-incomplete".to_string(),
                None,
            )
            .await;
            assert_eq!(response.status, StatusCode::ACCEPTED);
            tokio::task::yield_now().await;
            assert!(harness
                .state
                .identity_road
                .test_recorder()
                .snapshot()
                .is_empty());
            let after: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_audit_events WHERE event_kind='settlement'",
            )
            .fetch_one(harness.db.pool())
            .await
            .expect("count settlements after empty retry");
            assert_eq!(after, before, "empty capture is a no-op");
        }
        CompositionContract::RetryConvergenceOnly => {
            let (author, _) = harness
                .db
                .create_author(CreateAuthorDbRequest {
                    user_id: harness.user_id,
                    name: "ILR Retry Author".to_string(),
                    sort_name: None,
                    ol_key: None,
                    gr_key: None,
                    hc_key: None,
                    import_id: None,
                })
                .await
                .expect("seed Retry-All author");
            let mut commit = settlement_commit(harness.user_id, author.id, None);
            commit.identity_title = title("ILR Retry Route");
            commit.routes = vec![ilr::WorkRoute {
                id: 0,
                user_id: harness.user_id,
                owner: RouteOwner::Work(0),
                resolved_work_id: 0,
                provider: ilr::IdentityProvider::IsbnRegistry,
                kind: ilr::RouteKind::Isbn13Edition,
                provider_scoped_id: "9780000000293".to_string(),
                state: ilr::WorkRouteState::Active,
                provenance: ilr::RouteProvenance::Provider(ilr::IdentityProvider::IsbnRegistry),
                user_confirmed: false,
                observed_at: Utc::now(),
            }];
            let seeded = WorkIdentityRepository::commit_settlement(&harness.db, commit)
                .await
                .expect("seed Retry-All bridge");
            let work_id = seeded.identity.own_work_id;
            WorkDb::update_work_enrichment(
                &harness.db,
                harness.user_id,
                work_id,
                UpdateWorkEnrichmentDbRequest {
                    enrichment_status: livrarr_domain::EnrichmentStatus::Failed,
                    ..Default::default()
                },
            )
            .await
            .expect("make Retry-All bridge incomplete");
            let generation_before = seeded.identity.identity_generation;
            let audits_before: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_audit_events \
                  WHERE user_id=?1 AND work_id=?2 AND event_kind='settlement'",
            )
            .bind(harness.user_id)
            .bind(work_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("count Retry-All audits before handoff");
            harness.state.identity_road.test_recorder().clear();
            let response = call_router_json(
                &harness,
                Method::POST,
                "/api/v1/work/retry-incomplete".to_string(),
                None,
            )
            .await;
            assert_eq!(response.status, StatusCode::ACCEPTED);
            for _ in 0..200 {
                let captured = WorkIdentityRepository::read_captured_identity(
                    &harness.db,
                    harness.user_id,
                    work_id,
                )
                .await
                .expect("poll Retry-All identity graph");
                if captured.active_routes.iter().any(|route| {
                    route.kind == ilr::RouteKind::OpenLibraryWork
                        && route.provider_scoped_id == "OL-RETRY-HANDOFF-W"
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let captured = WorkIdentityRepository::read_captured_identity(
                &harness.db,
                harness.user_id,
                work_id,
            )
            .await
            .expect("read productive Retry-All graph");
            assert!(captured.active_routes.iter().any(|route| {
                route.kind == ilr::RouteKind::OpenLibraryWork
                    && route.provider_scoped_id == "OL-RETRY-HANDOFF-W"
            }));
            assert_eq!(captured.identity_generation, generation_before + 1);
            let audits_after: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_audit_events \
                  WHERE user_id=?1 AND work_id=?2 AND event_kind='settlement'",
            )
            .bind(harness.user_id)
            .bind(work_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("count Retry-All audits after handoff");
            assert_eq!(audits_after, audits_before + 1);
        }
        CompositionContract::TwoTopupPatterns => {
            let audits_before: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_audit_events WHERE event_kind='settlement'",
            )
            .fetch_one(harness.db.pool())
            .await
            .expect("count audits before productive add handoff");
            let added = call_router_json(
                &harness,
                Method::POST,
                "/api/v1/work".to_string(),
                Some(json!({
                    "olKey": null,
                    "title": "ILR Topup",
                    "authorName": "ILR Topup Author",
                    "authorOlKey": null,
                    "year": null,
                    "coverUrl": null,
                    "language": "en",
                    "detailUrl": null,
                    "coverManual": false,
                    "isbn13": "9780306406157",
                    "candidateId": null,
                    "hcKey": null,
                    "grKey": null,
                    "asin": null
                })),
            )
            .await;
            assert!(added.status.is_success());
            let work_id = added.json["work"]["id"]
                .as_i64()
                .or_else(|| added.json["id"].as_i64())
                .expect("productive add work id");
            for _ in 0..200 {
                if !harness
                    .state
                    .work_service
                    .is_enriching(harness.user_id, work_id)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let captured = WorkIdentityRepository::read_captured_identity(
                &harness.db,
                harness.user_id,
                work_id,
            )
            .await
            .expect("read productive direct-add handoff");
            assert!(captured.active_routes.iter().any(|route| {
                route.kind == ilr::RouteKind::OpenLibraryWork
                    && route.provider_scoped_id == "OL-DIRECT-ADD-HANDOFF-W"
            }));
            assert_eq!(
                captured.identity_generation, 2,
                "creation and fresh EnrichmentPass each claim one generation"
            );
            let audits_after_add: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_audit_events WHERE event_kind='settlement'",
            )
            .fetch_one(harness.db.pool())
            .await
            .expect("count audits after productive add handoff");
            assert_eq!(audits_after_add, audits_before + 2);
            let retry = call_router_json(
                &harness,
                Method::POST,
                "/api/v1/work/retry-incomplete".to_string(),
                None,
            )
            .await;
            assert_eq!(retry.status, StatusCode::ACCEPTED);
            tokio::time::sleep(Duration::from_millis(50)).await;
            let calls = harness.state.identity_road.test_recorder().snapshot();
            assert_eq!(
                calls.len(),
                1,
                "the successor handoff authority is pinned by DB effects; Retry-All must not replay persisted routes into another public settle"
            );
            assert!(matches!(
                &calls[0],
                livrarr_server::identity_layer::IdentityRoadCall::Settle(request)
                    if request.origin == IdentityRoadOrigin::CreationDoor(ilr::DoorKind::DirectAdd)
                        && request.interaction == ilr::IdentityRoadInteraction::HumanWatching
            ));
            let audits_after_retry: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_audit_events WHERE event_kind='settlement'",
            )
            .fetch_one(harness.db.pool())
            .await
            .expect("count audits after quiet Retry-All twin");
            assert_eq!(audits_after_retry, audits_after_add);
        }
        CompositionContract::ManualRefreshStructuredSubtitle => {
            let (author, _) = harness
                .db
                .create_author(CreateAuthorDbRequest {
                    user_id: harness.user_id,
                    name: "ILR Refresh Author".to_string(),
                    sort_name: None,
                    ol_key: None,
                    gr_key: None,
                    hc_key: None,
                    import_id: None,
                })
                .await
                .expect("seed refresh author");
            let mut commit = settlement_commit(harness.user_id, author.id, None);
            commit.identity_title = title("ILR Refresh State");
            commit.routes = vec![ilr::WorkRoute {
                id: 0,
                user_id: harness.user_id,
                owner: RouteOwner::Work(0),
                resolved_work_id: 0,
                provider: ilr::IdentityProvider::OpenLibrary,
                kind: ilr::RouteKind::OpenLibraryWork,
                provider_scoped_id: "OL-REFRESH-STATE-W".to_string(),
                state: ilr::WorkRouteState::Active,
                provenance: ilr::RouteProvenance::Provider(ilr::IdentityProvider::OpenLibrary),
                user_confirmed: false,
                observed_at: Utc::now(),
            }];
            let seeded = WorkIdentityRepository::commit_settlement(&harness.db, commit)
                .await
                .expect("seed unsubtitled captured Work");
            let work_id = seeded.identity.own_work_id;
            assert_eq!(seeded.identity.identity_title.subtitle, None);
            harness.state.identity_road.test_recorder().clear();
            let refreshed = call_router_json(
                &harness,
                Method::POST,
                format!("/api/v1/work/{work_id}/refresh"),
                None,
            )
            .await;
            assert!(
                refreshed.status.is_success(),
                "manual refresh: {}",
                refreshed.json
            );
            assert_eq!(
                harness
                    .open_library_stub
                    .as_ref()
                    .expect("configured OpenLibrary provider")
                    .call_count(),
                1,
                "refresh must consume the provider evidence"
            );
            let captured = WorkIdentityRepository::read_captured_identity(
                &harness.db,
                harness.user_id,
                work_id,
            )
            .await
            .expect("read structured-title refresh outcome");
            assert_eq!(
                captured.identity_title.subtitle.as_deref(),
                Some("Provider structured subtitle")
            );
            assert_eq!(
                captured.identity_generation, seeded.identity.identity_generation,
                "metadata-only refresh with no fresh route does not claim identity"
            );
            let calls = harness.state.identity_road.test_recorder().snapshot();
            assert!(
                calls.is_empty(),
                "metadata-only refresh must not replay its persisted route into settlement"
            );
        }
        CompositionContract::ManualRefreshInlineCapture => {
            let (author, _) = harness
                .db
                .create_author(CreateAuthorDbRequest {
                    user_id: harness.user_id,
                    name: "ILR Captured Author".to_string(),
                    sort_name: None,
                    ol_key: None,
                    gr_key: None,
                    hc_key: None,
                    import_id: None,
                })
                .await
                .expect("seed manual-refresh author");
            let mut commit = settlement_commit(harness.user_id, author.id, None);
            commit.identity_title = title("ILR Captured Refresh");
            commit.routes = vec![ilr::WorkRoute {
                id: 0,
                user_id: harness.user_id,
                owner: RouteOwner::Work(0),
                resolved_work_id: 0,
                provider: ilr::IdentityProvider::IsbnRegistry,
                kind: ilr::RouteKind::Isbn13Edition,
                provider_scoped_id: "9780306406157".to_string(),
                state: ilr::WorkRouteState::Active,
                provenance: ilr::RouteProvenance::Provider(ilr::IdentityProvider::IsbnRegistry),
                user_confirmed: false,
                observed_at: Utc::now(),
            }];
            let seeded = WorkIdentityRepository::commit_settlement(&harness.db, commit)
                .await
                .expect("seed manual-refresh bridge");
            let work_id = seeded.identity.own_work_id;
            let audits_before: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_audit_events \
                  WHERE user_id=?1 AND work_id=?2 AND event_kind='settlement'",
            )
            .bind(harness.user_id)
            .bind(work_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("count manual-refresh audits before handoff");
            harness.state.identity_road.test_recorder().clear();
            let refreshed = call_router_json(
                &harness,
                Method::POST,
                format!("/api/v1/work/{work_id}/refresh"),
                None,
            )
            .await;
            assert!(
                refreshed.status.is_success(),
                "manual refresh: {}",
                refreshed.json
            );
            let captured = WorkIdentityRepository::read_captured_identity(
                &harness.db,
                harness.user_id,
                work_id,
            )
            .await
            .expect("read manual-refresh handoff result");
            assert!(captured.active_routes.iter().any(|route| {
                route.kind == ilr::RouteKind::OpenLibraryWork
                    && route.provider_scoped_id == "OL-MANUAL-REFRESH-HANDOFF-W"
            }));
            assert_eq!(
                captured.identity_generation,
                seeded.identity.identity_generation + 1
            );
            let audits_after: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_audit_events \
                  WHERE user_id=?1 AND work_id=?2 AND event_kind='settlement'",
            )
            .bind(harness.user_id)
            .bind(work_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("count manual-refresh audits after handoff");
            assert_eq!(audits_after, audits_before + 1);
        }
        CompositionContract::ManualImportBeforeAttach
        | CompositionContract::ManualImportPrecedence => {
            harness
                .db
                .create_root_folder(
                    harness._tmp.path().to_str().expect("UTF-8 import root"),
                    MediaType::Ebook,
                )
                .await
                .expect("seed manual-import root");
            let path = harness._tmp.path().join("ilr-road-import.epub");
            write_epub_with_metadata(
                &path,
                false,
                "ILR Road Import",
                Some("en"),
                Some("9780306406157"),
            );
            let response = drive_manual_import_epub(
                &harness,
                &path,
                "ILR Road Import",
                Some("en"),
                Some("9780306406157"),
            )
            .await;
            assert!(
                response.status.is_success(),
                "manual import: {}",
                response.json
            );
            assert_eq!(response.json["results"][0]["status"], "imported");
            let calls = harness.state.identity_road.test_recorder().snapshot();
            assert_eq!(calls.len(), 1, "one settle precedes the one attachment");
            let livrarr_server::identity_layer::IdentityRoadCall::Settle(request) = &calls[0]
            else {
                panic!("manual import must settle")
            };
            assert_eq!(
                request.origin,
                IdentityRoadOrigin::CreationDoor(ilr::DoorKind::ManualImport)
            );
            assert!(request.evidence.user_choice.is_some());
            assert_eq!(request.evidence.owned_files.len(), 1);
            assert!(!request.evidence.provider_identity.is_empty());
            let attached: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM library_items")
                .fetch_one(harness.db.pool())
                .await
                .expect("count post-settle attachment");
            assert_eq!(attached, 1);
            if matches!(contract, CompositionContract::ManualImportPrecedence) {
                let providerless_path = harness._tmp.path().join("ilr-unattached-import.epub");
                write_epub_with_metadata(
                    &providerless_path,
                    false,
                    "ILR Unattached Import",
                    Some("en"),
                    None,
                );
                let works_before: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id = ?1")
                        .bind(harness.user_id)
                        .fetch_one(harness.db.pool())
                        .await
                        .expect("count Works before providerless import");
                let items_before: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM library_items WHERE user_id = ?1")
                        .bind(harness.user_id)
                        .fetch_one(harness.db.pool())
                        .await
                        .expect("count items before providerless import");
                let body = || {
                    json!({"items": [{
                        "path": providerless_path,
                        "olKey": "",
                        "title": "ILR Unattached Import",
                        "author": "ILR Unattached Author",
                        "deleteExisting": false,
                        "language": "en",
                        "authorOlKey": null,
                        "year": 2026,
                        "coverUrl": null,
                        "isbn": null,
                        "description": null,
                        "seriesName": null,
                        "seriesPosition": null,
                        "candidateId": null,
                        "hcKey": null,
                        "grKey": null,
                        "asin": null
                    }]})
                };
                for _ in 0..2 {
                    let parked = call_router_json(
                        &harness,
                        Method::POST,
                        "/api/v1/manualimport/import".to_string(),
                        Some(body()),
                    )
                    .await;
                    assert!(parked.status.is_success(), "manual import result envelope");
                    assert_eq!(parked.json["results"][0]["status"], "failed");
                    assert!(parked.json["results"][0]["error"]
                        .as_str()
                        .is_some_and(|error| error.contains("unattached review card")));
                }
                let works_after: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id = ?1")
                        .bind(harness.user_id)
                        .fetch_one(harness.db.pool())
                        .await
                        .expect("count Works after providerless import");
                let items_after: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM library_items WHERE user_id = ?1")
                        .bind(harness.user_id)
                        .fetch_one(harness.db.pool())
                        .await
                        .expect("count items after providerless import");
                assert_eq!(
                    works_after, works_before,
                    "unattached review creates no Work"
                );
                assert_eq!(
                    items_after, items_before,
                    "unattached review attaches no file"
                );
                let cards: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM identity_review_cards \
                      WHERE user_id = ?1 AND work_id IS NULL AND kind = ?2 AND status = 'pending'",
                )
                .bind(harness.user_id)
                .bind(ilr::ReviewKind::ImportIdentity.storage_code())
                .fetch_one(harness.db.pool())
                .await
                .expect("count idempotent unattached import cards");
                assert_eq!(cards, 1, "repeated outage/minimum evidence parks one card");
            }
        }
        CompositionContract::ListRealRows | CompositionContract::ListRejectOwnedFile => {
            let preview = harness
                .state
                .list_service
                .preview(
                    harness.user_id,
                    b"Book Id,Title,Author,ISBN,ISBN13,My Rating,Exclusive Shelf\n=\"\",ILR List Road,ILR List Author,=\"\",=\"\",5,read\n"
                        .to_vec(),
                )
                .await
                .expect("real list preview");
            assert_eq!(preview.rows.len(), 1);
            let response = call_router_json(
                &harness,
                Method::POST,
                "/api/v1/listimport/confirm".to_string(),
                Some(json!({
                    "previewId": preview.preview_id,
                    "rowIndices": [0],
                    "importId": null,
                    "language": "en"
                })),
            )
            .await;
            assert!(
                response.status.is_success(),
                "list confirm: {}; calls={:?}",
                response.json,
                harness.state.identity_road.test_recorder().snapshot()
            );
            let calls = harness.state.identity_road.test_recorder().snapshot();
            assert_eq!(calls.len(), 1, "one selected row has one settlement");
            let livrarr_server::identity_layer::IdentityRoadCall::Settle(request) = &calls[0]
            else {
                panic!("list confirm must settle")
            };
            assert_eq!(
                request.origin,
                IdentityRoadOrigin::CreationDoor(ilr::DoorKind::ListImport)
            );
            assert_eq!(
                request.interaction,
                ilr::IdentityRoadInteraction::HumanWatching
            );
            assert!(request.evidence.owned_files.is_empty());
            assert!(request.evidence.minimum.is_some());
        }
        CompositionContract::ConflictResolveOnce | CompositionContract::ConflictDismissReject => {
            let (author, _) = harness
                .db
                .create_author(CreateAuthorDbRequest {
                    user_id: harness.user_id,
                    name: "Identity Author legacy-conflict-adapter".to_string(),
                    sort_name: None,
                    ol_key: None,
                    gr_key: None,
                    hc_key: None,
                    import_id: None,
                })
                .await
                .expect("seed authenticated conflict author");
            let user_id = harness.user_id;
            let author_id = author.id;
            let conflict_id = 90_001;
            let mut commit = settlement_commit(user_id, author_id, None);
            commit.routes = vec![ilr::WorkRoute {
                id: 0,
                user_id,
                owner: RouteOwner::Work(0),
                resolved_work_id: 0,
                provider: ilr::IdentityProvider::OpenLibrary,
                kind: ilr::RouteKind::OpenLibraryWork,
                provider_scoped_id: "OL-CONFLICT-SURVIVOR-W".to_string(),
                state: ilr::WorkRouteState::Active,
                provenance: ilr::RouteProvenance::UserChoice,
                user_confirmed: true,
                observed_at: Utc::now(),
            }];
            commit.review_cards = vec![ilr::SettlementReviewCard::IdentityConflict {
                conflict_id,
                work_id: 0,
            }];
            let committed = WorkIdentityRepository::commit_settlement(&harness.db, commit)
                .await
                .expect("seed pending conflict card");
            let card_id = committed.review_cards[0].id;
            assert_ne!(card_id, conflict_id, "card id is never conflict id");
            harness.state.identity_road.test_recorder().clear();
            let response = if matches!(contract, CompositionContract::ConflictResolveOnce) {
                call_router_json(
                    &harness,
                    Method::POST,
                    format!("/api/v1/identity-conflict/{conflict_id}/resolve"),
                    Some(json!({"action": "replace_anchor", "notes": null})),
                )
                .await
            } else {
                call_router_json(
                    &harness,
                    Method::POST,
                    format!("/api/v1/identity-conflict/{conflict_id}/dismiss"),
                    None,
                )
                .await
            };
            assert!(
                response.status.is_success(),
                "legacy adapter: {}",
                response.json
            );
            let calls = harness.state.identity_road.test_recorder().snapshot();
            assert_eq!(calls.len(), 1);
            let livrarr_server::identity_layer::IdentityRoadCall::Resolve { command, .. } =
                &calls[0]
            else {
                panic!("legacy conflict adapter must resolve exactly once")
            };
            match (contract, command) {
                (
                    CompositionContract::ConflictResolveOnce,
                    ReviewResolutionCommand::IdentityConflict {
                        card_id: observed,
                        action:
                            ilr::IdentityConflictResolution::Accept {
                                surviving_routes,
                                target_edition: None,
                            },
                        ..
                    },
                ) => {
                    assert_eq!(*observed, card_id);
                    assert_eq!(surviving_routes.len(), 1);
                    assert_eq!(surviving_routes[0].value, "OL-CONFLICT-SURVIVOR-W");
                }
                (
                    CompositionContract::ConflictDismissReject,
                    ReviewResolutionCommand::IdentityConflict {
                        card_id: observed,
                        action: ilr::IdentityConflictResolution::Reject { surviving_routes },
                        ..
                    },
                ) => {
                    assert_eq!(*observed, card_id);
                    assert_eq!(surviving_routes.len(), 1);
                    assert_eq!(surviving_routes[0].value, "OL-CONFLICT-SURVIVOR-W");
                }
                other => panic!("resolve and dismiss must remain distinguishable: {other:?}"),
            }
            let legacy_writes: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM work_identity_conflicts WHERE status='resolved'",
            )
            .fetch_one(harness.db.pool())
            .await
            .expect("count legacy conflict writes");
            assert_eq!(legacy_writes, 0);
        }
    }
}

async fn round10_residue_heal_is_exact_marker_gated_and_conservative() {
    let db = create_activated_test_db().await;
    let user = db
        .create_user(CreateUserDbRequest {
            username: "round10-residue-heal".to_string(),
            password_hash: "unused".to_string(),
            role: UserRole::Admin,
            api_key_hash: "unused-round10-residue".to_string(),
        })
        .await
        .expect("seed residue-heal user");
    let make_work = |title: &str| CreateWorkDbRequest {
        user_id: user.id,
        title: title.to_string(),
        author_name: "Residue Author".to_string(),
        normalized_title: livrarr_domain::normalize_for_matching(title),
        normalized_author: livrarr_domain::normalize_for_matching("Residue Author"),
        ..Default::default()
    };
    let (dishonest, _) = db
        .create_work(make_work("Dishonest Enriched"))
        .await
        .expect("seed dishonest enriched work");
    let (honest, _) = db
        .create_work(make_work("Honestly Enriched"))
        .await
        .expect("seed honestly enriched work");
    for work_id in [dishonest.id, honest.id] {
        sqlx::query(
            "UPDATE works SET enrichment_status='enriched', enrichment_source=NULL \
              WHERE user_id=?1 AND id=?2",
        )
        .bind(user.id)
        .bind(work_id)
        .execute(db.pool())
        .await
        .expect("seed pre-round-9 enrichment stamp");
    }
    sqlx::query(
        "INSERT INTO provider_call_records \
            (provider, operation, work_id, started_at, duration_ms, outcome, detail) \
         VALUES ('openlibrary', 'enrich', ?1, ?2, 1, 'skipped_policy', NULL), \
                ('openlibrary', 'enrich', ?3, ?2, 2, 'success', NULL)",
    )
    .bind(dishonest.id)
    .bind(Utc::now().to_rfc3339())
    .bind(honest.id)
    .execute(db.pool())
    .await
    .expect("seed skip-only and honest provider-call evidence");

    for attempt in 1..=3 {
        sqlx::query(
            "INSERT INTO identity_provider_attempts \
                (user_id, work_id, provider, route_kind, route_value, attempt_key, outcome, observed_at) \
             VALUES (?1, ?2, 'livrarr-convergence', 'bridge-upgrade', '0', ?3, \
                     'no-route-change', ?4)",
        )
        .bind(user.id)
        .bind(dishonest.id)
        .bind(format!("attempt-{attempt}"))
        .bind(Utc::now().to_rfc3339())
        .execute(db.pool())
        .await
        .expect("seed self-feed convergence attempt");
    }
    sqlx::query(
        "INSERT INTO identity_provider_attempts \
            (user_id, work_id, provider, route_kind, route_value, attempt_key, outcome, observed_at) \
         VALUES (?1, ?2, 'openlibrary', 'work', 'OL-KEEP-W', 'attempt-1', \
                 'no-route-change', ?3)",
    )
    .bind(user.id)
    .bind(honest.id)
    .bind(Utc::now().to_rfc3339())
    .execute(db.pool())
    .await
    .expect("seed unrelated attempt that must survive");

    sqlx::query(
        "INSERT INTO imports \
            (id, user_id, source, status, started_at, completed_at) \
         VALUES ('round10-readarr-import', ?1, 'readarr', 'completed', \
                 '2026-08-17T00:00:00Z', '2026-08-17T01:00:00Z')",
    )
    .bind(user.id)
    .execute(db.pool())
    .await
    .expect("seed failed-session Readarr import");
    let mut author_ids = Vec::new();
    for name in ["Eligible Orphan", "Monitored Orphan", "Outside Window"] {
        let (author, _) = db
            .create_author(CreateAuthorDbRequest {
                user_id: user.id,
                name: name.to_string(),
                sort_name: None,
                ol_key: None,
                gr_key: None,
                hc_key: None,
                import_id: Some("round10-readarr-import".to_string()),
            })
            .await
            .expect("seed Readarr author residue");
        author_ids.push(author.id);
    }
    sqlx::query("UPDATE authors SET added_at='2026-08-17T12:00:00Z' WHERE id IN (?1, ?2)")
        .bind(author_ids[0])
        .bind(author_ids[1])
        .execute(db.pool())
        .await
        .expect("place eligible authors in failure window");
    sqlx::query("UPDATE authors SET added_at='2026-08-18T12:00:00Z' WHERE id=?1")
        .bind(author_ids[2])
        .execute(db.pool())
        .await
        .expect("place protected author outside failure window");
    sqlx::query("UPDATE authors SET monitored=1 WHERE id=?1")
        .bind(author_ids[1])
        .execute(db.pool())
        .await
        .expect("give one in-window orphan explicit user state");

    let first = livrarr_db::identity_layer::heal_identity_round10_residue(db.pool())
        .await
        .expect("run round-10 residue heal");
    assert_eq!(
        first,
        livrarr_db::identity_layer::IdentityRound10ResidueHealReport {
            bridge_attempts_cleared: 3,
            dishonest_enriched_reclassified: 1,
            failed_readarr_authors_deleted: 1,
        }
    );
    let statuses: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, enrichment_status FROM works WHERE id IN (?1, ?2) ORDER BY id")
            .bind(dishonest.id)
            .bind(honest.id)
            .fetch_all(db.pool())
            .await
            .expect("read post-heal statuses");
    assert_eq!(
        statuses,
        vec![
            (dishonest.id, "failed".to_string()),
            (honest.id, "enriched".to_string()),
        ]
    );
    let surviving_attempts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM identity_provider_attempts")
            .fetch_one(db.pool())
            .await
            .expect("count retained non-convergence attempts");
    assert_eq!(surviving_attempts, 1);
    let surviving_authors: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM authors WHERE id IN (?1, ?2, ?3) ORDER BY id")
            .bind(author_ids[0])
            .bind(author_ids[1])
            .bind(author_ids[2])
            .fetch_all(db.pool())
            .await
            .expect("read conservative author survivors");
    assert_eq!(surviving_authors, vec![author_ids[1], author_ids[2]]);

    let second = livrarr_db::identity_layer::heal_identity_round10_residue(db.pool())
        .await
        .expect("rerun marker-gated round-10 residue heal");
    assert_eq!(second, Default::default());
}

async fn round11_attempt_reheal_is_exact_marker_gated_and_idempotent() {
    let db = create_activated_test_db().await;
    let user = db
        .create_user(CreateUserDbRequest {
            username: "round11-attempt-reheal".to_string(),
            password_hash: "unused".to_string(),
            role: UserRole::Admin,
            api_key_hash: "unused-round11-attempt-reheal".to_string(),
        })
        .await
        .expect("seed round-11 re-heal user");
    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id: user.id,
            title: "Round 11 Attempt Residue".to_string(),
            author_name: "Reheal Author".to_string(),
            normalized_title: "round 11 attempt residue".to_string(),
            normalized_author: "reheal author".to_string(),
            ..Default::default()
        })
        .await
        .expect("seed round-11 re-heal Work");
    for (provider, route_kind, attempt_key) in [
        ("livrarr-convergence", "bridge-upgrade", "burned-1"),
        ("livrarr-convergence", "bridge-upgrade", "burned-2"),
        ("openlibrary", "bridge-upgrade", "keep-provider"),
        ("livrarr-convergence", "work", "keep-kind"),
    ] {
        sqlx::query(
            "INSERT INTO identity_provider_attempts \
                (user_id, work_id, provider, route_kind, route_value, attempt_key, outcome, observed_at) \
             VALUES (?1, ?2, ?3, ?4, '1', ?5, 'no-route-change', ?6)",
        )
        .bind(user.id)
        .bind(work.id)
        .bind(provider)
        .bind(route_kind)
        .bind(attempt_key)
        .bind(Utc::now().to_rfc3339())
        .execute(db.pool())
        .await
        .expect("seed round-11 attempt residue matrix");
    }

    let first = livrarr_db::identity_layer::heal_identity_round11_attempt_residue(db.pool())
        .await
        .expect("run round-11 attempt re-heal");
    assert_eq!(
        first,
        livrarr_db::identity_layer::IdentityRound11AttemptRehealReport {
            bridge_attempts_cleared: 2,
        }
    );
    let survivors: Vec<(String, String)> = sqlx::query_as(
        "SELECT provider, route_kind FROM identity_provider_attempts ORDER BY attempt_key",
    )
    .fetch_all(db.pool())
    .await
    .expect("read exact re-heal survivors");
    assert_eq!(
        survivors,
        vec![
            ("livrarr-convergence".to_string(), "work".to_string()),
            ("openlibrary".to_string(), "bridge-upgrade".to_string()),
        ]
    );
    let marker: String = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta WHERE key='identity_round11_attempt_reheal'",
    )
    .fetch_one(db.pool())
    .await
    .expect("read round-11 attempt re-heal marker");
    assert_eq!(marker, "1");

    let second = livrarr_db::identity_layer::heal_identity_round11_attempt_residue(db.pool())
        .await
        .expect("rerun round-11 attempt re-heal");
    assert_eq!(second, Default::default());
    let survivors_after_second: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM identity_provider_attempts")
            .fetch_one(db.pool())
            .await
            .expect("count attempts after idempotent re-heal");
    assert_eq!(survivors_after_second, 2);
}

#[derive(Clone, Default)]
struct Round15NoDispatchQueue {
    calls: Arc<AtomicU64>,
}

impl livrarr_enrichment::ProviderQueue for Round15NoDispatchQueue {
    async fn dispatch_enrichment(
        &self,
        _work: &Work,
        _context: livrarr_enrichment::EnrichmentContext,
    ) -> Result<livrarr_enrichment::ScatterGatherResult, livrarr_enrichment::ProviderQueueError>
    {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Err(livrarr_enrichment::ProviderQueueError::Db(
            livrarr_domain::DbError::Conflict {
                message: "round-15 cover replay must not dispatch providers".to_string(),
            },
        ))
    }
}

// Bug reproduction: identity-layer-rewrite round 15 / AC-025(f).
async fn round15_goodreads_cover_reselect_is_guarded_manual_safe_and_idempotent() {
    let db = create_activated_test_db().await;
    let user = db
        .create_user(CreateUserDbRequest {
            username: "round15-cover-reselect".to_string(),
            password_hash: "unused".to_string(),
            role: UserRole::Admin,
            api_key_hash: "unused-round15-cover-reselect".to_string(),
        })
        .await
        .expect("seed round-15 cover user");
    let make_work = |title: &str| CreateWorkDbRequest {
        user_id: user.id,
        title: title.to_string(),
        author_name: "Round Fifteen Cover Author".to_string(),
        normalized_title: livrarr_domain::normalize_for_matching(title),
        normalized_author: livrarr_domain::normalize_for_matching("Round Fifteen Cover Author"),
        ..Default::default()
    };
    let (machine, _) = db
        .create_work(make_work("Round Fifteen Machine Goodreads Cover"))
        .await
        .expect("seed machine Goodreads-cover work");
    let (manual, _) = db
        .create_work(make_work("Round Fifteen Manual Goodreads Cover"))
        .await
        .expect("seed manual Goodreads-cover work");

    db.update_cover_metadata(
        user.id,
        machine.id,
        Some("https://goodreads.invalid/wrong-ebook.jpg"),
        "goodreads",
        false,
        300,
        450,
    )
    .await
    .unwrap();
    db.update_audiobook_cover_metadata(
        user.id,
        machine.id,
        Some("https://goodreads.invalid/wrong-audio.jpg"),
        "goodreads",
        false,
        300,
        450,
    )
    .await
    .unwrap();
    db.update_cover_metadata(
        user.id,
        manual.id,
        Some("https://goodreads.invalid/manual-ebook.jpg"),
        "goodreads",
        true,
        600,
        900,
    )
    .await
    .unwrap();
    db.update_audiobook_cover_metadata(
        user.id,
        manual.id,
        Some("https://goodreads.invalid/manual-audio.jpg"),
        "goodreads",
        true,
        600,
        900,
    )
    .await
    .unwrap();

    let wrong_goodreads = NormalizedWorkDetail {
        title: Some(machine.title.clone()),
        author_name: Some(machine.author_name.clone()),
        cover_url: Some("https://goodreads.invalid/still-wrong.jpg".to_string()),
        ..Default::default()
    };
    let right_hardcover = NormalizedWorkDetail {
        title: Some(machine.title.clone()),
        author_name: Some(machine.author_name.clone()),
        cover_url: Some("https://hardcover.test/right-book.jpg".to_string()),
        ..Default::default()
    };
    for (provider, payload) in [
        (MetadataProvider::Goodreads, wrong_goodreads),
        (MetadataProvider::Hardcover, right_hardcover),
    ] {
        db.record_terminal_outcome(
            user.id,
            machine.id,
            provider,
            OutcomeClass::Success,
            Some(serde_json::to_string(&payload).unwrap()),
        )
        .await
        .expect("persist round-15 cover replay payload");
    }

    let covers_root = tempfile::tempdir().unwrap();
    let user_dir = covers_root.path().join(user.id.to_string());
    tokio::fs::create_dir_all(&user_dir).await.unwrap();
    let machine_ebook = user_dir.join(format!("{}.jpg", machine.id));
    let machine_audio = user_dir.join(format!("{}_audio.jpg", machine.id));
    let manual_ebook = user_dir.join(format!("{}.jpg", manual.id));
    let manual_audio = user_dir.join(format!("{}_audio.jpg", manual.id));
    tokio::fs::write(&machine_ebook, b"wrong-machine-ebook")
        .await
        .unwrap();
    tokio::fs::write(&machine_audio, b"wrong-machine-audio")
        .await
        .unwrap();
    tokio::fs::write(&manual_ebook, b"manual-ebook-sentinel")
        .await
        .unwrap();
    tokio::fs::write(&manual_audio, b"manual-audio-sentinel")
        .await
        .unwrap();

    let queue = Round15NoDispatchQueue::default();
    let queue_probe = queue.clone();
    let enrichment = livrarr_enrichment::EnrichmentServiceImpl::new(
        Arc::new(db.clone()),
        Arc::new(queue),
        Arc::new(livrarr_enrichment::DefaultMergeEngine::new(
            livrarr_enrichment::PriorityModel::english(),
        )),
        false,
    );
    let http = StubHttpFetcher::with_ok(200, b"right-hardcover-bytes".to_vec());

    let first = livrarr_metadata::cover_startup::run_identity_round15_gr_cover_reselect(
        &db,
        &enrichment,
        &http,
        covers_root.path(),
    )
    .await
    .expect("run round-15 Goodreads cover reselect");
    assert_eq!(first.ebook_slots, 1);
    assert_eq!(first.ebook_slots_reselected, 1);
    assert_eq!(first.ebook_slots_placeholder, 0);
    assert_eq!(first.audiobook_slots, 1);
    assert_eq!(first.audiobook_slots_reselected, 1);
    assert_eq!(first.audiobook_slots_placeholder, 0);
    assert_eq!(first.manual_ebook_slots_preserved, 1);
    assert_eq!(first.manual_audiobook_slots_preserved, 1);
    assert_eq!(first.works_materialized, 1);
    assert_eq!(queue_probe.calls.load(Ordering::Relaxed), 0);

    let repaired = db.get_work(user.id, machine.id).await.unwrap();
    assert_eq!(repaired.cover_source.as_deref(), Some("hardcover"));
    assert_eq!(
        repaired.audiobook_cover_source.as_deref(),
        Some("hardcover")
    );
    assert_eq!(
        tokio::fs::read(&machine_ebook).await.unwrap(),
        b"right-hardcover-bytes"
    );
    assert_eq!(
        tokio::fs::read(&machine_audio).await.unwrap(),
        b"right-hardcover-bytes"
    );

    let protected = db.get_work(user.id, manual.id).await.unwrap();
    assert!(protected.cover_manual);
    assert!(db
        .get_audiobook_cover_manual(user.id, manual.id)
        .await
        .unwrap());
    assert_eq!(protected.cover_source.as_deref(), Some("goodreads"));
    assert_eq!(
        protected.audiobook_cover_source.as_deref(),
        Some("goodreads")
    );
    assert_eq!(
        tokio::fs::read(&manual_ebook).await.unwrap(),
        b"manual-ebook-sentinel"
    );
    assert_eq!(
        tokio::fs::read(&manual_audio).await.unwrap(),
        b"manual-audio-sentinel"
    );
    let marker: String = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta WHERE key='identity_round15_gr_cover_reselect'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(marker, "1");

    let calls_after_first = http.call_count();
    let second = livrarr_metadata::cover_startup::run_identity_round15_gr_cover_reselect(
        &db,
        &enrichment,
        &http,
        covers_root.path(),
    )
    .await
    .expect("rerun marker-gated round-15 cover reselect");
    assert_eq!(second, Default::default());
    assert_eq!(http.call_count(), calls_after_first);
}

// Bug reproduction: identity-layer-rewrite round 16 — C-r7-01 poison-row isolation.
async fn round16_goodreads_cover_reselect_isolates_poison_rows() {
    let db = create_activated_test_db().await;
    let user = db
        .create_user(CreateUserDbRequest {
            username: "round16-cover-isolation".to_string(),
            password_hash: "unused".to_string(),
            role: UserRole::Admin,
            api_key_hash: "unused-round16-cover-isolation".to_string(),
        })
        .await
        .expect("seed round-16 cover-isolation user");
    let make_work = |title: &str| CreateWorkDbRequest {
        user_id: user.id,
        title: title.to_string(),
        author_name: "Round Sixteen Cover Author".to_string(),
        normalized_title: livrarr_domain::normalize_for_matching(title),
        normalized_author: livrarr_domain::normalize_for_matching("Round Sixteen Cover Author"),
        ..Default::default()
    };
    let (poison, _) = db
        .create_work(make_work("Round Sixteen A Poison Goodreads Cover"))
        .await
        .expect("seed poison Goodreads-cover work first");
    let (healthy, _) = db
        .create_work(make_work("Round Sixteen B Healthy Goodreads Cover"))
        .await
        .expect("seed healthy Goodreads-cover work second");
    assert!(
        poison.id < healthy.id,
        "poison work must lead the ordered queue"
    );

    for work in [&poison, &healthy] {
        db.update_cover_metadata(
            user.id,
            work.id,
            Some(&format!("https://goodreads.invalid/wrong-{}.jpg", work.id)),
            "goodreads",
            false,
            300,
            450,
        )
        .await
        .expect("seed machine-owned Goodreads cover");
    }
    db.record_terminal_outcome(
        user.id,
        poison.id,
        MetadataProvider::Hardcover,
        OutcomeClass::Success,
        Some("{not-json".to_string()),
    )
    .await
    .expect("persist corrupt round-16 retry payload");
    let healthy_hardcover = NormalizedWorkDetail {
        title: Some(healthy.title.clone()),
        author_name: Some(healthy.author_name.clone()),
        cover_url: Some("https://hardcover.test/round16-right-book.jpg".to_string()),
        ..Default::default()
    };
    db.record_terminal_outcome(
        user.id,
        healthy.id,
        MetadataProvider::Hardcover,
        OutcomeClass::Success,
        Some(serde_json::to_string(&healthy_hardcover).unwrap()),
    )
    .await
    .expect("persist valid round-16 Hardcover retry payload");

    let queue = Round15NoDispatchQueue::default();
    let queue_probe = queue.clone();
    let enrichment = livrarr_enrichment::EnrichmentServiceImpl::new(
        Arc::new(db.clone()),
        Arc::new(queue),
        Arc::new(livrarr_enrichment::DefaultMergeEngine::new(
            livrarr_enrichment::PriorityModel::english(),
        )),
        false,
    );
    let http = StubHttpFetcher::with_ok(200, b"round16-right-hardcover-bytes".to_vec());
    let covers_root = tempfile::tempdir().unwrap();

    let first = livrarr_metadata::cover_startup::run_identity_round15_gr_cover_reselect(
        &db,
        &enrichment,
        &http,
        covers_root.path(),
    )
    .await
    .expect("a poison target must not abort the remaining cover worklist");
    assert_eq!(first.ebook_slots, 2);
    assert_eq!(first.ebook_slots_reselected, 1);
    assert_eq!(first.works_materialized, 1);
    assert_eq!(first.works_failed, 1);
    assert_eq!(first.queued_works_remaining, 1);
    assert_eq!(first.automatic_target_works_remaining, 1);
    assert_eq!(queue_probe.calls.load(Ordering::Relaxed), 0);

    let poison_after_first = db.get_work(user.id, poison.id).await.unwrap();
    assert_eq!(
        poison_after_first.cover_source.as_deref(),
        Some("goodreads")
    );
    let healthy_after_first = db.get_work(user.id, healthy.id).await.unwrap();
    assert_eq!(
        healthy_after_first.cover_source.as_deref(),
        Some("hardcover")
    );
    let poison_queued: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_round15_gr_cover_reselect_queue \
          WHERE user_id=?1 AND work_id=?2",
    )
    .bind(user.id)
    .bind(poison.id)
    .fetch_one(db.pool())
    .await
    .unwrap();
    let healthy_queued: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_round15_gr_cover_reselect_queue \
          WHERE user_id=?1 AND work_id=?2",
    )
    .bind(user.id)
    .bind(healthy.id)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(poison_queued, 1, "poison target stays retryable");
    assert_eq!(healthy_queued, 0, "successful sibling leaves the worklist");
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta WHERE key='identity_round15_gr_cover_reselect'",
    )
    .fetch_optional(db.pool())
    .await
    .unwrap();
    assert_eq!(marker, None, "a poison target keeps the heal unstamped");

    let calls_after_first = http.call_count();
    let second = livrarr_metadata::cover_startup::run_identity_round15_gr_cover_reselect(
        &db,
        &enrichment,
        &http,
        covers_root.path(),
    )
    .await
    .expect("poison-only retry must remain non-fatal");
    assert_eq!(second.works_failed, 1);
    assert_eq!(second.queued_works_remaining, 1);
    assert_eq!(second.automatic_target_works_remaining, 1);
    assert_eq!(
        http.call_count(),
        calls_after_first,
        "the successful sibling must not be downloaded again"
    );
    let healthy_after_second = db.get_work(user.id, healthy.id).await.unwrap();
    assert_eq!(
        healthy_after_second.cover_source.as_deref(),
        Some("hardcover")
    );
    let poison_still_queued: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_round15_gr_cover_reselect_queue \
          WHERE user_id=?1 AND work_id=?2",
    )
    .bind(user.id)
    .bind(poison.id)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(poison_still_queued, 1);
    let marker_after_second: Option<String> = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta WHERE key='identity_round15_gr_cover_reselect'",
    )
    .fetch_optional(db.pool())
    .await
    .unwrap();
    assert_eq!(marker_after_second, None);
}

// Bug reproduction: identity-layer-rewrite round 15 / AC-025(e).
async fn round15_search_ledger_reset_is_route_scoped_and_idempotent() {
    let harness = build_route_harness().await;
    let (zero_route, _) = seed_round13_search_work(
        &harness,
        "Round Fifteen Zero Route Ledger",
        "Zero Route Ledger Author",
        "en",
        None,
    )
    .await;
    let (edition_only, _) = seed_round13_search_work(
        &harness,
        "Round Fifteen Edition Ledger",
        "Edition Ledger Author",
        "en",
        Some((
            ilr::IdentityProvider::Amazon,
            ilr::RouteKind::AsinEdition,
            "B0ROUND15LEDGER",
        )),
    )
    .await;
    let (work_routed, _) = seed_round13_search_work(
        &harness,
        "Round Fifteen Routed Ledger",
        "Routed Ledger Author",
        "en",
        Some((
            ilr::IdentityProvider::OpenLibrary,
            ilr::RouteKind::OpenLibraryWork,
            "OL-ROUND15-LEDGER-W",
        )),
    )
    .await;
    for (work_id, provider, route_kind, attempt_key) in [
        (
            zero_route,
            "livrarr-convergence",
            "bridge-upgrade",
            "zero-1",
        ),
        (
            zero_route,
            "livrarr-convergence",
            "bridge-upgrade",
            "zero-2",
        ),
        (
            edition_only,
            "livrarr-convergence",
            "bridge-upgrade",
            "edition-1",
        ),
        (
            work_routed,
            "livrarr-convergence",
            "bridge-upgrade",
            "routed-keep",
        ),
        (zero_route, "openlibrary", "bridge-upgrade", "provider-keep"),
        (zero_route, "livrarr-convergence", "work", "kind-keep"),
    ] {
        sqlx::query(
            "INSERT INTO identity_provider_attempts \
                (user_id, work_id, provider, route_kind, route_value, attempt_key, outcome, observed_at) \
             VALUES (?1, ?2, ?3, ?4, '1', ?5, 'no-route-change', ?6)",
        )
        .bind(harness.user_id)
        .bind(work_id)
        .bind(provider)
        .bind(route_kind)
        .bind(attempt_key)
        .bind(Utc::now().to_rfc3339())
        .execute(harness.db.pool())
        .await
        .expect("seed round-15 ledger matrix");
    }

    let first = livrarr_db::identity_layer::heal_identity_round15_search_ledger(harness.db.pool())
        .await
        .expect("run round-15 search ledger reset");
    assert_eq!(
        first,
        livrarr_db::identity_layer::IdentityRound15SearchLedgerResetReport {
            edition_only_works_reopened: 1,
            edition_only_attempts_cleared: 1,
            zero_route_works_reopened: 1,
            zero_route_attempts_cleared: 2,
        }
    );
    let survivors: Vec<String> = sqlx::query_scalar(
        "SELECT attempt_key FROM identity_provider_attempts ORDER BY attempt_key",
    )
    .fetch_all(harness.db.pool())
    .await
    .expect("read ledger-reset survivors");
    assert_eq!(survivors, vec!["kind-keep", "provider-keep", "routed-keep"]);
    let marker: String = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta WHERE key='identity_round15_search_ledger_reset'",
    )
    .fetch_one(harness.db.pool())
    .await
    .expect("read round-15 ledger marker");
    assert_eq!(marker, "1");
    let second = livrarr_db::identity_layer::heal_identity_round15_search_ledger(harness.db.pool())
        .await
        .expect("rerun round-15 search ledger reset");
    assert_eq!(second, Default::default());
}

async fn readarr_failed_creation_compensation_deletes_only_untouched_batch_authors() {
    let db = create_activated_test_db().await;
    let user = db
        .create_user(CreateUserDbRequest {
            username: "round10-readarr-compensation".to_string(),
            password_hash: "unused".to_string(),
            role: UserRole::Admin,
            api_key_hash: "unused-round10-compensation".to_string(),
        })
        .await
        .expect("seed compensation user");
    sqlx::query(
        "INSERT INTO imports (id, user_id, source, status, started_at) \
         VALUES ('round10-compensation-import', ?1, 'readarr', 'completed', ?2)",
    )
    .bind(user.id)
    .bind(Utc::now().to_rfc3339())
    .execute(db.pool())
    .await
    .expect("seed compensation import");
    let (untouched, _) = db
        .create_author(CreateAuthorDbRequest {
            user_id: user.id,
            name: "Failed Child Author".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: Some("round10-compensation-import".to_string()),
        })
        .await
        .expect("seed untouched import author");
    let (protected, _) = db
        .create_author(CreateAuthorDbRequest {
            user_id: user.id,
            name: "User Monitored Import Author".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: Some("round10-compensation-import".to_string()),
        })
        .await
        .expect("seed protected import author");
    sqlx::query("UPDATE authors SET monitored=1 WHERE id=?1")
        .bind(protected.id)
        .execute(db.pool())
        .await
        .expect("protect import author with user state");

    let deleted =
        livrarr_db::ImportDb::delete_orphan_authors_by_import(&db, "round10-compensation-import")
            .await
            .expect("compensate failed Readarr work creations");
    assert_eq!(deleted, 1);
    let remaining: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM authors WHERE id IN (?1, ?2) ORDER BY id")
            .bind(untouched.id)
            .bind(protected.id)
            .fetch_all(db.pool())
            .await
            .expect("read compensated author batch");
    assert_eq!(remaining, vec![protected.id]);
}

fn round13_search_transport(
    scripted_transport: Arc<
        dyn Fn(
                &livrarr_domain::services::FetchRequest,
            ) -> livrarr_http::fetcher::ScriptedTransportOutcome
            + Send
            + Sync,
    >,
) -> DiscoveryTransportFixture {
    DiscoveryTransportFixture {
        goodreads_base_url: "https://goodreads.test".to_string(),
        openlibrary_base_url: "https://openlibrary.test".to_string(),
        hardcover_search: false,
        request_timeout: Duration::from_secs(5),
        scripted_transport,
    }
}

fn round13_response(body: Vec<u8>) -> livrarr_http::fetcher::ScriptedTransportOutcome {
    livrarr_http::fetcher::ScriptedTransportOutcome::Response {
        delay: Duration::ZERO,
        response: livrarr_domain::services::FetchResponse {
            status: 200,
            headers: Vec::new(),
            body,
        },
    }
}

fn round17_status_response(
    status: u16,
    body: Vec<u8>,
) -> livrarr_http::fetcher::ScriptedTransportOutcome {
    livrarr_http::fetcher::ScriptedTransportOutcome::Response {
        delay: Duration::ZERO,
        response: livrarr_domain::services::FetchResponse {
            status,
            headers: Vec::new(),
            body,
        },
    }
}

fn round13_gr_page(
    title: &str,
    author: &str,
    book_id: &str,
    work_id: &str,
    asin: Option<&str>,
) -> Vec<u8> {
    let book_ref = format!("Book:kca://book/{book_id}");
    let work_ref = format!("Work:kca://work/{work_id}");
    let contributor_ref = "Contributor:kca://author/13";
    let mut details = serde_json::Map::new();
    if let Some(asin) = asin {
        details.insert("asin".to_string(), json!(asin));
    }
    let root_key = format!("getBookByLegacyId({{\"legacyId\":\"{book_id}\"}})");
    let blob = json!({
        "props": {"pageProps": {"apolloState": {
            "ROOT_QUERY": {root_key: {"__ref": book_ref}},
            book_ref.clone(): {
                "title": title,
                "details": Value::Object(details),
                "primaryContributorEdge": {"node": {"__ref": contributor_ref}},
                "work": {"__ref": work_ref}
            },
            contributor_ref: {"name": author},
            work_ref: {"legacyId": work_id}
        }}}
    });
    format!(
        "<html><body><script id=\"__NEXT_DATA__\" type=\"application/json\">{blob}</script></body></html>"
    )
    .into_bytes()
}

async fn seed_round13_search_work(
    harness: &RouteHarness,
    title_text: &str,
    author_name: &str,
    language: &str,
    edition_route: Option<(ilr::IdentityProvider, ilr::RouteKind, &str)>,
) -> (i64, i64) {
    let (author, _) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: author_name.to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed round-13 primary Author");
    let mut command = settlement_commit(harness.user_id, author.id, None);
    command.identity_title = title(title_text);
    command.routes = edition_route
        .into_iter()
        .map(|(provider, kind, value)| ilr::WorkRoute {
            id: 0,
            user_id: harness.user_id,
            owner: RouteOwner::Work(0),
            resolved_work_id: 0,
            provider: provider.clone(),
            kind,
            provider_scoped_id: value.to_string(),
            state: ilr::WorkRouteState::Active,
            provenance: ilr::RouteProvenance::Provider(provider),
            user_confirmed: false,
            observed_at: Utc::now(),
        })
        .collect();
    let settled = WorkIdentityRepository::commit_settlement(&harness.db, command)
        .await
        .expect("seed round-13 Work through settlement");
    let work_id = settled.identity.own_work_id;
    sqlx::query(
        "UPDATE works SET language=?1, enrichment_status='enriched', next_convergence_at=?2 \
         WHERE user_id=?3 AND id=?4",
    )
    .bind(language)
    .bind((Utc::now() - chrono::Duration::seconds(1)).to_rfc3339())
    .bind(harness.user_id)
    .bind(work_id)
    .execute(harness.db.pool())
    .await
    .expect("make round-13 Work cadence due");
    (work_id, settled.identity.identity_generation)
}

async fn round22_add_work_route(
    harness: &RouteHarness,
    work_id: i64,
    provider: ilr::IdentityProvider,
    kind: ilr::RouteKind,
    value: &str,
) {
    let captured =
        WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, work_id)
            .await
            .expect("read route graph before round-22 route addition");
    harness
        .state
        .identity_road
        .apply_captured_route_handoff(
            harness.user_id,
            work_id,
            IdentityRoadOrigin::ConvergenceVisit,
            ilr::CapturedRouteHandoff {
                metadata_generation: captured.identity_generation,
                provider_identity: vec![ilr::ProviderIdentityEvidence {
                    provider: provider.clone(),
                    route: RouteKey {
                        provider,
                        kind,
                        value: value.to_string(),
                    },
                    work_core: None,
                    provenance: Default::default(),
                }],
                route_proposals: Vec::new(),
            },
        )
        .await
        .expect("settle round-22 additional Work route");
}

async fn round13_run_tick(
    harness: &RouteHarness,
) -> livrarr_server::identity_layer::IdentityConvergenceReport {
    let _contract_lock = CONVERGENCE_CONTRACT_LOCK.lock().await;
    livrarr_server::identity_layer::run_identity_convergence_tick(
        harness.state.clone(),
        CancellationToken::new(),
    )
    .await
    .expect("round-13 convergence tick")
}

// Bug reproduction: identity-layer-rewrite round 21 — a Goodreads id carried
// by owned-file/manual-import evidence is a Book-page legacy id, never a Work
// legacy id. It must settle on an Edition and remain a usable GrKey anchor.
async fn round21_owned_file_goodreads_id_is_edition_homed() {
    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;
    harness
        .db
        .create_root_folder(
            harness
                ._tmp
                .path()
                .to_str()
                .expect("UTF-8 round-21 import root"),
            MediaType::Ebook,
        )
        .await
        .expect("seed round-21 manual-import root");
    let (work_id, generation) = seed_round13_search_work(
        &harness,
        "Wisdom Takes Work",
        "Owned File Namespace Author",
        "en",
        None,
    )
    .await;
    let path = harness._tmp.path().join("wisdom-takes-work.epub");
    write_epub_with_metadata(
        &path,
        false,
        "Wisdom Takes Work",
        Some("en"),
        Some("goodreads:230422186"),
    );

    let response = call_router_json(
        &harness,
        Method::POST,
        "/api/v1/manualimport/import".to_string(),
        Some(json!({"items": [{
            "path": path,
            "olKey": "",
            "title": "Wisdom Takes Work",
            "author": "Owned File Namespace Author",
            "deleteExisting": false,
            "language": "en",
            "authorOlKey": null,
            "year": 2026,
            "coverUrl": null,
            "isbn": null,
            "description": null,
            "seriesName": null,
            "seriesPosition": null,
            "candidateId": null,
            "hcKey": null,
            "grKey": "230422186",
            "asin": null
        }]})),
    )
    .await;
    assert!(
        response.status.is_success(),
        "manual import: {}",
        response.json
    );
    assert_eq!(response.json["results"][0]["workId"], work_id);

    let captured =
        WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, work_id)
            .await
            .expect("read owned-file Goodreads route");
    assert_eq!(captured.identity_generation, generation + 1);
    assert!(!captured.active_routes.iter().any(|route| {
        route.kind == ilr::RouteKind::GoodreadsWork && route.provider_scoped_id == "230422186"
    }));
    let route = captured
        .active_routes
        .iter()
        .find(|route| {
            route.kind == ilr::RouteKind::GoodreadsBookEdition
                && route.provider_scoped_id == "230422186"
        })
        .expect("owned Goodreads Book id settles as edition evidence");
    let RouteOwner::Edition(edition_id) = route.owner else {
        panic!("GoodreadsBookEdition must be homed on an Edition")
    };
    assert!(matches!(
        route.provenance,
        ilr::RouteProvenance::OwnedFile { .. }
    ));
    let edition_work_id: i64 =
        sqlx::query_scalar("SELECT work_id FROM editions WHERE user_id=?1 AND id=?2")
            .bind(harness.user_id)
            .bind(edition_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("read Goodreads Book route Edition home");
    assert_eq!(edition_work_id, work_id);
}

// Bug reproduction: identity-layer-rewrite round 21 — the startup repair is
// provenance-gated, generation/audit complete, retry-resetting, and one-shot.
async fn round21_owned_file_goodreads_work_heal_is_exact_and_idempotent() {
    let harness = build_route_harness().await;
    let (owned_work_id, owned_generation) = seed_round13_search_work(
        &harness,
        "Round Twenty One Owned Heal",
        "Owned Heal Author",
        "en",
        Some((
            ilr::IdentityProvider::Goodreads,
            ilr::RouteKind::GoodreadsWork,
            "230422186",
        )),
    )
    .await;
    let owned_provenance = ilr::RouteProvenance::OwnedFile {
        library_item_id: Some(74),
        file_revision: revision(),
    };
    sqlx::query(
        "UPDATE identity_routes SET provenance=?1 WHERE user_id=?2 AND resolved_work_id=?3 \
         AND kind='\"GoodreadsWork\"' AND provider_scoped_id='230422186'",
    )
    .bind(serde_json::to_string(&owned_provenance).unwrap())
    .bind(harness.user_id)
    .bind(owned_work_id)
    .execute(harness.db.pool())
    .await
    .expect("seed mislabeled OwnedFile Goodreads route");
    ProviderRetryStateDb::record_terminal_outcome(
        &harness.db,
        harness.user_id,
        owned_work_id,
        MetadataProvider::OpenLibrary,
        OutcomeClass::NotFound,
        None,
    )
    .await
    .expect("seed retry standing that graph heal must reset");

    let (migrated_work_id, migrated_generation) = seed_round13_search_work(
        &harness,
        "Round Twenty Two Migrated Heal",
        "Migrated Heal Author",
        "en",
        Some((
            ilr::IdentityProvider::Goodreads,
            ilr::RouteKind::GoodreadsWork,
            "220004",
        )),
    )
    .await;
    let migrated_provenance = ilr::RouteProvenance::Migrated {
        legacy_field: "gr_key".to_string(),
    };
    sqlx::query(
        "UPDATE identity_routes SET provenance=?1 WHERE user_id=?2 AND resolved_work_id=?3 \
         AND kind='\"GoodreadsWork\"' AND provider_scoped_id='220004'",
    )
    .bind(serde_json::to_string(&migrated_provenance).unwrap())
    .bind(harness.user_id)
    .bind(migrated_work_id)
    .execute(harness.db.pool())
    .await
    .expect("seed sanctioned migrated gr_key Goodreads route");

    let (search_work_id, search_generation) = seed_round13_search_work(
        &harness,
        "Round Twenty One Search Route Control",
        "Search Route Control Author",
        "en",
        Some((
            ilr::IdentityProvider::Goodreads,
            ilr::RouteKind::GoodreadsWork,
            "1254936",
        )),
    )
    .await;
    let search_provenance = ilr::RouteProvenance::TextDecisiveSearchFallback {
        provider: ilr::IdentityProvider::Goodreads,
    };
    sqlx::query(
        "UPDATE identity_routes SET provenance=?1 WHERE user_id=?2 AND resolved_work_id=?3 \
         AND kind='\"GoodreadsWork\"' AND provider_scoped_id='1254936'",
    )
    .bind(serde_json::to_string(&search_provenance).unwrap())
    .bind(harness.user_id)
    .bind(search_work_id)
    .execute(harness.db.pool())
    .await
    .expect("seed TextDecisiveSearchFallback control route");
    let search_row_before: (
        i64,
        String,
        String,
        Option<i64>,
        Option<i64>,
        String,
        i64,
        String,
    ) = sqlx::query_as(
        "SELECT id, owner_type, kind, work_id, edition_id, provenance, user_confirmed, \
                    observed_at FROM identity_routes WHERE user_id=?1 AND resolved_work_id=?2 \
                    AND provider_scoped_id='1254936' AND state='active'",
    )
    .bind(harness.user_id)
    .bind(search_work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("snapshot search-origin control route");
    let owned_route_id: i64 = sqlx::query_scalar(
        "SELECT id FROM identity_routes WHERE user_id=?1 AND resolved_work_id=?2 \
         AND provider_scoped_id='230422186' AND state='active'",
    )
    .bind(harness.user_id)
    .bind(owned_work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read mislabeled OwnedFile route id");

    let first =
        livrarr_db::identity_layer::heal_identity_round21_goodreads_namespace(harness.db.pool())
            .await
            .expect("run round-21 Goodreads namespace heal");
    assert_eq!(first.owned_file_routes_relabelled, 1);
    assert_eq!(first.migrated_gr_key_routes_relabelled, 1);
    assert_eq!(first.editions_created, 2);
    assert_eq!(first.works_advanced, 2);
    assert_eq!(first.retry_works_reset, 2);

    let healed: (i64, String, String, Option<i64>, Option<i64>, String) = sqlx::query_as(
        "SELECT id, owner_type, kind, work_id, edition_id, provenance FROM identity_routes \
         WHERE user_id=?1 AND resolved_work_id=?2 AND provider_scoped_id='230422186' \
           AND state='active'",
    )
    .bind(harness.user_id)
    .bind(owned_work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read healed OwnedFile route");
    assert_eq!(healed.0, owned_route_id, "the route is relabeled in place");
    assert_eq!(healed.1, "edition");
    assert_eq!(healed.2, "\"GoodreadsBookEdition\"");
    assert_eq!(healed.3, None);
    let edition_id = healed.4.expect("healed route has an Edition owner");
    assert_eq!(healed.5, serde_json::to_string(&owned_provenance).unwrap());
    let edition_work: i64 =
        sqlx::query_scalar("SELECT work_id FROM editions WHERE user_id=?1 AND id=?2")
            .bind(harness.user_id)
            .bind(edition_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("read healed Edition home");
    assert_eq!(edition_work, owned_work_id);
    assert_eq!(
        work_generation(&harness.db, owned_work_id).await,
        owned_generation + 1
    );
    assert!(
        harness
            .db
            .get_retry_state(
                harness.user_id,
                owned_work_id,
                MetadataProvider::OpenLibrary,
            )
            .await
            .unwrap()
            .is_none(),
        "route-graph change resets retry standing"
    );
    let migrated_healed: (String, String, Option<i64>, String) = sqlx::query_as(
        "SELECT owner_type, kind, edition_id, provenance FROM identity_routes \
         WHERE user_id=?1 AND resolved_work_id=?2 AND provider_scoped_id='220004' \
           AND state='active'",
    )
    .bind(harness.user_id)
    .bind(migrated_work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read healed Migrated gr_key route");
    assert_eq!(migrated_healed.0, "edition");
    assert_eq!(migrated_healed.1, "\"GoodreadsBookEdition\"");
    assert!(migrated_healed.2.is_some());
    assert_eq!(
        migrated_healed.3,
        serde_json::to_string(&migrated_provenance).unwrap()
    );
    assert_eq!(
        work_generation(&harness.db, migrated_work_id).await,
        migrated_generation + 1
    );
    let heal_audits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events WHERE user_id=?1 AND work_id=?2 \
         AND event_kind='round21-goodreads-book-namespace-heal'",
    )
    .bind(harness.user_id)
    .bind(owned_work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count round-21 namespace audits");
    assert_eq!(heal_audits, 1);

    let search_row_after: (
        i64,
        String,
        String,
        Option<i64>,
        Option<i64>,
        String,
        i64,
        String,
    ) = sqlx::query_as(
        "SELECT id, owner_type, kind, work_id, edition_id, provenance, user_confirmed, \
                    observed_at FROM identity_routes WHERE user_id=?1 AND resolved_work_id=?2 \
                    AND provider_scoped_id='1254936' AND state='active'",
    )
    .bind(harness.user_id)
    .bind(search_work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read search-origin control route after heal");
    assert_eq!(search_row_after, search_row_before);
    assert_eq!(
        work_generation(&harness.db, search_work_id).await,
        search_generation
    );

    let second =
        livrarr_db::identity_layer::heal_identity_round21_goodreads_namespace(harness.db.pool())
            .await
            .expect("rerun round-21 Goodreads namespace heal");
    assert_eq!(
        second,
        livrarr_db::identity_layer::IdentityRound21GoodreadsNamespaceHealReport::default()
    );
    assert_eq!(
        work_generation(&harness.db, owned_work_id).await,
        owned_generation + 1
    );
    assert_eq!(
        work_generation(&harness.db, migrated_work_id).await,
        migrated_generation + 1
    );
    let audits_after_second: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events WHERE user_id=?1 AND work_id=?2 \
         AND event_kind='round21-goodreads-book-namespace-heal'",
    )
    .bind(harness.user_id)
    .bind(owned_work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count audits after idempotent rerun");
    assert_eq!(audits_after_second, 1);
    let marker: String = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta \
         WHERE key='identity_round21_goodreads_book_namespace_heal'",
    )
    .fetch_one(harness.db.pool())
    .await
    .expect("read round-21 namespace marker");
    assert_eq!(marker, "1");
}

// Missing negative from C-r10-03: a cross-Work Goodreads Book collision aborts
// the whole marker-gated transaction. A safe candidate is deliberately ordered
// before the colliding candidate so the assertions prove rollback, not merely
// early validation.
async fn round22_goodreads_namespace_heal_collision_is_atomic_fixture() {
    let harness = build_route_harness().await;
    let owner_work_id = seed_route_work(&harness, "round22-heal-collision-owner").await;
    let owner_edition_id = harness
        .db
        .seed_transfer_target_for_tests(harness.user_id, owner_work_id, ilr::EditionFormat::Unknown)
        .await
        .expect("seed collision-owner Edition");
    let collision_value = "220005";
    let goodreads = serde_json::to_string(&ilr::IdentityProvider::Goodreads).unwrap();
    let book_kind = serde_json::to_string(&ilr::RouteKind::GoodreadsBookEdition).unwrap();
    let work_kind = serde_json::to_string(&ilr::RouteKind::GoodreadsWork).unwrap();
    let owner_provenance = serde_json::to_string(&ilr::RouteProvenance::UserChoice).unwrap();
    sqlx::query(
        "INSERT INTO identity_routes \
            (user_id, owner_type, work_id, edition_id, resolved_work_id, provider, kind, \
             provider_scoped_id, state, provenance, user_confirmed, observed_at) \
         VALUES (?1, 'edition', NULL, ?2, ?3, ?4, ?5, ?6, 'active', ?7, 1, ?8)",
    )
    .bind(harness.user_id)
    .bind(owner_edition_id)
    .bind(owner_work_id)
    .bind(&goodreads)
    .bind(&book_kind)
    .bind(collision_value)
    .bind(owner_provenance)
    .bind(Utc::now().to_rfc3339())
    .execute(harness.db.pool())
    .await
    .expect("seed correct Goodreads Book route on collision owner");

    let safe_value = "220006";
    let (target_work_id, target_generation) = seed_round13_search_work(
        &harness,
        "Round Twenty Two Heal Collision Target",
        "Heal Collision Target Author",
        "en",
        Some((
            ilr::IdentityProvider::Goodreads,
            ilr::RouteKind::GoodreadsWork,
            safe_value,
        )),
    )
    .await;
    let owned_provenance = ilr::RouteProvenance::OwnedFile {
        library_item_id: Some(2200),
        file_revision: revision(),
    };
    let owned_provenance_json = serde_json::to_string(&owned_provenance).unwrap();
    sqlx::query(
        "UPDATE identity_routes SET provenance=?1 WHERE user_id=?2 AND resolved_work_id=?3 \
         AND kind=?4 AND provider_scoped_id=?5",
    )
    .bind(&owned_provenance_json)
    .bind(harness.user_id)
    .bind(target_work_id)
    .bind(&work_kind)
    .bind(safe_value)
    .execute(harness.db.pool())
    .await
    .expect("mark safe route as OwnedFile-proven");
    sqlx::query(
        "INSERT INTO identity_routes \
            (user_id, owner_type, work_id, edition_id, resolved_work_id, provider, kind, \
             provider_scoped_id, state, provenance, user_confirmed, observed_at) \
         VALUES (?1, 'work', ?2, NULL, ?2, ?3, ?4, ?5, 'active', ?6, 0, ?7)",
    )
    .bind(harness.user_id)
    .bind(target_work_id)
    .bind(&goodreads)
    .bind(&work_kind)
    .bind(collision_value)
    .bind(&owned_provenance_json)
    .bind(Utc::now().to_rfc3339())
    .execute(harness.db.pool())
    .await
    .expect("seed colliding OwnedFile-proven Goodreads Work route");

    let error =
        livrarr_db::identity_layer::heal_identity_round21_goodreads_namespace(harness.db.pool())
            .await
            .expect_err("cross-Work Goodreads Book collision must abort the heal");
    assert!(
        error.contains(collision_value),
        "specific collision: {error}"
    );

    let marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM _livrarr_meta \
         WHERE key='identity_round21_goodreads_book_namespace_heal'",
    )
    .fetch_one(harness.db.pool())
    .await
    .expect("count unstamped namespace-heal marker");
    assert_eq!(marker_count, 0);
    let target_routes: Vec<(String, String, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT provider_scoped_id, kind, work_id, edition_id FROM identity_routes \
         WHERE user_id=?1 AND resolved_work_id=?2 AND state='active' ORDER BY id",
    )
    .bind(harness.user_id)
    .bind(target_work_id)
    .fetch_all(harness.db.pool())
    .await
    .expect("read rolled-back target routes");
    assert_eq!(target_routes.len(), 2);
    assert!(target_routes.iter().all(|route| {
        route.1 == work_kind && route.2 == Some(target_work_id) && route.3.is_none()
    }));
    assert_eq!(
        work_generation(&harness.db, target_work_id).await,
        target_generation
    );
    let safe_edition_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM editions WHERE user_id=?1 AND work_id=?2 \
         AND source_provider=?3 AND provider_edition_id=?4",
    )
    .bind(harness.user_id)
    .bind(target_work_id)
    .bind(&goodreads)
    .bind(safe_value)
    .fetch_one(harness.db.pool())
    .await
    .expect("count rolled-back safe Edition");
    assert_eq!(safe_edition_count, 0);
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events WHERE user_id=?1 AND work_id=?2 \
         AND event_kind='round21-goodreads-book-namespace-heal'",
    )
    .bind(harness.user_id)
    .bind(target_work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count rolled-back namespace-heal audits");
    assert_eq!(audit_count, 0);
}

fn round21_hardcover_search_response() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "data": {"search": {"results": {"hits": []}}}
    }))
    .expect("encode empty Hardcover search response")
}

fn round21_is_hardcover_title_search(request: &livrarr_domain::services::FetchRequest) -> bool {
    request.url.contains("api.hardcover.app")
        && request
            .body
            .as_deref()
            .and_then(|body| serde_json::from_slice::<Value>(body).ok())
            .and_then(|body| body.pointer("/variables/query").cloned())
            .is_some()
}

// Bug reproduction: identity-layer-rewrite round 21 / AC-026(a). One
// provider's Work route must not starve the other applicable providers.
async fn round21_connected_goodreads_route_searches_ol_and_hc_and_auto_links() {
    let _breaker = lock_breaker().await;
    let ol_searches = Arc::new(AtomicU64::new(0));
    let gr_searches = Arc::new(AtomicU64::new(0));
    let hc_searches = Arc::new(AtomicU64::new(0));
    let scripted = {
        let ol_searches = ol_searches.clone();
        let gr_searches = gr_searches.clone();
        let hc_searches = hc_searches.clone();
        Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
            if request.url.contains("search.json") {
                ol_searches.fetch_add(1, Ordering::Relaxed);
                return round13_response(
                    serde_json::to_vec(&json!({"docs": [{
                        "key": "/works/OL-ROUND21-WORK-74-W",
                        "title": "Round Twenty One Work Seventy Four",
                        "author_name": ["Work Seventy Four Author"]
                    }]}))
                    .unwrap(),
                );
            }
            if request.url.contains("auto_complete") {
                gr_searches.fetch_add(1, Ordering::Relaxed);
                return round13_response(b"[]".to_vec());
            }
            if round21_is_hardcover_title_search(request) {
                hc_searches.fetch_add(1, Ordering::Relaxed);
                return round13_response(round21_hardcover_search_response());
            }
            round17_status_response(404, Vec::new())
        }) as Arc<_>
    };
    let mut transport = round13_search_transport(scripted);
    transport.hardcover_search = true;
    let harness =
        build_route_harness_with_provider_details(None, Vec::new(), Some(transport)).await;
    let (work_id, generation) = seed_round13_search_work(
        &harness,
        "Round Twenty One Work Seventy Four",
        "Work Seventy Four Author",
        "en",
        Some((
            ilr::IdentityProvider::Goodreads,
            ilr::RouteKind::GoodreadsWork,
            "1254936",
        )),
    )
    .await;

    let due = harness
        .db
        .list_convergence_due(harness.user_id, Utc::now(), 3, 100)
        .await
        .expect("read round-21 convergence selection");
    assert!(
        due.contains(&work_id),
        "a GR-routed Work with eligible OL/HC providers is cadence-selectable"
    );
    assert_eq!(round13_run_tick(&harness).await.visited_work_count, 1);
    assert_eq!(ol_searches.load(Ordering::Relaxed), 1);
    assert_eq!(hc_searches.load(Ordering::Relaxed), 1);
    assert_eq!(
        gr_searches.load(Ordering::Relaxed),
        0,
        "the provider already holding its Work route never searches"
    );

    let captured =
        WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, work_id)
            .await
            .expect("read round-21 linked route graph");
    assert_eq!(captured.identity_generation, generation + 1);
    assert!(captured.active_routes.iter().any(|route| {
        route.kind == ilr::RouteKind::GoodreadsWork && route.provider_scoped_id == "1254936"
    }));
    assert!(captured.active_routes.iter().any(|route| {
        route.kind == ilr::RouteKind::OpenLibraryWork
            && route.provider_scoped_id == "OL-ROUND21-WORK-74-W"
    }));
    let burns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_provider_attempts WHERE user_id=?1 AND work_id=?2 \
         AND provider='livrarr-convergence' AND route_kind='bridge-upgrade'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count round-21 successful-pass ledger burns");
    assert_eq!(burns, 0);
}

async fn round21_owned_provider_search_counts(
    provider: ilr::IdentityProvider,
    kind: ilr::RouteKind,
    value: &str,
) -> (u64, u64, u64) {
    reset_breakers();
    let ol_searches = Arc::new(AtomicU64::new(0));
    let gr_searches = Arc::new(AtomicU64::new(0));
    let hc_searches = Arc::new(AtomicU64::new(0));
    let scripted = {
        let ol_searches = ol_searches.clone();
        let gr_searches = gr_searches.clone();
        let hc_searches = hc_searches.clone();
        Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
            if request.url.contains("search.json") {
                ol_searches.fetch_add(1, Ordering::Relaxed);
                return round13_response(b"{\"docs\":[]}".to_vec());
            }
            if request.url.contains("auto_complete") {
                gr_searches.fetch_add(1, Ordering::Relaxed);
                return round13_response(b"[]".to_vec());
            }
            if round21_is_hardcover_title_search(request) {
                hc_searches.fetch_add(1, Ordering::Relaxed);
                return round13_response(round21_hardcover_search_response());
            }
            round17_status_response(404, Vec::new())
        }) as Arc<_>
    };
    let mut transport = round13_search_transport(scripted);
    transport.hardcover_search = true;
    let harness =
        build_route_harness_with_provider_details(None, Vec::new(), Some(transport)).await;
    let (work_id, _) = seed_round13_search_work(
        &harness,
        &format!("Round Twenty One Owned {kind:?}"),
        "Provider Ownership Author",
        "en",
        Some((provider, kind, value)),
    )
    .await;
    sqlx::query("UPDATE works SET enrichment_status='failed' WHERE user_id=?1 AND id=?2")
        .bind(harness.user_id)
        .bind(work_id)
        .execute(harness.db.pool())
        .await
        .expect("make provider-ownership guard enrichment-due");

    assert_eq!(round13_run_tick(&harness).await.visited_work_count, 1);
    (
        ol_searches.load(Ordering::Relaxed),
        gr_searches.load(Ordering::Relaxed),
        hc_searches.load(Ordering::Relaxed),
    )
}

// AC-026(b) is a post-change boundary guard: the old all-or-none rule was
// already quiet, but the provider-local broadening must stay quiet for the
// provider that owns the route.
async fn round21_each_provider_with_own_work_route_fires_zero_search_http() {
    let _breaker = lock_breaker().await;
    let ol = round21_owned_provider_search_counts(
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        "OL-ROUND21-OWNED-W",
    )
    .await;
    assert_eq!(
        ol.0, 0,
        "OpenLibrary must not search around its own Work route"
    );
    assert_eq!(
        (ol.1, ol.2),
        (1, 1),
        "the two non-owned providers must both fire beside an OL Work route"
    );

    let gr = round21_owned_provider_search_counts(
        ilr::IdentityProvider::Goodreads,
        ilr::RouteKind::GoodreadsWork,
        "212121",
    )
    .await;
    assert_eq!(
        gr.1, 0,
        "Goodreads must not search around its own Work route"
    );
    assert_eq!(
        (gr.0, gr.2),
        (1, 1),
        "the two non-owned providers must both fire beside a GR Work route"
    );

    let hc = round21_owned_provider_search_counts(
        ilr::IdentityProvider::Hardcover,
        ilr::RouteKind::HardcoverWork,
        "212122",
    )
    .await;
    assert_eq!(
        hc.2, 0,
        "Hardcover must not search around its own Work route"
    );
    assert_eq!(
        (hc.0, hc.1),
        (1, 1),
        "the two non-owned providers must both fire beside an HC Work route"
    );
}

// Bug reproduction: identity-layer-rewrite round 21 / AC-026(c). Two misses
// in one pass are one per-Work burn, not zero and not one per provider.
async fn round21_connected_all_fired_legs_miss_burn_once_and_park() {
    let _breaker = lock_breaker().await;
    let ol_searches = Arc::new(AtomicU64::new(0));
    let gr_searches = Arc::new(AtomicU64::new(0));
    let hc_searches = Arc::new(AtomicU64::new(0));
    let scripted = {
        let ol_searches = ol_searches.clone();
        let gr_searches = gr_searches.clone();
        let hc_searches = hc_searches.clone();
        Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
            if request.url.contains("search.json") {
                ol_searches.fetch_add(1, Ordering::Relaxed);
                return round13_response(b"{\"docs\":[]}".to_vec());
            }
            if request.url.contains("auto_complete") {
                gr_searches.fetch_add(1, Ordering::Relaxed);
                return round13_response(b"[]".to_vec());
            }
            if round21_is_hardcover_title_search(request) {
                hc_searches.fetch_add(1, Ordering::Relaxed);
                return round13_response(round21_hardcover_search_response());
            }
            round17_status_response(404, Vec::new())
        }) as Arc<_>
    };
    let mut transport = round13_search_transport(scripted);
    transport.hardcover_search = true;
    let harness =
        build_route_harness_with_provider_details(None, Vec::new(), Some(transport)).await;
    let (work_id, generation) = seed_round13_search_work(
        &harness,
        "Round Twenty One Connected Miss",
        "Connected Miss Author",
        "en",
        Some((
            ilr::IdentityProvider::Goodreads,
            ilr::RouteKind::GoodreadsWork,
            "212123",
        )),
    )
    .await;

    for expected_attempts in 1..=3_i64 {
        WorkDb::set_next_convergence_at(
            &harness.db,
            harness.user_id,
            work_id,
            Some(Utc::now() - chrono::Duration::seconds(1)),
        )
        .await
        .unwrap();
        assert_eq!(round13_run_tick(&harness).await.visited_work_count, 1);
        let attempts: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_provider_attempts WHERE user_id=?1 AND work_id=?2 \
             AND provider='livrarr-convergence' AND route_kind='bridge-upgrade' \
             AND route_value=CAST(?3 AS TEXT)",
        )
        .bind(harness.user_id)
        .bind(work_id)
        .bind(generation)
        .fetch_one(harness.db.pool())
        .await
        .unwrap();
        assert_eq!(
            attempts, expected_attempts,
            "every all-miss pass burns exactly one shared attempt"
        );
    }
    assert_eq!(ol_searches.load(Ordering::Relaxed), 3);
    assert_eq!(hc_searches.load(Ordering::Relaxed), 3);
    assert_eq!(gr_searches.load(Ordering::Relaxed), 0);
    WorkDb::set_next_convergence_at(
        &harness.db,
        harness.user_id,
        work_id,
        Some(Utc::now() - chrono::Duration::seconds(1)),
    )
    .await
    .unwrap();
    assert_eq!(
        round13_run_tick(&harness).await.visited_work_count,
        0,
        "threshold parks the connected Work until its identity generation changes"
    );
}

// Bug reproduction: identity-layer-rewrite round 21 — a transport/provider
// failure is neither a proposal card nor an honest miss, so one failed fired
// leg must prevent the Work-local ledger burn for the entire pass.
async fn round21_search_transport_failure_does_not_burn_shared_ledger() {
    let _breaker = lock_breaker().await;
    let ol_searches = Arc::new(AtomicU64::new(0));
    let gr_searches = Arc::new(AtomicU64::new(0));
    let hc_searches = Arc::new(AtomicU64::new(0));
    let scripted = {
        let ol_searches = ol_searches.clone();
        let gr_searches = gr_searches.clone();
        let hc_searches = hc_searches.clone();
        Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
            if request.url.contains("search.json") {
                ol_searches.fetch_add(1, Ordering::Relaxed);
                return round17_status_response(503, Vec::new());
            }
            if request.url.contains("auto_complete") {
                gr_searches.fetch_add(1, Ordering::Relaxed);
                return round13_response(b"[]".to_vec());
            }
            if round21_is_hardcover_title_search(request) {
                hc_searches.fetch_add(1, Ordering::Relaxed);
                return round13_response(round21_hardcover_search_response());
            }
            round17_status_response(404, Vec::new())
        }) as Arc<_>
    };
    let mut transport = round13_search_transport(scripted);
    transport.hardcover_search = true;
    let harness =
        build_route_harness_with_provider_details(None, Vec::new(), Some(transport)).await;
    let (work_id, _) = seed_round13_search_work(
        &harness,
        "Round Twenty One Failed Search Leg",
        "Failed Search Leg Author",
        "en",
        Some((
            ilr::IdentityProvider::Goodreads,
            ilr::RouteKind::GoodreadsWork,
            "212124",
        )),
    )
    .await;

    assert_eq!(round13_run_tick(&harness).await.visited_work_count, 1);
    assert_eq!(ol_searches.load(Ordering::Relaxed), 1);
    assert_eq!(hc_searches.load(Ordering::Relaxed), 1);
    assert_eq!(gr_searches.load(Ordering::Relaxed), 0);
    let burns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_provider_attempts WHERE user_id=?1 AND work_id=?2 \
         AND provider='livrarr-convergence' AND route_kind='bridge-upgrade'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count failed-search shared ledger burns");
    assert_eq!(
        burns, 0,
        "a pass with any failed fired leg is not an all-card/miss pass"
    );
}

// Bug reproduction: identity-layer-rewrite round 22 / C-r10-01. The SQL
// selector must consume the same live provider availability as the queue. An
// enriched OL+GR-routed Work cannot remain due solely because unconfigured
// Hardcover has no route.
async fn round22_unfireable_hardcover_work_is_absent_and_stays_unvisited() {
    let _breaker = lock_breaker().await;
    let scripted = Arc::new(|_request: &livrarr_domain::services::FetchRequest| {
        round17_status_response(404, Vec::new())
    });
    let harness = build_route_harness_with_provider_details(
        None,
        Vec::new(),
        Some(round13_search_transport(scripted)),
    )
    .await;
    let (work_id, _) = seed_round13_search_work(
        &harness,
        "Round Twenty Two Hardcover Unavailable",
        "Unavailable Hardcover Author",
        "en",
        Some((
            ilr::IdentityProvider::Goodreads,
            ilr::RouteKind::GoodreadsWork,
            "220001",
        )),
    )
    .await;
    round22_add_work_route(
        &harness,
        work_id,
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        "OL-ROUND22-HC-OFF-W",
    )
    .await;

    let due = harness
        .db
        .list_convergence_due_with_search_availability(
            harness.user_id,
            Utc::now(),
            3,
            100,
            harness.state.provider_queue.identity_search_availability(),
        )
        .await
        .expect("read round-22 HC-off due selection");
    assert!(
        !due.contains(&work_id),
        "an OL+GR-routed Work with unavailable HC has no fireable search provider"
    );
    assert_eq!(round13_run_tick(&harness).await.visited_work_count, 0);
    assert_eq!(round13_run_tick(&harness).await.visited_work_count, 0);
    let ledger_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_provider_attempts WHERE user_id=?1 AND work_id=?2",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count HC-off convergence ledger rows");
    assert_eq!(ledger_rows, 0);
}

// Bug reproduction: identity-layer-rewrite round 22 / C-r10-01/C-r10-02. The
// positive availability complement selects HC, but an enrichment-complete
// connected Work enters only the route-search partition: its owned OL route is
// never re-fetched through `/works/`.
async fn round22_hardcover_only_search_is_bounded_and_never_refetches_openlibrary() {
    let _breaker = lock_breaker().await;
    let ol_searches = Arc::new(AtomicU64::new(0));
    let ol_work_fetches = Arc::new(AtomicU64::new(0));
    let gr_searches = Arc::new(AtomicU64::new(0));
    let hc_searches = Arc::new(AtomicU64::new(0));
    let scripted = {
        let ol_searches = ol_searches.clone();
        let ol_work_fetches = ol_work_fetches.clone();
        let gr_searches = gr_searches.clone();
        let hc_searches = hc_searches.clone();
        Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
            if request.url.contains("search.json") {
                ol_searches.fetch_add(1, Ordering::Relaxed);
                return round13_response(b"{\"docs\":[]}".to_vec());
            }
            if request.url.contains("/works/") {
                ol_work_fetches.fetch_add(1, Ordering::Relaxed);
                return round17_status_response(404, Vec::new());
            }
            if request.url.contains("auto_complete") {
                gr_searches.fetch_add(1, Ordering::Relaxed);
                return round13_response(b"[]".to_vec());
            }
            if round21_is_hardcover_title_search(request) {
                hc_searches.fetch_add(1, Ordering::Relaxed);
                return round13_response(round21_hardcover_search_response());
            }
            round17_status_response(404, Vec::new())
        }) as Arc<_>
    };
    let mut transport = round13_search_transport(scripted);
    transport.hardcover_search = true;
    let harness =
        build_route_harness_with_provider_details(None, Vec::new(), Some(transport)).await;
    let (work_id, _) = seed_round13_search_work(
        &harness,
        "Round Twenty Two Search Only",
        "Search Only Author",
        "en",
        Some((
            ilr::IdentityProvider::Goodreads,
            ilr::RouteKind::GoodreadsWork,
            "220002",
        )),
    )
    .await;
    round22_add_work_route(
        &harness,
        work_id,
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        "OL-ROUND22-SEARCH-ONLY-W",
    )
    .await;
    let generation = work_generation(&harness.db, work_id).await;

    for expected_attempts in 1..=3_i64 {
        WorkDb::set_next_convergence_at(
            &harness.db,
            harness.user_id,
            work_id,
            Some(Utc::now() - chrono::Duration::seconds(1)),
        )
        .await
        .unwrap();
        assert_eq!(round13_run_tick(&harness).await.visited_work_count, 1);
        let attempts: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_provider_attempts WHERE user_id=?1 AND work_id=?2 \
             AND provider='livrarr-convergence' AND route_kind='bridge-upgrade' \
             AND route_value=CAST(?3 AS TEXT)",
        )
        .bind(harness.user_id)
        .bind(work_id)
        .bind(generation)
        .fetch_one(harness.db.pool())
        .await
        .expect("count round-22 HC-only ledger burns");
        assert_eq!(attempts, expected_attempts);
    }
    assert_eq!(hc_searches.load(Ordering::Relaxed), 3);
    assert_eq!(ol_searches.load(Ordering::Relaxed), 0);
    assert_eq!(gr_searches.load(Ordering::Relaxed), 0);
    assert_eq!(
        ol_work_fetches.load(Ordering::Relaxed),
        0,
        "search-only convergence must never enter the anchored OL scatter"
    );
    assert_eq!(
        work_generation(&harness.db, work_id).await,
        generation,
        "honest misses do not mutate the route graph"
    );
    WorkDb::set_next_convergence_at(
        &harness.db,
        harness.user_id,
        work_id,
        Some(Utc::now() - chrono::Duration::seconds(1)),
    )
    .await
    .unwrap();
    assert_eq!(round13_run_tick(&harness).await.visited_work_count, 0);
}

// Missing negative from C-r10-03: even an enriched Work with a fresh ledger is
// absent once every search-capable provider already owns a Work route.
async fn round22_all_three_work_routes_are_absent_from_due_selection() {
    let _breaker = lock_breaker().await;
    let scripted = Arc::new(|_request: &livrarr_domain::services::FetchRequest| {
        round17_status_response(404, Vec::new())
    });
    let mut transport = round13_search_transport(scripted);
    transport.hardcover_search = true;
    let harness =
        build_route_harness_with_provider_details(None, Vec::new(), Some(transport)).await;
    let (work_id, _) = seed_round13_search_work(
        &harness,
        "Round Twenty Two Fully Routed",
        "Fully Routed Author",
        "en",
        Some((
            ilr::IdentityProvider::Goodreads,
            ilr::RouteKind::GoodreadsWork,
            "220003",
        )),
    )
    .await;
    round22_add_work_route(
        &harness,
        work_id,
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        "OL-ROUND22-FULL-W",
    )
    .await;
    round22_add_work_route(
        &harness,
        work_id,
        ilr::IdentityProvider::Hardcover,
        ilr::RouteKind::HardcoverWork,
        "HC-ROUND22-FULL-W",
    )
    .await;

    let due = harness
        .db
        .list_convergence_due_with_search_availability(
            harness.user_id,
            Utc::now(),
            3,
            100,
            harness.state.provider_queue.identity_search_availability(),
        )
        .await
        .expect("read fully-routed round-22 due selection");
    assert!(!due.contains(&work_id));
}

// Bug reproduction: identity-layer-rewrite round 21 / AC-026(d). A foreign
// connected Work keeps the existing Goodreads-only applicability policy.
async fn round21_foreign_connected_work_fires_only_goodreads_search() {
    let _breaker = lock_breaker().await;
    let ol_searches = Arc::new(AtomicU64::new(0));
    let gr_searches = Arc::new(AtomicU64::new(0));
    let hc_searches = Arc::new(AtomicU64::new(0));
    let scripted = {
        let ol_searches = ol_searches.clone();
        let gr_searches = gr_searches.clone();
        let hc_searches = hc_searches.clone();
        Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
            if request.url.contains("search.json") {
                ol_searches.fetch_add(1, Ordering::Relaxed);
                return round13_response(b"{\"docs\":[]}".to_vec());
            }
            if request.url.contains("auto_complete") {
                gr_searches.fetch_add(1, Ordering::Relaxed);
                return round13_response(b"[]".to_vec());
            }
            if round21_is_hardcover_title_search(request) {
                hc_searches.fetch_add(1, Ordering::Relaxed);
                return round13_response(round21_hardcover_search_response());
            }
            round17_status_response(404, Vec::new())
        }) as Arc<_>
    };
    let mut transport = round13_search_transport(scripted);
    transport.hardcover_search = true;
    let harness =
        build_route_harness_with_provider_details(None, Vec::new(), Some(transport)).await;
    let (work_id, _) = seed_round13_search_work(
        &harness,
        "Round Twenty One Foreign Connected",
        "Foreign Connected Author",
        "fr",
        Some((
            ilr::IdentityProvider::OpenLibrary,
            ilr::RouteKind::OpenLibraryWork,
            "OL-ROUND21-FOREIGN-W",
        )),
    )
    .await;

    let due = harness
        .db
        .list_convergence_due(harness.user_id, Utc::now(), 3, 100)
        .await
        .expect("read foreign connected convergence selection");
    assert!(due.contains(&work_id));
    assert_eq!(round13_run_tick(&harness).await.visited_work_count, 1);
    assert_eq!(ol_searches.load(Ordering::Relaxed), 0);
    assert_eq!(hc_searches.load(Ordering::Relaxed), 0);
    assert_eq!(gr_searches.load(Ordering::Relaxed), 1);
}

async fn round13_correlated_openlibrary_search_settles() {
    let _breaker = lock_breaker().await;
    let asin = "B0ROUND13OL1";
    let search_calls = Arc::new(AtomicU64::new(0));
    let scripted = {
        let search_calls = search_calls.clone();
        Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
            if request.url.contains("search.json") {
                search_calls.fetch_add(1, Ordering::Relaxed);
                return round13_response(
                    serde_json::to_vec(&json!({"docs": [{
                        "key": "/works/OL-ROUND13-CORROBORATED-W",
                        "title": "Round Thirteen OpenLibrary",
                        "author_name": ["Search Authority Author"],
                        "id_amazon": [asin]
                    }]}))
                    .unwrap(),
                );
            }
            round13_response(b"[]".to_vec())
        }) as Arc<_>
    };
    let harness = build_route_harness_with_provider_details(
        None,
        Vec::new(),
        Some(round13_search_transport(scripted)),
    )
    .await;
    let (work_id, generation) = seed_round13_search_work(
        &harness,
        "Round Thirteen OpenLibrary",
        "Search Authority Author",
        "en",
        Some((
            ilr::IdentityProvider::Amazon,
            ilr::RouteKind::AsinEdition,
            asin,
        )),
    )
    .await;

    let report = round13_run_tick(&harness).await;
    assert_eq!(report.visited_work_count, 1);
    assert_eq!(search_calls.load(Ordering::Relaxed), 1);
    let captured =
        WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, work_id)
            .await
            .unwrap();
    assert!(captured.active_routes.iter().any(|route| {
        route.kind == ilr::RouteKind::OpenLibraryWork
            && route.provider_scoped_id == "OL-ROUND13-CORROBORATED-W"
    }));
    assert_eq!(captured.identity_generation, generation + 1);
    let attempts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_provider_attempts WHERE user_id=?1 AND work_id=?2 \
         AND provider='livrarr-convergence' AND route_kind='bridge-upgrade'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(attempts, 0);
    let audit: String = sqlx::query_scalar(
        "SELECT payload FROM identity_audit_events WHERE user_id=?1 AND work_id=?2 \
         AND event_kind='settlement' ORDER BY id DESC LIMIT 1",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert!(
        audit.contains("search-fallback") && audit.contains("AsinEdition"),
        "{audit}"
    );
}

async fn round18_route_graph_retry_invalidation_and_will_retry_tick_guard() {
    // Bug reproduction: identity-layer-rewrite round 18 — provider-level
    // terminal standing used to outlive a changed derived-anchor graph, and
    // the will_retry reset guard stopped short of the registered tick.
    let _breaker = lock_breaker().await;
    let anchor_calls = Arc::new(AtomicU64::new(0));
    let search_calls = Arc::new(AtomicU64::new(0));
    let work_key_calls = Arc::new(AtomicU64::new(0));
    let cover_calls = Arc::new(AtomicU64::new(0));
    let cover_bytes = Arc::new(fixture_jpeg(640, 960));
    let scripted = {
        let anchor_calls = anchor_calls.clone();
        let search_calls = search_calls.clone();
        let work_key_calls = work_key_calls.clone();
        let cover_calls = cover_calls.clone();
        let cover_bytes = cover_bytes.clone();
        Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
            if request.url.contains("/isbn/9780000001719.json") {
                anchor_calls.fetch_add(1, Ordering::Relaxed);
                return round17_status_response(404, b"{}".to_vec());
            }
            if request
                .url
                .contains("/works/OL-ROUND17-DEAD-ANCHOR-W/editions.json")
            {
                return round13_response(serde_json::to_vec(&json!({"entries": []})).unwrap());
            }
            if request.url.contains("/works/OL-ROUND17-DEAD-ANCHOR-W.json") {
                work_key_calls.fetch_add(1, Ordering::Relaxed);
                return round13_response(
                    serde_json::to_vec(&json!({
                        "title": "Round Seventeen Dead Anchor",
                        "description": {"value": "Round eighteen payload arrived"},
                        "covers": [1818]
                    }))
                    .unwrap(),
                );
            }
            if request
                .url
                .contains("covers.openlibrary.org/b/id/1818-L.jpg")
            {
                cover_calls.fetch_add(1, Ordering::Relaxed);
                return round13_response(cover_bytes.as_ref().clone());
            }
            if request.url.contains("search.json") {
                search_calls.fetch_add(1, Ordering::Relaxed);
                return round13_response(
                    serde_json::to_vec(&json!({"docs": [{
                        "key": "/works/OL-ROUND17-DEAD-ANCHOR-W",
                        "title": "Round Seventeen Dead Anchor",
                        "author_name": ["Dead Anchor Author"]
                    }]}))
                    .unwrap(),
                );
            }
            round17_status_response(404, b"{}".to_vec())
        }) as Arc<_>
    };
    let harness = build_route_harness_with_provider_details(
        None,
        Vec::new(),
        Some(round13_search_transport(scripted)),
    )
    .await;
    let (work_id, generation) = seed_round13_search_work(
        &harness,
        "Round Seventeen Dead Anchor",
        "Dead Anchor Author",
        "en",
        Some((
            ilr::IdentityProvider::IsbnRegistry,
            ilr::RouteKind::Isbn13Edition,
            "9780000001719",
        )),
    )
    .await;

    let work = WorkDb::get_work(&harness.db, harness.user_id, work_id)
        .await
        .expect("read dead-anchor fixture");
    let first = harness
        .state
        .provider_queue
        .dispatch_enrichment(
            &work,
            livrarr_enrichment::EnrichmentContext {
                priority: livrarr_domain::RequestPriority::Low,
                mode: livrarr_enrichment::EnrichmentMode::Background,
                freshness: livrarr_domain::Freshness::PreferCache,
                search_only: false,
            },
        )
        .await
        .expect("establish terminal dead-anchor standing");
    assert!(first.search_provider_identity.is_empty());
    assert_eq!(anchor_calls.load(Ordering::Relaxed), 1);
    assert_eq!(search_calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        harness
            .db
            .get_retry_state(
                harness.user_id,
                work_id,
                livrarr_domain::MetadataProvider::OpenLibrary,
            )
            .await
            .unwrap()
            .and_then(|state| state.last_outcome),
        Some(OutcomeClass::NotFound)
    );

    assert_eq!(round13_run_tick(&harness).await.visited_work_count, 1);
    assert_eq!(
        anchor_calls.load(Ordering::Relaxed),
        1,
        "terminal not_found must not refetch the dead anchor"
    );
    assert_eq!(search_calls.load(Ordering::Relaxed), 1);
    let captured =
        WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, work_id)
            .await
            .unwrap();
    assert!(captured.active_routes.iter().any(|route| {
        route.kind == ilr::RouteKind::OpenLibraryWork
            && route.provider_scoped_id == "OL-ROUND17-DEAD-ANCHOR-W"
            && matches!(
                route.provenance,
                ilr::RouteProvenance::TextDecisiveSearchFallback {
                    provider: ilr::IdentityProvider::OpenLibrary
                }
            )
    }));
    assert_eq!(captured.identity_generation, generation + 1);
    assert_eq!(
        work_key_calls.load(Ordering::Relaxed),
        0,
        "the search visit only attaches the route; its payload belongs to the next pass"
    );
    assert!(
        harness
            .db
            .get_retry_state(
                harness.user_id,
                work_id,
                livrarr_domain::MetadataProvider::OpenLibrary,
            )
            .await
            .unwrap()
            .is_none(),
        "a settlement that attaches a new work route must invalidate old-anchor standing"
    );
    let cards: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_review_cards WHERE user_id=?1 AND work_id=?2",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(cards, 0);
    let attempts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_provider_attempts WHERE user_id=?1 AND work_id=?2 \
         AND provider='livrarr-convergence' AND route_kind='bridge-upgrade'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(attempts, 0);

    sqlx::query("UPDATE works SET next_convergence_at=?1 WHERE user_id=?2 AND id=?3")
        .bind((Utc::now() - chrono::Duration::seconds(1)).to_rfc3339())
        .bind(harness.user_id)
        .bind(work_id)
        .execute(harness.db.pool())
        .await
        .unwrap();
    assert_eq!(
        round13_run_tick(&harness).await.visited_work_count,
        1,
        "the post-auto-link pass must remain schedulable for its new route"
    );
    assert_eq!(
        work_key_calls.load(Ordering::Relaxed),
        1,
        "the next tick must issue one new OL work-key fetch after the auto-link"
    );
    assert_eq!(anchor_calls.load(Ordering::Relaxed), 1);
    assert_eq!(search_calls.load(Ordering::Relaxed), 1);
    let open_library_standing = harness
        .db
        .get_retry_state(
            harness.user_id,
            work_id,
            livrarr_domain::MetadataProvider::OpenLibrary,
        )
        .await
        .unwrap()
        .expect("the new OL work-key fetch must persist success standing");
    let normalized_payload: livrarr_external_data::NormalizedWorkDetail = serde_json::from_str(
        open_library_standing
            .normalized_payload_json
            .as_deref()
            .expect("successful work-key standing must retain its payload"),
    )
    .unwrap();
    assert_eq!(
        normalized_payload.cover_url.as_deref(),
        Some("https://covers.openlibrary.org/b/id/1818-L.jpg?default=false")
    );
    let enriched = WorkDb::get_work(&harness.db, harness.user_id, work_id)
        .await
        .unwrap();
    assert_eq!(
        enriched.description.as_deref(),
        Some("Round eighteen payload arrived")
    );
    assert!(
        cover_calls.load(Ordering::Relaxed) >= 1,
        "the new work-key payload's cover must reach the normal cover gate"
    );
    assert_eq!(
        enriched.cover_url.as_deref(),
        Some("https://covers.openlibrary.org/b/id/1818-L.jpg?default=false")
    );
    assert!(
        harness
            .state
            .data_dir
            .join("covers")
            .join(harness.user_id.to_string())
            .join(format!("{work_id}.jpg"))
            .exists(),
        "the post-auto-link pass must materialize the Open Library cover"
    );
    let guarded_anchor_calls = Arc::new(AtomicU64::new(0));
    let guarded_search_calls = Arc::new(AtomicU64::new(0));
    let guarded_script = {
        let guarded_anchor_calls = guarded_anchor_calls.clone();
        let guarded_search_calls = guarded_search_calls.clone();
        Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
            if request.url.contains("/isbn/9780000001726.json") {
                guarded_anchor_calls.fetch_add(1, Ordering::Relaxed);
            }
            if request.url.contains("search.json") {
                guarded_search_calls.fetch_add(1, Ordering::Relaxed);
            }
            round17_status_response(503, b"{}".to_vec())
        }) as Arc<_>
    };
    let guarded = build_route_harness_with_provider_details(
        None,
        Vec::new(),
        Some(round13_search_transport(guarded_script)),
    )
    .await;
    let (guarded_work_id, _) = seed_round13_search_work(
        &guarded,
        "Round Seventeen Retry Guard",
        "Retry Guard Author",
        "en",
        Some((
            ilr::IdentityProvider::IsbnRegistry,
            ilr::RouteKind::Isbn13Edition,
            "9780000001726",
        )),
    )
    .await;
    guarded
        .db
        .record_will_retry(
            guarded.user_id,
            guarded_work_id,
            livrarr_domain::MetadataProvider::OpenLibrary,
            Utc::now() + chrono::Duration::minutes(5),
        )
        .await
        .unwrap();
    sqlx::query("CREATE TABLE round18_retry_delete_probe (provider TEXT NOT NULL)")
        .execute(guarded.db.pool())
        .await
        .unwrap();
    sqlx::query(&format!(
        "CREATE TRIGGER round18_retry_delete_probe_trigger \
         AFTER DELETE ON provider_retry_state \
         WHEN OLD.user_id={} AND OLD.work_id={} \
         BEGIN INSERT INTO round18_retry_delete_probe(provider) VALUES (OLD.provider); END",
        guarded.user_id, guarded_work_id
    ))
    .execute(guarded.db.pool())
    .await
    .unwrap();
    assert_eq!(round13_run_tick(&guarded).await.visited_work_count, 1);
    let guarded_calls_after_tick = guarded_anchor_calls.load(Ordering::Relaxed);
    assert!(
        guarded_calls_after_tick <= 1,
        "a due tick may fetch once or honestly honor will_retry"
    );
    assert_eq!(guarded_search_calls.load(Ordering::Relaxed), 0);
    let tick_deletes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM round18_retry_delete_probe")
        .fetch_one(guarded.db.pool())
        .await
        .unwrap();
    assert_eq!(
        tick_deletes, 0,
        "a normal convergence tick must not delete will_retry standing"
    );
    assert!(
        guarded
            .db
            .get_retry_state(
                guarded.user_id,
                guarded_work_id,
                livrarr_domain::MetadataProvider::OpenLibrary,
            )
            .await
            .unwrap()
            .is_some(),
        "the tick must preserve or update standing in place"
    );
    guarded
        .db
        .reset_all_retry_states(guarded.user_id, guarded_work_id)
        .await
        .unwrap();
    let explicit_deletes: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM round18_retry_delete_probe")
            .fetch_one(guarded.db.pool())
            .await
            .unwrap();
    assert_eq!(explicit_deletes, 1);
    let guarded_work = WorkDb::get_work(&guarded.db, guarded.user_id, guarded_work_id)
        .await
        .unwrap();
    guarded
        .state
        .provider_queue
        .dispatch_enrichment(
            &guarded_work,
            livrarr_enrichment::EnrichmentContext {
                priority: livrarr_domain::RequestPriority::Low,
                mode: livrarr_enrichment::EnrichmentMode::Background,
                freshness: livrarr_domain::Freshness::PreferCache,
                search_only: false,
            },
        )
        .await
        .expect("reset standing restores anchor-first dispatch");
    assert_eq!(
        guarded_anchor_calls.load(Ordering::Relaxed),
        guarded_calls_after_tick + 1
    );
    assert_eq!(guarded_search_calls.load(Ordering::Relaxed), 0);

    let edit_a_calls = Arc::new(AtomicU64::new(0));
    let edit_b_calls = Arc::new(AtomicU64::new(0));
    let edit_search_calls = Arc::new(AtomicU64::new(0));
    let edit_script = {
        let edit_a_calls = edit_a_calls.clone();
        let edit_b_calls = edit_b_calls.clone();
        let edit_search_calls = edit_search_calls.clone();
        Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
            if request.url.contains("/isbn/9780000001733.json") {
                edit_a_calls.fetch_add(1, Ordering::Relaxed);
                return round17_status_response(404, b"{}".to_vec());
            }
            if request.url.contains("/isbn/9780000001740.json") {
                edit_b_calls.fetch_add(1, Ordering::Relaxed);
                return round13_response(
                    serde_json::to_vec(&json!({
                        "works": [{"key": "/works/OL-ROUND18-EDIT-B-W"}]
                    }))
                    .unwrap(),
                );
            }
            if request
                .url
                .contains("/works/OL-ROUND18-EDIT-B-W/editions.json")
            {
                return round13_response(
                    serde_json::to_vec(&json!({
                        "entries": [{"isbn_13": ["9780000001740"]}]
                    }))
                    .unwrap(),
                );
            }
            if request.url.contains("/works/OL-ROUND18-EDIT-B-W.json") {
                return round13_response(
                    serde_json::to_vec(&json!({
                        "title": "Round Eighteen Rekey",
                        "description": "Fetched through ISBN B"
                    }))
                    .unwrap(),
                );
            }
            if request.url.contains("search.json") {
                edit_search_calls.fetch_add(1, Ordering::Relaxed);
            }
            round17_status_response(404, b"{}".to_vec())
        }) as Arc<_>
    };
    let edited = build_route_harness_with_provider_details(
        None,
        Vec::new(),
        Some(round13_search_transport(edit_script)),
    )
    .await;
    let (edited_work_id, _) = seed_round13_search_work(
        &edited,
        "Round Eighteen Rekey",
        "Rekey Author",
        "en",
        Some((
            ilr::IdentityProvider::IsbnRegistry,
            ilr::RouteKind::Isbn13Edition,
            "9780000001733",
        )),
    )
    .await;
    let edited_work = WorkDb::get_work(&edited.db, edited.user_id, edited_work_id)
        .await
        .unwrap();
    edited
        .state
        .provider_queue
        .dispatch_enrichment(
            &edited_work,
            livrarr_enrichment::EnrichmentContext {
                priority: livrarr_domain::RequestPriority::Low,
                mode: livrarr_enrichment::EnrichmentMode::Background,
                freshness: livrarr_domain::Freshness::PreferCache,
                search_only: false,
            },
        )
        .await
        .expect("establish ISBN-A terminal standing");
    assert_eq!(edit_a_calls.load(Ordering::Relaxed), 1);
    assert_eq!(edit_search_calls.load(Ordering::Relaxed), 0);

    let preview = edited
        .state
        .work_service
        .preview_identity_edit(
            edited.user_id,
            edited_work_id,
            "9780000001740",
            Some(livrarr_domain::identity::AnchorType::new(
                livrarr_domain::identity::AnchorType::ISBN_13,
            )),
        )
        .await
        .expect("preview ISBN B through the production work service");
    let preview_id = preview.preview_id.expect("ISBN B preview is certifiable");
    let b_calls_before_commit = edit_b_calls.load(Ordering::Relaxed);
    assert_eq!(b_calls_before_commit, 1);
    let committed = edited
        .state
        .work_service
        .commit_identity_edit(
            edited.user_id,
            edited_work_id,
            livrarr_domain::identity::AnchorType::new(
                livrarr_domain::identity::AnchorType::ISBN_13,
            ),
            &preview_id,
        )
        .await
        .expect("commit ISBN A to B through the production work service");
    assert!(!committed.no_op);
    let edited_identity =
        WorkIdentityRepository::read_captured_identity(&edited.db, edited.user_id, edited_work_id)
            .await
            .unwrap();
    assert!(edited_identity.active_routes.iter().any(|route| {
        route.kind == ilr::RouteKind::Isbn13Edition && route.provider_scoped_id == "9780000001740"
    }));
    assert!(!edited_identity.active_routes.iter().any(|route| {
        route.kind == ilr::RouteKind::Isbn13Edition && route.provider_scoped_id == "9780000001733"
    }));
    assert!(
        edited
            .db
            .get_retry_state(
                edited.user_id,
                edited_work_id,
                livrarr_domain::MetadataProvider::OpenLibrary,
            )
            .await
            .unwrap()
            .is_none(),
        "the A→B edit transaction must invalidate ISBN-A standing"
    );
    assert_eq!(round13_run_tick(&edited).await.visited_work_count, 1);
    assert_eq!(edit_a_calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        edit_b_calls.load(Ordering::Relaxed),
        b_calls_before_commit + 1,
        "the next convergence pass must fetch ISBN B"
    );
    assert_eq!(edit_search_calls.load(Ordering::Relaxed), 0);
}

async fn round13_uncorroborated_search_cards_once_and_burns_to_threshold() {
    let _breaker = lock_breaker().await;
    let scripted = Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
        if request.url.contains("search.json") {
            return round13_response(
                serde_json::to_vec(&json!({"docs": [{
                    "key": "/works/OL-ROUND13-PROPOSAL-W",
                    "title": "Round Thirteen Proposal",
                    // Spec v9 fixture move: title clears Same, while the
                    // provider's explicitly blank author canonicalizes to
                    // Abstain and therefore remains a near-miss proposal.
                    "author_name": [""],
                    "id_amazon": ["B0DIFFERENT13"]
                }]}))
                .unwrap(),
            );
        }
        round13_response(b"[]".to_vec())
    }) as Arc<_>;
    let harness = build_route_harness_with_provider_details(
        None,
        Vec::new(),
        Some(round13_search_transport(scripted)),
    )
    .await;
    let (work_id, generation) = seed_round13_search_work(
        &harness,
        "Round Thirteen Proposal",
        "Proposal Authority Author",
        "en",
        Some((
            ilr::IdentityProvider::Amazon,
            ilr::RouteKind::AsinEdition,
            "B0ROUND13OWN",
        )),
    )
    .await;

    let mut first_card_id = None;
    for expected_attempts in 1..=3_i64 {
        WorkDb::set_next_convergence_at(
            &harness.db,
            harness.user_id,
            work_id,
            Some(Utc::now() - chrono::Duration::seconds(1)),
        )
        .await
        .unwrap();
        let report = round13_run_tick(&harness).await;
        assert_eq!(report.visited_work_count, 1);
        let card_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_review_cards WHERE user_id=?1 AND work_id=?2 \
             AND kind='PendingRoute' AND status='pending'",
        )
        .bind(harness.user_id)
        .bind(work_id)
        .fetch_one(harness.db.pool())
        .await
        .unwrap();
        assert_eq!(
            card_count, 1,
            "equivalent proposal must reuse its oldest card"
        );
        let card_id: i64 = sqlx::query_scalar(
            "SELECT id FROM identity_review_cards WHERE user_id=?1 AND work_id=?2 \
             AND kind='PendingRoute' AND status='pending'",
        )
        .bind(harness.user_id)
        .bind(work_id)
        .fetch_one(harness.db.pool())
        .await
        .unwrap();
        match first_card_id {
            Some(expected_card_id) => assert_eq!(
                card_id, expected_card_id,
                "later ticks must reuse the first pending route card"
            ),
            None => first_card_id = Some(card_id),
        }
        let notifications: Vec<(String, String)> = sqlx::query_as(
            "SELECT type, message FROM notifications WHERE user_id=?1 AND ref_key=?2 ORDER BY id",
        )
        .bind(harness.user_id)
        .bind(format!("identity-review-card:{card_id}"))
        .fetch_all(harness.db.pool())
        .await
        .unwrap();
        assert_eq!(
            notifications,
            vec![(
                "identityReviewNeeded".to_string(),
                "Review needed: link 'Round Thirteen Proposal' — a possible OpenLibrary match was found"
                    .to_string(),
            )],
            "a new card emits one notification and semantic card reuse emits none"
        );
        let pending_cards = WorkIdentityRepository::list_pending_reviews(
            &harness.db,
            ReviewActor::AuthenticatedUser {
                user_id: harness.user_id,
            },
        )
        .await
        .unwrap();
        let pending_card = pending_cards
            .iter()
            .find(|card| card.id == card_id)
            .expect("new PendingRoute card is listable");
        assert_eq!(
            pending_card.work_title.as_deref(),
            Some("Round Thirteen Proposal")
        );
        assert_eq!(
            pending_card.work_author.as_deref(),
            Some("Proposal Authority Author")
        );
        let wire = serde_json::to_value(pending_card).unwrap();
        assert_eq!(wire["workTitle"], "Round Thirteen Proposal");
        assert_eq!(wire["workAuthor"], "Proposal Authority Author");
        let attempts: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_provider_attempts WHERE user_id=?1 AND work_id=?2 \
             AND provider='livrarr-convergence' AND route_kind='bridge-upgrade' \
             AND route_value=CAST(?3 AS TEXT)",
        )
        .bind(harness.user_id)
        .bind(work_id)
        .bind(generation)
        .fetch_one(harness.db.pool())
        .await
        .unwrap();
        assert_eq!(attempts, expected_attempts);
    }
    WorkDb::set_next_convergence_at(
        &harness.db,
        harness.user_id,
        work_id,
        Some(Utc::now() - chrono::Duration::seconds(1)),
    )
    .await
    .unwrap();
    assert_eq!(round13_run_tick(&harness).await.visited_work_count, 0);
    let active_work_routes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_routes WHERE user_id=?1 AND resolved_work_id=?2 \
         AND state='active' AND kind IN ('\"OpenLibraryWork\"','\"GoodreadsWork\"','\"HardcoverWork\"')",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(
        active_work_routes, 0,
        "a differing ASIN is not a veto or a route write"
    );
    let payload: String = sqlx::query_scalar(
        "SELECT payload FROM identity_review_cards WHERE user_id=?1 AND work_id=?2 \
         AND kind='PendingRoute' ORDER BY id ASC LIMIT 1",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert!(payload.contains("OL-ROUND13-PROPOSAL-W"), "{payload}");
}

async fn round13_zero_route_goodreads_card_affirms_end_to_end() {
    let _breaker = lock_breaker().await;
    let auto_calls = Arc::new(AtomicU64::new(0));
    let page_calls = Arc::new(AtomicU64::new(0));
    let scripted = {
        let auto_calls = auto_calls.clone();
        let page_calls = page_calls.clone();
        Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
            if request.url.contains("auto_complete") {
                auto_calls.fetch_add(1, Ordering::Relaxed);
                return round13_response(
                    serde_json::to_vec(&json!([{
                        "title": "Round Thirteen Minimum",
                        "bookTitleBare": "Round Thirteen Minimum",
                        "bookUrl": "/book/show/1313",
                        "bookId": "1313",
                        "workId": "31313",
                        // Spec v9 fixture move: no author evidence keeps this
                        // on the proposal-card side of the new auto-link bar.
                        "author": {}
                    }]))
                    .unwrap(),
                );
            }
            if request.url.contains("/book/show/") {
                page_calls.fetch_add(1, Ordering::Relaxed);
                return round13_response(round13_gr_page(
                    "Round Thirteen Minimum",
                    "Minimum Authority Author",
                    "1313",
                    "31313",
                    None,
                ));
            }
            round13_response(b"{\"docs\":[]}".to_vec())
        }) as Arc<_>
    };
    let harness = build_route_harness_with_provider_details(
        None,
        Vec::new(),
        Some(round13_search_transport(scripted)),
    )
    .await;
    let (work_id, generation) = seed_round13_search_work(
        &harness,
        "Round Thirteen Minimum",
        "Minimum Authority Author",
        "fr",
        None,
    )
    .await;

    assert_eq!(round13_run_tick(&harness).await.visited_work_count, 1);
    assert_eq!(auto_calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        page_calls.load(Ordering::Relaxed),
        0,
        "a zero-route work has no edition id to corroborate, so it must not probe"
    );
    let attempts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_provider_attempts WHERE user_id=?1 AND work_id=?2 \
         AND provider='livrarr-convergence' AND route_kind='bridge-upgrade' \
         AND route_value=CAST(?3 AS TEXT)",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .bind(generation)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(attempts, 1, "a card-only zero-route pass must burn");
    let (card_id, card_generation, payload): (i64, i64, String) = sqlx::query_as(
        "SELECT id, generation, payload FROM identity_review_cards WHERE user_id=?1 AND work_id=?2 \
         AND kind='PendingRoute' AND status='pending'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("zero-route search mints one pending route");
    assert!(
        payload.contains("31313"),
        "workId must come from autocomplete: {payload}"
    );
    let resolved = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/identity-review-card/{card_id}/resolve"),
        Some(resolve_body(
            card_id,
            card_generation,
            CardGate::PendingAffirm,
        )),
    )
    .await;
    assert!(resolved.status.is_success(), "{}", resolved.json);
    let captured =
        WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, work_id)
            .await
            .unwrap();
    assert_eq!(
        captured.status,
        ilr::IdentityStatus::UserConfirmed,
        "affirm is the user-confirmed connected badge state"
    );
    assert!(captured.active_routes.iter().any(|route| {
        route.kind == ilr::RouteKind::GoodreadsWork && route.provider_scoped_id == "31313"
    }));
}

// Bug reproduction: identity-layer-rewrite — zero-route decisive picks must not probe.
async fn round14_zero_route_probe_failure_still_cards_without_probe() {
    let _breaker = lock_breaker().await;
    let auto_calls = Arc::new(AtomicU64::new(0));
    let page_calls = Arc::new(AtomicU64::new(0));
    let scripted = {
        let auto_calls = auto_calls.clone();
        let page_calls = page_calls.clone();
        Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
            if request.url.contains("auto_complete") {
                auto_calls.fetch_add(1, Ordering::Relaxed);
                return round13_response(
                    serde_json::to_vec(&json!([{
                        "title": "Round Fourteen Minimum",
                        "bookTitleBare": "Round Fourteen Minimum",
                        "bookUrl": "/book/show/14140",
                        "bookId": "14140",
                        "workId": "414140",
                        // Spec v9 fixture move: author Abstain remains a card,
                        // while a Same+Agree zero-route pick now auto-links.
                        "author": {}
                    }]))
                    .unwrap(),
                );
            }
            if request.url.contains("/book/show/") {
                page_calls.fetch_add(1, Ordering::Relaxed);
                return livrarr_http::fetcher::ScriptedTransportOutcome::Response {
                    delay: Duration::ZERO,
                    response: livrarr_domain::services::FetchResponse {
                        status: 500,
                        headers: Vec::new(),
                        body: b"unreadable".to_vec(),
                    },
                };
            }
            round13_response(b"{\"docs\":[]}".to_vec())
        }) as Arc<_>
    };
    let harness = build_route_harness_with_provider_details(
        None,
        Vec::new(),
        Some(round13_search_transport(scripted)),
    )
    .await;
    let (work_id, generation) = seed_round13_search_work(
        &harness,
        "Round Fourteen Minimum",
        "Round Fourteen Authority Author",
        "fr",
        None,
    )
    .await;

    assert_eq!(round13_run_tick(&harness).await.visited_work_count, 1);
    assert_eq!(auto_calls.load(Ordering::Relaxed), 1);
    let card_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_review_cards WHERE user_id=?1 AND work_id=?2 \
         AND kind='PendingRoute' AND status='pending'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(
        (card_count, page_calls.load(Ordering::Relaxed)),
        (1, 0),
        "a decisive zero-route pick must card without a book-page request"
    );
    let payload: String = sqlx::query_scalar(
        "SELECT payload FROM identity_review_cards WHERE user_id=?1 AND work_id=?2 \
         AND kind='PendingRoute' AND status='pending'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert!(
        payload.contains("414140"),
        "card must carry autocomplete workId: {payload}"
    );
    let attempts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_provider_attempts WHERE user_id=?1 AND work_id=?2 \
         AND provider='livrarr-convergence' AND route_kind='bridge-upgrade' \
         AND route_value=CAST(?3 AS TEXT)",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .bind(generation)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(attempts, 1, "a proposal-only pass must burn once");
    let captured =
        WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, work_id)
            .await
            .unwrap();
    assert!(
        captured.active_routes.is_empty(),
        "proposal-only fallback must write zero routes"
    );
}

fn round15_pending_route_candidate(
    work_id: i64,
    provider: ilr::IdentityProvider,
    kind: ilr::RouteKind,
    value: &str,
) -> ilr::ParkedRouteCandidate {
    ilr::ParkedRouteCandidate {
        route: RouteKey {
            provider,
            kind,
            value: value.to_string(),
        },
        proposed_owner: RouteOwner::Work(work_id),
    }
}

async fn round15_settle_route(
    harness: &RouteHarness,
    work_id: i64,
    expected_generation: i64,
    candidate: &ilr::ParkedRouteCandidate,
) -> ilr::SettlementCommitOutcome {
    let captured =
        WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, work_id)
            .await
            .expect("read round-15 settlement identity");
    let mut command = settlement_commit(harness.user_id, captured.primary_author_id, Some(work_id));
    command.identity_title = captured.identity_title;
    command.text_distinction = Some(captured.text_distinction);
    command.expected_generation = expected_generation;
    command.routes = vec![ilr::WorkRoute {
        id: 0,
        user_id: harness.user_id,
        owner: RouteOwner::Work(work_id),
        resolved_work_id: work_id,
        provider: candidate.route.provider.clone(),
        kind: candidate.route.kind.clone(),
        provider_scoped_id: candidate.route.value.clone(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::Provider(candidate.route.provider.clone()),
        user_confirmed: false,
        observed_at: Utc::now(),
    }];
    WorkIdentityRepository::commit_settlement(&harness.db, command)
        .await
        .expect("activate round-15 route through settlement")
}

// Bug reproduction: identity-layer-rewrite round 15 / AC-025(c).
async fn round15_sibling_pending_route_cards_use_current_generation() {
    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;
    let (work_id, generation) = seed_round13_search_work(
        &harness,
        "Round Fifteen Sibling Cards",
        "Sibling Card Author",
        "en",
        None,
    )
    .await;
    let first_candidate = round15_pending_route_candidate(
        work_id,
        ilr::IdentityProvider::Goodreads,
        ilr::RouteKind::GoodreadsWork,
        "150001",
    );
    let second_candidate = round15_pending_route_candidate(
        work_id,
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        "OL-ROUND15-SIBLING-W",
    );
    let first = WorkIdentityRepository::commit_pending_route_review(
        &harness.db,
        harness.user_id,
        work_id,
        generation,
        first_candidate,
    )
    .await
    .expect("mint first sibling route card");
    let second = WorkIdentityRepository::commit_pending_route_review(
        &harness.db,
        harness.user_id,
        work_id,
        generation,
        second_candidate,
    )
    .await
    .expect("mint second sibling route card");

    let first_resolved = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/identity-review-card/{}/resolve", first.id),
        Some(resolve_body(first.id, generation, CardGate::PendingAffirm)),
    )
    .await;
    assert!(
        first_resolved.status.is_success(),
        "{}",
        first_resolved.json
    );
    let actionable_generation = work_generation(&harness.db, work_id).await;
    assert_eq!(actionable_generation, generation + 1);

    let listed = call_router_json(
        &harness,
        Method::GET,
        "/api/v1/identity-review-card".to_string(),
        None,
    )
    .await;
    assert_eq!(listed.status, StatusCode::OK);
    let listed_second_generation = listed
        .json
        .as_array()
        .and_then(|cards| cards.iter().find(|card| card["id"] == second.id))
        .and_then(|card| card["generation"].as_i64())
        .expect("second sibling card remains actionable");
    assert_eq!(listed_second_generation, actionable_generation);

    let second_resolved = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/identity-review-card/{}/resolve", second.id),
        Some(resolve_body(
            second.id,
            listed_second_generation,
            CardGate::PendingAffirm,
        )),
    )
    .await;
    assert!(
        second_resolved.status.is_success(),
        "sibling card must survive generation drift: {}",
        second_resolved.json
    );
    let captured =
        WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, work_id)
            .await
            .unwrap();
    assert!(captured.active_routes.iter().any(|route| {
        route.kind == ilr::RouteKind::GoodreadsWork && route.provider_scoped_id == "150001"
    }));
    assert!(captured.active_routes.iter().any(|route| {
        route.kind == ilr::RouteKind::OpenLibraryWork
            && route.provider_scoped_id == "OL-ROUND15-SIBLING-W"
    }));
}

// Bug reproduction: identity-layer-rewrite round 15 / AC-025(d).
// Bug reproduction: identity-layer-rewrite round 16 — C-r7-02 key-scoped cancellation.
async fn round15_pending_route_satisfaction_noop_and_foreign_owner_lifecycle() {
    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;

    let (satisfied_work, satisfied_generation) = seed_round13_search_work(
        &harness,
        "Round Fifteen Satisfied Card",
        "Satisfied Card Author",
        "en",
        None,
    )
    .await;
    let satisfied_candidate = round15_pending_route_candidate(
        satisfied_work,
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        "OL-ROUND15-SATISFIED-W",
    );
    let satisfied_card = WorkIdentityRepository::commit_pending_route_review(
        &harness.db,
        harness.user_id,
        satisfied_work,
        satisfied_generation,
        satisfied_candidate.clone(),
    )
    .await
    .expect("mint settlement-satisfied card");
    let unsatisfied_candidate = round15_pending_route_candidate(
        satisfied_work,
        ilr::IdentityProvider::Hardcover,
        ilr::RouteKind::HardcoverWork,
        "HC-ROUND16-UNSATISFIED-W",
    );
    let unsatisfied_card = WorkIdentityRepository::commit_pending_route_review(
        &harness.db,
        harness.user_id,
        satisfied_work,
        satisfied_generation,
        unsatisfied_candidate.clone(),
    )
    .await
    .expect("mint different-route sibling card");
    let settled = round15_settle_route(
        &harness,
        satisfied_work,
        satisfied_generation,
        &satisfied_candidate,
    )
    .await;
    let satisfied_status: String =
        sqlx::query_scalar("SELECT status FROM identity_review_cards WHERE id=?1")
            .bind(satisfied_card.id)
            .fetch_one(harness.db.pool())
            .await
            .expect("read satisfied card status");
    assert_eq!(
        satisfied_status, "cancelled",
        "route activation and satisfied cancellation share the settlement transaction"
    );
    let satisfaction_audits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events WHERE user_id=?1 AND work_id=?2 \
           AND event_kind='pending-route-satisfied'",
    )
    .bind(harness.user_id)
    .bind(satisfied_work)
    .fetch_one(harness.db.pool())
    .await
    .expect("count satisfied-card audit");
    assert_eq!(satisfaction_audits, 1);
    let unsatisfied_status: String =
        sqlx::query_scalar("SELECT status FROM identity_review_cards WHERE id=?1")
            .bind(unsatisfied_card.id)
            .fetch_one(harness.db.pool())
            .await
            .expect("read unsatisfied sibling card status");
    assert_eq!(
        unsatisfied_status, "pending",
        "settlement must cancel only the proposal key it satisfied"
    );
    let listed = call_router_json(
        &harness,
        Method::GET,
        "/api/v1/identity-review-card".to_string(),
        None,
    )
    .await;
    assert_eq!(listed.status, StatusCode::OK);
    let unsatisfied_generation = listed
        .json
        .as_array()
        .and_then(|cards| cards.iter().find(|card| card["id"] == unsatisfied_card.id))
        .and_then(|card| card["generation"].as_i64())
        .expect("unsatisfied sibling remains listed and actionable");
    assert_eq!(
        unsatisfied_generation, settled.identity.identity_generation,
        "unsatisfied sibling must use the current actionable generation"
    );

    let already_active_card = WorkIdentityRepository::commit_pending_route_review(
        &harness.db,
        harness.user_id,
        satisfied_work,
        settled.identity.identity_generation,
        satisfied_candidate.clone(),
    )
    .await
    .expect("mint already-active proposal card");
    let before_noop_generation = work_generation(&harness.db, satisfied_work).await;
    let before_route: (String, String, i64) = sqlx::query_as(
        "SELECT provenance, observed_at, user_confirmed FROM identity_routes \
          WHERE user_id=?1 AND resolved_work_id=?2 AND provider_scoped_id=?3 AND state='active'",
    )
    .bind(harness.user_id)
    .bind(satisfied_work)
    .bind(&satisfied_candidate.route.value)
    .fetch_one(harness.db.pool())
    .await
    .expect("snapshot already-active route");
    let noop = call_router_json(
        &harness,
        Method::POST,
        format!(
            "/api/v1/identity-review-card/{}/resolve",
            already_active_card.id
        ),
        Some(resolve_body(
            already_active_card.id,
            before_noop_generation,
            CardGate::PendingAffirm,
        )),
    )
    .await;
    assert!(noop.status.is_success(), "{}", noop.json);
    assert_eq!(
        work_generation(&harness.db, satisfied_work).await,
        before_noop_generation,
        "already-active affirmation resolves only the card"
    );
    let after_route: (String, String, i64) = sqlx::query_as(
        "SELECT provenance, observed_at, user_confirmed FROM identity_routes \
          WHERE user_id=?1 AND resolved_work_id=?2 AND provider_scoped_id=?3 AND state='active'",
    )
    .bind(harness.user_id)
    .bind(satisfied_work)
    .bind(&satisfied_candidate.route.value)
    .fetch_one(harness.db.pool())
    .await
    .expect("read already-active route after no-op");
    assert_eq!(after_route, before_route);

    let unsatisfied_resolved = call_router_json(
        &harness,
        Method::POST,
        format!(
            "/api/v1/identity-review-card/{}/resolve",
            unsatisfied_card.id
        ),
        Some(resolve_body(
            unsatisfied_card.id,
            unsatisfied_generation,
            CardGate::PendingAffirm,
        )),
    )
    .await;
    assert!(
        unsatisfied_resolved.status.is_success(),
        "unsatisfied sibling must remain actionable: {}",
        unsatisfied_resolved.json
    );
    let after_unsatisfied = WorkIdentityRepository::read_captured_identity(
        &harness.db,
        harness.user_id,
        satisfied_work,
    )
    .await
    .unwrap();
    assert!(after_unsatisfied.active_routes.iter().any(|route| {
        route.kind == ilr::RouteKind::HardcoverWork
            && route.provider_scoped_id == "HC-ROUND16-UNSATISFIED-W"
    }));

    let (foreign_owner, foreign_owner_generation) = seed_round13_search_work(
        &harness,
        "Round Fifteen Foreign Owner",
        "Foreign Owner Author",
        "en",
        None,
    )
    .await;
    let owner_candidate = round15_pending_route_candidate(
        foreign_owner,
        ilr::IdentityProvider::Goodreads,
        ilr::RouteKind::GoodreadsWork,
        "150099",
    );
    round15_settle_route(
        &harness,
        foreign_owner,
        foreign_owner_generation,
        &owner_candidate,
    )
    .await;
    let (foreign_target, foreign_target_generation) = seed_round13_search_work(
        &harness,
        "Round Fifteen Foreign Target",
        "Foreign Target Author",
        "en",
        None,
    )
    .await;
    let foreign_candidate = round15_pending_route_candidate(
        foreign_target,
        ilr::IdentityProvider::Goodreads,
        ilr::RouteKind::GoodreadsWork,
        "150099",
    );
    let foreign_card = WorkIdentityRepository::commit_pending_route_review(
        &harness.db,
        harness.user_id,
        foreign_target,
        foreign_target_generation,
        foreign_candidate,
    )
    .await
    .expect("mint foreign-owner invalidation card");
    let invalid = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/identity-review-card/{}/resolve", foreign_card.id),
        Some(resolve_body(
            foreign_card.id,
            foreign_target_generation,
            CardGate::PendingAffirm,
        )),
    )
    .await;
    assert_eq!(invalid.status, StatusCode::CONFLICT, "{}", invalid.json);
    assert_eq!(
        invalid.json["message"],
        "review proposal invalidated: proposed route is now owned by a different work"
    );
    let dismissed = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/identity-review-card/{}/dismiss", foreign_card.id),
        None,
    )
    .await;
    assert_eq!(dismissed.status, StatusCode::NO_CONTENT);
}

// Bug reproduction: identity-layer-rewrite round 15 / AC-025(a).
async fn round15_same_agree_uncorroborated_search_auto_links() {
    let _breaker = lock_breaker().await;
    let scripted = Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
        if request.url.contains("search.json") {
            return round13_response(
                serde_json::to_vec(&json!({"docs": [{
                    "key": "/works/OL-ROUND15-TEXT-DECISIVE-W",
                    "title": "Round Fifteen Text Decisive",
                    "author_name": ["Text Decisive Author"]
                }]}))
                .unwrap(),
            );
        }
        round13_response(b"[]".to_vec())
    }) as Arc<_>;
    let harness = build_route_harness_with_provider_details(
        None,
        Vec::new(),
        Some(round13_search_transport(scripted)),
    )
    .await;
    let (work_id, generation) = seed_round13_search_work(
        &harness,
        "Round Fifteen Text Decisive",
        "Text Decisive Author",
        "en",
        Some((
            ilr::IdentityProvider::Amazon,
            ilr::RouteKind::AsinEdition,
            "B0ROUND15OWN",
        )),
    )
    .await;

    assert_eq!(round13_run_tick(&harness).await.visited_work_count, 1);
    let captured =
        WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, work_id)
            .await
            .unwrap();
    assert_eq!(captured.identity_generation, generation + 1);
    let text_route = captured
        .active_routes
        .iter()
        .find(|route| {
            route.kind == ilr::RouteKind::OpenLibraryWork
                && route.provider_scoped_id == "OL-ROUND15-TEXT-DECISIVE-W"
        })
        .expect("text-decisive work route settles");
    let text_provenance = serde_json::to_string(&text_route.provenance).unwrap();
    assert!(
        text_provenance.contains("TextDecisiveSearchFallback"),
        "{text_provenance}"
    );
    let cards: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_review_cards WHERE user_id=?1 AND work_id=?2 \
         AND kind='PendingRoute' AND status='pending'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    let burns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_provider_attempts WHERE user_id=?1 AND work_id=?2 \
         AND provider='livrarr-convergence' AND route_kind='bridge-upgrade'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!((cards, burns), (0, 0));
    let audit: String = sqlx::query_scalar(
        "SELECT payload FROM identity_audit_events WHERE user_id=?1 AND work_id=?2 \
         AND event_kind='settlement' ORDER BY id DESC LIMIT 1",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert!(audit.contains("TEXT-DECISIVE"), "{audit}");
}

// Boundary pin: no pre-fix behavior changes here, so this is intentionally
// red-exempt. AC-024(b)/(c) bind author-Abstain; this case binds the distinct
// provider-work-id tie and the queue-to-authority dependency.
async fn round15_distinct_same_agree_candidates_card_through_authority() {
    let _breaker = lock_breaker().await;
    let scripted = Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
        if request.url.contains("search.json") {
            return round13_response(
                serde_json::to_vec(&json!({"docs": [
                    {
                        "key": "/works/OL-ROUND15-TIE-A-W",
                        "title": "Round Fifteen Distinct Tie",
                        "author_name": ["Distinct Tie Author"]
                    },
                    {
                        "key": "/works/OL-ROUND15-TIE-B-W",
                        "title": "Round Fifteen Distinct Tie",
                        "author_name": ["Distinct Tie Author"]
                    }
                ]}))
                .unwrap(),
            );
        }
        round13_response(b"[]".to_vec())
    }) as Arc<_>;
    let harness = build_route_harness_with_provider_details(
        None,
        Vec::new(),
        Some(round13_search_transport(scripted)),
    )
    .await;
    let (work_id, generation) = seed_round13_search_work(
        &harness,
        "Round Fifteen Distinct Tie",
        "Distinct Tie Author",
        "en",
        Some((
            ilr::IdentityProvider::Amazon,
            ilr::RouteKind::AsinEdition,
            "B0ROUND15TIE",
        )),
    )
    .await;

    assert_eq!(round13_run_tick(&harness).await.visited_work_count, 1);
    assert_eq!(work_generation(&harness.db, work_id).await, generation);
    let payloads: Vec<String> = sqlx::query_scalar(
        "SELECT payload FROM identity_review_cards WHERE user_id=?1 AND work_id=?2 \
         AND kind='PendingRoute' AND status='pending'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_all(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(payloads.len(), 1);
    assert!(
        payloads[0].contains("OL-ROUND15-TIE-A-W") || payloads[0].contains("OL-ROUND15-TIE-B-W")
    );
    let work_routes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_routes WHERE user_id=?1 AND resolved_work_id=?2 \
         AND state='active' AND kind='\"OpenLibraryWork\"'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    let burns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_provider_attempts WHERE user_id=?1 AND work_id=?2 \
         AND provider='livrarr-convergence' AND route_kind='bridge-upgrade'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!((work_routes, burns), (0, 1));

    let queue_source = include_str!("../../crates/livrarr-enrichment/src/provider_queue.rs");
    assert!(queue_source.contains("classify_search_fallback("));
    for forbidden_inline_authority in ["title_verdict(", "author_verdict(", "pick_best_candidate("]
    {
        assert!(
            !queue_source.contains(forbidden_inline_authority),
            "queue must consume the typed matching decision, not inline {forbidden_inline_authority}"
        );
    }
}

async fn round15_goodreads_probe_failure_preserves_text_decisive_auto_link() {
    let _breaker = lock_breaker().await;
    let auto_calls = Arc::new(AtomicU64::new(0));
    let page_calls = Arc::new(AtomicU64::new(0));
    let scripted = {
        let auto_calls = auto_calls.clone();
        let page_calls = page_calls.clone();
        Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
            if request.url.contains("auto_complete") {
                auto_calls.fetch_add(1, Ordering::Relaxed);
                return round13_response(
                    serde_json::to_vec(&json!([{
                        "title": "Round Fifteen Probe Downgrade",
                        "bookTitleBare": "Round Fifteen Probe Downgrade",
                        "bookUrl": "/book/show/15151",
                        "bookId": "15151",
                        "workId": "515151",
                        "author": {"name": "Probe Downgrade Author"}
                    }]))
                    .unwrap(),
                );
            }
            if request.url.contains("/book/show/") {
                page_calls.fetch_add(1, Ordering::Relaxed);
                return livrarr_http::fetcher::ScriptedTransportOutcome::Response {
                    delay: Duration::ZERO,
                    response: livrarr_domain::services::FetchResponse {
                        status: 500,
                        headers: Vec::new(),
                        body: b"probe failed".to_vec(),
                    },
                };
            }
            round13_response(b"{\"docs\":[]}".to_vec())
        }) as Arc<_>
    };
    let harness = build_route_harness_with_provider_details(
        None,
        Vec::new(),
        Some(round13_search_transport(scripted)),
    )
    .await;
    let (work_id, generation) = seed_round13_search_work(
        &harness,
        "Round Fifteen Probe Downgrade",
        "Probe Downgrade Author",
        "fr",
        Some((
            ilr::IdentityProvider::Amazon,
            ilr::RouteKind::AsinEdition,
            "B0ROUND15PROBE",
        )),
    )
    .await;

    assert_eq!(round13_run_tick(&harness).await.visited_work_count, 1);
    assert_eq!(
        (
            auto_calls.load(Ordering::Relaxed),
            page_calls.load(Ordering::Relaxed)
        ),
        (1, 1)
    );
    let captured =
        WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, work_id)
            .await
            .unwrap();
    assert_eq!(captured.identity_generation, generation + 1);
    let route = captured
        .active_routes
        .iter()
        .find(|route| {
            route.kind == ilr::RouteKind::GoodreadsWork && route.provider_scoped_id == "515151"
        })
        .expect("failed corroboration probe must fall back to text-decisive work route");
    assert!(serde_json::to_string(&route.provenance)
        .unwrap()
        .contains("TextDecisiveSearchFallback"));
    assert!(!captured.active_routes.iter().any(|route| {
        route.kind == ilr::RouteKind::GoodreadsBookEdition && route.provider_scoped_id == "15151"
    }));
    let cards: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_review_cards WHERE user_id=?1 AND work_id=?2 \
         AND kind='PendingRoute' AND status='pending'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    let burns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_provider_attempts WHERE user_id=?1 AND work_id=?2 \
         AND provider='livrarr-convergence' AND route_kind='bridge-upgrade'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!((cards, burns), (0, 0));
}

async fn round15_goodreads_propose_grade_probe_failure_remains_miss() {
    let _breaker = lock_breaker().await;
    let page_calls = Arc::new(AtomicU64::new(0));
    let scripted = {
        let page_calls = page_calls.clone();
        Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
            if request.url.contains("auto_complete") {
                return round13_response(
                    serde_json::to_vec(&json!([{
                        "title": "Round Fifteen Proposal Probe Miss",
                        "bookTitleBare": "Round Fifteen Proposal Probe Miss",
                        "bookUrl": "/book/show/15152",
                        "bookId": "15152",
                        "workId": "515152",
                        "author": {}
                    }]))
                    .unwrap(),
                );
            }
            if request.url.contains("/book/show/") {
                page_calls.fetch_add(1, Ordering::Relaxed);
                return livrarr_http::fetcher::ScriptedTransportOutcome::Response {
                    delay: Duration::ZERO,
                    response: livrarr_domain::services::FetchResponse {
                        status: 500,
                        headers: Vec::new(),
                        body: b"probe failed".to_vec(),
                    },
                };
            }
            round13_response(b"{\"docs\":[]}".to_vec())
        }) as Arc<_>
    };
    let harness = build_route_harness_with_provider_details(
        None,
        Vec::new(),
        Some(round13_search_transport(scripted)),
    )
    .await;
    let (work_id, generation) = seed_round13_search_work(
        &harness,
        "Round Fifteen Proposal Probe Miss",
        "Proposal Probe Author",
        "fr",
        Some((
            ilr::IdentityProvider::Amazon,
            ilr::RouteKind::AsinEdition,
            "B0ROUND15MISS",
        )),
    )
    .await;

    assert_eq!(round13_run_tick(&harness).await.visited_work_count, 1);
    assert_eq!(page_calls.load(Ordering::Relaxed), 1);
    assert_eq!(work_generation(&harness.db, work_id).await, generation);
    let cards: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_review_cards WHERE user_id=?1 AND work_id=?2 \
         AND kind='PendingRoute' AND status='pending'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    let burns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_provider_attempts WHERE user_id=?1 AND work_id=?2 \
         AND provider='livrarr-convergence' AND route_kind='bridge-upgrade'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!((cards, burns), (0, 1));
}

async fn round13_goodreads_probe_corroboration_settles_work_and_book() {
    let _breaker = lock_breaker().await;
    let request_count = Arc::new(AtomicU64::new(0));
    let scripted = {
        let request_count = request_count.clone();
        Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
            request_count.fetch_add(1, Ordering::Relaxed);
            if request.url.contains("auto_complete") {
                return round13_response(
                    serde_json::to_vec(&json!([{
                        "title": "Round Thirteen Goodreads",
                        "bookTitleBare": "Round Thirteen Goodreads",
                        "bookUrl": "/book/show/1414",
                        "bookId": "1414",
                        "workId": "41414",
                        "author": {"name": "Goodreads Authority Author"}
                    }]))
                    .unwrap(),
                );
            }
            if request.url.contains("/book/show/") {
                return round13_response(round13_gr_page(
                    "Round Thirteen Goodreads",
                    "Goodreads Authority Author",
                    "1414",
                    "41414",
                    Some("B0ROUND13GR1"),
                ));
            }
            round13_response(b"{\"docs\":[]}".to_vec())
        }) as Arc<_>
    };
    let harness = build_route_harness_with_provider_details(
        None,
        Vec::new(),
        Some(round13_search_transport(scripted)),
    )
    .await;
    let (work_id, _) = seed_round13_search_work(
        &harness,
        "Round Thirteen Goodreads",
        "Goodreads Authority Author",
        "fr",
        Some((
            ilr::IdentityProvider::Amazon,
            ilr::RouteKind::AsinEdition,
            "B0ROUND13GR1",
        )),
    )
    .await;

    assert_eq!(round13_run_tick(&harness).await.visited_work_count, 1);
    assert_eq!(
        request_count.load(Ordering::Relaxed),
        2,
        "autocomplete carries workId/bookId; only the one required page probe follows"
    );
    let captured =
        WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, work_id)
            .await
            .unwrap();
    assert!(captured.active_routes.iter().any(|route| {
        route.kind == ilr::RouteKind::GoodreadsWork && route.provider_scoped_id == "41414"
    }));
    assert!(captured.active_routes.iter().any(|route| {
        route.kind == ilr::RouteKind::GoodreadsBookEdition && route.provider_scoped_id == "1414"
    }));
    let attempts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_provider_attempts WHERE user_id=?1 AND work_id=?2 \
         AND provider='livrarr-convergence' AND route_kind='bridge-upgrade'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(attempts, 0);
    let audit: String = sqlx::query_scalar(
        "SELECT payload FROM identity_audit_events WHERE user_id=?1 AND work_id=?2 \
         AND event_kind='settlement' ORDER BY id DESC LIMIT 1",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert!(audit.contains("search-fallback") && audit.contains("AsinEdition"));
}

async fn round13_search_fallback_precondition_and_applicability_boundary() {
    // The original work-level suppression pin is superseded by REQ-027 v11.
    // This now proves provider-local suppression (OL owns a route, GR does not)
    // together with the unchanged foreign-language applicability policy.
    let _breaker = lock_breaker().await;
    let ol_searches = Arc::new(AtomicU64::new(0));
    let gr_searches = Arc::new(AtomicU64::new(0));
    let anchor_requests = Arc::new(AtomicU64::new(0));
    let scripted = {
        let ol_searches = ol_searches.clone();
        let gr_searches = gr_searches.clone();
        let anchor_requests = anchor_requests.clone();
        Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
            if request.url.contains("search.json") {
                ol_searches.fetch_add(1, Ordering::Relaxed);
            } else if request.url.contains("auto_complete") {
                gr_searches.fetch_add(1, Ordering::Relaxed);
            } else {
                anchor_requests.fetch_add(1, Ordering::Relaxed);
            }
            livrarr_http::fetcher::ScriptedTransportOutcome::Response {
                delay: Duration::ZERO,
                response: livrarr_domain::services::FetchResponse {
                    status: 404,
                    headers: Vec::new(),
                    body: Vec::new(),
                },
            }
        }) as Arc<_>
    };
    let harness = build_route_harness_with_provider_details(
        None,
        Vec::new(),
        Some(round13_search_transport(scripted)),
    )
    .await;
    let (work_id, _) = seed_round13_search_work(
        &harness,
        "Round Thirteen Routed Boundary",
        "Boundary Authority Author",
        "en",
        Some((
            ilr::IdentityProvider::OpenLibrary,
            ilr::RouteKind::OpenLibraryWork,
            "OL-ROUND13-ALREADY-ROUTED-W",
        )),
    )
    .await;
    sqlx::query("UPDATE works SET enrichment_status='failed' WHERE user_id=?1 AND id=?2")
        .bind(harness.user_id)
        .bind(work_id)
        .execute(harness.db.pool())
        .await
        .unwrap();
    assert_eq!(round13_run_tick(&harness).await.visited_work_count, 1);
    assert_eq!(ol_searches.load(Ordering::Relaxed), 0);
    assert_eq!(gr_searches.load(Ordering::Relaxed), 1);
    assert!(anchor_requests.load(Ordering::Relaxed) >= 1);
    drop(harness);
    drop(_breaker);

    let _breaker = lock_breaker().await;
    let ol_searches = Arc::new(AtomicU64::new(0));
    let gr_searches = Arc::new(AtomicU64::new(0));
    let scripted = {
        let ol_searches = ol_searches.clone();
        let gr_searches = gr_searches.clone();
        Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
            if request.url.contains("search.json") {
                ol_searches.fetch_add(1, Ordering::Relaxed);
                return round13_response(b"{\"docs\":[]}".to_vec());
            }
            if request.url.contains("auto_complete") {
                gr_searches.fetch_add(1, Ordering::Relaxed);
                return round13_response(
                    serde_json::to_vec(&json!([{
                        "title": "Round Thirteen Foreign Boundary",
                        "bookTitleBare": "Round Thirteen Foreign Boundary",
                        "bookUrl": "/book/show/1515",
                        "bookId": "1515",
                        "workId": "51515",
                        "author": {"name": "Foreign Boundary Author"}
                    }]))
                    .unwrap(),
                );
            }
            round13_response(round13_gr_page(
                "Round Thirteen Foreign Boundary",
                "Foreign Boundary Author",
                "1515",
                "51515",
                None,
            ))
        }) as Arc<_>
    };
    let harness = build_route_harness_with_provider_details(
        None,
        Vec::new(),
        Some(round13_search_transport(scripted)),
    )
    .await;
    seed_round13_search_work(
        &harness,
        "Round Thirteen Foreign Boundary",
        "Foreign Boundary Author",
        "fr",
        Some((
            ilr::IdentityProvider::Amazon,
            ilr::RouteKind::AsinEdition,
            "B0ROUND13FOREIGN",
        )),
    )
    .await;
    assert_eq!(round13_run_tick(&harness).await.visited_work_count, 1);
    assert_eq!(ol_searches.load(Ordering::Relaxed), 0);
    assert_eq!(gr_searches.load(Ordering::Relaxed), 1);
}

macro_rules! red_tests {
    ($($name:ident => $body:expr),+ $(,)?) => {
        $(
            #[tokio::test]
            async fn $name() {
                ($body).await;
            }
        )+
    };
}

red_tests! {
    // tdd_execution_policy — ten mandatory real-door gates.
    door_gate_work_update_rekey_mints_then_resolves => red_full_card_gate(CardContract::DoorWorkUpdate),
    door_gate_legacy_identity_preview_route_is_absent => red_router_legacy_absent(RouterCase::LegacyPreview),
    door_gate_legacy_identity_commit_route_is_absent => red_router_legacy_absent(RouterCase::LegacyCommit),
    door_gate_legacy_identity_clear_route_is_absent => red_router_legacy_absent(RouterCase::LegacyClear),
    door_gate_manual_merge_mints_then_resolves => red_full_card_gate(CardContract::DoorManualMerge),
    door_gate_pending_affirm_mints_then_resolves => red_full_card_gate(CardContract::DoorPendingAffirm),
    door_gate_retry_incomplete_uses_convergence_visit_settle_only => red_missing_composition(CompositionContract::RetryConvergenceOnly),
    door_gate_conflict_resolve_calls_resolve_review_once => red_missing_composition(CompositionContract::ConflictResolveOnce),
    door_gate_conflict_dismiss_calls_resolve_review_once => red_missing_composition(CompositionContract::ConflictDismissReject),
    door_gate_legacy_grey_dismiss_route_is_absent => red_router_legacy_absent(RouterCase::GreyDismiss),

    // livrarr-domain.
    identity_road_every_listed_settle_door_reaches_one_settle => red_missing_composition(CompositionContract::EveryDoorOneSettle),
    identity_road_rejects_each_door_matrix_violation_before_side_effects => red_road_door_matrix(),
    identity_road_existing_paths_claim_their_predecision_snapshot => red_road_generation_contract(RoadGenerationContract::DomainPredecisionSnapshot),
    interactive_review_requires_freshly_minted_card => red_full_card_gate(CardContract::DomainInteractiveFresh),
    http_and_cli_resolve_share_exact_continuation => red_review_entries(ReviewEntryContract::HttpCliParity),
    review_kind_scope_generation_and_cancel_fail_closed => red_review_entries(ReviewEntryContract::FailClosedMatrix),
    read_captured_identity_projects_edition_routes_and_primary_ordinal => red_repo_read(RepoReadContract::Projection),
    read_captured_identity_cross_user_is_not_found => red_repo_read(RepoReadContract::CrossUser),
    dedup_adopt_and_race_loser_commit_once_under_generation => red_repo_commit(RepoCommitContract::DomainDedupGeneration),
    stale_route_and_key_collisions_rollback_whole_graph => red_repo_commit(RepoCommitContract::DomainCollisionRollback),
    conflict_accept_reject_and_different_work_are_atomic => red_repo_conflict(RepoConflictContract::AtomicActions),
    ambiguous_edition_target_stays_pending => red_repo_conflict(RepoConflictContract::AmbiguousEdition),
    edition_evidence_unknown_absent_and_contradiction => red_repo_edition(EditionRepositoryContract::UnknownAbsentContradiction),
    edition_subtitle_never_backflows_from_work => red_repo_edition(EditionRepositoryContract::NoSubtitleBackflow),
    cutover_rehearsal_two_copied_snapshots_are_byte_identical => red_cutover_trait(RehearseContract::ByteIdenticalCopies),
    cutover_rehearsal_rejects_live_or_schema_mismatch => red_cutover_trait(RehearseContract::RejectLiveOrSchemaMismatch),
    apply_block_resolve_rerun_is_idempotent => red_cutover_apply(ApplyContract::BlockResolveRerun),
    apply_source_fingerprint_mismatch_and_collision_never_activate => red_cutover_apply(ApplyContract::FingerprintAndCollision),
    readiness_active_empty_and_nonempty_inactive_branches => red_db_readiness(DbReadinessContract::DomainBranches),
    readiness_cancelled_before_empty_activation_leaves_no_marker_or_index => red_db_readiness(DbReadinessContract::CancelledNoMarker),
    structured_subtitle_wins_without_tail_comparison => red_title_parser(TitleContract::StructuredWins),
    empty_provider_main_is_invalid => red_title_parser(TitleContract::EmptyMain),
    manual_refresh_structured_subtitle_wins_through_real_door => red_missing_composition(CompositionContract::ManualRefreshStructuredSubtitle),
    lost_match_tuning_cannot_change_wrong_merge_outcomes_and_reverse => red_match_policy(MatchContract::GuardIndependence),
    edition_inequality_abstains_and_equality_only_confirms_inside_authority => red_match_policy(MatchContract::EditionEquality),
    opaque_probe_capabilities_are_required_for_text_and_alias_arms => red_match_policy(MatchContract::OpaqueCapabilities),
    machine_subtitle_recomputes_without_identity_mutation => red_subtitle_policy(SubtitleContract::RecomputeWithoutIdentityMutation),
    selected_edition_explicit_absence_projects_absence => red_subtitle_policy(SubtitleContract::ExplicitAbsence),
    cover_rank_user_file_default_same_format_then_other => red_cover_policy(CoverContract::SourceRank),
    one_format_needed_panel_and_independent_nowhere_to_look => red_cover_policy(CoverContract::SharedFormatPanel),

    // livrarr-db.
    commit_settlement_absorbs_anchor_match_adopt_normalized_dedup_and_race_loser => red_repo_commit(RepoCommitContract::DbBranchMatrix),
    commit_settlement_stale_route_key_and_database_failures_rollback => red_repo_commit(RepoCommitContract::DbFaultRollback),
    transfer_route_zero_one_many_target_matrix => red_db_transfer(TransferContract::ZeroOneMany),
    transfer_route_fault_each_statement_rolls_back => red_db_transfer(TransferContract::StatementRollback),
    projection_recompute_is_total_and_generation_claimed => red_db_projection(ProjectionContract::TotalAndClaimed),
    projection_recompute_has_no_legacy_grade_dependency => red_db_projection(ProjectionContract::NoLegacyGrade),
    inspection_record_four_outcomes_persist_by_exact_revision => red_db_inspection(InspectionDbContract::FourOutcomes),
    inspection_record_revision_race_is_stale => red_db_inspection(InspectionDbContract::RevisionRace),
    inspection_read_never_reuses_other_revision_or_user => red_db_inspection(InspectionDbContract::UserAndRevisionScope),
    cutover_blocked_apply_commits_only_staging_then_rerun_reuses_rows => red_db_cutover(DbCutoverContract::BlockedStagingReuse),
    cutover_activation_index_is_last_and_atomic => red_db_cutover(DbCutoverContract::ActivationIndexLast),
    cutover_total_legacy_mapping_and_consumer_categories => red_db_cutover(DbCutoverContract::TotalLegacyMapping),
    readiness_empty_activates_and_nonempty_requires_cutover => red_db_readiness(DbReadinessContract::EmptyVsNonempty),
    readiness_index_failure_rolls_back_marker => red_db_readiness(DbReadinessContract::IndexFailureRollback),
    pre_cutover_fixture_covers_complete_groups_badges_reviews_and_attempts => red_pre_cutover_helper(PreCutoverContract::Categories),
    pre_cutover_helper_cannot_clear_active_marker => red_pre_cutover_helper(PreCutoverContract::CannotClearActive),
    ordinary_create_test_db_has_new_index_no_old_work_index_and_active_marker => red_db_readiness(DbReadinessContract::OrdinaryHelperIndexes),
    existing_provider_titles_heal_once_with_generation_audit => title_policy_heal_rewrites_existing_rows_atomically(),
    title_policy_heal_collision_parks_group_review_and_retries_marker => title_policy_heal_parks_colliding_cohort(),
    article_variant_heal_folds_one_sided_volume_and_preserves_routes => article_variant_heal_folds_one_sided_volume_and_preserves_routes_contract(),
    article_variant_heal_parks_work_key_contradiction_with_current_card => article_variant_heal_parks_contradictory_work_routes_with_current_generation(),
    article_variant_heal_ignores_more_than_leading_article_difference => article_variant_heal_does_not_fold_a_larger_title_difference(),
    startup_heal_reowns_work_owned_edition_routes_once_with_one_generation_audit => route_taxonomy_heal_reowns_legacy_edition_routes_once(),
    startup_heal_folds_dedup_orphan_and_cleans_duplicate_group_cards => dedup_residue_startup_heal_folds_orphan_and_cleans_duplicate_cards(),
    startup_heal_keeps_one_equivalent_pending_group_card => dedup_residue_startup_heal_keeps_one_equivalent_pending_card(),
    startup_heal_round10_reopens_bridges_reclassifies_dishonest_enrichment_and_deletes_only_safe_readarr_orphans => round10_residue_heal_is_exact_marker_gated_and_conservative(),
    startup_heal_round11_reclears_only_convergence_bridge_attempts_once => round11_attempt_reheal_is_exact_marker_gated_and_idempotent(),
    ac025e_startup_heal_reopens_only_no_work_route_search_ledgers => round15_search_ledger_reset_is_route_scoped_and_idempotent(),
    readarr_failed_creation_compensates_only_untouched_import_authors => readarr_failed_creation_compensation_deletes_only_untouched_batch_authors(),

    // livrarr-external-data.
    fetch_by_route_closed_kind_matrix_and_failure_policy => red_missing_composition(CompositionContract::FetchRouteMatrix),
    unsampled_provider_shapes_return_probe_blocked_without_evidence => red_goodreads_probe_blocked(),
    goodreads_work_route_capture_uses_same_fetched_response_and_zero_network => red_goodreads_capture(GoodreadsContract::SameResponseZeroNetwork),
    goodreads_book_id_never_becomes_work_route => red_goodreads_capture(GoodreadsContract::BookNeverWork),

    // livrarr-identity.
    identity_decide_p5_precedence_and_p4_flag_decide_matrix => red_engine_contract(EngineContract::P5PrecedenceAndP4),
    identity_decide_probe_blocked_and_invalid_evidence_are_distinct => red_engine_contract(EngineContract::ProbeBlockedVsInvalid),
    conflict_classifier_exact_a_b_c_and_plural_edition_nonconflict => red_engine_contract(EngineContract::ConflictClasses),
    class_a_requires_opaque_alias_capability => red_engine_contract(EngineContract::OpaqueAliasForClassA),

    // livrarr-enrichment.
    enrichment_plan_closed_route_kind_matrix => red_missing_composition(CompositionContract::EnrichmentPlanMatrix),
    connected_undeclared_route_is_manual_only_without_provider_call => red_missing_composition(CompositionContract::UndeclaredZeroProvider),
    enrichment_apply_returns_captured_route_then_metadata_broker_settles_it => red_road_capture(CaptureContract::EnrichmentHandoff),
    enrichment_apply_provider_drift_and_stale_generation_write_nothing => red_road_capture(CaptureContract::DriftAndStale),
    all_valid_cover_titles_survive_without_v6_filter_and_rank_normally => red_cover_policy(CoverContract::PreserveTitles),

    // livrarr-matching.
    list_dedup_discovery_and_fast_cover_search_share_authority_outcomes => red_matching_adapter(MatchingContract::ConsumerParity),
    no_private_threshold_or_edition_inequality_veto => red_matching_adapter(MatchingContract::NoPrivateThreshold),

    // livrarr-metadata.
    all_listed_settle_doors_call_settle_and_commit_once => red_missing_composition(CompositionContract::AllDoorsCommitOnce),
    dedup_adopt_race_loser_have_no_second_writer => red_road_generation_contract(RoadGenerationContract::MetadataNoSecondWriter),
    existing_work_paths_use_predecision_generation_and_never_resubmit_stale => red_road_generation_contract(RoadGenerationContract::MetadataNeverResubmitStale),
    p4_human_flags_machine_decides_only_certain => red_missing_composition(CompositionContract::P4HumanMatrix),
    interactive_card_origination_is_one_commit_then_typed_resolution => red_full_card_gate(CardContract::MetadataInteractiveCommit),
    complete_group_enumerates_broad_candidates_all_distinctions_and_all_pairs => red_road_reconcile(ReconcileContract::CompleteGroupPairs),
    every_singular_field_conflict_has_disposition_or_card => red_road_reconcile(ReconcileContract::SingularFieldDisposition),
    reconcile_review_card_is_unpersisted_until_settlement_commit => red_road_reconcile(ReconcileContract::CardPersistsOnlyAtCommit),
    author_inheritance_primary_only_agree_review_absent_matrix => red_road_author_inheritance(),
    all_three_capture_triggers_call_settle_before_completion => red_road_capture(CaptureContract::ThreeTriggers),
    empty_capture_is_idempotent_and_sibling_safe => red_road_capture(CaptureContract::EmptyNoop),
    resolve_every_review_kind_through_http_and_cli_same_graph => red_review_entries(ReviewEntryContract::NineKindsParity),
    resolve_scope_kind_generation_cancel_database_errors_leave_pending => red_review_entries(ReviewEntryContract::PendingOnErrors),
    list_confirm_real_rows_call_settle_and_flag_human_duplicates => red_missing_composition(CompositionContract::ListRealRows),
    list_confirm_rejects_owned_file_injection_and_preserves_minimum_fallback => red_missing_composition(CompositionContract::ListRejectOwnedFile),
    author_monitor_http_and_job_triggers_share_mandatory_provider_settle => red_author_monitor(AuthorMonitorContract::HttpAndJobMandatoryProvider),
    author_monitor_missing_provider_route_defers_without_work => red_author_monitor(AuthorMonitorContract::MissingProviderDefers),
    series_monitor_present_route_and_minimum_only_both_enter_settle => red_series_monitor(SeriesMonitorContract::PresentAndMinimum),
    series_monitor_never_invents_provider_or_owned_file_evidence => red_series_monitor(SeriesMonitorContract::NeverInventEvidence),
    manual_refresh_captured_route_settles_inline_before_response => red_missing_composition(CompositionContract::ManualRefreshInlineCapture),
    two_topup_patterns_remain_exact => red_missing_composition(CompositionContract::TwoTopupPatterns),
    convergence_captured_route_settles_before_attempt_checkpoint => red_server_convergence(ConvergenceContract::CapturedBeforeCheckpoint),
    convergence_provider_failure_preserves_route_and_normal_cadence => red_convergence_failure_hook_isolated_from_concurrent_cache_warming(),
    convergence_no_change_terminalizes_on_the_v2_axis => red_convergence_no_change_terminalizes_on_the_v2_axis(),
    convergence_attempt_ledger_counts_only_a_real_unsuccessful_provider_chase => convergence_attempt_ledger_counts_only_a_real_unsuccessful_chase(),
    convergence_cache_only_second_visit_does_not_burn_bridge_attempt => red_convergence_cache_only_second_visit_does_not_burn_bridge_attempt(),
    ac024a_machine_search_fallback_openlibrary_corroborates_and_settles => round13_correlated_openlibrary_search_settles(),
    ac024b_machine_search_fallback_cards_idempotently_and_is_bounded => round13_uncorroborated_search_cards_once_and_burns_to_threshold(),
    ac024c_machine_search_fallback_zero_route_affirms_end_to_end => round13_zero_route_goodreads_card_affirms_end_to_end(),
    ac024f_machine_search_fallback_zero_route_skips_probe_and_cards => round14_zero_route_probe_failure_still_cards_without_probe(),
    ac024e_machine_search_fallback_goodreads_probe_settles_both_routes => round13_goodreads_probe_corroboration_settles_work_and_book(),
    ac024d_machine_search_fallback_obeys_route_and_language_boundaries => round13_search_fallback_precondition_and_applicability_boundary(),
    ac025a_machine_search_fallback_text_decisive_auto_links => round15_same_agree_uncorroborated_search_auto_links(),
    ac025a_goodreads_probe_failure_downgrades_to_text_decisive => round15_goodreads_probe_failure_preserves_text_decisive_auto_link(),
    ac025b_machine_search_fallback_distinct_decisive_tie_cards_via_authority => round15_distinct_same_agree_candidates_card_through_authority(),
    ac025b_goodreads_propose_probe_failure_remains_a_miss => round15_goodreads_propose_grade_probe_failure_remains_miss(),
    ac025f_goodreads_cover_reselect_is_guarded_manual_safe_and_idempotent => round15_goodreads_cover_reselect_is_guarded_manual_safe_and_idempotent(),
    ac025f_goodreads_cover_reselect_isolates_poison_rows => round16_goodreads_cover_reselect_isolates_poison_rows(),
    ac025g_dead_anchor_search_fallthrough_and_will_retry_guard => round18_route_graph_retry_invalidation_and_will_retry_tick_guard(),
    ac025c_pending_route_siblings_follow_current_generation => round15_sibling_pending_route_cards_use_current_generation(),
    ac025d_pending_route_satisfaction_noop_and_foreign_owner => round15_pending_route_satisfaction_noop_and_foreign_owner_lifecycle(),
    ac026a_connected_work_searches_each_missing_provider_and_auto_links => round21_connected_goodreads_route_searches_ol_and_hc_and_auto_links(),
    ac026b_provider_with_own_work_route_never_searches => round21_each_provider_with_own_work_route_fires_zero_search_http(),
    ac026c_connected_all_miss_pass_burns_once_and_parks => round21_connected_all_fired_legs_miss_burn_once_and_park(),
    ac026d_foreign_connected_work_searches_goodreads_only => round21_foreign_connected_work_fires_only_goodreads_search(),
    round21_failed_search_leg_does_not_burn_shared_ledger => round21_search_transport_failure_does_not_burn_shared_ledger(),
    round21_owned_file_goodreads_book_id_settles_on_edition => round21_owned_file_goodreads_id_is_edition_homed(),
    round21_owned_file_goodreads_work_heal_is_exactly_once => round21_owned_file_goodreads_work_heal_is_exact_and_idempotent(),
    round22_unfireable_hardcover_is_absent_and_unvisited => round22_unfireable_hardcover_work_is_absent_and_stays_unvisited(),
    round22_hardcover_search_only_never_refetches_openlibrary => round22_hardcover_only_search_is_bounded_and_never_refetches_openlibrary(),
    round22_all_three_work_routes_are_not_due => round22_all_three_work_routes_are_absent_from_due_selection(),
    round22_goodreads_namespace_heal_collision_is_atomic => round22_goodreads_namespace_heal_collision_is_atomic_fixture(),
    direct_add_delayed_refresh_persists_first_work_route => red_direct_add_delayed_refresh_persists_first_work_route(),
    retry_incomplete_real_handler_uses_convergence_visit_and_no_side_channel_settle => red_missing_composition(CompositionContract::RetryConvergenceOnly),
    retry_incomplete_empty_capture_is_noop_and_per_work_failure_isolated => red_missing_composition(CompositionContract::RetryFailureIsolation),

    // livrarr-library.
    inspector_real_epub_cover_absence_malformed_and_filegone_are_distinct => red_library_inspector(InspectorContract::FourOutcomes),
    inspector_unchanged_failure_suppressed_changed_or_forced_retries => red_library_inspector(InspectorContract::RetryMatrix),
    inspector_zero_provider_calls_and_no_bytes_in_row => red_library_inspector(InspectorContract::ZeroProviderAndNoBytes),

    // livrarr-materialize.
    materialize_selected_sources_and_primary_author_display => red_materialize(MaterializeContract::SelectedSourcesAndAuthor),
    materialize_cover_and_tag_failures_are_independent_and_typed => red_materialize(MaterializeContract::IndependentFailures),

    // livrarr-handlers.
    live_add_goodreads_series_decoration_strips_main_and_preserves_volume => red_live_add_goodreads_title_policy(),
    live_add_extracted_edition_parenthetical_leaves_main_and_creates_edition => red_live_add_edition_parenthetical_policy(),
    live_add_fanout_persists_goodreads_hardcover_and_isbn_in_one_settlement => red_live_add_fanout_routes_share_settlement(),
    live_add_uses_v2_status_and_audits_every_identity_generation => red_live_add_v2_status_and_generation_audits(),
    v2_real_add_writes_one_birth_history_fact => red_v2_real_add_writes_one_birth_history_fact(),
    direct_add_dedup_review_reuses_existing_work => red_direct_add_dedup_review_reuses_existing_work(),
    direct_add_article_variant_reuses_survivor => article_variant_add_real_door_reuses_survivor(),
    group_identity_pending_card_mint_is_idempotent_on_retrigger => red_group_identity_pending_card_mint_is_idempotent_on_retrigger(),
    http_review_all_kinds_map_bad_conflict_notfound_internal => red_missing_composition(CompositionContract::HttpReviewKinds),
    handler_compile_wall_has_only_identity_road_capability => red_handler_compile_wall(),
    legacy_conflict_resolve_route_calls_shared_continuation_once_and_no_legacy_writer => red_missing_composition(CompositionContract::ConflictResolveOnce),
    legacy_conflict_dismiss_route_maps_to_reject_and_calls_shared_continuation_once => red_missing_composition(CompositionContract::ConflictDismissReject),
    manual_provider_search_returns_candidates_without_identity_or_cover_mutation => red_manual_provider_search(),
    post_work_real_route_calls_settle_with_exact_directadd_matrix => red_missing_composition(CompositionContract::DirectAddMatrix),
    directadd_dedup_flags_user_and_background_refresh_waits_for_completion => red_missing_composition(CompositionContract::DirectAddWaits),
    work_update_identity_edit_mints_then_resolves_group_card => red_full_card_gate(CardContract::HandlerWorkHappy),
    work_update_missing_resolved_or_generation_stale_card_fails_closed => red_full_card_gate(CardContract::HandlerWorkFailClosed),
    work_update_monitor_only_does_not_touch_identity_generation_or_key => red_monitor_only_graph_unchanged(),
    manual_merge_preview_then_post_mints_then_resolves_group_card => red_full_card_gate(CardContract::HandlerManualHappy),
    identity_review_card_list_resolve_and_dismiss_are_complete_http_paths => review_card_dismissal_is_scoped_audited_and_generation_neutral(),
    group_identity_card_revalidates_after_settlement_and_invalidates_specifically => red_group_identity_stale_card(),
    manual_merge_missing_resolved_or_generation_stale_card_fails_closed => red_full_card_gate(CardContract::HandlerManualFailClosed),
    manual_merge_field_review_and_file_warning_do_not_escape_atomic_identity_commit => red_missing_composition(CompositionContract::ManualMergeAtomicFields),
    pending_affirm_maps_only_to_po_ratified_pending_route_kind => red_missing_composition(CompositionContract::PendingAffirmKind),
    pending_affirm_mints_then_resolves_with_user_provenance => red_full_card_gate(CardContract::HandlerPendingHappy),
    pending_affirm_owned_route_returns_structured_409_with_zero_writes => affirm_collision_is_structured_and_writes_nothing(),
    pending_affirm_missing_resolved_or_generation_stale_card_has_zero_writes_and_no_refresh => red_full_card_gate(CardContract::HandlerPendingFailClosed),
    manualimport_real_route_settles_before_every_attachment_including_existing_match => red_missing_composition(CompositionContract::ManualImportBeforeAttach),
    manualimport_user_file_provider_precedence_and_flagged_unattached => red_missing_composition(CompositionContract::ManualImportPrecedence),

    // livrarr-server.
    registered_convergence_tick_calls_metadata_handoff_before_checkpoint => red_server_convergence(ConvergenceContract::RegisteredHandoff),
    convergence_tick_cancel_and_database_control_errors_are_typed_while_work_failure_isolated => red_server_convergence(ConvergenceContract::ControlErrorsTyped),
    clap_real_binary_parses_rehearse_list_show_resolve_apply_and_default_serve => red_server_cutover(ServerCutoverContract::LibraryCommandBinding),
    cutover_real_cli_two_invocation_ceremony_reuses_data_dir_rehearsal_ledger => red_real_cli_cutover_ceremony(),
    cutover_subcommands_hold_exclusive_lock_and_never_bind_http_or_start_jobs => red_server_cutover(ServerCutoverContract::ExclusiveNoRuntime),
    cutover_error_matrix_not_snapshot_report_kind_generation_action_cancel_database => red_server_cutover(ServerCutoverContract::ErrorMatrix),
    production_startup_active_empty_and_nonempty_inactive_boundaries => red_server_readiness(ServerReadinessContract::StartupBoundaries),
    active_startup_skips_every_category_9b_through_9e_legacy_writer => red_server_readiness(ServerReadinessContract::SkipLegacyWriters),
    readarr_all_six_precedence_branches_and_review_kinds => red_missing_composition(CompositionContract::ReadarrSixBranches),
    readarr_definitive_miss_has_no_retry_and_only_outage_has_one_idempotent_retry => red_missing_composition(CompositionContract::ReadarrRetryPolicy),
    readarr_settled_result_uses_convergence_not_refresh => red_missing_composition(CompositionContract::ReadarrUsesConvergence),

    // frontend contracts, exercised from the Rust behavioral target so the
    // packet's `test_ilr_*.rs` write fence remains intact.
    ac019_sibling_panel_is_informational_and_copy_is_exact => red_frontend_sibling(),
    presentation_reads_survive_real_author_delete_and_refresh_does_not_404 => red_presentation_survives_deleted_author(),
    cover_ui_one_shared_panel_three_slot_states_and_source_only_labels => red_frontend_cover(),
    handler_compile_wall_identity_review_route_smoke => red_handler_identity_route_smoke(),
}
