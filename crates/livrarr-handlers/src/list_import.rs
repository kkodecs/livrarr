use axum::extract::{Multipart, Path, State};
use axum::Json;
use serde::Deserialize;

use crate::context::AppContext;
use crate::types::api_error::ApiError;
use crate::types::auth::AuthContext;
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

pub async fn preview<S: AppContext>(
    State(state): State<S>,
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
        .list_service()
        .preview(ctx.user.id, bytes.to_vec())
        .await?;
    Ok(Json(result))
}

pub async fn confirm<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Json(req): Json<ConfirmRequest>,
) -> Result<Json<ListConfirmResponse>, ApiError> {
    let result = state
        .list_service()
        .confirm(
            ctx.user.id,
            &req.preview_id,
            req.import_id.as_deref(),
            &req.row_indices,
        )
        .await?;
    state.enrichment_notify().notify_one();
    Ok(Json(result))
}

pub async fn complete<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(import_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .list_service()
        .complete(ctx.user.id, &import_id)
        .await?;
    Ok(Json(serde_json::json!({ "status": "completed" })))
}

pub async fn undo<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(import_id): Path<String>,
) -> Result<Json<ListUndoResponse>, ApiError> {
    let result = state.list_service().undo(ctx.user.id, &import_id).await?;
    Ok(Json(result))
}

pub async fn list<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
) -> Result<Json<Vec<ListImportSummary>>, ApiError> {
    let imports = state.list_service().list_imports(ctx.user.id).await?;
    Ok(Json(imports))
}
