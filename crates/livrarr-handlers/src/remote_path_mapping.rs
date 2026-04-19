use axum::extract::{Path, State};
use axum::Json;

use crate::context::AppContext;
use crate::middleware::RequireAdmin;
use crate::types::api_error::ApiError;
use crate::types::remote_path_mapping::{
    CreateRemotePathMappingApiRequest, RemotePathMappingResponse, UpdateRemotePathMappingRequest,
};
use livrarr_domain::services::SettingsService;
use livrarr_domain::RemotePathMapping;

fn to_response(m: RemotePathMapping) -> RemotePathMappingResponse {
    RemotePathMappingResponse {
        id: m.id,
        host: m.host,
        remote_path: m.remote_path,
        local_path: m.local_path,
    }
}

pub async fn list<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
) -> Result<Json<Vec<RemotePathMappingResponse>>, ApiError> {
    let mappings = state.settings_service().list_remote_path_mappings().await?;
    Ok(Json(mappings.into_iter().map(to_response).collect()))
}

pub async fn get<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
) -> Result<Json<RemotePathMappingResponse>, ApiError> {
    let m = state.settings_service().get_remote_path_mapping(id).await?;
    Ok(Json(to_response(m)))
}

pub async fn create<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Json(req): Json<CreateRemotePathMappingApiRequest>,
) -> Result<Json<RemotePathMappingResponse>, ApiError> {
    if req.host.is_empty() {
        return Err(ApiError::BadRequest("host is required".into()));
    }
    if req.remote_path.is_empty() || req.local_path.is_empty() {
        return Err(ApiError::BadRequest("paths are required".into()));
    }

    let m = state
        .settings_service()
        .create_remote_path_mapping(&req.host, &req.remote_path, &req.local_path)
        .await?;

    Ok(Json(to_response(m)))
}

pub async fn update<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
    Json(req): Json<UpdateRemotePathMappingRequest>,
) -> Result<Json<RemotePathMappingResponse>, ApiError> {
    let existing = state.settings_service().get_remote_path_mapping(id).await?;

    let host = req.host.unwrap_or(existing.host);
    let remote_path = req.remote_path.unwrap_or(existing.remote_path);
    let local_path = req.local_path.unwrap_or(existing.local_path);

    let m = state
        .settings_service()
        .update_remote_path_mapping(id, &host, &remote_path, &local_path)
        .await?;

    Ok(Json(to_response(m)))
}

pub async fn delete<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    state
        .settings_service()
        .delete_remote_path_mapping(id)
        .await?;
    Ok(())
}
