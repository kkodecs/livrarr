use axum::extract::{Path, State};
use axum::Json;

use crate::context::HasReadarrImportWorkflow;
use crate::middleware::RequireAdmin;
use crate::ApiError;
use livrarr_domain::readarr::*;
use livrarr_domain::services::ReadarrImportWorkflow;

pub async fn connect<S: HasReadarrImportWorkflow>(
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

pub async fn preview<S: HasReadarrImportWorkflow>(
    State(state): State<S>,
    ctx: crate::AuthContext,
    Json(req): Json<ReadarrImportRequest>,
) -> Result<Json<ReadarrPreviewResponse>, ApiError> {
    state
        .readarr_import_workflow()
        .preview(ctx.user.id, req)
        .await
        .map(Json)
        .map_err(|e| ApiError::Internal(e.to_string()))
}

pub async fn start<S: HasReadarrImportWorkflow>(
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

pub async fn progress<S: HasReadarrImportWorkflow>(
    State(state): State<S>,
    ctx: crate::AuthContext,
) -> Result<Json<ReadarrImportProgress>, ApiError> {
    // Never the caller's own import id filter yet (no route param for it) —
    // this always asks "what's MY current/last run?" (Unit B3 Part 2).
    let progress = state
        .readarr_import_workflow()
        .progress(ctx.user.id, None)
        .await?;
    Ok(Json(progress))
}

pub async fn history<S: HasReadarrImportWorkflow>(
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

pub async fn undo<S: HasReadarrImportWorkflow>(
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

// ---------------------------------------------------------------------------
// Origin trust boundary (Unit B3 Part 1) — admin-managed allowlist
// ---------------------------------------------------------------------------
//
// Import itself stays open to every authenticated user (see `connect` /
// `preview` / `start` above) — only APPROVING a private origin is an admin
// action.

pub async fn list_origins<S: HasReadarrImportWorkflow>(
    State(state): State<S>,
    RequireAdmin(_auth): RequireAdmin,
) -> Result<Json<Vec<ReadarrOriginInfo>>, ApiError> {
    let origins = state.readarr_import_workflow().list_origins().await?;
    Ok(Json(origins))
}

pub async fn add_origin<S: HasReadarrImportWorkflow>(
    State(state): State<S>,
    RequireAdmin(_auth): RequireAdmin,
    Json(req): Json<AddReadarrOriginRequest>,
) -> Result<Json<ReadarrOriginInfo>, ApiError> {
    let origin = state.readarr_import_workflow().add_origin(req.url).await?;
    Ok(Json(origin))
}

pub async fn remove_origin<S: HasReadarrImportWorkflow>(
    State(state): State<S>,
    RequireAdmin(_auth): RequireAdmin,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    state.readarr_import_workflow().remove_origin(id).await?;
    Ok(())
}
