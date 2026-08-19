//! C3 provider-response pins for `docs/design-subtitle-matching.md`, test-plan item 5.
//!
//! Both cases enter through the production router and authentication middleware,
//! then traverse the surviving manual-refresh handler, service stack, Goodreads
//! client, provider queue, call sink, and SQLite persistence. The only double is
//! the canned Goodreads HTTP endpoint at the transport seam.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicI64};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{AuthorDb, CreateAuthorDbRequest, CreateUserDbRequest, UserDb};
use livrarr_domain::identity_layer::{
    EvidenceProvenance, IdentityProvider, IdentityTitleTuple, RouteKind, RouteOwner,
    RouteProvenance, SettlementCommit, WorkContributor, WorkIdentityRepository, WorkRoute,
    WorkRouteState,
};
use livrarr_domain::services::{ProviderCallSink, RateBucket};
use livrarr_domain::{MetadataProvider, UserRole, WorkId};
use livrarr_enrichment::{DefaultProviderQueueBuilder, ProviderQueueConfig};
use livrarr_external_data::live_config::LiveMetadataConfig;
use livrarr_external_data::{GoodreadsClient, ProviderClient};
use livrarr_http::breaker::CircuitBreakerConfig;
use livrarr_http::outbound_queue::{self, AdmissionError};
use livrarr_metadata as metadata;
use livrarr_server::auth_crypto::{AuthCryptoService, RealAuthCrypto};
use livrarr_server::state::AppState;
use serde_json::Value;
use sqlx::Row;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

static GOODREADS_BREAKER_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct GoodreadsBreakerGuard {
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

impl Drop for GoodreadsBreakerGuard {
    fn drop(&mut self) {
        outbound_queue::shared().reset_breaker_for_tests(RateBucket::Goodreads);
    }
}

async fn lock_goodreads_breaker() -> GoodreadsBreakerGuard {
    let lock = GOODREADS_BREAKER_LOCK.lock().await;
    let queue = outbound_queue::shared();
    queue.reset_breaker_for_tests(RateBucket::Goodreads);
    queue.set_breaker_config_for_tests(
        RateBucket::Goodreads,
        CircuitBreakerConfig {
            failure_threshold: 1,
            evaluation_window_secs: 60,
            open_duration_secs: 60,
            half_open_probe_count: 1,
        },
    );
    GoodreadsBreakerGuard { _lock: lock }
}

#[derive(Clone, Copy)]
enum ProviderShape {
    Forbidden,
    EmptyOk,
}

impl ProviderShape {
    fn response(self) -> axum::response::Response {
        match self {
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                Body::empty(),
            )
                .into_response(),
            Self::EmptyOk => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                Body::empty(),
            )
                .into_response(),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Forbidden => "HTTP 403",
            Self::EmptyOk => "HTTP 200 empty body",
        }
    }
}

struct CannedGoodreads {
    base_url: String,
    task: JoinHandle<()>,
}

impl CannedGoodreads {
    async fn wait(self) {
        timeout(Duration::from_secs(5), self.task)
            .await
            .expect("canned Goodreads server wedged")
            .expect("canned Goodreads server task panicked");
    }
}

async fn spawn_canned_goodreads(shape: ProviderShape) -> CannedGoodreads {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind canned Goodreads");
    let address = listener.local_addr().expect("canned Goodreads address");
    let (served_tx, served_rx) = oneshot::channel();
    let served_tx = Arc::new(Mutex::new(Some(served_tx)));
    let app = Router::new().route(
        "/book/show/{id}",
        get(move || {
            let served_tx = served_tx.clone();
            async move {
                if let Some(tx) = served_tx
                    .lock()
                    .expect("canned Goodreads served lock poisoned")
                    .take()
                {
                    let _ = tx.send(());
                }
                shape.response()
            }
        }),
    );
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = served_rx.await;
            })
            .await
            .expect("serve canned Goodreads");
    });
    CannedGoodreads {
        base_url: format!("http://{address}"),
        task,
    }
}

