//! RED-first behavioral coverage for identity-review-fixes U7 (REQ-007 / AC-007).
//!
//! This is a standalone `[[test]]` target. HTTP cases traverse the production
//! router and authentication middleware, all state is a real `SqliteDb` from
//! `create_activated_test_db()` (the deployed post-activation shape), every
//! card case drives a production writer, and CLI codec cases invoke both the
//! production command function and the real `livrarr` binary. A failing
//! scripted transport proves that no U7 case crosses a provider HTTP boundary.
//!
//! U5 boundary: this file constructs no standing dismissal, dismissal ledger,
//! tombstone, revoke, adoption, origin-suppression matrix, or upgrade fixture.
//! Every replay asserted here is `Minted` or `ReusedPending`; the third helper
//! result is named only by the compile surface and remains AC-005 coverage.

use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use chrono::Utc;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_activated_test_db;
use livrarr_db::{
    AuthorDb, AuthorLinkDb, CreateAuthorDbRequest, CreateAuthorGateRequest, CreateUserDbRequest,
    CreateWorkDbRequest, RootFolderDb, UserDb, WorkDbCreate,
};
use livrarr_domain::identity::{
    AnchorType, ConflictResolutionAction, ConflictSource, IdentityConflictKind,
    IncomingConflictPayload, NewIdentityConflict,
};
use livrarr_domain::identity_layer::{self as ilr};
use livrarr_domain::identity_layer::{
    EditionRepository, IdentityRoadService, WorkIdentityRepository,
};
use livrarr_domain::services::{IdentityConflictService, WorkIdentityRepository as _};
use livrarr_domain::{AuthorLinkTrigger, AuthorNameSource, MediaType, UserRole};
use livrarr_server::auth_crypto::{AuthCryptoService, RealAuthCrypto};
use livrarr_server::state::AppState;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

static CASE_ID: AtomicU64 = AtomicU64::new(1);
static LIVRARR_BINARY: OnceLock<PathBuf> = OnceLock::new();
static U7_TRACE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct RouteHarness {
    app: Router,
    state: AppState,
    api_key: String,
    db: SqliteDb,
    user_id: i64,
    provider_attempts: Arc<AtomicU64>,
    _tmp: tempfile::TempDir,
}

struct RouteResponse {
    status: StatusCode,
    json: Value,
}

async fn build_route_harness() -> RouteHarness {
    // Binding U7 rule: every fixture uses the live post-activation schema,
    // including `idx_works_identity_v2` and no legacy Work identity index.
    let db = create_activated_test_db().await;
    let tmp = tempfile::tempdir().expect("U7 route harness tempdir");
    let data_dir = tmp.path().to_path_buf();
    let data_dir_arc = Arc::new(data_dir.clone());

    let api_key = "irf-u7-admin-api-key".to_string();
    let api_key_hash = RealAuthCrypto
        .hash_token(&api_key)
        .await
        .expect("hash U7 API key");
    let user = db
        .create_user(CreateUserDbRequest {
            username: format!("irf-u7-admin-{}", CASE_ID.fetch_add(1, Ordering::Relaxed)),
            password_hash: "unused-password-hash".to_string(),
            role: UserRole::Admin,
            api_key_hash,
        })
        .await
        .expect("create authenticated U7 user");

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
    let provider_attempts = Arc::new(AtomicU64::new(0));
    let attempted = provider_attempts.clone();
    let http_fetcher = livrarr_http::fetcher::HttpFetcherImpl::new()
        .expect("shared HTTP fetcher")
        .with_scripted_transport(move |_| {
            attempted.fetch_add(1, Ordering::SeqCst);
            livrarr_http::fetcher::ScriptedTransportOutcome::Error {
                delay: Duration::ZERO,
                error: livrarr_domain::services::FetchError::Connection(
                    "U7 forbids provider HTTP".to_string(),
                ),
            }
        });
    let llm_http_client = livrarr_http::HttpClient::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(&user_agent)
        .build()
        .expect("LLM HTTP client");
    let live_metadata_config =
        livrarr_external_data::live_config::LiveMetadataConfig::new(livrarr_db::MetadataConfig {
            hardcover_enabled: false,
            hardcover_api_token: None,
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

    // Empty maps/queues are production implementations with no registered
    // provider. The scripted exchange above makes an accidental crossing
    // observable rather than relying on an environmental network failure.
    let identity_resolver_arc = livrarr_server::state::build_live_identity_resolver(
        std::collections::HashMap::new(),
        transport_cache,
        livrarr_metadata::english_identity_resolver::ResolverConfig::default(),
    );
    let db_arc = Arc::new(db.clone());
    let queue = Arc::new(
        livrarr_metadata::DefaultProviderQueueBuilder::new()
            .with_identity_route_dispatch()
            .build(db_arc.clone()),
    );
    let enrichment_service = Arc::new(livrarr_metadata::EnrichmentServiceImpl::new(
        db_arc,
        queue.clone(),
        Arc::new(livrarr_metadata::DefaultMergeEngine::new(
            livrarr_metadata::PriorityModel::english(),
        )),
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
    let discovery_service_arc = Arc::new(
        livrarr_metadata::discovery_service::DiscoveryServiceImpl::new(
            db.clone(),
            http_fetcher.clone(),
            livrarr_external_data::llm_caller_service::LlmCallerImpl::new(
                live_metadata_config.clone(),
                llm_http_client.clone(),
            ),
        )
        .with_resolver(identity_resolver_arc.clone()),
    );
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
            .with_identity_road(identity_road_arc),
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
        provider_attempts,
        _tmp: tmp,
    }
}

async fn call_router_json(
    harness: &RouteHarness,
    method: Method,
    path: impl Into<String>,
    body: Option<Value>,
) -> RouteResponse {
    let mut request = Request::builder()
        .method(method)
        .uri(path.into())
        .header("x-api-key", &harness.api_key);
    let request_body = match body {
        Some(value) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let mut request = request.body(request_body).expect("build U7 request");
    request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 77)),
        31_001,
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
        .expect("read response body");
    let json = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({"unparsedBody": String::from_utf8_lossy(&bytes).into_owned()}));
    RouteResponse { status, json }
}

fn assert_no_provider_http(harness: &RouteHarness) {
    assert_eq!(
        harness.provider_attempts.load(Ordering::SeqCst),
        0,
        "U7 is persistence/UI/codec work and must not invoke provider HTTP",
    );
}

async fn seed_user(db: &SqliteDb, label: &str) -> i64 {
    let n = CASE_ID.fetch_add(1, Ordering::Relaxed);
    db.create_user(CreateUserDbRequest {
        username: format!("irf-u7-{label}-{n}"),
        password_hash: "unused".to_string(),
        role: UserRole::Admin,
        api_key_hash: format!("unused-u7-{label}-{n}"),
    })
    .await
    .expect("seed U7 user")
    .id
}

async fn seed_author(db: &SqliteDb, user_id: i64, label: &str) -> i64 {
    let n = CASE_ID.fetch_add(1, Ordering::Relaxed);
    let name = format!("U7 {label} Author {n}");
    let (author, created) = AuthorLinkDb::create_or_adopt_author(
        db,
        CreateAuthorGateRequest {
            user_id,
            name,
            sort_name: None,
            import_id: None,
            initial_name_source: AuthorNameSource::User,
            trigger: AuthorLinkTrigger::AuthorCreated,
        },
    )
    .await
    .expect("seed Author through production create/adopt gate");
    assert!(created, "U7 fixture Author must be new");
    author.id
}

async fn seed_work(
    db: &SqliteDb,
    user_id: i64,
    author_id: i64,
    label: &str,
) -> ilr::CapturedIdentity {
    let n = CASE_ID.fetch_add(1, Ordering::Relaxed);
    let title = format!("U7 {label} {n}");
    let mut identity_title =
        ilr::title_parts_from_provider(title, None).expect("valid U7 fixture title");
    identity_title.provenance = ilr::EvidenceProvenance::User;
    let committed = WorkIdentityRepository::commit_settlement(
        db,
        ilr::SettlementCommit {
            user_id,
            existing_work_id: None,
            add_source: None,
            identity_title,
            text_distinction: None,
            contributors: vec![ilr::WorkContributor {
                user_id,
                work_id: 0,
                author_id,
                ordinal: 0,
                roles: vec![],
            }],
            routes: vec![],
            absorbed_work_ids: vec![],
            expected_generation: 0,
            review_cards: vec![],
        },
    )
    .await
    .expect("seed Work through the sole production settlement writer");
    assert_live_identity_pair(db, user_id, committed.identity.own_work_id, author_id).await;
    committed.identity
}

async fn assert_live_identity_pair(db: &SqliteDb, user_id: i64, work_id: i64, author_id: i64) {
    let row: (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        i64,
    ) = sqlx::query_as(
        "SELECT normalized_identity_main, normalized_identity_subtitle, \
                    normalized_identity_volume, primary_author_id, identity_generation \
               FROM works WHERE user_id=?1 AND id=?2",
    )
    .bind(user_id)
    .bind(work_id)
    .fetch_one(db.pool())
    .await
    .expect("read deployed identity pair");
    assert!(row
        .0
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty()));
    assert!(row.1.is_some(), "deployed shape stores normalized subtitle");
    assert!(row.2.is_some(), "deployed shape stores normalized volume");
    assert_eq!(row.3, Some(author_id));
    assert!(row.4 > 0);
    let legacy_indexes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' \
           AND name IN ('idx_works_user_normalized','idx_works_test_helper_creation_dedup')",
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect deployed index shape");
    assert_eq!(
        legacy_indexes, 0,
        "fixture must be post-activation, not compatibility shape"
    );
}

fn pending_candidate(
    work_id: i64,
    provider: ilr::IdentityProvider,
    kind: ilr::RouteKind,
    value: &str,
    owner: ilr::RouteOwner,
) -> ilr::SettlementReviewCard {
    ilr::SettlementReviewCard::PendingRoute {
        work_id,
        candidate: ilr::ParkedRouteCandidate {
            route: ilr::RouteKey {
                provider,
                kind,
                value: value.to_string(),
            },
            proposed_owner: owner,
        },
    }
}

fn settlement_with_card(
    captured: &ilr::CapturedIdentity,
    user_id: i64,
    card: ilr::SettlementReviewCard,
) -> ilr::SettlementCommit {
    ilr::SettlementCommit {
        user_id,
        existing_work_id: Some(captured.own_work_id),
        add_source: None,
        identity_title: captured.identity_title.clone(),
        text_distinction: (captured.text_distinction != "common")
            .then(|| captured.text_distinction.clone()),
        contributors: vec![ilr::WorkContributor {
            user_id,
            work_id: captured.own_work_id,
            author_id: captured.primary_author_id,
            ordinal: 0,
            roles: vec![],
        }],
        routes: captured.active_routes.clone(),
        absorbed_work_ids: vec![],
        expected_generation: captured.identity_generation,
        review_cards: vec![card],
    }
}

