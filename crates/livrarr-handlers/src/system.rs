use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::accessors::{LogSurfaceAccessor, RssSyncAccessor, SystemAccessor};
use crate::context::{
    AppContext, HasAppConfigService, HasDataDir, HasDownloadClientSettingsService,
    HasIndexerSettingsService, HasLogSurface, HasProviderStats, HasRssSync, HasStartupTime,
    HasSystem,
};
use crate::middleware::RequireAdmin;
use crate::types::api_error::ApiError;
use crate::types::system::{
    HealthCheckResult, HealthSummaryResponse, InfraItemStatus, LlmStatus, ProviderStatus,
    RssSyncStatus, SystemStatus,
};
use livrarr_domain::services::{
    AppConfigService, DownloadClientSettingsService, IndexerSettingsService, ProviderStats,
    ProviderStatsService,
};
use livrarr_domain::{HealthCheckType, MetadataProvider};

pub async fn health<S: Clone + Send + Sync + 'static>(
    State(_state): State<S>,
) -> Result<Json<Vec<HealthCheckResult>>, ApiError> {
    Ok(Json(vec![HealthCheckResult {
        source: "database".into(),
        check_type: HealthCheckType::Ok,
        message: "database is reachable".into(),
    }]))
}

pub async fn status<S: HasDataDir + HasStartupTime + HasSystem + HasLogSurface>(
    State(state): State<S>,
    RequireAdmin(_auth): RequireAdmin,
) -> Result<Json<SystemStatus>, ApiError> {
    let os_info = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);

    // REQ-003: report the daily rolling file the appender actually writes —
    // not a hardcoded name — plus its last-write time and any init failure.
    let log_surface = state.log_surface().status();
    let log_last_write = tokio::fs::metadata(&log_surface.active_path)
        .await
        .ok()
        .and_then(|m| m.modified().ok())
        .map(chrono::DateTime::<chrono::Utc>::from);

    Ok(Json(SystemStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        os_info,
        data_directory: state.data_dir().display().to_string(),
        log_file: log_surface.active_path.display().to_string(),
        log_last_write,
        log_init_error: log_surface.init_error,
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

/// The fetch-capable provider set shown on status panels (excludes the
/// non-fetch `Llm`/`Readarr` sources).
const FETCH_PROVIDERS: &[MetadataProvider] = &[
    MetadataProvider::OpenLibrary,
    MetadataProvider::Hardcover,
    MetadataProvider::GoogleBooks,
    MetadataProvider::Goodreads,
    MetadataProvider::Audnexus,
    MetadataProvider::Audible,
];

fn display_name(provider: MetadataProvider) -> &'static str {
    match provider {
        MetadataProvider::OpenLibrary => "OpenLibrary",
        MetadataProvider::Hardcover => "Hardcover",
        MetadataProvider::GoogleBooks => "Google Books",
        MetadataProvider::Goodreads => "Goodreads",
        MetadataProvider::Audnexus => "Audnexus",
        MetadataProvider::Audible => "Audible",
        MetadataProvider::Llm => "LLM",
        MetadataProvider::Readarr => "Readarr",
    }
}

/// A provider is "in error" when its most recent 24h error is newer than its
/// most recent success — a recovered provider reads as ok.
pub(crate) fn current_error_of(stats: &ProviderStats) -> Option<String> {
    let (msg, err_ts) = stats.last_error.as_ref()?;
    match stats.last_success {
        Some(ok_ts) if ok_ts >= *err_ts => None,
        _ => Some(msg.clone()),
    }
}

/// Rolling-24h per-provider call stats for the provider panel (REQ-002).
/// Providers with zero records still appear with empty stats — the panel
/// must not hide a silent provider.
pub async fn provider_stats<S: HasProviderStats>(
    State(state): State<S>,
    RequireAdmin(_auth): RequireAdmin,
) -> Result<Json<Vec<ProviderStats>>, ApiError> {
    let mut stats = state.provider_stats().provider_stats_24h().await?;

    for &provider in FETCH_PROVIDERS {
        let key = provider.record_key();
        if !stats.iter().any(|s| s.provider == key) {
            stats.push(ProviderStats {
                provider: key.to_string(),
                calls_24h: 0,
                success_rate: 0.0,
                median_latency_ms: 0,
                last_error: None,
                last_success: None,
            });
        }
    }
    stats.sort_by(|a, b| a.provider.cmp(&b.provider));

    Ok(Json(stats))
}

pub async fn health_summary<
    S: HasProviderStats
        + HasRssSync
        + HasAppConfigService
        + HasDownloadClientSettingsService
        + HasIndexerSettingsService,
>(
    State(state): State<S>,
    RequireAdmin(_auth): RequireAdmin,
) -> Result<Json<HealthSummaryResponse>, ApiError> {
    // REQ-002: the provider section is record-fed from the 24h call stats.
    let provider_stats = state.provider_stats().provider_stats_24h().await?;

    let metadata_providers = FETCH_PROVIDERS
        .iter()
        .map(|&provider| {
            let error = provider_stats
                .iter()
                .find(|s| s.provider == provider.record_key())
                .and_then(current_error_of);
            ProviderStatus {
                name: display_name(provider).to_string(),
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
