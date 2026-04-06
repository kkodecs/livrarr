use axum::extract::State;
use axum::Json;

use crate::state::AppState;
use crate::{
    ApiError, ApiKeyResponse, AuthContext, AuthService, UpdateProfileRequest, UserResponse,
};

/// PUT /api/v1/auth/profile
pub async fn update_profile(
    State(state): State<AppState>,
    ctx: AuthContext,
    Json(req): Json<UpdateProfileRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    let user = state.auth_service.update_profile(ctx.user.id, req).await?;
    Ok(Json(user))
}

/// POST /api/v1/auth/apikey
pub async fn regenerate_api_key(
    State(state): State<AppState>,
    ctx: AuthContext,
) -> Result<Json<ApiKeyResponse>, ApiError> {
    let resp = state.auth_service.regenerate_api_key(ctx.user.id).await?;
    Ok(Json(resp))
}