async fn refresh_identity(db: &SqliteDb, user_id: i64, work_id: i64) -> ilr::CapturedIdentity {
    WorkIdentityRepository::read_captured_identity(db, user_id, work_id)
        .await
        .expect("refresh U7 captured identity")
}

async fn pending_ids(db: &SqliteDb, user_id: i64, kind: ilr::ReviewKind) -> Vec<i64> {
    sqlx::query_scalar(
        "SELECT id FROM identity_review_cards \
          WHERE user_id=?1 AND kind=?2 AND status='pending' ORDER BY id",
    )
    .bind(user_id)
    .bind(kind.storage_code())
    .fetch_all(db.pool())
    .await
    .expect("list pending U7 cards")
}

async fn notification_rows(db: &SqliteDb, user_id: i64) -> Vec<(i64, String, String)> {
    sqlx::query_as("SELECT id, type, ref_key FROM notifications WHERE user_id=?1 ORDER BY id")
        .bind(user_id)
        .fetch_all(db.pool())
        .await
        .expect("list U7 notifications")
}

fn clear_mint_trace() {
    livrarr_db::identity_layer::clear_review_card_mint_observations_for_tests();
}

fn take_mint_trace() -> Vec<livrarr_db::identity_layer::ReviewCardMintObservation> {
    livrarr_db::identity_layer::take_review_card_mint_observations_for_tests()
}

fn assert_trace_outcome(
    observation: &livrarr_db::identity_layer::ReviewCardMintObservation,
    site: livrarr_db::identity_layer::ReviewCardMintSite,
    kind: ilr::ReviewKind,
    outcome: &str,
    card_id: i64,
) {
    use livrarr_db::identity_layer::ReviewCardMintOutcome;
    assert_eq!(observation.site, site);
    assert_eq!(observation.kind, kind);
    match (&observation.outcome, outcome) {
        (ReviewCardMintOutcome::Minted(card), "Minted")
        | (ReviewCardMintOutcome::ReusedPending(card), "ReusedPending") => {
            assert_eq!(card.id, card_id)
        }
        (ReviewCardMintOutcome::SuppressedByDismissal, "SuppressedByDismissal") => {
            panic!("U5 boundary violation: U7 must never exercise SuppressedByDismissal")
        }
        (actual, expected) => panic!("expected {expected} for card {card_id}, got {actual:?}"),
    }
}

// RED-UNTIL-U7: today generic settlement has a local GroupIdentity-only pending scan, inserts PendingRoute duplicates, and emits no notification for either kind instead of invoking the one authority.
#[tokio::test]
async fn generic_settlement_uses_req005_keys_for_both_runtime_kinds() {
    let _serial = U7_TRACE_LOCK.lock().await;
    let db = create_activated_test_db().await;
    let user_id = seed_user(&db, "generic").await;
    let author_id = seed_author(&db, user_id, "generic").await;
    let mut anchor = seed_work(&db, user_id, author_id, "generic anchor").await;
    let peer = seed_work(&db, user_id, author_id, "generic peer").await;

    // Constructed-state compatibility justification: AC-007 names generic
    // settlement itself as a production mint writer. Supplying its typed card
    // draft directly is the only deterministic way to vary every semantic-key
    // element without inventing a door; the real SqliteDb transaction and
    // production writer remain under test.
    clear_mint_trace();

    let route_value = format!(
        "OL-U7-GENERIC-{}-W",
        CASE_ID.fetch_add(1, Ordering::Relaxed)
    );
    let first_pending = pending_candidate(
        anchor.own_work_id,
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        &format!("  {route_value}  "),
        ilr::RouteOwner::Work(anchor.own_work_id),
    );
    let first = WorkIdentityRepository::commit_settlement(
        &db,
        settlement_with_card(&anchor, user_id, first_pending),
    )
    .await
    .expect("generic settlement mints PendingRoute");
    let pending_id = first.review_cards[0].id;
    anchor = first.identity;

    let owner_free_trimmed_replay = pending_candidate(
        anchor.own_work_id,
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        &route_value,
        ilr::RouteOwner::Edition(9_000_001),
    );
    let replay = WorkIdentityRepository::commit_settlement(
        &db,
        settlement_with_card(&anchor, user_id, owner_free_trimmed_replay),
    )
    .await
    .expect("generic settlement reuses owner-free trimmed PendingRoute key");
    assert_eq!(replay.review_cards[0].id, pending_id);
    anchor = replay.identity;

    let changed_pending = pending_candidate(
        anchor.own_work_id,
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        &format!("{route_value}-CHANGED"),
        ilr::RouteOwner::Work(anchor.own_work_id),
    );
    let changed = WorkIdentityRepository::commit_settlement(
        &db,
        settlement_with_card(&anchor, user_id, changed_pending),
    )
    .await
    .expect("changed PendingRoute value mints");
    let changed_pending_id = changed.review_cards[0].id;
    assert_ne!(changed_pending_id, pending_id);
    anchor = changed.identity;
    assert_eq!(
        pending_ids(&db, user_id, ilr::ReviewKind::PendingRoute).await,
        vec![pending_id, changed_pending_id],
    );
    let pending_notifications = notification_rows(&db, user_id).await;
    assert_eq!(pending_notifications.len(), 2);
    for (card_id, row) in [pending_id, changed_pending_id]
        .into_iter()
        .zip(&pending_notifications)
    {
        assert_eq!(row.1, "identityReviewNeeded");
        assert_eq!(row.2, format!("identity-review-card:{card_id}"));
    }

    let proposed_route = ilr::WorkRoute {
        id: 0,
        user_id,
        owner: ilr::RouteOwner::Work(anchor.own_work_id),
        resolved_work_id: anchor.own_work_id,
        provider: ilr::IdentityProvider::Goodreads,
        kind: ilr::RouteKind::GoodreadsWork,
        provider_scoped_id: "U7-GROUP-SEMANTIC-ROUTE".to_string(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::Provider(ilr::IdentityProvider::Goodreads),
        user_confirmed: false,
        observed_at: Utc::now(),
    };
    let proposed = ilr::WorkIdentityEvidence {
        title: anchor.identity_title.clone(),
        primary_author_id: author_id,
        routes: vec![proposed_route.clone()],
    };
    let group = ilr::SettlementReviewCard::GroupIdentity {
        work_ids: vec![peer.own_work_id, anchor.own_work_id],
        proposed_identity: Some(proposed.clone()),
        merge_choices: vec![],
    };
    let first_group = WorkIdentityRepository::commit_settlement(
        &db,
        settlement_with_card(&anchor, user_id, group),
    )
    .await
    .expect("generic settlement mints GroupIdentity");
    let group_id = first_group.review_cards[0].id;
    anchor = first_group.identity;

    let mut decorated_route = proposed_route;
    decorated_route.id = 71;
    decorated_route.owner = ilr::RouteOwner::Edition(8_000_001);
    decorated_route.resolved_work_id = peer.own_work_id;
    decorated_route.provenance = ilr::RouteProvenance::Migrated {
        legacy_field: "decoration-not-key".to_string(),
    };
    decorated_route.user_confirmed = true;
    decorated_route.observed_at = Utc::now() + chrono::Duration::hours(1);
    let reordered = ilr::SettlementReviewCard::GroupIdentity {
        work_ids: vec![anchor.own_work_id, peer.own_work_id],
        proposed_identity: Some(ilr::WorkIdentityEvidence {
            title: proposed.title.clone(),
            primary_author_id: author_id,
            routes: vec![decorated_route],
        }),
        merge_choices: vec![],
    };
    let group_replay = WorkIdentityRepository::commit_settlement(
        &db,
        settlement_with_card(&anchor, user_id, reordered),
    )
    .await
    .expect("insight-92 semantic GroupIdentity key reuses pending card");
    assert_eq!(group_replay.review_cards[0].id, group_id);
    anchor = group_replay.identity;

    let mut changed_title = proposed.title;
    changed_title.subtitle = Some("Changed semantic subtitle".to_string());
    changed_title.normalized_subtitle = "changed semantic subtitle".to_string();
    let changed_group = ilr::SettlementReviewCard::GroupIdentity {
        work_ids: vec![peer.own_work_id, anchor.own_work_id],
        proposed_identity: Some(ilr::WorkIdentityEvidence {
            title: changed_title,
            primary_author_id: author_id,
            routes: proposed.routes,
        }),
        merge_choices: vec![],
    };
    let group_changed = WorkIdentityRepository::commit_settlement(
        &db,
        settlement_with_card(&anchor, user_id, changed_group),
    )
    .await
    .expect("changed GroupIdentity semantic key mints");
    let changed_group_id = group_changed.review_cards[0].id;
    assert_ne!(changed_group_id, group_id);
    assert_eq!(
        pending_ids(&db, user_id, ilr::ReviewKind::GroupIdentity).await,
        vec![group_id, changed_group_id],
    );
    assert_eq!(
        notification_rows(&db, user_id).await,
        pending_notifications,
        "GroupIdentity must remain unnotified",
    );

    let trace = take_mint_trace();
    assert_eq!(trace.len(), 6, "each proposal traverses the one helper");
    use livrarr_db::identity_layer::ReviewCardMintSite::GenericSettlement;
    assert_trace_outcome(
        &trace[0],
        GenericSettlement,
        ilr::ReviewKind::PendingRoute,
        "Minted",
        pending_id,
    );
    assert_trace_outcome(
        &trace[1],
        GenericSettlement,
        ilr::ReviewKind::PendingRoute,
        "ReusedPending",
        pending_id,
    );
    assert_trace_outcome(
        &trace[2],
        GenericSettlement,
        ilr::ReviewKind::PendingRoute,
        "Minted",
        changed_pending_id,
    );
    assert_trace_outcome(
        &trace[3],
        GenericSettlement,
        ilr::ReviewKind::GroupIdentity,
        "Minted",
        group_id,
    );
    assert_trace_outcome(
        &trace[4],
        GenericSettlement,
        ilr::ReviewKind::GroupIdentity,
        "ReusedPending",
        group_id,
    );
    assert_trace_outcome(
        &trace[5],
        GenericSettlement,
        ilr::ReviewKind::GroupIdentity,
        "Minted",
        changed_group_id,
    );
}

// RED-UNTIL-U7: today generic PendingRoute settlement emits no notification, so a notification abort never fires and the card commits instead of rolling back with its notification.
#[tokio::test]
async fn pending_route_card_and_notification_roll_back_as_one_transaction() {
    let _serial = U7_TRACE_LOCK.lock().await;
    let db = create_activated_test_db().await;
    let user_id = seed_user(&db, "notification-atomicity").await;
    let author_id = seed_author(&db, user_id, "notification-atomicity").await;
    let captured = seed_work(&db, user_id, author_id, "notification atomicity").await;

    // Constructed-state compatibility justification: this SQLite trigger is
    // a failure probe, not a replacement writer or lock. It is the smallest
    // deterministic way to distinguish one card+notification transaction
    // from a card commit followed by a separate notification write; the real
    // SqliteDb generic settlement writer remains the operation under test.
    sqlx::query(
        "CREATE TRIGGER u7_abort_identity_review_notification \
         BEFORE INSERT ON notifications \
         WHEN NEW.type='identityReviewNeeded' \
         BEGIN SELECT RAISE(ABORT, 'U7 notification transaction probe'); END",
    )
    .execute(db.pool())
    .await
    .expect("install U7 notification transaction probe");
    let before_generation = captured.identity_generation;
    clear_mint_trace();

    let result = WorkIdentityRepository::commit_settlement(
        &db,
        settlement_with_card(
            &captured,
            user_id,
            pending_candidate(
                captured.own_work_id,
                ilr::IdentityProvider::OpenLibrary,
                ilr::RouteKind::OpenLibraryWork,
                &format!("OL-U7-ATOMIC-{}-W", CASE_ID.fetch_add(1, Ordering::Relaxed)),
                ilr::RouteOwner::Work(captured.own_work_id),
            ),
        ),
    )
    .await;
    assert!(
        result.is_err(),
        "notification failure must abort settlement"
    );
    assert!(pending_ids(&db, user_id, ilr::ReviewKind::PendingRoute)
        .await
        .is_empty());
    assert!(notification_rows(&db, user_id).await.is_empty());
    assert_eq!(
        refresh_identity(&db, user_id, captured.own_work_id)
            .await
            .identity_generation,
        before_generation,
        "card, notification, and settlement mutation roll back together",
    );
}

// RED-UNTIL-U7: today captured-route handoff is the one site with correct local PendingRoute reuse plus one identityReviewNeeded insert, but it does not delegate either decision to the shared authority.
#[tokio::test]
async fn captured_route_handoff_uses_owner_free_trimmed_key_and_one_notification() {
    let _serial = U7_TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "captured").await;
    let captured = seed_work(&harness.db, harness.user_id, author_id, "captured route").await;
    clear_mint_trace();

    let value = format!(
        "OL-U7-CAPTURED-{}-W",
        CASE_ID.fetch_add(1, Ordering::Relaxed)
    );
    let handoff = |route_value: String| ilr::CapturedRouteHandoff {
        metadata_generation: captured.identity_generation,
        provider_identity: vec![],
        route_proposals: vec![ilr::RouteKey {
            provider: ilr::IdentityProvider::OpenLibrary,
            kind: ilr::RouteKind::OpenLibraryWork,
            value: route_value,
        }],
    };
    let first = harness
        .state
        .identity_road
        .apply_captured_route_handoff(
            harness.user_id,
            captured.own_work_id,
            ilr::IdentityRoadOrigin::ConvergenceVisit,
            handoff(format!("  {value}  ")),
        )
        .await
        .expect("captured-route handoff mints")
        .expect("proposal handoff returns a review");
    let first_id = match first {
        ilr::IdentityRoadOutcome::ReviewPending { review_id, .. } => review_id,
        other => panic!("expected captured PendingRoute review, got {other:?}"),
    };

    let replay = harness
        .state
        .identity_road
        .apply_captured_route_handoff(
            harness.user_id,
            captured.own_work_id,
            ilr::IdentityRoadOrigin::ConvergenceVisit,
            handoff(value.clone()),
        )
        .await
        .expect("captured-route replay succeeds")
        .expect("replay returns reused review");
    assert!(matches!(
        replay,
        ilr::IdentityRoadOutcome::ReviewPending { review_id, .. } if review_id == first_id
    ));

    let changed = harness
        .state
        .identity_road
        .apply_captured_route_handoff(
            harness.user_id,
            captured.own_work_id,
            ilr::IdentityRoadOrigin::ConvergenceVisit,
            handoff(format!("{value}-CHANGED")),
        )
        .await
        .expect("changed captured route succeeds")
        .expect("changed route returns review");
    let changed_id = match changed {
        ilr::IdentityRoadOutcome::ReviewPending { review_id, .. } => review_id,
        other => panic!("expected changed PendingRoute review, got {other:?}"),
    };
    assert_ne!(changed_id, first_id);
    assert_eq!(
        pending_ids(&harness.db, harness.user_id, ilr::ReviewKind::PendingRoute).await,
        vec![first_id, changed_id],
    );
    let notifications = notification_rows(&harness.db, harness.user_id).await;
    assert_eq!(notifications.len(), 2, "reuse emits no second notification");
    assert_eq!(
        notifications[0].2,
        format!("identity-review-card:{first_id}")
    );
    assert_eq!(
        notifications[1].2,
        format!("identity-review-card:{changed_id}")
    );

    let trace = take_mint_trace();
    assert_eq!(trace.len(), 3);
    use livrarr_db::identity_layer::ReviewCardMintSite::CapturedRouteHandoff;
    assert_trace_outcome(
        &trace[0],
        CapturedRouteHandoff,
        ilr::ReviewKind::PendingRoute,
        "Minted",
        first_id,
    );
    assert_trace_outcome(
        &trace[1],
        CapturedRouteHandoff,
        ilr::ReviewKind::PendingRoute,
        "ReusedPending",
        first_id,
    );
    assert_trace_outcome(
        &trace[2],
        CapturedRouteHandoff,
        ilr::ReviewKind::PendingRoute,
        "Minted",
        changed_id,
    );
    assert_no_provider_http(&harness);
}

