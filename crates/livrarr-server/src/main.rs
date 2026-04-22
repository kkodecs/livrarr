use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tokio::net::TcpListener;
use tracing::{error, info, warn};

use livrarr_server::config::{AppConfig, LogFormat, LogLevel};
use livrarr_server::router::build_router;
use livrarr_server::state::{AppState, ProviderHealthState};

/// Validate an LLM endpoint URL at startup (best-effort, non-fatal).
fn validate_llm_endpoint_startup(endpoint: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(endpoint).map_err(|e| format!("invalid URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(format!("unsupported scheme: {other}")),
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("URL contains embedded credentials".into());
    }
    if let Some(host) = parsed.host_str() {
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            if livrarr_http::ssrf::is_private_ip(ip) {
                return Err("URL points to a private IP address".into());
            }
        }
    }
    Ok(())
}

/// Livrarr — self-hosted ebook and audiobook library manager.
#[derive(Parser)]
#[command(name = "livrarr", version)]
struct Cli {
    /// Data directory (config, database, covers).
    #[arg(long, default_value = "./data")]
    data: PathBuf,

    /// UI assets directory. Defaults to {data}/ui when not set.
    #[arg(long)]
    ui_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let data_dir = cli.data;
    let ui_dir = cli.ui_dir.unwrap_or_else(|| data_dir.join("ui"));

