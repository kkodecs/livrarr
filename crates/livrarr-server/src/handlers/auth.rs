use axum::extract::State;
use axum::Json;

use crate::state::AppState;
use crate::{ApiError, AuthContext, AuthMeResponse, AuthService, LoginRequest, LoginResponse};

/// POST /api/v1/auth/login
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let resp = state.auth_service.login(req).await?;
    Ok(Json(resp))
}

/// POST /api/v1/auth/logout
pub async fn logout(State(state): State<AppState>, ctx: AuthContext) -> Result<(), ApiError> {
    if let Some(token_hash) = &ctx.session_token_hash {
        state.auth_service.logout(token_hash).await?;
    }
    Ok(())
}

/// GET /api/v1/auth/me
pub async fn me(
    State(state): State<AppState>,
    ctx: AuthContext,
) -> Result<Json<AuthMeResponse>, ApiError> {
    let resp = state.auth_service.get_current_user(&ctx).await?;
    Ok(Json(resp))
}