async fn edition_card_ids(db: &SqliteDb, user_id: i64, edition_id: i64) -> Vec<i64> {
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, payload FROM identity_review_cards \
          WHERE user_id=?1 AND kind='EditionEvidence' AND status='pending' ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(db.pool())
    .await
    .expect("read EditionEvidence cards");
    rows.into_iter()
        .filter_map(|(id, payload)| {
            let decoded: ilr::SettlementReviewCard = serde_json::from_str(&payload).ok()?;
            matches!(
                decoded,
                ilr::SettlementReviewCard::EditionEvidence {
                    edition_id: observed,
                    ..
                } if observed == edition_id
            )
            .then_some(id)
        })
        .collect()
}

async fn apply_direct_edition_conflict(db: &SqliteDb, user_id: i64, edition_id: i64) {
    let result = EditionRepository::apply_evidence(
        db,
        ilr::EditionEvidenceCommand {
            user_id,
            edition_id,
            format: Some(ilr::EditionFormat::Audiobook),
            language: Some("fr".to_string()),
            provenance: ilr::EvidenceProvenance::OwnedFile,
        },
    )
    .await;
    assert!(matches!(
        result,
        Err(ilr::EditionRepositoryError::ContradictoryEvidenceParked)
    ));
}

fn write_edition_epub(path: &Path, title: &str, language: &str) {
    let file = std::fs::File::create(path).expect("create U7 Edition EPUB");
    let mut zip = zip::ZipWriter::new(file);
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("mimetype", stored).expect("EPUB mimetype");
    zip.write_all(b"application/epub+zip")
        .expect("write EPUB mimetype");
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("META-INF/container.xml", deflated)
        .expect("EPUB container");
    zip.write_all(br#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#)
        .expect("write EPUB container");
    zip.start_file("OEBPS/content.opf", deflated)
        .expect("EPUB OPF");
    zip.write_all(
        format!(r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>{title}</dc:title><dc:language>{language}</dc:language></metadata><manifest/><spine/></package>"#).as_bytes(),
    )
    .expect("write EPUB OPF");
    zip.finish().expect("finish U7 Edition EPUB");
}

async fn drive_manual_import_edition_evidence(
    harness: &RouteHarness,
    path: &Path,
    title: &str,
    author: &str,
    language: &str,
) -> RouteResponse {
    write_edition_epub(path, title, language);
    call_router_json(
        harness,
        Method::POST,
        "/api/v1/manualimport/import",
        Some(json!({"items": [{
            "path": path,
            "olKey": "",
            "title": title,
            "author": author,
            "deleteExisting": false,
            "language": language,
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
        }]})),
    )
    .await
}

fn assert_manual_import_result(response: &RouteResponse, status: &str) -> i64 {
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    let result = &response.json["results"][0];
    assert_eq!(result["status"], status, "{}", response.json);
    result["workId"]
        .as_i64()
        .unwrap_or_else(|| panic!("ManualImport result has no Work id: {}", response.json))
}

// RED-UNTIL-U7: today apply_evidence and apply_work_evidence insert EditionEvidence with no pending-card check, so same-(user, edition) duplicates accumulate and neither writer invokes the authority.
#[tokio::test]
async fn both_edition_writers_key_on_user_and_edition_not_generation() {
    let _serial = U7_TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "edition").await;

    // Constructed-state compatibility justification: `apply_evidence` has no
    // live caller in the v11 tree; AC-007 expressly requires driving this
    // repository method as the compatibility fixture, and the method remains
    // a runtime-capable production writer.
    let direct_work = seed_work(&harness.db, harness.user_id, author_id, "direct edition").await;
    let direct_edition = EditionRepository::apply_work_evidence(
        &harness.db,
        ilr::EditionWorkEvidenceCommand {
            user_id: harness.user_id,
            work_id: direct_work.own_work_id,
            format: ilr::EditionFormat::Ebook,
            language: Some("en".to_string()),
            provenance: ilr::EvidenceProvenance::OwnedFile,
        },
    )
    .await
    .expect("seed direct-writer Edition")
    .edition;

    clear_mint_trace();
    apply_direct_edition_conflict(&harness.db, harness.user_id, direct_edition.id).await;
    let direct_id = edition_card_ids(&harness.db, harness.user_id, direct_edition.id).await[0];

    // Bump the owning Work generation through production settlement. The
    // EditionEvidence key is still (user, edition), independent of generation.
    let current = refresh_identity(&harness.db, harness.user_id, direct_work.own_work_id).await;
    let mut bump = settlement_with_card(
        &current,
        harness.user_id,
        ilr::SettlementReviewCard::InvariantRepair {
            work_id: Some(current.own_work_id),
            invariant: "removed before commit; generation-only fixture".to_string(),
        },
    );
    bump.review_cards.clear();
    WorkIdentityRepository::commit_settlement(&harness.db, bump)
        .await
        .expect("bump generation through settlement");
    apply_direct_edition_conflict(&harness.db, harness.user_id, direct_edition.id).await;
    assert_eq!(
        edition_card_ids(&harness.db, harness.user_id, direct_edition.id).await,
        vec![direct_id],
        "same Edition reuses the oldest card after Work generation changes",
    );

    let direct_changed_work = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "direct changed edition",
    )
    .await;
    let direct_changed_edition = EditionRepository::apply_work_evidence(
        &harness.db,
        ilr::EditionWorkEvidenceCommand {
            user_id: harness.user_id,
            work_id: direct_changed_work.own_work_id,
            format: ilr::EditionFormat::Ebook,
            language: Some("en".to_string()),
            provenance: ilr::EvidenceProvenance::OwnedFile,
        },
    )
    .await
    .expect("seed changed direct Edition")
    .edition;
    apply_direct_edition_conflict(&harness.db, harness.user_id, direct_changed_edition.id).await;
    let direct_changed_id =
        edition_card_ids(&harness.db, harness.user_id, direct_changed_edition.id).await[0];
    assert_ne!(direct_changed_id, direct_id, "changed Edition id mints");

    // This half drives apply_work_evidence through the registered ManualImport
    // HTTP door and its production AppState EditionEvidenceCapability adapter.
    let library_root = harness._tmp.path().join("u7-edition-library");
    let incoming = harness._tmp.path().join("u7-edition-incoming");
    std::fs::create_dir_all(&library_root).expect("create U7 Edition library root");
    std::fs::create_dir_all(&incoming).expect("create U7 Edition incoming root");
    harness
        .db
        .create_root_folder(
            library_root.to_str().expect("UTF-8 U7 Edition root"),
            MediaType::Ebook,
        )
        .await
        .expect("register U7 Edition library root");
    let manual_author_id = seed_author(&harness.db, harness.user_id, "manual edition door").await;
    let author_name: String =
        sqlx::query_scalar("SELECT name FROM authors WHERE user_id=?1 AND id=?2")
            .bind(harness.user_id)
            .bind(manual_author_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("read ManualImport Author name");
    let manual_title = format!(
        "U7 Manual Edition {}",
        CASE_ID.fetch_add(1, Ordering::Relaxed)
    );
    let seeded = drive_manual_import_edition_evidence(
        &harness,
        &incoming.join("manual-seed-en.epub"),
        &manual_title,
        &author_name,
        "en",
    )
    .await;
    let manual_work_id = assert_manual_import_result(&seeded, "imported");
    assert_live_identity_pair(
        &harness.db,
        harness.user_id,
        manual_work_id,
        manual_author_id,
    )
    .await;
    let manual_edition_id: i64 = sqlx::query_scalar(
        "SELECT id FROM editions WHERE user_id=?1 AND work_id=?2 AND state='active' ORDER BY id LIMIT 1",
    )
    .bind(harness.user_id)
    .bind(manual_work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read ManualImport Edition");

    let first_conflict = drive_manual_import_edition_evidence(
        &harness,
        &incoming.join("manual-conflict-fr-1.epub"),
        &manual_title,
        &author_name,
        "fr",
    )
    .await;
    assert_eq!(
        assert_manual_import_result(&first_conflict, "failed"),
        manual_work_id
    );
    assert!(first_conflict.json["results"][0]["error"]
        .as_str()
        .is_some_and(|error| error.contains("edition evidence failed")));
    let manual_id = edition_card_ids(&harness.db, harness.user_id, manual_edition_id).await[0];

    let current = refresh_identity(&harness.db, harness.user_id, manual_work_id).await;
    let mut bump = settlement_with_card(
        &current,
        harness.user_id,
        ilr::SettlementReviewCard::InvariantRepair {
            work_id: Some(current.own_work_id),
            invariant: "removed before commit; ManualImport generation-only fixture".to_string(),
        },
    );
    bump.review_cards.clear();
    WorkIdentityRepository::commit_settlement(&harness.db, bump)
        .await
        .expect("bump ManualImport Work generation through settlement");
    let replay = drive_manual_import_edition_evidence(
        &harness,
        &incoming.join("manual-conflict-fr-2.epub"),
        &manual_title,
        &author_name,
        "fr",
    )
    .await;
    assert_eq!(
        assert_manual_import_result(&replay, "failed"),
        manual_work_id
    );
    assert_eq!(
        edition_card_ids(&harness.db, harness.user_id, manual_edition_id).await,
        vec![manual_id],
        "same ManualImport Edition reuses across Work generation changes",
    );

    let changed_title = format!(
        "U7 Polar Clockwork Atlas {}",
        CASE_ID.fetch_add(1, Ordering::Relaxed)
    );
    let changed_seed = drive_manual_import_edition_evidence(
        &harness,
        &incoming.join("manual-changed-seed-en.epub"),
        &changed_title,
        &author_name,
        "en",
    )
    .await;
    let manual_changed_work_id = assert_manual_import_result(&changed_seed, "imported");
    assert_ne!(manual_changed_work_id, manual_work_id);
    assert_live_identity_pair(
        &harness.db,
        harness.user_id,
        manual_changed_work_id,
        manual_author_id,
    )
    .await;
    let manual_changed_edition_id: i64 = sqlx::query_scalar(
        "SELECT id FROM editions WHERE user_id=?1 AND work_id=?2 AND state='active' ORDER BY id LIMIT 1",
    )
    .bind(harness.user_id)
    .bind(manual_changed_work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read changed ManualImport Edition");
    assert_ne!(manual_changed_edition_id, manual_edition_id);
    let changed_conflict = drive_manual_import_edition_evidence(
        &harness,
        &incoming.join("manual-changed-conflict-fr.epub"),
        &changed_title,
        &author_name,
        "fr",
    )
    .await;
    assert_eq!(
        assert_manual_import_result(&changed_conflict, "failed"),
        manual_changed_work_id
    );
    let manual_changed_id =
        edition_card_ids(&harness.db, harness.user_id, manual_changed_edition_id).await[0];
    assert_ne!(manual_changed_id, manual_id);
    assert!(notification_rows(&harness.db, harness.user_id)
        .await
        .is_empty());

    let trace = take_mint_trace();
    assert_eq!(trace.len(), 6);
    use livrarr_db::identity_layer::ReviewCardMintSite::{ApplyEvidence, ApplyWorkEvidence};
    assert_trace_outcome(
        &trace[0],
        ApplyEvidence,
        ilr::ReviewKind::EditionEvidence,
        "Minted",
        direct_id,
    );
    assert_trace_outcome(
        &trace[1],
        ApplyEvidence,
        ilr::ReviewKind::EditionEvidence,
        "ReusedPending",
        direct_id,
    );
    assert_trace_outcome(
        &trace[2],
        ApplyEvidence,
        ilr::ReviewKind::EditionEvidence,
        "Minted",
        direct_changed_id,
    );
    assert_trace_outcome(
        &trace[3],
        ApplyWorkEvidence,
        ilr::ReviewKind::EditionEvidence,
        "Minted",
        manual_id,
    );
    assert_trace_outcome(
        &trace[4],
        ApplyWorkEvidence,
        ilr::ReviewKind::EditionEvidence,
        "ReusedPending",
        manual_id,
    );
    assert_trace_outcome(
        &trace[5],
        ApplyWorkEvidence,
        ilr::ReviewKind::EditionEvidence,
        "Minted",
        manual_changed_id,
    );
    assert_no_provider_http(&harness);
}

async fn seed_title_heal_collision(db: &SqliteDb, user_id: i64) -> (i64, i64, i64) {
    let (author, created) = db
        .create_author(CreateAuthorDbRequest {
            user_id,
            name: format!("U7 Heal Author {}", CASE_ID.fetch_add(1, Ordering::Relaxed)),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed title-heal Author");
    assert!(created);
    let first = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "U7 Cloud Cuckoo Land target".to_string(),
            author_name: author.name.clone(),
            normalized_title: "u7 cloud cuckoo land target".to_string(),
            normalized_author: author.name.to_lowercase(),
            author_id: Some(author.id),
            ..Default::default()
        })
        .await
        .expect("seed title-heal target")
        .0;
    let second = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "U7 Cloud Cuckoo Land legacy".to_string(),
            author_name: author.name,
            normalized_title: "u7 cloud cuckoo land legacy".to_string(),
            normalized_author: "u7 heal author".to_string(),
            author_id: Some(author.id),
            ..Default::default()
        })
        .await
        .expect("seed title-heal source")
        .0;
    sqlx::query(
        "UPDATE works SET title='U7 Cloud Cuckoo Land', subtitle='large print edition', \
                normalized_title='u7 cloud cuckoo land', \
                normalized_identity_main='u7 cloud cuckoo land', \
                normalized_identity_subtitle='large print edition', \
                normalized_identity_volume='', primary_author_id=?1, \
                text_distinction='common', identity_generation=1 \
          WHERE user_id=?2 AND id=?3",
    )
    .bind(author.id)
    .bind(user_id)
    .bind(first.id)
    .execute(db.pool())
    .await
    .expect("shape title-heal collision target");
    sqlx::query(
        "UPDATE works SET title='U7 Cloud Cuckoo Land (Large Print Edition)', subtitle=NULL, \
                normalized_title='u7 cloud cuckoo land (large print edition)', \
                normalized_identity_main='u7 cloud cuckoo land (large print edition)', \
                normalized_identity_subtitle='', normalized_identity_volume='', \
                primary_author_id=?1, text_distinction='common', identity_generation=1 \
          WHERE user_id=?2 AND id=?3",
    )
    .bind(author.id)
    .bind(user_id)
    .bind(second.id)
    .execute(db.pool())
    .await
    .expect("shape title-heal pre-policy source");
    assert_live_identity_pair(db, user_id, first.id, author.id).await;
    assert_live_identity_pair(db, user_id, second.id, author.id).await;
    (author.id, first.id, second.id)
}

// RED-UNTIL-U7: today startup title heal only checks for an equivalent PENDING payload (and therefore re-mints a GroupIdentity card the user cancelled); mint/reuse never traverses the shared authority.
#[tokio::test]
async fn second_boot_reuses_startup_heal_card_and_changed_cohort_members_mint() {
    let _serial = U7_TRACE_LOCK.lock().await;
    let db = create_activated_test_db().await;
    let user_id = seed_user(&db, "startup-heal").await;
    let (author_id, anchor_id, peer_id) = seed_title_heal_collision(&db, user_id).await;
    clear_mint_trace();

    let first = livrarr_db::pool::heal_identity_title_policy(db.pool())
        .await
        .expect("first collision boot");
    assert_eq!(
        (
            first.healed,
            first.blocked_cohorts,
            first.review_cards_minted
        ),
        (0, 1, 1)
    );
    let first_id = pending_ids(&db, user_id, ilr::ReviewKind::GroupIdentity).await[0];
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta WHERE key='identity_title_policy_generation'",
    )
    .fetch_optional(db.pool())
    .await
    .expect("read held title-policy marker");
    assert!(marker.is_none());

    let second = livrarr_db::pool::heal_identity_title_policy(db.pool())
        .await
        .expect("second collision boot");
    assert_eq!(second.review_cards_minted, 0);
    assert_eq!(
        pending_ids(&db, user_id, ilr::ReviewKind::GroupIdentity).await,
        vec![first_id],
        "second boot returns ReusedPending for the same semantic key",
    );

    // Change the insight-92 key's sorted cohort Work ids. The hyphenated
    // parenthetical is a distinct pre-policy tuple, while the shared parser
    // canonicalizes its proposed subtitle to the same "large print edition".
    let author_name: String =
        sqlx::query_scalar("SELECT name FROM authors WHERE user_id=?1 AND id=?2")
            .bind(user_id)
            .bind(author_id)
            .fetch_one(db.pool())
            .await
            .expect("read title-heal Author");
    let third_member = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "U7 Cloud Cuckoo Land third legacy".to_string(),
            author_name: author_name.clone(),
            normalized_title: "u7 cloud cuckoo land third legacy".to_string(),
            normalized_author: author_name.to_lowercase(),
            author_id: Some(author_id),
            ..Default::default()
        })
        .await
        .expect("seed third title-heal member")
        .0;
    sqlx::query(
        "UPDATE works SET title='U7 Cloud Cuckoo Land (Large-Print Edition)', subtitle=NULL, \
                normalized_title='u7 cloud cuckoo land (large-print edition)', \
                normalized_identity_main='u7 cloud cuckoo land (large-print edition)', \
                normalized_identity_subtitle='large print edition', \
                normalized_identity_volume='', primary_author_id=?1, \
                text_distinction='common', identity_generation=1 \
          WHERE user_id=?2 AND id=?3",
    )
    .bind(author_id)
    .bind(user_id)
    .bind(third_member.id)
    .execute(db.pool())
    .await
    .expect("shape third colliding pre-policy member");
    assert_live_identity_pair(&db, user_id, anchor_id, author_id).await;
    assert_live_identity_pair(&db, user_id, peer_id, author_id).await;
    assert_live_identity_pair(&db, user_id, third_member.id, author_id).await;

    let third = livrarr_db::pool::heal_identity_title_policy(db.pool())
        .await
        .expect("boot after cohort semantic-key change");
    assert_eq!(third.review_cards_minted, 1);
    let ids = pending_ids(&db, user_id, ilr::ReviewKind::GroupIdentity).await;
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], first_id);
    let changed_id = ids[1];
    assert!(notification_rows(&db, user_id).await.is_empty());

    let trace = take_mint_trace();
    assert_eq!(trace.len(), 3, "every boot proposal reaches the helper");
    use livrarr_db::identity_layer::ReviewCardMintSite::StartupTitleHeal;
    assert_trace_outcome(
        &trace[0],
        StartupTitleHeal,
        ilr::ReviewKind::GroupIdentity,
        "Minted",
        first_id,
    );
    assert_trace_outcome(
        &trace[1],
        StartupTitleHeal,
        ilr::ReviewKind::GroupIdentity,
        "ReusedPending",
        first_id,
    );
    assert_trace_outcome(
        &trace[2],
        StartupTitleHeal,
        ilr::ReviewKind::GroupIdentity,
        "Minted",
        changed_id,
    );
}

