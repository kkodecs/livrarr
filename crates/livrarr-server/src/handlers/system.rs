use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;

use crate::middleware::RequireAdmin;
use crate::state::AppState;
use crate::{ApiError, AuthContext, HealthCheckResult, HealthCheckType, SystemStatus};

/// GET /api/v1/health
pub async fn health(
    State(_state): State<AppState>,
    _ctx: AuthContext,
) -> Result<Json<Vec<HealthCheckResult>>, ApiError> {
    // Basic health check — just report database as OK.
    // In production, this would check Prowlarr, download clients, etc.
    Ok(Json(vec![HealthCheckResult {
        source: "database".into(),
        check_type: HealthCheckType::Ok,
        message: "database is reachable".into(),
    }]))
}

/// GET /api/v1/system/status
pub async fn status(
    State(state): State<AppState>,
    RequireAdmin(_auth): RequireAdmin,
) -> Result<Json<SystemStatus>, ApiError> {
    let os_info = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);

    let log_file = state.data_dir.join("logs").join("livrarr.txt");

    Ok(Json(SystemStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        os_info,
        data_directory: state.data_dir.display().to_string(),
        log_file: log_file.display().to_string(),
        startup_time: state.startup_time,
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

/// GET /api/v1/system/logs/tail?lines=30
pub async fn log_tail(
    State(state): State<AppState>,
    RequireAdmin(_auth): RequireAdmin,
    Query(q): Query<LogTailQuery>,
) -> Result<Json<Vec<String>>, ApiError> {
    let n = q.lines.min(200);
    Ok(Json(state.log_buffer.tail(n)))
}

#[derive(Deserialize)]
pub struct SetLogLevelRequest {
    pub level: String,
}

/// PUT /api/v1/system/logs/level
pub async fn set_log_level(
    State(state): State<AppState>,
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
        .log_level_handle
        .set_level(&level)
        .map_err(|e| ApiError::BadRequest(e))?;
    Ok(Json(serde_json::json!({ "level": level })))
}
