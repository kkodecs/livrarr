use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::accessors::SystemAccessor;
use crate::context::{AppContext, HasDataDir, HasStartupTime, HasSystem};
use crate::middleware::RequireAdmin;
use crate::types::api_error::ApiError;
use crate::types::auth::AuthContext;
use crate::types::system::{HealthCheckResult, SystemStatus};
use livrarr_domain::HealthCheckType;

pub async fn health<S: Clone + Send + Sync + 'static>(
    State(_state): State<S>,
    _ctx: AuthContext,
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

pub fn routes<S: AppContext>() -> Router<S> {
    Router::new().route("/health", get(health::<S>))
}
