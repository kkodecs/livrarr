use axum::extract::State;
use axum::Json;

use crate::context::AppContext;
use crate::types::api_error::ApiError;
use crate::types::auth::{
    ApiKeyResponse, AuthContext, AuthService, UpdateProfileRequest, UserResponse,
};

pub async fn update_profile<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Json(req): Json<UpdateProfileRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    let user = state
        .auth_service()
        .update_profile(ctx.user.id, req)
        .await?;
    Ok(Json(user))
}

pub async fn regenerate_api_key<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
) -> Result<Json<ApiKeyResponse>, ApiError> {
    let resp = state.auth_service().regenerate_api_key(ctx.user.id).await?;
    Ok(Json(resp))
}
