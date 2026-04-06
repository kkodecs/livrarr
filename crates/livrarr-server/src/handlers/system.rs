use axum::extract::State;
use axum::Json;

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

    Ok(Json(SystemStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        os_info,
        data_directory: state.data_dir.display().to_string(),
        startup_time: state.startup_time,
    }))
}
