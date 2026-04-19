use axum::extract::{Path, State};
use axum::Json;

use crate::context::AppContext;
use crate::middleware::RequireAdmin;
use crate::types::api_error::ApiError;
use crate::types::auth::{
    AdminCreateUserRequest, AdminUpdateUserRequest, ApiKeyResponse, AuthService, UserResponse,
};

pub async fn list<S: AppContext>(
    State(state): State<S>,
    RequireAdmin(_auth): RequireAdmin,
) -> Result<Json<Vec<UserResponse>>, ApiError> {
    let users = state.auth_service().list_users().await?;
    Ok(Json(users))
}

pub async fn get<S: AppContext>(
    State(state): State<S>,
    RequireAdmin(_auth): RequireAdmin,
    Path(id): Path<i64>,
) -> Result<Json<UserResponse>, ApiError> {
    let user = state.auth_service().get_user(id).await?;
    Ok(Json(user))
}

pub async fn create<S: AppContext>(
    State(state): State<S>,
    RequireAdmin(_auth): RequireAdmin,
    Json(req): Json<AdminCreateUserRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    let user = state.auth_service().create_user(req).await?;
    Ok(Json(user))
}

pub async fn update<S: AppContext>(
    State(state): State<S>,
    RequireAdmin(_auth): RequireAdmin,
    Path(id): Path<i64>,
    Json(req): Json<AdminUpdateUserRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    let user = state.auth_service().update_user(id, req).await?;
    Ok(Json(user))
}

pub async fn delete<S: AppContext>(
    State(state): State<S>,
    RequireAdmin(auth): RequireAdmin,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    state.auth_service().delete_user(auth.user.id, id).await?;
    Ok(())
}

pub async fn regenerate_user_api_key<S: AppContext>(
    State(state): State<S>,
    RequireAdmin(_auth): RequireAdmin,
    Path(id): Path<i64>,
) -> Result<Json<ApiKeyResponse>, ApiError> {
    let resp = state.auth_service().regenerate_user_api_key(id).await?;
    Ok(Json(resp))
}
