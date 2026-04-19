use axum::extract::State;
use axum::Json;

use crate::context::AppContext;
use crate::types::api_error::ApiError;
use crate::types::auth::{AuthContext, AuthMeResponse, AuthService, LoginRequest, LoginResponse};

pub async fn login<S: AppContext>(
    State(state): State<S>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let resp = state.auth_service().login(req).await?;
    Ok(Json(resp))
}

pub async fn logout<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
) -> Result<(), ApiError> {
    if let Some(token_hash) = &ctx.session_token_hash {
        state.auth_service().logout(token_hash).await?;
    }
    Ok(())
}

pub async fn me<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
) -> Result<Json<AuthMeResponse>, ApiError> {
    let resp = state.auth_service().get_current_user(&ctx).await?;
    Ok(Json(resp))
}