struct RouteHarness {
    app: Router,
    db: SqliteDb,
    api_key: String,
    work_id: WorkId,
    call_sink_cancel: CancellationToken,
    call_sink_task: Option<JoinHandle<()>>,
    _tmp: tempfile::TempDir,
}

impl RouteHarness {
    async fn drain_call_sink(&mut self) {
        self.call_sink_cancel.cancel();
        let task = self
            .call_sink_task
            .take()
            .expect("call sink is drained exactly once");
        timeout(Duration::from_secs(5), task)
            .await
            .expect("provider call sink wedged")
            .expect("provider call sink task panicked");
    }
}

async fn build_route_harness(goodreads_base_url: String) -> RouteHarness {
    let db = livrarr_db::test_helpers::create_test_db().await;
    let tmp = tempfile::tempdir().expect("route harness tempdir");
    let data_dir = tmp.path().to_path_buf();
    let data_dir_arc = Arc::new(data_dir.clone());

    let api_key = "subtitle-matching-b4-api-key".to_string();
    let api_key_hash = RealAuthCrypto
        .hash_token(&api_key)
        .await
        .expect("hash route API key");
    let user_id = db
        .create_user(CreateUserDbRequest {
            username: "subtitle-matching-b4-admin".to_string(),
            password_hash: "unused-password-hash".to_string(),
            role: UserRole::Admin,
            api_key_hash,
        })
        .await
        .expect("create route user")
        .id;
    let (author, _) = db
        .create_author(CreateAuthorDbRequest {
            user_id,
            name: "Case Writer".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("create route author");
    // Bug reproduction: identity-layer-rewrite — a post-activation fixture
    // has a complete identity graph and no legacy scalar route; live refresh
    // must dispatch from that graph rather than silently making zero calls.
    let settled = WorkIdentityRepository::commit_settlement(
        &db,
        SettlementCommit {
            user_id,
            existing_work_id: None,
            add_source: None,
            identity_title: IdentityTitleTuple {
                main: "The Certified Book".to_string(),
                subtitle: None,
                volume: None,
                normalized_main: "the certified book".to_string(),
                normalized_subtitle: String::new(),
                normalized_volume: String::new(),
                provenance: EvidenceProvenance::User,
            },
            text_distinction: None,
            contributors: vec![WorkContributor {
                user_id,
                work_id: 0,
                author_id: author.id,
                ordinal: 0,
                roles: Vec::new(),
            }],
            routes: vec![WorkRoute {
                id: 0,
                user_id,
                owner: RouteOwner::Work(0),
                resolved_work_id: 0,
                provider: IdentityProvider::Goodreads,
                kind: RouteKind::GoodreadsWork,
                provider_scoped_id: "12345".to_string(),
                state: WorkRouteState::Active,
                provenance: RouteProvenance::Provider(IdentityProvider::Goodreads),
                user_confirmed: true,
                observed_at: chrono::Utc::now(),
            }],
            absorbed_work_ids: Vec::new(),
            expected_generation: 0,
            review_cards: Vec::new(),
        },
    )
    .await
    .expect("settle route work fixture");
    assert!(
        settled.created,
        "fixture must use the settlement create branch"
    );
    assert_eq!(settled.identity.primary_author_id, author.id);
    assert!(settled.identity.active_routes.iter().any(|route| {
        route.provider == IdentityProvider::Goodreads
            && route.kind == RouteKind::GoodreadsWork
            && route.provider_scoped_id == "12345"
    }));
    let work_id = settled.identity.own_work_id;

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
    let http_fetcher = livrarr_http::fetcher::HttpFetcherImpl::new().expect("shared HTTP fetcher");
    let llm_http_client = livrarr_http::HttpClient::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(&user_agent)
        .build()
        .expect("LLM HTTP client");

    let live_metadata_config = LiveMetadataConfig::new(livrarr_db::MetadataConfig {
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

    let identity_resolver_arc = Arc::new(
        metadata::english_identity_resolver::LiveEnglishIdentityResolver {
            clients: std::collections::HashMap::new(),
            cache: transport_cache,
            config: metadata::english_identity_resolver::ResolverConfig::default(),
        },
    );

    let call_sink_cancel = CancellationToken::new();
    let (call_sink, call_sink_task) =
        livrarr_server::call_sink::spawn_call_sink(db.clone(), call_sink_cancel.clone());
    let call_sink: Arc<dyn ProviderCallSink> = Arc::new(call_sink);

    let goodreads = GoodreadsClient::new(
        http_fetcher.clone(),
        http_client.clone(),
        goodreads_base_url,
    )
    .with_retry_backoff(0);
    let db_arc = Arc::new(db.clone());
    let queue = Arc::new(
        DefaultProviderQueueBuilder::new()
            .with_identity_route_dispatch()
            .add_provider(
                MetadataProvider::Goodreads,
                ProviderClient::Goodreads(goodreads).with_call_sink(call_sink.clone()),
                ProviderQueueConfig {
                    provider: MetadataProvider::Goodreads,
                    max_attempts: 1,
                },
            )
            .with_call_sink(call_sink)
            .build(db_arc.clone()),
    );
    let merge_engine = Arc::new(metadata::DefaultMergeEngine::new(
        metadata::PriorityModel::english(),
    ));
    let enrichment_service = Arc::new(metadata::EnrichmentServiceImpl::new(
        db_arc,
        queue.clone(),
        merge_engine,
        false,
    ));
    let work_service_arc: Arc<livrarr_server::state::LiveWorkService> = {
        let workflow = metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
            enrichment_service.clone(),
        );
        Arc::new(
            metadata::work_service::WorkServiceImpl::new(
                db.clone(),
                workflow,
                http_fetcher.clone(),
                data_dir.clone(),
            )
            .with_resolver(identity_resolver_arc.clone())
            .with_identity_routes_authoritative(),
        )
    };
    let discovery_service_arc = Arc::new(
        metadata::discovery_service::DiscoveryServiceImpl::new(
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
    let identity_road_arc = Arc::new(livrarr_server::identity_layer::build_live_identity_road(
        db.clone(),
        http_fetcher.clone(),
        http_client.clone(),
        live_metadata_config.clone(),
    ));

    let state = AppState {
        db: db.clone(),
        auth_service,
        http_client: http_client.clone(),
        http_client_safe,
        http_fetcher: http_fetcher.clone(),
        config: Arc::new(livrarr_server::config::AppConfig::default()),
        data_dir: data_dir_arc.clone(),
        startup_time: chrono::Utc::now(),
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
        author_service: Arc::new(metadata::author_service::AuthorServiceImpl::new(
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
        series_service: Arc::new(metadata::series_service::SeriesServiceImpl::new(db.clone())),
        series_query_service: Arc::new(
            metadata::series_query_service::SeriesQueryServiceImpl::new(
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
            Arc::new(metadata::rss_sync_workflow::RssSyncWorkflowImpl::new(
                Arc::new(db.clone()),
                Arc::new(http_fetcher.clone()),
                release_service,
            ))
        },
        list_service: {
            let workflow = metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                enrichment_service.clone(),
            );
            let work_service = metadata::work_service::WorkServiceImpl::new(
                db.clone(),
                workflow,
                http_fetcher.clone(),
                data_dir.clone(),
            );
            Arc::new(metadata::list_service::ListServiceImpl::with_identity_road(
                db.clone(),
                work_service,
                http_fetcher.clone(),
                metadata::list_service::NoOpBibliographyTrigger,
                identity_road_arc.clone(),
            ))
        },
        identity_conflict_service: Arc::new(
            livrarr_server::services::identity_conflict_service::LiveIdentityConflictService::new(
                db.clone(),
            ),
        ),
        identity_resolver: identity_resolver_arc,
        enrichment_workflow: Arc::new(
            metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                enrichment_service.clone(),
            ),
        ),
        author_monitor_workflow: {
            let workflow = metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                enrichment_service.clone(),
            );
            let work_service = metadata::work_service::WorkServiceImpl::new(
                db.clone(),
                workflow,
                http_fetcher.clone(),
                data_dir.clone(),
            );
            Arc::new(
                metadata::author_monitor_workflow::AuthorMonitorWorkflowImpl::with_identity_road(
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
            ),
        ),
        cover_service,
        preadd_cover_service: Arc::new(
            metadata::preadd_cover_service::LivePreaddCoverService::new(
                std::collections::HashMap::new(),
            ),
        ),
        hmac_key,
        trusted_origins_rebuilder: livrarr_server::state::TrustedOriginsRebuilderImpl(
            trusted_origins_arc,
        ),
    };

    let ui_dir = state.data_dir.join("ui-not-present-in-test");
    let app = livrarr_server::router::build_router(state, ui_dir);
    RouteHarness {
        app,
        db,
        api_key,
        work_id,
        call_sink_cancel,
        call_sink_task: Some(call_sink_task),
        _tmp: tmp,
    }
}

async fn route_json(
    app: &Router,
    api_key: &str,
    method: Method,
    uri: String,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-api-key", api_key);
    let request_body = match body {
        Some(value) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let mut request = request.body(request_body).expect("build route request");
    request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 17)),
        31000,
    )));
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("production route response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read route body");
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("route JSON response")
    };
    (status, body)
}

async fn run_preview_failure_case(shape: ProviderShape) {
    let _breaker_guard = lock_goodreads_breaker().await;
    let canned = spawn_canned_goodreads(shape).await;
    let mut harness = build_route_harness(canned.base_url.clone()).await;

    let (preview_status, preview_body) = route_json(
        &harness.app,
        &harness.api_key,
        Method::POST,
        format!("/api/v1/work/{}/refresh", harness.work_id),
        None,
    )
    .await;
    assert_eq!(
        preview_status,
        StatusCode::OK,
        "{} must reach the real manual-refresh handler",
        shape.label()
    );
    assert_eq!(
        preview_body["reason"],
        "provider_unavailable",
        "{} must be presented as provider-unavailable",
        shape.label()
    );
    assert_ne!(
        preview_body["reason"],
        "not_found",
        "{} must not write off the identifier",
        shape.label()
    );

    canned.wait().await;
    harness.drain_call_sink().await;

    let record = sqlx::query(
        "SELECT outcome, detail FROM provider_call_records \
         WHERE provider = 'goodreads' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(harness.db.pool())
    .await
    .expect("Goodreads call record");
    let recorded_outcome: String = record.get("outcome");
    let recorded_detail: Option<String> = record.get("detail");

    let (health_status, health_body) = route_json(
        &harness.app,
        &harness.api_key,
        Method::GET,
        "/api/v1/system/health-summary".to_string(),
        None,
    )
    .await;
    assert_eq!(
        health_status,
        StatusCode::OK,
        "{} health summary route",
        shape.label()
    );
    let goodreads_health = health_body["metadataProviders"]
        .as_array()
        .expect("metadataProviders array")
        .iter()
        .find(|provider| provider["name"] == "Goodreads")
        .expect("Goodreads health row");

    let row_is_error = matches!(
        recorded_outcome.as_str(),
        "rate_limited" | "timeout" | "error"
    );
    let health_is_error = goodreads_health["status"] == "error";
    assert!(
        row_is_error && health_is_error,
        "{} must persist an is_error outcome and drive the real health surface; \
         outcome={recorded_outcome:?}, detail={recorded_detail:?}, health={goodreads_health}",
        shape.label()
    );

    assert!(
        matches!(
            outbound_queue::shared()
                .acquire(
                    RateBucket::Goodreads,
                    livrarr_domain::RequestPriority::Normal
                )
                .await,
            Err(AdmissionError::CircuitOpen { .. })
        ),
        "{} must report BreakerSignal::Failure to the Goodreads bucket",
        shape.label()
    );
}

#[tokio::test]
async fn preview_403_marks_response_health_and_breaker_unavailable() {
    run_preview_failure_case(ProviderShape::Forbidden).await;
}

#[tokio::test]
async fn preview_empty_200_marks_response_health_and_breaker_unavailable() {
    run_preview_failure_case(ProviderShape::EmptyOk).await;
}