    // Step 1: Ensure data directory exists.
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        eprintln!(
            "Failed to create data directory {}: {e}",
            data_dir.display()
        );
        std::process::exit(1);
    }

    // Step 2: Read config.toml.
    let config = match load_config(&data_dir) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Configuration error: {e}");
            std::process::exit(1);
        }
    };

    // Step 3: Initialize tracing.
    let log_buffer = Arc::new(livrarr_server::state::LogBuffer::new());
    let log_level_handle = init_tracing(&config.log, log_buffer.clone(), &data_dir);

    info!("Livrarr starting — data directory: {}", data_dir.display());

    // Step 4: Permission check — verify data dir is writable.
    if let Err(e) = livrarr_db::pool::check_data_dir_permissions(&data_dir) {
        error!("{e}");
        std::process::exit(1);
    }

    // Step 5: PID lock — ensure single instance.
    if let Err(e) = livrarr_db::pool::acquire_pid_lock(&data_dir) {
        error!("{e}");
        std::process::exit(1);
    }

    // Step 6: Connect to SQLite.
    let pool = match livrarr_db::pool::create_sqlite_pool(&data_dir).await {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to connect to SQLite: {e}");
            livrarr_db::pool::release_pid_lock(&data_dir);
            std::process::exit(1);
        }
    };

    // Step 7: Pre-migration backup (only if DB file already exists).
    let db_path = data_dir.join("livrarr.db");
    let db_exists = tokio::fs::try_exists(&db_path).await.unwrap_or(false);
    if db_exists {
        match livrarr_db::pool::create_backup(&pool, &data_dir).await {
            Ok(_) => {}
            Err(e) => {
                error!("Pre-migration backup failed: {e}");
                livrarr_db::pool::release_pid_lock(&data_dir);
                std::process::exit(1);
            }
        }
    }

    // Step 8: Run migrations.
    if let Err(e) = livrarr_db::pool::run_migrations(&pool).await {
        error!("Migration failed: {e}");
        livrarr_db::pool::release_pid_lock(&data_dir);
        std::process::exit(1);
    }
    info!("Database migrations complete");

    // Step 9: Version gate — verify DB compatibility.
    if let Err(e) = livrarr_db::pool::check_version_gate(&pool).await {
        error!("{e}");
        livrarr_db::pool::release_pid_lock(&data_dir);
        std::process::exit(1);
    }

    // Step 10: Clean up old backups (keep 3).
    {
        let data_dir_clone = data_dir.clone();
        tokio::task::spawn_blocking(move || {
            livrarr_db::pool::cleanup_old_backups(&data_dir_clone, 3);
        })
        .await
        .ok();
    }

    // Construct AppState.
    let db = livrarr_db::sqlite::SqliteDb::new(pool);
    let auth_service = Arc::new(livrarr_server::auth_service::ServerAuthService::new(
        db.clone(),
        livrarr_server::auth_crypto::RealAuthCrypto,
    ));
    let http_client = livrarr_http::HttpClient::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(&format!("Livrarr/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("failed to build HTTP client");
    let http_client_safe = livrarr_http::HttpClient::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(&format!("Livrarr/{}", env!("CARGO_PKG_VERSION")))
        .ssrf_safe(true)
        .build()
        .expect("failed to build SSRF-safe HTTP client");
    let job_runner = livrarr_server::jobs::JobRunner::new();

    // Phase 1.5 plumbing: build the live DefaultProviderQueue + EnrichmentServiceImpl
    // from a startup-time snapshot of MetadataConfig. Live config changes (token
    // added, URL changed) require a server restart for now — runtime reload comes
    // alongside the orchestration cutover.
    // LiveMetadataConfig — all credential-dependent components hold a clone
    // of this and read fresh per call. The update_metadata_config handler
    // calls .replace() after a DB write so the new credentials are live on
    // the next enrichment without restart.
    let live_metadata_config = {
        use livrarr_db::ConfigDb;
        let initial = db.get_metadata_config().await.unwrap_or_else(|e| {
            warn!("Failed to read metadata config at startup ({e}); using defaults");
            livrarr_db::MetadataConfig {
                hardcover_enabled: false,
                hardcover_api_token: None,
                llm_enabled: false,
                llm_provider: None,
                llm_endpoint: None,
                llm_api_key: None,
                llm_model: None,
                audnexus_url: "https://api.audnex.us".to_string(),
                languages: vec!["en".to_string()],
            }
        });
        livrarr_metadata::live_config::LiveMetadataConfig::new(initial)
    };

    // Warn at startup if the configured LLM endpoint is invalid (but don't fail).
    {
        let cfg = live_metadata_config.snapshot();
        if let Some(ref endpoint) = cfg.llm_endpoint {
            if !endpoint.is_empty() {
                if let Err(reason) = validate_llm_endpoint_startup(endpoint) {
                    warn!("LLM endpoint validation: {reason} — LLM features may not work");
                }
            }
        }
    }

    let (provider_queue, enrichment_service) = {
        use livrarr_domain::MetadataProvider as P;
        use livrarr_metadata as m;

        let cfg_snapshot = live_metadata_config.snapshot();

        let queue_cfg = |provider| m::ProviderQueueConfig {
            provider,
            concurrency: 2,
            requests_per_second: 1.0,
            circuit_breaker: m::CircuitBreakerConfig {
                failure_threshold: 5,
                evaluation_window_secs: 60,
                open_duration_secs: 60,
                half_open_probe_count: 1,
            },
            max_attempts: 5,
            max_suppressed_passes: 3,
            max_suppression_window_secs: 3600,
        };

        let mut builder = m::DefaultProviderQueueBuilder::new();

        // Audnexus — always available. URL is captured at startup; if you
        // want a custom audnexus_url to take effect live too, that's a
        // small follow-up (same LiveMetadataConfig pattern).
        builder = builder.add_provider(
            P::Audnexus,
            m::ProviderClient::Audnexus(m::AudnexusClient::new(
                http_client.clone(),
                cfg_snapshot.audnexus_url.clone(),
            )),
            queue_cfg(P::Audnexus),
        );

        // OpenLibrary — always available, no credentials needed.
        builder = builder.add_provider(
            P::OpenLibrary,
            m::ProviderClient::OpenLibrary(m::OpenLibraryClient::new(http_client.clone())),
            queue_cfg(P::OpenLibrary),
        );

        // Hardcover — always registered. The client itself reads the live
        // config per-fetch; if `hardcover_enabled=false` or the token is
        // empty, it returns NotFound without a network call. Enabling HC
        // via the UI takes effect on the next enrichment.
        builder = builder.add_provider(
            P::Hardcover,
            m::ProviderClient::Hardcover(m::HardcoverClient::new(
                http_client.clone(),
                live_metadata_config.clone(),
            )),
            queue_cfg(P::Hardcover),
        );

        // Goodreads — always registered. The LLM extraction fallback for
        // foreign-language pages reads live config per-fetch.
        let gr_client = m::GoodreadsClient::production(http_client.clone())
            .with_live_config(live_metadata_config.clone());
        builder = builder.add_provider(
            P::Goodreads,
            m::ProviderClient::Goodreads(gr_client),
            queue_cfg(P::Goodreads),
        );

        let db_arc = Arc::new(db.clone());
        let queue = Arc::new(builder.build(db_arc.clone()));
        let merge_engine = Arc::new(m::DefaultMergeEngine::new(m::PriorityModel::english()));

        // LLM validator — single LiveLlmValidator that reads credentials
        // from live config per-call. When `llm_enabled=false` or
        // `llm_api_key` is empty, behaves as a pass-through (no-op).
        // Per Principle 11, LLM is value-add and never gatekeeps enrichment.
        let validator = m::llm_validator::LiveLlmValidator::new(
            http_client.clone(),
            live_metadata_config.clone(),
        );

        let service = Arc::new(m::EnrichmentServiceImpl::new(
            db_arc,
            queue.clone(),
            merge_engine,
            Arc::new(validator),
        ));
        (queue, service)
    };

    let svc_db = db.clone();
    let svc_enrichment = enrichment_service.clone();
    let import_semaphore = Arc::new(tokio::sync::Semaphore::new(2));
    let data_dir_arc = Arc::new(data_dir.clone());
    let provider_health = Arc::new(ProviderHealthState::new());
    let cover_proxy_cache = Arc::new(livrarr_server::infra::cover_cache::CoverProxyCache::new());
    let rss_last_run = Arc::new(std::sync::atomic::AtomicI64::new(0));
    let rss_sync_running = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let http_client_for_scan = http_client.clone();
    let settings_service_arc = Arc::new(
        livrarr_server::services::settings_service::LiveSettingsService::new(svc_db.clone()),
    );
    let import_io_arc = Arc::new(livrarr_server::import_io_service::ImportIoServiceImpl::new(
        svc_db.clone(),
    ));
    let ol_rate_limiter_shared = Arc::new(livrarr_server::state::OlRateLimiter::new());
    let manual_import_scans_shared = Arc::new(dashmap::DashMap::new());
    let state = AppState {
        db,
        auth_service,
        http_client,
        http_client_safe,
        config: Arc::new(config.clone()),
        data_dir: data_dir_arc.clone(),
        startup_time: chrono::Utc::now(),
        job_runner: Some(job_runner.clone()),
        provider_health: provider_health.clone(),
        cover_proxy_cache: cover_proxy_cache.clone(),
        goodreads_rate_limiter: Arc::new(livrarr_server::state::GoodreadsRateLimiter::new()),
        live_metadata_config: live_metadata_config.clone(),
        log_buffer: log_buffer.clone(),
        log_level_handle: log_level_handle.clone(),
        refresh_in_progress: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        import_semaphore: import_semaphore.clone(),
        grab_search_cache: Arc::new(livrarr_server::state::GrabSearchCache::new()),
        rss_last_run: rss_last_run.clone(),
        rss_sync_running: rss_sync_running.clone(),
        readarr_import_progress: Arc::new(tokio::sync::Mutex::new(
            livrarr_server::readarr_import_service::ReadarrImportProgress::default(),
        )),
        ol_rate_limiter: ol_rate_limiter_shared.clone(),
        manual_import_scans: manual_import_scans_shared.clone(),
        provider_queue,
        enrichment_service: enrichment_service.clone(),

        // --- Service layer (Phase 4) ---
        author_service: Arc::new(livrarr_metadata::author_service::AuthorServiceImpl::new(
            svc_db.clone(),
            livrarr_http::fetcher::HttpFetcherImpl::new()
                .expect("HttpFetcherImpl construction for author service"),
            livrarr_metadata::llm_caller_service::LlmCallerImpl::new(
                live_metadata_config.clone(),
                livrarr_http::HttpClient::builder()
                    .build()
                    .expect("LLM HttpClient"),
            ),
        )),
        series_service: Arc::new(livrarr_metadata::series_service::SeriesServiceImpl::new(
            svc_db.clone(),
        )),
        series_query_service: Arc::new(
            livrarr_metadata::series_query_service::SeriesQueryServiceImpl::new(
                svc_db.clone(),
                livrarr_http::fetcher::HttpFetcherImpl::new()
                    .expect("HttpFetcherImpl construction for series query service"),
                {
                    let ew =
                        livrarr_metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                            svc_enrichment.clone(),
                            svc_db.clone(),
                        );
                    Arc::new(ew)
                },
                data_dir.clone(),
                livrarr_metadata::llm_caller_service::LlmCallerImpl::new(
                    live_metadata_config.clone(),
                    livrarr_http::HttpClient::builder()
                        .build()
                        .expect("LLM HttpClient for series query"),
                ),
            ),
        ),
        work_service: {
            let ew = livrarr_metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                svc_enrichment.clone(),
                svc_db.clone(),
            );
            Arc::new(
                livrarr_metadata::work_service::WorkServiceImpl::new_with_llm(
                    svc_db.clone(),
                    ew,
                    livrarr_http::fetcher::HttpFetcherImpl::new()
                        .expect("HttpFetcherImpl construction for work service"),
                    livrarr_metadata::llm_caller_service::LlmCallerImpl::new(
                        live_metadata_config.clone(),
                        livrarr_http::HttpClient::builder()
                            .build()
                            .expect("LLM HttpClient for work service"),
                    ),
                    data_dir.clone(),
                ),
            )
        },
        grab_service: Arc::new(livrarr_download::grab_service::GrabServiceImpl::new(
            svc_db.clone(),
        )),
        release_service: Arc::new(livrarr_download::release_service::ReleaseServiceImpl::new(
            svc_db.clone(),
            livrarr_http::fetcher::HttpFetcherImpl::new().expect("HttpFetcherImpl construction"),
        )),
        file_service: Arc::new(livrarr_library::file_service::FileServiceImpl::new(
            svc_db.clone(),
        )),
        import_workflow: Arc::new(livrarr_library::import_workflow::ImportWorkflowImpl::new(
            svc_db.clone(),
            import_semaphore.clone(),
            data_dir_arc.clone(),
        )),
        rss_sync_workflow: {
            let rs = Arc::new(livrarr_download::release_service::ReleaseServiceImpl::new(
                svc_db.clone(),
                livrarr_http::fetcher::HttpFetcherImpl::new()
                    .expect("HttpFetcherImpl construction for rss sync"),
            ));
            Arc::new(
                livrarr_metadata::rss_sync_workflow::RssSyncWorkflowImpl::new(
                    Arc::new(svc_db.clone()),
                    Arc::new(
                        livrarr_http::fetcher::HttpFetcherImpl::new()
                            .expect("HttpFetcherImpl construction for rss sync fetch"),
                    ),
                    rs,
                ),
            )
        },
        list_service: {
            let ew = livrarr_metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                svc_enrichment.clone(),
                svc_db.clone(),
            );
            let ws = livrarr_metadata::work_service::WorkServiceImpl::new_with_llm(
                svc_db.clone(),
                ew,
                livrarr_http::fetcher::HttpFetcherImpl::new()
                    .expect("HttpFetcherImpl construction for list work service"),
                livrarr_metadata::llm_caller_service::LlmCallerImpl::new(
                    live_metadata_config.clone(),
                    livrarr_http::HttpClient::builder()
                        .build()
                        .expect("LLM HttpClient for list service"),
                ),
                data_dir.clone(),
            );
            Arc::new(livrarr_metadata::list_service::ListServiceImpl::new(
                svc_db.clone(),
                ws,
                livrarr_http::fetcher::HttpFetcherImpl::new()
                    .expect("HttpFetcherImpl construction for list service"),
                livrarr_metadata::list_service::NoOpBibliographyTrigger,
            ))
        },
        enrichment_workflow: Arc::new(
            livrarr_metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                svc_enrichment.clone(),
                svc_db.clone(),
            ),
        ),
        author_monitor_workflow: {
            let ew = livrarr_metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                svc_enrichment.clone(),
                svc_db.clone(),
            );
            let ws = livrarr_metadata::work_service::WorkServiceImpl::new_with_llm(
                svc_db.clone(),
                ew,
                livrarr_http::fetcher::HttpFetcherImpl::new()
                    .expect("HttpFetcherImpl construction for author monitor work service"),
                livrarr_metadata::llm_caller_service::LlmCallerImpl::new(
                    live_metadata_config.clone(),
                    livrarr_http::HttpClient::builder()
                        .build()
                        .expect("LLM HttpClient for author monitor"),
                ),
                data_dir.clone(),
            );
            Arc::new(
                livrarr_metadata::author_monitor_workflow::AuthorMonitorWorkflowImpl::new(
                    Arc::new(svc_db.clone()),
                    Arc::new(ws),
                    Arc::new(
                        livrarr_http::fetcher::HttpFetcherImpl::new()
                            .expect("HttpFetcherImpl construction for author monitor"),
                    ),
                ),
            )
        },
        readarr_import_service: Arc::new(
            livrarr_server::readarr_import_service::LiveReadarrImportService::new(svc_db.clone()),
        ),
        settings_service: settings_service_arc.clone(),
        notification_service: Arc::new(
            livrarr_server::notification_service::NotificationServiceImpl::new(svc_db.clone()),
        ),
        history_service: Arc::new(livrarr_server::history_service::HistoryServiceImpl::new(
            svc_db.clone(),
        )),
        queue_service: Arc::new(livrarr_server::queue_service::QueueServiceImpl::new(
            svc_db.clone(),
            http_client_for_scan.clone(),
        )),
        import_io_service: import_io_arc.clone(),
        manual_import_db_service: Arc::new(
            livrarr_server::manual_import_service::ManualImportServiceImpl::new(svc_db.clone()),
        ),

        // --- Phase 5: infrastructure accessors (share Arcs with fields above) ---
        rss_sync_state: livrarr_server::state::RssSyncState {
            running: rss_sync_running.clone(),
            last_run: rss_last_run.clone(),
        },
        system_state: livrarr_server::state::SystemState {
            log_buffer: log_buffer.clone(),
            log_level_handle: log_level_handle.clone(),
        },
        provider_health_accessor: livrarr_server::state::ProviderHealthAccessorImpl(
            provider_health.clone(),
        ),
        live_metadata_config_accessor: livrarr_server::state::LiveMetadataConfigAccessorImpl(
            live_metadata_config.clone(),
        ),
        cover_proxy_cache_accessor: livrarr_server::state::CoverProxyCacheAccessorImpl(
            cover_proxy_cache.clone(),
        ),
        tag_service: Arc::new(livrarr_server::tag_service::LiveTagService::new(
            import_io_arc.clone(),
            data_dir_arc.clone(),
        )),
        email_svc: Arc::new(livrarr_server::email_service::LiveEmailService::new(
            settings_service_arc.clone(),
        )),
        import_svc: Arc::new(livrarr_server::import_service::LiveImportService::new()),
        matching_svc: livrarr_server::matching_service::LiveMatchingService,
        manual_import_scan_svc:
            livrarr_server::manual_import_scan_service::LiveManualImportScanService {
                scans: manual_import_scans_shared.clone(),
                ol_rate_limiter: ol_rate_limiter_shared.clone(),
                http_client: http_client_for_scan,
            },
        readarr_import_wf: Arc::new(
            livrarr_server::readarr_import_workflow::LiveReadarrImportWorkflow::new(),
        ),
        enrichment_notify: Arc::new(tokio::sync::Notify::new()),
    };

    // Late-init: wire services that need AppState (breaks circular dep via OnceLock<Box<AppState>>).
    state.import_svc.init(state.clone());
    state.readarr_import_wf.init(state.clone());

    // Step 7: Startup recovery — reset stale state from unclean shutdown (JOBS-003).
    livrarr_server::jobs::recover_interrupted_state(&state).await;

    // Pre-warm SQLite page cache so first request isn't slow.
    let _ = sqlx::query("SELECT COUNT(*) FROM works")
        .fetch_one(state.db.pool())
        .await;
    let _ = sqlx::query("SELECT COUNT(*) FROM library_items")
        .fetch_one(state.db.pool())
        .await;

    // Step 8: Start background jobs (JOBS-001).
    job_runner.start(state.clone()).await;

    // Step 9: Build router.
    let app = build_router(state, ui_dir);

    // Step 10: Bind HTTP server.
    let addr = format!("{}:{}", config.server.bind_address, config.server.port);
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            error!("Failed to bind to {addr}: {e}");
            std::process::exit(1);
        }
    };

    info!("Listening on {addr}");

    // Step 9: Serve with graceful shutdown on SIGTERM/Ctrl+C.
    // Cancel background jobs immediately when signal fires (before HTTP drain).
    let job_cancel = job_runner.cancel_token();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        info!("Cancelling background jobs");
        job_cancel.cancel();
    })
    .await
    .unwrap_or_else(|e| {
        error!("Server error: {e}");
        std::process::exit(1);
    });

    // Await job completion (cancel already signalled above).
    job_runner.shutdown().await;

    livrarr_db::pool::release_pid_lock(&data_dir);
    info!("Livrarr stopped");
}