// RED-UNTIL-U7: today simultaneous PendingRoute proposals serialize only through site 2's private reuse loop; the shared transactional authority is not the serialization seat.
#[tokio::test]
async fn simultaneous_same_key_proposals_serialize_to_one_card() {
    let _serial = U7_TRACE_LOCK.lock().await;
    let db = create_activated_test_db().await;
    let user_id = seed_user(&db, "concurrent").await;
    let author_id = seed_author(&db, user_id, "concurrent").await;
    let work = seed_work(&db, user_id, author_id, "concurrent proposal").await;
    let candidate = ilr::ParkedRouteCandidate {
        route: ilr::RouteKey {
            provider: ilr::IdentityProvider::Hardcover,
            kind: ilr::RouteKind::HardcoverWork,
            value: format!(
                "HC-U7-CONCURRENT-{}",
                CASE_ID.fetch_add(1, Ordering::Relaxed)
            ),
        },
        proposed_owner: ilr::RouteOwner::Work(work.own_work_id),
    };
    clear_mint_trace();

    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let work_id = work.own_work_id;
    let generation = work.identity_generation;
    let spawn =
        |db: SqliteDb, barrier: Arc<tokio::sync::Barrier>, candidate: ilr::ParkedRouteCandidate| {
            tokio::spawn(async move {
                barrier.wait().await;
                WorkIdentityRepository::commit_pending_route_review(
                    &db, user_id, work_id, generation, candidate,
                )
                .await
                .expect("concurrent production PendingRoute writer")
            })
        };
    let left = spawn(db.clone(), barrier.clone(), candidate.clone());
    let right = spawn(db.clone(), barrier.clone(), candidate);
    barrier.wait().await;
    let (left, right) = tokio::join!(left, right);
    let left = left.expect("left proposal task");
    let right = right.expect("right proposal task");
    assert_eq!(
        left.id, right.id,
        "both callers receive the same durable card id"
    );
    assert_eq!(
        pending_ids(&db, user_id, ilr::ReviewKind::PendingRoute).await,
        vec![left.id],
    );
    let notifications = notification_rows(&db, user_id).await;
    assert_eq!(notifications.len(), 1);
    assert_eq!(
        notifications[0].2,
        format!("identity-review-card:{}", left.id)
    );

    let trace = take_mint_trace();
    assert_eq!(trace.len(), 2);
    assert!(trace.iter().all(|row| {
        row.site == livrarr_db::identity_layer::ReviewCardMintSite::CapturedRouteHandoff
            && row.kind == ilr::ReviewKind::PendingRoute
    }));
    let minted = trace
        .iter()
        .filter(|row| matches!(&row.outcome, livrarr_db::identity_layer::ReviewCardMintOutcome::Minted(card) if card.id == left.id))
        .count();
    let reused = trace
        .iter()
        .filter(|row| matches!(&row.outcome, livrarr_db::identity_layer::ReviewCardMintOutcome::ReusedPending(card) if card.id == left.id))
        .count();
    assert_eq!((minted, reused), (1, 1));
}

