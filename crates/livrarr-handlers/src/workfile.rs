use axum::extract::{Path, Query, State};
use axum::Json;

use crate::context::HasFileService;
use crate::types::api_error::ApiError;
use crate::types::auth::AuthContext;
use crate::types::pagination::{PaginatedResponse, PaginationQuery};
use crate::types::work::LibraryItemResponse;
use livrarr_domain::services::FileService;

fn to_response(li: &livrarr_domain::LibraryItem) -> LibraryItemResponse {
    LibraryItemResponse {
        id: li.id,
        path: li.path.clone(),
        media_type: li.media_type,
        file_size: li.file_size,
        imported_at: li.imported_at.to_rfc3339(),
    }
}

pub async fn list<S: HasFileService>(
    State(state): State<S>,
    ctx: AuthContext,
    Query(pq): Query<PaginationQuery>,
) -> Result<Json<PaginatedResponse<LibraryItemResponse>>, ApiError> {
    let page = pq.page();
    let page_size = pq.page_size();
    let (items, total) = state
        .file_service()
        .list_paginated(ctx.user.id, page, page_size)
        .await?;
    Ok(Json(PaginatedResponse {
        items: items.iter().map(to_response).collect(),
        total,
        page,
        page_size,
    }))
}

pub async fn get<S: HasFileService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<LibraryItemResponse>, ApiError> {
    let item = state.file_service().get(ctx.user.id, id).await?;
    Ok(Json(to_response(&item)))
}

pub async fn delete<S: HasFileService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    state.file_service().delete(ctx.user.id, id).await?;
    Ok(())
}

pub async fn get_progress<S: HasFileService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let progress = state.file_service().get_progress(ctx.user.id, id).await?;
    match progress {
        Some(p) => Ok(Json(serde_json::json!({
            "library_item_id": p.library_item_id,
            "position": p.position,
            "progress_pct": p.progress_pct,
            "updated_at": p.updated_at.to_rfc3339(),
        }))),
        None => Err(ApiError::NotFound),
    }
}

#[derive(serde::Deserialize)]
pub struct UpdateProgressRequest {
    pub position: String,
    pub progress_pct: f64,
}

pub async fn update_progress<S: HasFileService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(body): Json<UpdateProgressRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .file_service()
        .update_progress(ctx.user.id, id, &body.position, body.progress_pct)
        .await?;
    Ok(Json(serde_json::json!({ "success": true })))
}
