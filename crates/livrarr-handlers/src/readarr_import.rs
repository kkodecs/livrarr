use axum::extract::{Path, State};
use axum::Json;

use crate::context::AppContext;
use crate::ApiError;
use livrarr_domain::readarr::*;
use livrarr_domain::services::ReadarrImportWorkflow;

pub async fn connect<S: AppContext>(
    State(state): State<S>,
    _ctx: crate::AuthContext,
    Json(req): Json<ReadarrConnectRequest>,
) -> Result<Json<ReadarrConnectResponse>, ApiError> {
    state
        .readarr_import_workflow()
        .connect(req)
        .await
        .map(Json)
        .map_err(|e| ApiError::Internal(e.to_string()))
}

pub async fn preview<S: AppContext>(
    State(state): State<S>,
    _ctx: crate::AuthContext,
    Json(req): Json<ReadarrImportRequest>,
) -> Result<Json<ReadarrPreviewResponse>, ApiError> {
    state
        .readarr_import_workflow()
        .preview(req)
        .await
        .map(Json)
        .map_err(|e| ApiError::Internal(e.to_string()))
}

pub async fn start<S: AppContext>(
    State(state): State<S>,
    ctx: crate::AuthContext,
    Json(req): Json<ReadarrImportRequest>,
) -> Result<Json<ReadarrStartResponse>, ApiError> {
    state
        .readarr_import_workflow()
        .start(ctx.user.id, req)
        .await
        .map(Json)
        .map_err(|e| ApiError::Internal(e.to_string()))
}

pub async fn progress<S: AppContext>(
    State(state): State<S>,
    _ctx: crate::AuthContext,
) -> Json<ReadarrImportProgress> {
    Json(state.readarr_import_workflow().progress().await)
}

pub async fn history<S: AppContext>(
    State(state): State<S>,
    ctx: crate::AuthContext,
) -> Result<Json<ReadarrHistoryResponse>, ApiError> {
    state
        .readarr_import_workflow()
        .history(ctx.user.id)
        .await
        .map(Json)
        .map_err(|e| ApiError::Internal(e.to_string()))
}

pub async fn undo<S: AppContext>(
    State(state): State<S>,
    ctx: crate::AuthContext,
    Path(import_id): Path<String>,
) -> Result<Json<ReadarrUndoResponse>, ApiError> {
    state
        .readarr_import_workflow()
        .undo(ctx.user.id, import_id)
        .await
        .map(Json)
        .map_err(|e| ApiError::Internal(e.to_string()))
}