// RED-UNTIL-U7: today an EditionEvidence replay inserts a third pending row; it neither keeps the oldest nor machine-cancels later pre-U7 equivalents with a distinct duplicate-cleanup audit.
#[tokio::test]
async fn pre_u7_equivalent_duplicates_collapse_oldest_wins_without_review_actor() {
    let _serial = U7_TRACE_LOCK.lock().await;
    let db = create_activated_test_db().await;
    let user_id = seed_user(&db, "duplicate-cleanup").await;
    let author_id = seed_author(&db, user_id, "duplicate-cleanup").await;
    let work = seed_work(&db, user_id, author_id, "duplicate cleanup").await;
    let edition = EditionRepository::apply_work_evidence(
        &db,
        ilr::EditionWorkEvidenceCommand {
            user_id,
            work_id: work.own_work_id,
            format: ilr::EditionFormat::Ebook,
            language: Some("en".to_string()),
            provenance: ilr::EvidenceProvenance::OwnedFile,
        },
    )
    .await
    .expect("seed duplicate-cleanup Edition")
    .edition;

    // Constructed-state compatibility justification: equivalent pending
    // duplicates are historical pre-U7 state that no correct post-U7 door can
    // create. The oldest row is minted through today's real apply_evidence
    // writer; INSERT..SELECT copies that exact production payload and row shape
    // to reconstruct the second call that today's writer made before U7.
    apply_direct_edition_conflict(&db, user_id, edition.id).await;
    let oldest = edition_card_ids(&db, user_id, edition.id).await[0];
    let later = sqlx::query(
        "INSERT INTO identity_review_cards \
            (user_id, work_id, kind, generation, status, payload, created_at) \
         SELECT user_id, work_id, kind, generation, status, payload, ?1 \
           FROM identity_review_cards WHERE id=?2",
    )
    .bind((Utc::now() + chrono::Duration::seconds(1)).to_rfc3339())
    .bind(oldest)
    .execute(db.pool())
    .await
    .expect("reconstruct second pre-U7 production-writer row")
    .last_insert_rowid();
    assert_eq!(
        edition_card_ids(&db, user_id, edition.id).await,
        vec![oldest, later],
    );
    clear_mint_trace();

    apply_direct_edition_conflict(&db, user_id, edition.id).await;
    let statuses: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, status FROM identity_review_cards WHERE id IN (?1,?2) ORDER BY id",
    )
    .bind(oldest)
    .bind(later)
    .fetch_all(db.pool())
    .await
    .expect("read duplicate-cleanup statuses");
    assert_eq!(
        statuses,
        vec![
            (oldest, "pending".to_string()),
            (later, "cancelled".to_string())
        ],
        "oldest equivalent remains the reusable pending card",
    );
    assert_eq!(
        edition_card_ids(&db, user_id, edition.id).await,
        vec![oldest],
    );
    let cleanup: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT event_kind, actor, payload FROM identity_audit_events \
          WHERE user_id=?1 AND event_kind LIKE '%duplicate-cleanup%' ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(db.pool())
    .await
    .expect("read duplicate-cleanup audit");
    assert_eq!(cleanup.len(), 1, "cleanup has one distinct machine audit");
    assert!(cleanup[0].0.contains("duplicate-cleanup"));
    assert!(cleanup[0].2.contains(&oldest.to_string()));
    assert!(cleanup[0].2.contains(&later.to_string()));
    assert!(
        serde_json::from_str::<ilr::ReviewActor>(&cleanup[0].1).is_err(),
        "ST-013: machine cleanup actor must never parse as ReviewActor",
    );
    let dismissals: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events \
          WHERE user_id=?1 AND event_kind='review-dismissal'",
    )
    .bind(user_id)
    .fetch_one(db.pool())
    .await
    .expect("count ReviewActor dismissals");
    assert_eq!(dismissals, 0);
    assert!(notification_rows(&db, user_id).await.is_empty());

    let trace = take_mint_trace();
    assert_eq!(trace.len(), 1);
    assert_trace_outcome(
        &trace[0],
        livrarr_db::identity_layer::ReviewCardMintSite::ApplyEvidence,
        ilr::ReviewKind::EditionEvidence,
        "ReusedPending",
        oldest,
    );
}

