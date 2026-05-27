use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::accessors::{ProviderHealthAccessor, RssSyncAccessor, SystemAccessor};
use crate::context::{
    AppContext, HasAppConfigService, HasDataDir, HasDownloadClientSettingsService,
    HasIndexerSettingsService, HasProviderHealth, HasRssSync, HasStartupTime, HasSystem,
};
use crate::middleware::RequireAdmin;
use crate::types::api_error::ApiError;
use crate::types::system::{
    HealthCheckResult, HealthSummaryResponse, InfraItemStatus, LlmStatus, ProviderStatus,
    RssSyncStatus, SystemStatus,
};
use livrarr_domain::services::{
    AppConfigService, DownloadClientSettingsService, IndexerSettingsService,
};
use livrarr_domain::HealthCheckType;

pub async fn health<S: Clone + Send + Sync + 'static>(
    State(_state): State<S>,
) -> Result<Json<Vec<HealthCheckResult>>, ApiError> {
    Ok(Json(vec![HealthCheckResult {
        source: "database".into(),
        check_type: HealthCheckType::Ok,
        message: "database is reachable".into(),
    }]))
}

pub async fn status<S: HasDataDir + HasStartupTime + HasSystem>(
    State(state): State<S>,
    RequireAdmin(_auth): RequireAdmin,
) -> Result<Json<SystemStatus>, ApiError> {
    let os_info = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
    let log_file = state.data_dir().join("logs").join("livrarr.txt");

    Ok(Json(SystemStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        os_info,
        data_directory: state.data_dir().display().to_string(),
        log_file: log_file.display().to_string(),
        startup_time: state.startup_time(),
        log_level: state.system().current_log_level(),
    }))
}

#[derive(Deserialize)]
pub struct LogTailQuery {
    #[serde(default = "default_log_lines")]
    pub lines: usize,
}

fn default_log_lines() -> usize {
    30
}

pub async fn log_tail<S: HasSystem>(
    State(state): State<S>,
    RequireAdmin(_auth): RequireAdmin,
    Query(q): Query<LogTailQuery>,
) -> Result<Json<Vec<String>>, ApiError> {
    let n = q.lines.min(200);
    Ok(Json(state.system().log_tail(n)))
}

#[derive(Deserialize)]
pub struct SetLogLevelRequest {
    pub level: String,
}

pub async fn set_log_level<S: HasSystem>(
    State(state): State<S>,
    RequireAdmin(_auth): RequireAdmin,
    Json(req): Json<SetLogLevelRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let level = req.level.to_lowercase();
    match level.as_str() {
        "trace" | "debug" | "info" | "warn" | "error" => {}
        _ => return Err(ApiError::BadRequest(format!("invalid log level: {level}"))),
    }
    tracing::warn!("log level changing to {level}");
    state
        .system()
        .set_log_level(&level)
        .map_err(ApiError::BadRequest)?;
    Ok(Json(serde_json::json!({ "level": level })))
}

const KNOWN_PROVIDERS: &[&str] = &[
    "OpenLibrary",
    "Hardcover",
    "Google Books",
    "Goodreads",
    "Audnexus",
    "Audible",
];

pub async fn health_summary<
    S: HasProviderHealth
        + HasRssSync
        + HasAppConfigService
        + HasDownloadClientSettingsService
        + HasIndexerSettingsService,
>(
    State(state): State<S>,
    RequireAdmin(_auth): RequireAdmin,
) -> Result<Json<HealthSummaryResponse>, ApiError> {
    let provider_errors = state.provider_health().statuses().await;

    let metadata_providers = KNOWN_PROVIDERS
        .iter()
        .map(|&name| {
            let error = provider_errors.get(name).cloned();
            ProviderStatus {
                name: name.to_string(),
                status: if error.is_some() { "error" } else { "ok" },
                last_error: error,
            }
        })
        .collect();

    let rss = state.rss_sync();
    let last_run_ts = rss.last_run_at();
    let rss_sync = RssSyncStatus {
        running: rss.is_running(),
        last_run_at: if last_run_ts > 0 {
            chrono::DateTime::from_timestamp(last_run_ts, 0)
        } else {
            None
        },
    };

    let metadata_config = state
        .app_config_service()
        .get_metadata_config()
        .await
        .map_err(|e: livrarr_domain::DbError| ApiError::Internal(e.to_string()))?;

    let llm = LlmStatus {
        configured: metadata_config.llm_enabled
            && metadata_config.llm_endpoint.is_some()
            && metadata_config.llm_api_key.is_some(),
        enabled: metadata_config.llm_enabled,
        provider: metadata_config
            .llm_provider
            .map(|p| format!("{:?}", p).to_lowercase()),
        model: metadata_config.llm_model,
    };

    let download_clients = state
        .download_client_settings_service()
        .list_download_clients()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|dc| InfraItemStatus {
            id: dc.id,
            name: dc.name,
            implementation: format!("{:?}", dc.implementation).to_lowercase(),
            enabled: dc.enabled,
        })
        .collect();

    let indexers = state
        .indexer_settings_service()
        .list_indexers()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|ix| InfraItemStatus {
            id: ix.id,
            name: ix.name,
            implementation: ix.protocol.clone(),
            enabled: ix.enabled,
        })
        .collect();

    Ok(Json(HealthSummaryResponse {
        llm,
        indexers,
        download_clients,
        rss_sync,
        metadata_providers,
        library: crate::types::system::LibraryStats {
            work_count: 0,
            library_item_count: 0,
            total_size_bytes: 0,
        },
    }))
}

pub fn routes<S: AppContext>() -> Router<S> {
    Router::new().route("/health", get(health::<S>))
}
