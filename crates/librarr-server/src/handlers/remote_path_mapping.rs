use axum::extract::{Path, State};
use axum::Json;

use crate::state::AppState;
use crate::{
    ApiError, CreateRemotePathMappingApiRequest, RemotePathMappingResponse,
    UpdateRemotePathMappingRequest,
};
use librarr_db::RemotePathMappingDb;
use librarr_domain::RemotePathMapping;

fn to_response(m: RemotePathMapping) -> RemotePathMappingResponse {
    RemotePathMappingResponse {
        id: m.id,
        host: m.host,
        remote_path: m.remote_path,
        local_path: m.local_path,
    }
}

/// GET /api/v1/remotepathmapping
pub async fn list(
    State(state): State<AppState>,
) -> Result<Json<Vec<RemotePathMappingResponse>>, ApiError> {
    let mappings = state.db.list_remote_path_mappings().await?;
    Ok(Json(mappings.into_iter().map(to_response).collect()))
}

/// GET /api/v1/remotepathmapping/:id
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<RemotePathMappingResponse>, ApiError> {
    let m = state.db.get_remote_path_mapping(id).await?;
    Ok(Json(to_response(m)))
}

/// POST /api/v1/remotepathmapping
pub async fn create(
    State(state): State<AppState>,
    Json(req): Json<CreateRemotePathMappingApiRequest>,
) -> Result<Json<RemotePathMappingResponse>, ApiError> {
    if req.host.is_empty() {
        return Err(ApiError::BadRequest("host is required".into()));
    }
    if req.remote_path.is_empty() || req.local_path.is_empty() {
        return Err(ApiError::BadRequest("paths are required".into()));
    }

    let m = state
        .db
        .create_remote_path_mapping(&req.host, &req.remote_path, &req.local_path)
        .await?;

    Ok(Json(to_response(m)))
}

/// PUT /api/v1/remotepathmapping/:id
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateRemotePathMappingRequest>,
) -> Result<Json<RemotePathMappingResponse>, ApiError> {
    // Get existing to fill in unchanged fields.
    let existing = state.db.get_remote_path_mapping(id).await?;

    let host = req.host.unwrap_or(existing.host);
    let remote_path = req.remote_path.unwrap_or(existing.remote_path);
    let local_path = req.local_path.unwrap_or(existing.local_path);

    let m = state
        .db
        .update_remote_path_mapping(id, &host, &remote_path, &local_path)
        .await?;

    Ok(Json(to_response(m)))
}

/// DELETE /api/v1/remotepathmapping/:id
pub async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> Result<(), ApiError> {
    state.db.delete_remote_path_mapping(id).await?;
    Ok(())
}
