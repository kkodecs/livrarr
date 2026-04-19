use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::state::AppState;
use crate::{ApiError, AuthContext, LibraryItemResponse, PaginatedResponse, PaginationQuery};
use livrarr_db::ConfigDb;
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

/// GET /api/v1/workfile
pub async fn list(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(pq): Query<PaginationQuery>,
) -> Result<Json<PaginatedResponse<LibraryItemResponse>>, ApiError> {
    let page = pq.page();
    let page_size = pq.page_size();
    let (items, total) = state
        .file_service
        .list_paginated(ctx.user.id, page, page_size)
        .await?;
    Ok(Json(PaginatedResponse {
        items: items.iter().map(to_response).collect(),
        total,
        page,
        page_size,
    }))
}

/// GET /api/v1/workfile/:id
pub async fn get(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<LibraryItemResponse>, ApiError> {
    let item = state.file_service.get(ctx.user.id, id).await?;
    Ok(Json(to_response(&item)))
}

/// DELETE /api/v1/workfile/:id
pub async fn delete(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    state.file_service.delete(ctx.user.id, id).await?;
    Ok(())
}

/// POST /api/v1/workfile/:id/send-email
pub async fn send_email(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let payload = state.file_service.prepare_email(ctx.user.id, id).await?;

    let cfg = state.db.get_email_config().await?;

    super::email::send_file(
        &cfg,
        payload.file_bytes,
        &payload.filename,
        &payload.extension,
    )
    .await
    .map_err(|e| {
        tracing::error!("Email send failed: {e}");
        ApiError::Internal(e)
    })?;

    tracing::info!(file = %payload.filename, "Email sent");
    Ok(Json(serde_json::json!({ "success": true })))
}

fn mime_for_ext(ext: &str) -> &'static str {
    match ext {
        "epub" => "application/epub+zip",
        "pdf" => "application/pdf",
        "mobi" => "application/x-mobipocket-ebook",
        "azw3" => "application/x-mobi8-ebook",
        "m4b" | "m4a" => "audio/mp4",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        _ => "application/octet-stream",
    }
}

/// GET /api/v1/workfile/:id/download
pub async fn download(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    req: axum::http::Request<axum::body::Body>,
) -> Result<Response, ApiError> {
    let path = state.file_service.resolve_path(ctx.user.id, id).await?;

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let content_type = mime_for_ext(&ext);

    use tower::Service;
    use tower_http::services::ServeFile;
    let mut svc = ServeFile::new(&path);
    let resp = svc
        .call(req)
        .await
        .map_err(|e| ApiError::Internal(format!("File serve error: {e}")))?;

    let (mut parts, body) = resp.into_response().into_parts();
    parts.headers.insert(
        axum::http::header::CONTENT_TYPE,
        content_type.parse().unwrap(),
    );
    Ok(Response::from_parts(parts, body))
}

/// GET /api/v1/stream/:id?token=<bearer_token>
pub async fn stream(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(params): Query<StreamQuery>,
    req: axum::http::Request<axum::body::Body>,
) -> Result<Response, ApiError> {
    let token = params.token.as_deref().ok_or(ApiError::Unauthorized)?;

    use crate::auth_crypto::{AuthCryptoService, RealAuthCrypto};
    let crypto = RealAuthCrypto;
    let token_hash = crypto
        .hash_token(token)
        .await
        .map_err(|_| ApiError::Unauthorized)?;

    use livrarr_db::SessionDb;
    let session = state
        .db
        .get_session(&token_hash)
        .await
        .map_err(|_| ApiError::Unauthorized)?
        .ok_or(ApiError::Unauthorized)?;

    let path = state.file_service.resolve_path(session.user_id, id).await?;

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let content_type = mime_for_ext(&ext);

    use tower::Service;
    use tower_http::services::ServeFile;
    let mut svc = ServeFile::new(&path);
    let resp = svc
        .call(req)
        .await
        .map_err(|e| ApiError::Internal(format!("File serve error: {e}")))?;

    let (mut parts, body) = resp.into_response().into_parts();
    parts.headers.insert(
        axum::http::header::CONTENT_TYPE,
        content_type.parse().unwrap(),
    );
    Ok(Response::from_parts(parts, body))
}

#[derive(serde::Deserialize)]
pub struct StreamQuery {
    pub token: Option<String>,
}

/// GET /api/v1/workfile/:id/progress
pub async fn get_progress(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let progress = state.file_service.get_progress(ctx.user.id, id).await?;
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

/// PUT /api/v1/workfile/:id/progress
pub async fn update_progress(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(body): Json<UpdateProgressRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .file_service
        .update_progress(ctx.user.id, id, &body.position, body.progress_pct)
        .await?;
    Ok(Json(serde_json::json!({ "success": true })))
}
