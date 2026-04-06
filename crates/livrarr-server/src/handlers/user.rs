use axum::extract::{Path, State};
use axum::Json;

use crate::middleware::RequireAdmin;
use crate::state::AppState;
use crate::{
    AdminCreateUserRequest, AdminUpdateUserRequest, ApiError, ApiKeyResponse, AuthService,
    UserResponse,
};

/// GET /api/v1/user
pub async fn list(
    State(state): State<AppState>,
    RequireAdmin(_auth): RequireAdmin,
) -> Result<Json<Vec<UserResponse>>, ApiError> {
    let users = state.auth_service.list_users().await?;
    Ok(Json(users))
}

/// GET /api/v1/user/:id
pub async fn get(
    State(state): State<AppState>,
    RequireAdmin(_auth): RequireAdmin,
    Path(id): Path<i64>,
) -> Result<Json<UserResponse>, ApiError> {
    let user = state.auth_service.get_user(id).await?;
    Ok(Json(user))
}

/// POST /api/v1/user
pub async fn create(
    State(state): State<AppState>,
    RequireAdmin(_auth): RequireAdmin,
    Json(req): Json<AdminCreateUserRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    let user = state.auth_service.create_user(req).await?;
    Ok(Json(user))
}

/// PUT /api/v1/user/:id
pub async fn update(
    State(state): State<AppState>,
    RequireAdmin(_auth): RequireAdmin,
    Path(id): Path<i64>,
    Json(req): Json<AdminUpdateUserRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    let user = state.auth_service.update_user(id, req).await?;
    Ok(Json(user))
}

/// DELETE /api/v1/user/:id
pub async fn delete(
    State(state): State<AppState>,
    RequireAdmin(auth): RequireAdmin,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    state.auth_service.delete_user(auth.user.id, id).await?;
    Ok(())
}

/// POST /api/v1/user/:id/apikey
pub async fn regenerate_user_api_key(
    State(state): State<AppState>,
    RequireAdmin(_auth): RequireAdmin,
    Path(id): Path<i64>,
) -> Result<Json<ApiKeyResponse>, ApiError> {
    let resp = state.auth_service.regenerate_user_api_key(id).await?;
    Ok(Json(resp))
}