fn load_config(data_dir: &std::path::Path) -> Result<AppConfig, String> {
    let config_path = data_dir.join("config.toml");

    let config: AppConfig = if config_path.exists() {
        let raw = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("failed to read config.toml: {e}"))?;

        if raw.trim().is_empty() {
            AppConfig::default()
        } else {
            // Parse for unknown key warnings.
            if let Ok(val) = raw.parse::<toml::Value>() {
                livrarr_server::config::warn_unknown_keys(&val);
            }

            toml::from_str(&raw).map_err(|e| format!("failed to parse config.toml: {e}"))?
        }
    } else {
        AppConfig::default()
    };

    livrarr_server::config::validate_config(&config).map_err(|e| e.to_string())?;
    Ok(config)
}

fn init_tracing(
    log: &livrarr_server::config::LogConfig,
    log_buffer: Arc<livrarr_server::state::LogBuffer>,
    data_dir: &std::path::Path,
) -> Arc<livrarr_server::state::LogLevelHandle> {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::EnvFilter;

    let level = match log.level {
        LogLevel::Trace => "trace",
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    };

    let filter = EnvFilter::try_new(format!("livrarr={level},tower_http={level}"))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let (filter, reload_handle) = tracing_subscriber::reload::Layer::new(filter);

    // Console output — text or JSON per config.
    let use_json = log.format == LogFormat::Json;
    let fmt_layer: Box<dyn tracing_subscriber::Layer<_> + Send + Sync> = if use_json {
        Box::new(tracing_subscriber::fmt::layer().json().with_target(false))
    } else {
        Box::new(tracing_subscriber::fmt::layer().with_target(false))
    };

    // In-memory ring buffer for UI
    let buf_layer = LogBufferLayer(log_buffer);

    // File output: {data_dir}/logs/livrarr.txt (Servarr convention)
    let log_dir = data_dir.join("logs");
    std::fs::create_dir_all(&log_dir).ok();
    let file_appender = tracing_appender::rolling::daily(&log_dir, "livrarr.log");
    let file_layer: Box<dyn tracing_subscriber::Layer<_> + Send + Sync> = if use_json {
        Box::new(
            tracing_subscriber::fmt::layer()
                .json()
                .with_target(false)
                .with_ansi(false)
                .with_writer(file_appender),
        )
    } else {
        Box::new(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_ansi(false)
                .with_writer(file_appender),
        )
    };

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(file_layer)
        .with(buf_layer)
        .init();

    Arc::new(livrarr_server::state::LogLevelHandle::new(
        reload_handle,
        level,
    ))
}

/// Tracing layer that captures formatted log lines into a shared ring buffer.
struct LogBufferLayer(Arc<livrarr_server::state::LogBuffer>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for LogBufferLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let meta = event.metadata();
        let mut message = String::new();
        let mut visitor = MessageVisitor(&mut message);
        event.record(&mut visitor);
        let line = format!(
            "{} {:>5} {}",
            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
            meta.level(),
            message,
        );
        self.0.push(line);
    }
}

struct MessageVisitor<'a>(&'a mut String);

impl tracing::field::Visit for MessageVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        if field.name() == "message" {
            let _ = write!(self.0, "{:?}", value);
        } else if !self.0.is_empty() {
            let _ = write!(self.0, " {}={:?}", field.name(), value);
        } else {
            let _ = write!(self.0, "{}={:?}", field.name(), value);
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        use std::fmt::Write;
        if field.name() == "message" {
            let _ = write!(self.0, "{}", value);
        } else if !self.0.is_empty() {
            let _ = write!(self.0, " {}={}", field.name(), value);
        } else {
            let _ = write!(self.0, "{}={}", field.name(), value);
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to register SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => {},
            _ = sigterm.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
    }
    info!("Shutdown signal received");
}