// RED-UNTIL-U7: today update, merge-with-choices, and affirm already emit no notification, but each mints through generic settlement's local insert rather than the shared newly-minted-and-still-pending notification gate.
#[tokio::test]
async fn registered_inline_doors_mint_then_resolve_and_emit_zero_notifications() {
    let _serial = U7_TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "inline").await;
    let update = seed_work(&harness.db, harness.user_id, author_id, "inline update").await;
    let merge_survivor = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "inline merge survivor",
    )
    .await;
    let merge_loser = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "inline merge loser",
    )
    .await;
    let affirm = seed_work(&harness.db, harness.user_id, author_id, "inline affirm").await;
    let affirm_value = format!("U7-GR-{}", CASE_ID.fetch_add(1, Ordering::Relaxed));
    harness
        .db
        .record_pending_anchor(
            affirm.own_work_id,
            AnchorType::new(AnchorType::GR_WORK),
            &affirm_value,
        )
        .await
        .expect("seed pending anchor through production writer");
    clear_mint_trace();

    let updated = call_router_json(
        &harness,
        Method::PUT,
        format!("/api/v1/work/{}", update.own_work_id),
        Some(json!({"title": "U7 Inline Updated Title"})),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.json);

    let preview = call_router_json(
        &harness,
        Method::GET,
        format!(
            "/api/v1/work/{}/merge/{}/preview",
            merge_survivor.own_work_id, merge_loser.own_work_id,
        ),
        None,
    )
    .await;
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.json);
    let merged = call_router_json(
        &harness,
        Method::POST,
        format!(
            "/api/v1/work/{}/merge/{}",
            merge_survivor.own_work_id, merge_loser.own_work_id,
        ),
        Some(json!({"choices": [{
            "field": "series_name",
            "choice": "keep_survivor"
        }]})),
    )
    .await;
    assert_eq!(merged.status, StatusCode::OK, "{}", merged.json);

    let affirmed = call_router_json(
        &harness,
        Method::POST,
        format!(
            "/api/v1/work/{}/pending-anchors/gr_work/affirm",
            affirm.own_work_id,
        ),
        None,
    )
    .await;
    assert_eq!(affirmed.status, StatusCode::NO_CONTENT, "{}", affirmed.json);

    let cards: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT id, kind, status FROM identity_review_cards \
          WHERE user_id=?1 ORDER BY id",
    )
    .bind(harness.user_id)
    .fetch_all(harness.db.pool())
    .await
    .expect("read inline mint-then-resolve cards");
    assert_eq!(cards.len(), 3);
    assert_eq!(
        cards
            .iter()
            .map(|row| (row.1.as_str(), row.2.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("GroupIdentity", "resolved"),
            ("GroupIdentity", "resolved"),
            ("PendingRoute", "resolved"),
        ],
    );
    assert!(notification_rows(&harness.db, harness.user_id)
        .await
        .is_empty());

    let trace = take_mint_trace();
    assert_eq!(trace.len(), 3);
    for (index, (kind, card_id)) in [
        (ilr::ReviewKind::GroupIdentity, cards[0].0),
        (ilr::ReviewKind::GroupIdentity, cards[1].0),
        (ilr::ReviewKind::PendingRoute, cards[2].0),
    ]
    .into_iter()
    .enumerate()
    {
        assert_trace_outcome(
            &trace[index],
            livrarr_db::identity_layer::ReviewCardMintSite::GenericSettlement,
            kind,
            "Minted",
            card_id,
        );
    }
    assert_no_provider_http(&harness);
}

fn incoming_conflict(label: &str) -> IncomingConflictPayload {
    IncomingConflictPayload {
        ol_key: Some(format!("OL-{label}-NEW-W")),
        gr_key: None,
        hc_key: None,
        isbn_13: None,
        asin: None,
        title: format!("Incoming {label}"),
        author_name: "U7 Conflict Author".to_string(),
        year: Some(2026),
        cover_url: None,
        top_candidates: vec![],
    }
}

async fn conflict_notes_snapshot(db: &SqliteDb, user_id: i64, conflict_id: i64) -> String {
    sqlx::query_scalar(
        "SELECT json_object( \
            'conflict', (SELECT json_object('status',status,'resolved_at',resolved_at, \
                'action',resolution_action,'notes',resolution_notes) \
              FROM work_identity_conflicts WHERE user_id=?1 AND id=?2), \
            'cards', (SELECT COALESCE(json_group_array(json_object('id',id,'status',status, \
                'payload',payload)), '[]') FROM (SELECT * FROM identity_review_cards \
                WHERE user_id=?1 ORDER BY id)), \
            'audits', (SELECT COALESCE(json_group_array(json_object('id',id,'kind',event_kind, \
                'actor',actor,'payload',payload)), '[]') FROM (SELECT * FROM identity_audit_events \
                WHERE user_id=?1 ORDER BY id)))",
    )
    .bind(user_id)
    .bind(conflict_id)
    .fetch_one(db.pool())
    .await
    .expect("snapshot HTTP notes persistence surfaces")
}

// PIN: old HTTP notes are already deserialized as an unused optional field and U1's conflict refusal leaves every stored row unchanged; U7 must preserve that compatibility after removing the field.
#[tokio::test]
async fn old_http_client_notes_are_ignored_and_persist_nowhere() {
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "http notes").await;
    let work = seed_work(&harness.db, harness.user_id, author_id, "http notes").await;
    let conflict_id = harness
        .state
        .identity_conflict_service
        .raise(NewIdentityConflict {
            user_id: harness.user_id,
            existing_work_id: work.own_work_id,
            kind: IdentityConflictKind::IncomingDifferentOlKey,
            incoming: incoming_conflict("U7-NOTES"),
            raised_by: ConflictSource::ManualAdd,
            raised_source_path: None,
        })
        .await
        .expect("seed legacy conflict through production writer");
    let before = conflict_notes_snapshot(&harness.db, harness.user_id, conflict_id).await;
    let changes_before: i64 = sqlx::query_scalar("SELECT total_changes()")
        .fetch_one(harness.db.pool())
        .await
        .expect("read HTTP notes total_changes baseline");
    let sentinel = "U7-NOTES-MUST-NOT-PERSIST-6e9d7";
    let response = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/identity-conflict/{conflict_id}/resolve"),
        Some(json!({
            "action": ConflictResolutionAction::KeepExisting,
            "notes": sentinel,
        })),
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::CONFLICT,
        "an ignored old field must reach U1's kind guard, not fail JSON extraction: {}",
        response.json,
    );
    assert!(response.json["message"]
        .as_str()
        .is_some_and(|message| message.contains("IdentityConflict")));
    let changes_after: i64 = sqlx::query_scalar("SELECT total_changes()")
        .fetch_one(harness.db.pool())
        .await
        .expect("read HTTP notes total_changes after refusal");
    assert_eq!(
        changes_after, changes_before,
        "old-client notes must reach the refusal guard before any database write",
    );
    let after = conflict_notes_snapshot(&harness.db, harness.user_id, conflict_id).await;
    assert_eq!(after, before, "ignored HTTP notes write no row or audit");
    assert!(!after.contains(sentinel));
    let stored_notes: Option<String> = sqlx::query_scalar(
        "SELECT resolution_notes FROM work_identity_conflicts WHERE user_id=?1 AND id=?2",
    )
    .bind(harness.user_id)
    .bind(conflict_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read legacy conflict notes column");
    assert!(stored_notes.is_none());
    assert_no_provider_http(&harness);
}

fn braced_item<'a>(source: &'a str, marker: &str) -> &'a str {
    let start = source
        .find(marker)
        .unwrap_or_else(|| panic!("missing {marker}"));
    let open = start + source[start..].find('{').expect("item open brace");
    let mut depth = 0_i64;
    for (offset, ch) in source[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[start..=open + offset];
                }
            }
            _ => {}
        }
    }
    panic!("unterminated {marker}")
}

// RED-UNTIL-U7: today both ResolveRequest and ResolveIdentityConflictRequest still declare notes even though the HTTP handler never uses it.
#[test]
fn current_rust_and_typescript_conflict_request_types_omit_notes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rust =
        std::fs::read_to_string(root.join("crates/livrarr-handlers/src/identity_conflicts.rs"))
            .expect("read production Rust conflict request");
    let rust_request = braced_item(&rust, "pub struct ResolveRequest");
    assert!(rust_request.contains("pub action: ConflictResolutionAction"));
    assert!(
        !rust_request.contains("notes"),
        "current Rust request type must rely on default unknown-field behavior: {rust_request}",
    );

    let typescript = std::fs::read_to_string(root.join("frontend/src/types/api.ts"))
        .expect("read production TypeScript API types");
    let ts_request = braced_item(
        &typescript,
        "export interface ResolveIdentityConflictRequest",
    );
    assert!(ts_request.contains("action: ConflictResolutionAction"));
    assert!(
        !ts_request.contains("notes"),
        "current TypeScript request type must omit the retired field: {ts_request}",
    );
}

// RED-UNTIL-U7: today runtime sites 1, 2, 4, and 5 each retain an INSERT/local guard; there is no single repository-side authority for the instrumentation above to observe.
#[test]
fn every_runtime_writer_calls_the_one_helper_while_cutover_staging_stays_excluded() {
    const HELPER: &str = "mint_reuse_or_suppress_review_card_in_tx";
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let repository = std::fs::read_to_string(root.join("crates/livrarr-db/src/identity_layer.rs"))
        .expect("read identity repository source");
    let pool = std::fs::read_to_string(root.join("crates/livrarr-db/src/pool.rs"))
        .expect("read startup-heal source");

    for (source, marker) in [
        (repository.as_str(), "async fn commit_settlement_in_tx("),
        (repository.as_str(), "async fn commit_pending_route_review("),
        (repository.as_str(), "async fn apply_evidence("),
        (repository.as_str(), "async fn apply_work_evidence("),
        (pool.as_str(), "pub async fn heal_identity_title_policy("),
    ] {
        let body = braced_item(source, marker);
        assert!(body.contains(HELPER), "{marker} bypasses {HELPER}");
        assert!(
            !body.contains("INSERT INTO identity_review_cards"),
            "{marker} retains a local mint: {body}",
        );
    }

    let helper = braced_item(
        &repository,
        "async fn mint_reuse_or_suppress_review_card_in_tx(",
    );
    assert_eq!(
        helper.matches("INSERT INTO identity_review_cards").count(),
        1,
        "the helper owns exactly one durable card INSERT",
    );
    assert!(repository.contains("enum ReviewCardMintOutcome"));
    for outcome in ["Minted", "ReusedPending", "SuppressedByDismissal"] {
        assert!(
            braced_item(&repository, "enum ReviewCardMintOutcome").contains(outcome),
            "missing helper outcome {outcome}",
        );
    }

    // PIN: site 6 is pre-activation cutover staging and remains outside U7.
    let cutover = braced_item(&repository, "async fn stage_legacy_identity_rows(");
    assert!(cutover.contains("INSERT INTO identity_review_cards"));
    assert!(!cutover.contains(HELPER));
    assert_eq!(
        repository
            .matches("INSERT INTO identity_review_cards")
            .count()
            + pool.matches("INSERT INTO identity_review_cards").count(),
        2,
        "only the runtime helper and excluded cutover staging retain raw INSERTs",
    );
}

#[derive(Clone, Copy, Debug)]
struct CliCard {
    id: i64,
    generation: i64,
    kind: ilr::ReviewKind,
}

struct CliFixture {
    data_dir: tempfile::TempDir,
    user_id: i64,
    migration: CliCard,
    contributor: CliCard,
    action_file: PathBuf,
}

