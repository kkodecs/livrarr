use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tokio::net::TcpListener;
use tracing::{error, info};

use livrarr_server::config::{AppConfig, LogLevel};
use livrarr_server::router::build_router;
use livrarr_server::state::AppState;

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
    init_tracing(&config.log);

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
    let job_runner = livrarr_server::jobs::JobRunner::new();
    let state = AppState {
        db,
        auth_service,
        http_client,
        config: Arc::new(config.clone()),
        data_dir: Arc::new(data_dir.clone()),
        startup_time: chrono::Utc::now(),
        job_runner: Some(job_runner.clone()),
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
    axum::serve(listener, app)
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

fn init_tracing(log: &livrarr_server::config::LogConfig) {
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

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
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
