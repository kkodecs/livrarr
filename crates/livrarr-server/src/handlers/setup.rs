use axum::extract::State;
use axum::Json;

use crate::state::AppState;
use crate::{ApiError, AuthService, SetupRequest, SetupResponse, SetupStatusResponse};
use livrarr_db::UserDb;

/// GET /api/v1/setup/status
pub async fn setup_status(
    State(state): State<AppState>,
) -> Result<Json<SetupStatusResponse>, ApiError> {
    let user = state
        .db
        .get_user(1)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to check setup status: {e}")))?;
    Ok(Json(SetupStatusResponse {
        setup_required: user.setup_pending,
    }))
}

/// POST /api/v1/setup
pub async fn setup(
    State(state): State<AppState>,
    Json(req): Json<SetupRequest>,
) -> Result<Json<SetupResponse>, ApiError> {
    let resp = state.auth_service.complete_setup(req).await?;
    Ok(Json(resp))
}
