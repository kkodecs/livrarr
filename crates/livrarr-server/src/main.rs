use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{error, info};

use livrarr_db::ConfigDb;
use livrarr_metadata::registry::ProviderRegistry;
use livrarr_server::config::{AppConfig, LogLevel};
use livrarr_server::router::build_router;
use livrarr_server::state::{AppState, ProviderHealthState};

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
    if db_path.exists() {
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
    livrarr_db::pool::cleanup_old_backups(&data_dir, 3);

    // Construct AppState.
    let db = livrarr_db::sqlite::SqliteDb::new(pool);
    let auth_service = Arc::new(livrarr_server::auth_service::ServerAuthService::new(
        db.clone(),
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
    // Build provider registry from saved metadata config.
    let provider_registry = {
        let meta_cfg = match db.get_metadata_config().await {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::warn!("failed to load metadata config for provider registry: {e}");
                // Fallback: English only, no LLM
                livrarr_db::MetadataConfig {
                    hardcover_enabled: false,
                    hardcover_api_token: None,
                    llm_enabled: false,
                    llm_provider: None,
                    llm_endpoint: None,
                    llm_api_key: None,
                    llm_model: None,
                    audnexus_url: String::new(),
                    languages: vec!["en".to_string()],
                }
            }
        };
        let llm = livrarr_metadata::registry::build_llm_client(
            &http_client,
            meta_cfg.llm_enabled,
            meta_cfg.llm_endpoint.as_deref(),
            meta_cfg.llm_api_key.as_deref(),
            meta_cfg.llm_model.as_deref(),
        );
        ProviderRegistry::build(&meta_cfg.languages, llm, http_client.clone())
            .expect("failed to build provider registry")
    };
    let state = AppState {
        db,
        auth_service,
        http_client,
        http_client_safe,
        config: Arc::new(config.clone()),
        data_dir: Arc::new(data_dir.clone()),
        startup_time: chrono::Utc::now(),
        job_runner: Some(job_runner.clone()),
        provider_registry: Arc::new(RwLock::new(provider_registry)),
        provider_health: Arc::new(ProviderHealthState::new()),
        cover_proxy_cache: Arc::new(livrarr_server::handlers::coverproxy::CoverProxyCache::new()),
        detail_url_cache: Arc::new(livrarr_server::state::DetailUrlCache::new()),
        log_buffer,
        log_level_handle,
        refresh_in_progress: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        import_semaphore: Arc::new(tokio::sync::Semaphore::new(2)),
        import_locks: Arc::new(dashmap::DashMap::new()),
    };

    // Step 7: Startup recovery — reset stale state from unclean shutdown (JOBS-003).
    livrarr_server::jobs::recover_interrupted_state(&state).await;

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

    // Console output
    let fmt_layer = tracing_subscriber::fmt::layer().with_target(false);

    // In-memory ring buffer for UI
    let buf_layer = LogBufferLayer(log_buffer);

    // File output: {data_dir}/logs/livrarr.txt (Servarr convention)
    let log_dir = data_dir.join("logs");
    std::fs::create_dir_all(&log_dir).ok();
    let file_appender = tracing_appender::rolling::never(&log_dir, "livrarr.txt");
    let file_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_ansi(false)
        .with_writer(file_appender);

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
