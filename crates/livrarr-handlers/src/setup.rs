use axum::extract::State;
use axum::Json;

use crate::context::AppContext;
use crate::types::api_error::ApiError;
use crate::types::auth::{AuthService, SetupRequest, SetupResponse, SetupStatusResponse};

pub async fn setup_status<S: AppContext>(
    State(state): State<S>,
) -> Result<Json<SetupStatusResponse>, ApiError> {
    let complete = state
        .auth_service()
        .is_setup_complete()
        .await
        .map_err(|e| ApiError::Internal(format!("failed to check setup status: {e}")))?;
    Ok(Json(SetupStatusResponse {
        setup_required: !complete,
    }))
}

pub async fn setup<S: AppContext>(
    State(state): State<S>,
    Json(req): Json<SetupRequest>,
) -> Result<Json<SetupResponse>, ApiError> {
    let resp = state.auth_service().complete_setup(req).await?;
    Ok(Json(resp))
}
