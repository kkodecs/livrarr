use axum::extract::{Path, Query, State};
use axum::Json;

use crate::context::{HasFileService, HasHmacKey};
use crate::stream_token::{mint_stream_token, StreamTokenResponse};
use crate::types::api_error::ApiError;
use crate::types::auth::AuthContext;
use crate::types::pagination::{PaginatedResponse, PaginationQuery};
use crate::types::work::LibraryItemResponse;
use livrarr_domain::services::{FileService, ProgressKind};

fn to_response(li: &livrarr_domain::LibraryItem) -> LibraryItemResponse {
    LibraryItemResponse {
        id: li.id,
        path: li.path.clone(),
        media_type: li.media_type,
        file_size: li.file_size,
        imported_at: li.imported_at.to_rfc3339(),
        progress_pct: None,
        duration_seconds: li.duration_seconds,
        finished_at: None,
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
    /// Defaults to `Seek` when absent so stale clients can never advance the
    /// cross-format furthest mark (REQ-003).
    #[serde(default)]
    pub kind: ProgressKind,
    #[serde(default)]
    pub cross_format_ts: Option<f64>,
}

pub async fn update_progress<S: HasFileService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(body): Json<UpdateProgressRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .file_service()
        .update_progress(
            ctx.user.id,
            id,
            &body.position,
            body.progress_pct,
            body.kind,
            body.cross_format_ts,
        )
        .await?;
    Ok(Json(serde_json::json!({ "success": true })))
}

/// `POST /workfile/{id}/stream-token` (Unit C) — mint a short-lived (24h),
/// scoped stream token for this specific item. The `<audio src>` element
/// can't send an `Authorization` header, so playback needs a
/// URL-embeddable credential; this route mints one instead of exposing
/// the raw session token.
///
/// Auth-middleware-protected: `AuthContext` only exists once the request
/// has passed `auth_middleware`. `resolve_path` both confirms this item
/// belongs to this user and that it actually resolves on disk, BEFORE any
/// token is signed — a caller can never mint a token for an item they
/// don't own.
pub async fn mint_stream_token_route<S: HasFileService + HasHmacKey>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<StreamTokenResponse>, ApiError> {
    state.file_service().resolve_path(ctx.user.id, id).await?;

    let (token, exp) = mint_stream_token(state.hmac_key(), ctx.user.id, id, chrono::Utc::now());
    Ok(Json(StreamTokenResponse { token, exp }))
}