async fn prepare_cli_fixture() -> CliFixture {
    // Constructed-state compatibility justification: the real CLI requires a
    // file-backed production data directory. This uses the production pool,
    // production migrations, activation, real SqliteDb, production payloads,
    // and generic settlement writer; only the two unproduced card kinds are
    // AC-007/ST-004 compatibility fixtures.
    let data_dir = tempfile::tempdir().expect("U7 CLI data dir");
    let pool = livrarr_db::pool::create_sqlite_pool(data_dir.path())
        .await
        .expect("create U7 CLI database");
    livrarr_db::pool::run_migrations(&pool)
        .await
        .expect("migrate U7 CLI database");
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_authors_identity \
         ON authors(user_id, normalized_name) WHERE normalized_name IS NOT NULL",
    )
    .execute(&pool)
    .await
    .expect("install startup Author identity index");
    let db = SqliteDb::new(pool);
    db.ensure_identity_authority_ready()
        .await
        .expect("activate CLI database");
    let user_id = seed_user(&db, "cli-codec").await;
    let author_id = seed_author(&db, user_id, "cli-codec").await;
    let mut work = seed_work(&db, user_id, author_id, "cli codec").await;

    let migration_out = WorkIdentityRepository::commit_settlement(
        &db,
        settlement_with_card(
            &work,
            user_id,
            ilr::SettlementReviewCard::MigrationRepair {
                legacy_key: "U7-CLI-LEGACY".to_string(),
                reason: "AC-007 ordinary struct-action compatibility fixture".to_string(),
            },
        ),
    )
    .await
    .expect("seed MigrationRepair compatibility card");
    let migration = CliCard {
        id: migration_out.review_cards[0].id,
        generation: migration_out.review_cards[0].generation,
        kind: ilr::ReviewKind::MigrationRepair,
    };
    work = migration_out.identity;
    let contributor_out = WorkIdentityRepository::commit_settlement(
        &db,
        settlement_with_card(
            &work,
            user_id,
            ilr::SettlementReviewCard::ContributorOrder {
                work_id: work.own_work_id,
                contributors: vec![ilr::WorkContributor {
                    user_id,
                    work_id: work.own_work_id,
                    author_id,
                    ordinal: 0,
                    roles: vec![],
                }],
            },
        ),
    )
    .await
    .expect("seed ContributorOrder compatibility card");
    let contributor = CliCard {
        id: contributor_out.review_cards[0].id,
        generation: contributor_out.review_cards[0].generation,
        kind: ilr::ReviewKind::ContributorOrder,
    };
    db.pool().close().await;
    let action_file = data_dir.path().join("u7-action.json");
    CliFixture {
        data_dir,
        user_id,
        migration,
        contributor,
        action_file,
    }
}

async fn cli_state(path: &Path, user_id: i64) -> String {
    let pool = livrarr_db::pool::create_sqlite_pool(path)
        .await
        .expect("open U7 CLI database for snapshot");
    let db = SqliteDb::new(pool);
    let state: String = sqlx::query_scalar(
        "SELECT json_object( \
            'works', (SELECT COALESCE(json_group_array(json_object('id',id,'title',title, \
                'generation',identity_generation)), '[]') FROM (SELECT * FROM works \
                WHERE user_id=?1 ORDER BY id)), \
            'routes', (SELECT COALESCE(json_group_array(json_object('id',id,'state',state, \
                'value',provider_scoped_id)), '[]') FROM (SELECT * FROM identity_routes \
                WHERE user_id=?1 ORDER BY id)), \
            'cards', (SELECT COALESCE(json_group_array(json_object('id',id,'status',status, \
                'generation',generation,'payload',payload,'resolved_at',resolved_at)), '[]') \
                FROM (SELECT * FROM identity_review_cards WHERE user_id=?1 ORDER BY id)), \
            'audits', (SELECT COALESCE(json_group_array(json_object('id',id,'kind',event_kind, \
                'actor',actor,'payload',payload)), '[]') FROM (SELECT * FROM identity_audit_events \
                WHERE user_id=?1 ORDER BY id)), \
            'notifications', (SELECT COALESCE(json_group_array(json_object('id',id,'type',type, \
                'ref',ref_key,'data',data)), '[]') FROM (SELECT * FROM notifications \
                WHERE user_id=?1 ORDER BY id)))",
    )
    .bind(user_id)
    .fetch_one(db.pool())
    .await
    .expect("snapshot U7 CLI state");
    db.pool().close().await;
    state
}

fn production_livrarr_binary() -> &'static PathBuf {
    LIVRARR_BINARY.get_or_init(|| {
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
            String::from_utf8_lossy(&build.stderr),
        );
        let configured_target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace.join("target"));
        let target = if configured_target.is_absolute() {
            configured_target
        } else {
            workspace.join(configured_target)
        };
        target.join("debug/livrarr")
    })
}

#[derive(Clone, Copy, Debug)]
enum CliExpected {
    InvalidActionFile,
    ContinuationUnavailable(ilr::ReviewKind),
}

async fn run_cli_codec_case(
    fixture: &CliFixture,
    card: CliCard,
    action: Value,
    expected: CliExpected,
) {
    std::fs::write(
        &fixture.action_file,
        serde_json::to_vec_pretty(&action).expect("encode U7 CLI action"),
    )
    .expect("write U7 CLI action file");
    let before = cli_state(fixture.data_dir.path(), fixture.user_id).await;
    let observer_pool = livrarr_db::pool::create_sqlite_pool(fixture.data_dir.path())
        .await
        .expect("open U7 CLI no-write observer");
    let mut observer = observer_pool
        .acquire()
        .await
        .expect("acquire U7 CLI no-write observer");
    let data_version_before: i64 = sqlx::query_scalar("PRAGMA data_version")
        .fetch_one(&mut *observer)
        .await
        .expect("read U7 CLI data version");
    let direct = livrarr_server::identity_layer::run_identity_cutover_command(
        livrarr_server::identity_layer::IdentityCutoverCliCommand::ResolveReview {
            card_id: card.id,
            expected_generation: card.generation,
            action_file: fixture.action_file.clone(),
        },
        fixture.data_dir.path().to_path_buf(),
        CancellationToken::new(),
    )
    .await;
    match expected {
        CliExpected::InvalidActionFile => assert!(matches!(
            direct,
            Err(livrarr_server::identity_layer::IdentityCutoverCommandError::InvalidActionFile)
        )),
        CliExpected::ContinuationUnavailable(kind) => assert!(matches!(
            direct,
            Err(livrarr_server::identity_layer::IdentityCutoverCommandError::ContinuationUnavailable(observed))
                if observed == kind
        )),
    }
    let data_version_after_direct: i64 = sqlx::query_scalar("PRAGMA data_version")
        .fetch_one(&mut *observer)
        .await
        .expect("read data version after production command function");
    assert_eq!(
        data_version_after_direct, data_version_before,
        "production command function committed a database write for {expected:?}: {action}",
    );
    assert_eq!(
        cli_state(fixture.data_dir.path(), fixture.user_id).await,
        before,
        "production command function wrote for {expected:?}: {action}",
    );

    let output = std::process::Command::new(production_livrarr_binary())
        .arg("--data")
        .arg(fixture.data_dir.path())
        .args(["identity-cutover", "resolve"])
        .arg(card.id.to_string())
        .arg("--expected-generation")
        .arg(card.generation.to_string())
        .arg("--action-file")
        .arg(&fixture.action_file)
        .output()
        .expect("run real U7 identity-cutover resolve CLI");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    match expected {
        CliExpected::InvalidActionFile => assert!(
            stderr.contains("invalid action file"),
            "binary must expose InvalidActionFile: {stderr}",
        ),
        CliExpected::ContinuationUnavailable(kind) => assert!(
            stderr.contains("continuation unavailable") && stderr.contains(kind.storage_code()),
            "binary must prove parsing reached U1's named guard: {stderr}",
        ),
    }
    let data_version_after_binary: i64 = sqlx::query_scalar("PRAGMA data_version")
        .fetch_one(&mut *observer)
        .await
        .expect("read data version after real CLI");
    assert_eq!(
        data_version_after_binary, data_version_before,
        "real CLI committed a database write for {expected:?}: {action}",
    );
    assert_eq!(
        cli_state(fixture.data_dir.path(), fixture.user_id).await,
        before,
        "real CLI wrote for {expected:?}: {action}",
    );
    drop(observer);
    observer_pool.close().await;
}

fn ordinary_action(extra_inner: Option<(&str, Value)>) -> Value {
    let mut payload = serde_json::Map::from_iter([(
        "reason".to_string(),
        Value::String("U7 ordinary codec".to_string()),
    )]);
    if let Some((key, value)) = extra_inner {
        payload.insert(key.to_string(), value);
    }
    json!({"DiscardProvenNonIdentity": Value::Object(payload)})
}

fn contributor_action(card: CliCard) -> Value {
    json!({
        "ContributorOrder": {
            "card_id": card.id,
            "expected_generation": card.generation,
            "partition": [{
                "normalized_full_author_identity_name": "u7 author",
                "sorted_provider_route_set": [],
                "sorted_exact_source_name_set": [],
            }],
            "order": ["author:u7"],
            "primary": "author:u7",
        }
    })
}

// RED-UNTIL-U7: today nested notes in an ordinary struct action are ignored and reach ContinuationUnavailable; the non-notes location baselines below already have ST-004's required results.
#[tokio::test]
async fn real_cli_preserves_ordinary_unknown_baselines_but_rejects_notes_recursively() {
    let _serial = U7_TRACE_LOCK.lock().await;
    let fixture = prepare_cli_fixture().await;
    let card = fixture.migration;
    assert_eq!(card.kind, ilr::ReviewKind::MigrationRepair);

    // PIN: ordinary struct-action inner unrelated unknown accepted.
    run_cli_codec_case(
        &fixture,
        card,
        ordinary_action(Some(("unrelated", json!({"kept": true})))),
        CliExpected::ContinuationUnavailable(ilr::ReviewKind::MigrationRepair),
    )
    .await;

    // PIN: a second top-level unrelated key is invalid.
    let mut second_top = ordinary_action(None);
    second_top
        .as_object_mut()
        .unwrap()
        .insert("unrelatedTop".to_string(), json!(true));
    run_cli_codec_case(&fixture, card, second_top, CliExpected::InvalidActionFile).await;

    // RED-UNTIL-U7: nested notes are accepted by today's derived Deserialize.
    run_cli_codec_case(
        &fixture,
        card,
        ordinary_action(Some(("notes", json!("must reject")))),
        CliExpected::InvalidActionFile,
    )
    .await;

    // PIN: top-level notes already make the externally tagged action invalid;
    // U7's recursive check must retain the same InvalidActionFile/no-write result.
    let mut top_notes = ordinary_action(None);
    top_notes
        .as_object_mut()
        .unwrap()
        .insert("notes".to_string(), json!("must reject"));
    run_cli_codec_case(&fixture, card, top_notes, CliExpected::InvalidActionFile).await;
}

