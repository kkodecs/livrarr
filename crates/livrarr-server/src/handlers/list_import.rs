//! List import handler — CSV imports from Goodreads and Hardcover.
//!
//! Thin handlers: validate → call ListService → map result.

use axum::extract::{Multipart, Path, State};
use axum::Json;
use serde::Deserialize;

use crate::state::AppState;
use crate::{ApiError, AuthContext};
use livrarr_domain::services::{
    ListConfirmResponse, ListImportSummary, ListPreviewResponse, ListService, ListUndoResponse,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmRequest {
    pub preview_id: String,
    pub row_indices: Vec<usize>,
    pub import_id: Option<String>,
}

/// POST /api/v1/listimport/preview
pub async fn preview(
    State(state): State<AppState>,
    ctx: AuthContext,
    mut multipart: Multipart,
) -> Result<Json<ListPreviewResponse>, ApiError> {
    let field = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("multipart error: {e}")))?
        .ok_or_else(|| ApiError::BadRequest("no file uploaded".into()))?;

    let bytes = field
        .bytes()
        .await
        .map_err(|e| ApiError::BadRequest(format!("failed to read upload: {e}")))?;

    if bytes.len() > 20 * 1024 * 1024 {
        return Err(ApiError::BadRequest("file too large (max 20MB)".into()));
    }
    if bytes.is_empty() {
        return Err(ApiError::BadRequest("uploaded file is empty".into()));
    }

    let result = state
        .list_service
        .preview(ctx.user.id, bytes.to_vec())
        .await?;
    Ok(Json(result))
}

/// POST /api/v1/listimport/confirm
pub async fn confirm(
    State(state): State<AppState>,
    ctx: AuthContext,
    Json(req): Json<ConfirmRequest>,
) -> Result<Json<ListConfirmResponse>, ApiError> {
    let result = state
        .list_service
        .confirm(
            ctx.user.id,
            &req.preview_id,
            req.import_id.as_deref(),
            &req.row_indices,
        )
        .await?;
    Ok(Json(result))
}

/// POST /api/v1/listimport/{import_id}/complete
pub async fn complete(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(import_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state.list_service.complete(ctx.user.id, &import_id).await?;
    Ok(Json(serde_json::json!({ "status": "completed" })))
}

/// DELETE /api/v1/listimport/{import_id}
pub async fn undo(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(import_id): Path<String>,
) -> Result<Json<ListUndoResponse>, ApiError> {
    let result = state.list_service.undo(ctx.user.id, &import_id).await?;
    Ok(Json(result))
}

/// GET /api/v1/listimport
pub async fn list(
    State(state): State<AppState>,
    ctx: AuthContext,
) -> Result<Json<Vec<ListImportSummary>>, ApiError> {
    let imports = state.list_service.list_imports(ctx.user.id).await?;
    Ok(Json(imports))
}