// RED-UNTIL-U7: today ContributorOrder notes inside its command payload or partition entry are ignored and reach ContinuationUnavailable; its unrelated and string-shape baselines must remain unchanged.
#[tokio::test]
async fn real_cli_contributor_order_preserves_per_shape_baseline_and_rejects_notes() {
    let _serial = U7_TRACE_LOCK.lock().await;
    let fixture = prepare_cli_fixture().await;
    let card = fixture.contributor;
    assert_eq!(card.kind, ilr::ReviewKind::ContributorOrder);

    // PIN: unrelated command-payload and partition-entry unknowns both parse,
    // then the real U1 guard returns ContinuationUnavailable.
    let mut unrelated = contributor_action(card);
    let command = unrelated["ContributorOrder"].as_object_mut().unwrap();
    command.insert("unrelatedCommandPayload".to_string(), json!({"kept": true}));
    command["partition"][0]
        .as_object_mut()
        .unwrap()
        .insert("unrelatedPartitionEntry".to_string(), json!([1, 2, 3]));
    run_cli_codec_case(
        &fixture,
        card,
        unrelated,
        CliExpected::ContinuationUnavailable(ilr::ReviewKind::ContributorOrder),
    )
    .await;

    // RED-UNTIL-U7: notes in the ContributorOrder command payload are ignored today.
    let mut command_notes = contributor_action(card);
    command_notes["ContributorOrder"]
        .as_object_mut()
        .unwrap()
        .insert("notes".to_string(), json!("must reject"));
    run_cli_codec_case(
        &fixture,
        card,
        command_notes,
        CliExpected::InvalidActionFile,
    )
    .await;

    // RED-UNTIL-U7: notes in a partition entry are ignored today.
    let mut partition_notes = contributor_action(card);
    partition_notes["ContributorOrder"]["partition"][0]
        .as_object_mut()
        .unwrap()
        .insert("notes".to_string(), json!("must reject"));
    run_cli_codec_case(
        &fixture,
        card,
        partition_notes,
        CliExpected::InvalidActionFile,
    )
    .await;

    // PIN: order elements are string AuthorRefs; an object remains invalid.
    let mut object_order = contributor_action(card);
    object_order["ContributorOrder"]["order"] = json!([{"author": "u7"}]);
    run_cli_codec_case(&fixture, card, object_order, CliExpected::InvalidActionFile).await;

    // PIN: primary is a string AuthorRef; an object remains invalid.
    let mut object_primary = contributor_action(card);
    object_primary["ContributorOrder"]["primary"] = json!({"author": "u7"});
    run_cli_codec_case(
        &fixture,
        card,
        object_primary,
        CliExpected::InvalidActionFile,
    )
    .await;

    // PIN: ContributorOrder is the whole one-key command; a second top-level key remains invalid.
    let mut second_top = contributor_action(card);
    second_top
        .as_object_mut()
        .unwrap()
        .insert("unrelatedTop".to_string(), json!(false));
    run_cli_codec_case(&fixture, card, second_top, CliExpected::InvalidActionFile).await;
}

// RED-UNTIL-U7-FIX-R1: today Work A's replay under its real id misses the
// placeholder-keyed row from its NEW-Work settlement and mints a duplicate.
// The same-Work replay uses the captured-route `commit_pending_route_review`
// production entry; Work A and Work B use generic settlement.
#[tokio::test]
async fn pending_route_placeholder_reuse_is_scoped_to_the_durable_work() {
    let _serial = U7_TRACE_LOCK.lock().await;
    let db = create_activated_test_db().await;
    let user_id = seed_user(&db, "durable-placeholder-key").await;
    let author_a_id = seed_author(&db, user_id, "durable-placeholder-a").await;
    let author_b_id = seed_author(&db, user_id, "durable-placeholder-b").await;
    assert_ne!(
        author_a_id, author_b_id,
        "Work fixtures need distinct authors"
    );

    let route_value = format!(
        "OL-U7-DURABLE-WORK-{}-W",
        CASE_ID.fetch_add(1, Ordering::Relaxed)
    );
    let mut title_a = ilr::title_parts_from_provider(
        format!(
            "U7 durable placeholder Work A {}",
            CASE_ID.fetch_add(1, Ordering::Relaxed)
        ),
        None,
    )
    .expect("valid Work A fixture title");
    title_a.provenance = ilr::EvidenceProvenance::User;

    clear_mint_trace();
    let created_a = WorkIdentityRepository::commit_settlement(
        &db,
        ilr::SettlementCommit {
            user_id,
            existing_work_id: None,
            add_source: None,
            identity_title: title_a,
            text_distinction: None,
            contributors: vec![ilr::WorkContributor {
                user_id,
                work_id: 0,
                author_id: author_a_id,
                ordinal: 0,
                roles: vec![],
            }],
            routes: vec![],
            absorbed_work_ids: vec![],
            expected_generation: 0,
            review_cards: vec![pending_candidate(
                0,
                ilr::IdentityProvider::OpenLibrary,
                ilr::RouteKind::OpenLibraryWork,
                &route_value,
                ilr::RouteOwner::Work(0),
            )],
        },
    )
    .await
    .expect("NEW Work A settlement mints placeholder PendingRoute");
    assert!(created_a.created, "Work A must be newly allocated");
    assert_eq!(created_a.review_cards.len(), 1);
    let work_a = created_a.identity;
    let work_a_id = work_a.own_work_id;
    let card_a_id = created_a.review_cards[0].id;
    assert_live_identity_pair(&db, user_id, work_a_id, author_a_id).await;
    assert_eq!(
        pending_ids(&db, user_id, ilr::ReviewKind::PendingRoute).await,
        vec![card_a_id],
        "placeholder mint creates exactly one pending card",
    );
    let card_a_work_id: i64 =
        sqlx::query_scalar("SELECT work_id FROM identity_review_cards WHERE user_id=?1 AND id=?2")
            .bind(user_id)
            .bind(card_a_id)
            .fetch_one(db.pool())
            .await
            .expect("read Work A card's durable work column");
    assert_eq!(card_a_work_id, work_a_id);
    let notifications_after_a = notification_rows(&db, user_id).await;
    assert_eq!(notifications_after_a.len(), 1, "genuine mint notifies");
    assert_eq!(notifications_after_a[0].1, "identityReviewNeeded");
    assert_eq!(
        notifications_after_a[0].2,
        format!("identity-review-card:{card_a_id}")
    );

    let replay = WorkIdentityRepository::commit_pending_route_review(
        &db,
        user_id,
        work_a_id,
        work_a.identity_generation,
        ilr::ParkedRouteCandidate {
            route: ilr::RouteKey {
                provider: ilr::IdentityProvider::OpenLibrary,
                kind: ilr::RouteKind::OpenLibraryWork,
                value: route_value.clone(),
            },
            proposed_owner: ilr::RouteOwner::Work(work_a_id),
        },
    )
    .await
    .expect("real-id replay for Work A succeeds");
    assert_eq!(
        replay.id, card_a_id,
        "real-id replay must reuse Work A's placeholder-minted card",
    );
    assert_eq!(
        pending_ids(&db, user_id, ilr::ReviewKind::PendingRoute).await,
        vec![card_a_id],
        "same-Work replay must leave one pending card",
    );
    assert_eq!(
        notification_rows(&db, user_id).await,
        notifications_after_a,
        "same-Work reuse emits no second notification",
    );

    let mut title_b = ilr::title_parts_from_provider(
        format!(
            "U7 durable placeholder Work B {}",
            CASE_ID.fetch_add(1, Ordering::Relaxed)
        ),
        None,
    )
    .expect("valid Work B fixture title");
    title_b.provenance = ilr::EvidenceProvenance::User;
    let created_b = WorkIdentityRepository::commit_settlement(
        &db,
        ilr::SettlementCommit {
            user_id,
            existing_work_id: None,
            add_source: None,
            identity_title: title_b,
            text_distinction: None,
            contributors: vec![ilr::WorkContributor {
                user_id,
                work_id: 0,
                author_id: author_b_id,
                ordinal: 0,
                roles: vec![],
            }],
            routes: vec![],
            absorbed_work_ids: vec![],
            expected_generation: 0,
            review_cards: vec![pending_candidate(
                0,
                ilr::IdentityProvider::OpenLibrary,
                ilr::RouteKind::OpenLibraryWork,
                &route_value,
                ilr::RouteOwner::Work(0),
            )],
        },
    )
    .await
    .expect("NEW Work B settlement mints its own placeholder PendingRoute");
    assert!(created_b.created, "Work B must be newly allocated");
    assert_eq!(created_b.review_cards.len(), 1);
    let work_b_id = created_b.identity.own_work_id;
    let card_b_id = created_b.review_cards[0].id;
    assert_ne!(work_b_id, work_a_id);
    assert_ne!(
        created_b.identity.identity_title.main, work_a.identity_title.main,
        "Work fixtures need distinct titles",
    );
    assert_ne!(
        card_b_id, card_a_id,
        "Work B's placeholder must not reuse Work A's pending card",
    );
    assert_live_identity_pair(&db, user_id, work_b_id, author_b_id).await;
    let attached_cards: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT id, work_id FROM identity_review_cards \
          WHERE user_id=?1 AND id IN (?2,?3) ORDER BY id",
    )
    .bind(user_id)
    .bind(card_a_id)
    .bind(card_b_id)
    .fetch_all(db.pool())
    .await
    .expect("read durable Work attachment for both cards");
    assert_eq!(
        attached_cards,
        vec![(card_a_id, work_a_id), (card_b_id, work_b_id)]
    );
    assert_eq!(
        pending_ids(&db, user_id, ilr::ReviewKind::PendingRoute).await,
        vec![card_a_id, card_b_id],
    );
    let notifications_after_b = notification_rows(&db, user_id).await;
    assert_eq!(notifications_after_b.len(), 2, "each genuine mint notifies");
    assert_eq!(notifications_after_b[0], notifications_after_a[0]);
    assert_eq!(notifications_after_b[1].1, "identityReviewNeeded");
    assert_eq!(
        notifications_after_b[1].2,
        format!("identity-review-card:{card_b_id}")
    );

    let trace = take_mint_trace();
    assert_eq!(trace.len(), 3, "all proposals traverse the shared helper");
    use livrarr_db::identity_layer::ReviewCardMintSite::{CapturedRouteHandoff, GenericSettlement};
    assert_trace_outcome(
        &trace[0],
        GenericSettlement,
        ilr::ReviewKind::PendingRoute,
        "Minted",
        card_a_id,
    );
    assert_trace_outcome(
        &trace[1],
        CapturedRouteHandoff,
        ilr::ReviewKind::PendingRoute,
        "ReusedPending",
        card_a_id,
    );
    assert_trace_outcome(
        &trace[2],
        GenericSettlement,
        ilr::ReviewKind::PendingRoute,
        "Minted",
        card_b_id,
    );
}
