use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::state::AppState;
use crate::{ApiError, AuthContext, LibraryItemResponse, PaginatedResponse, PaginationQuery};
use livrarr_db::{ConfigDb, LibraryItemDb, PlaybackProgressDb, RootFolderDb, SessionDb};

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
        .db
        .list_library_items_paginated(ctx.user.id, page, page_size)
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
    let item = state.db.get_library_item(ctx.user.id, id).await?;
    Ok(Json(to_response(&item)))
}

/// DELETE /api/v1/workfile/:id
pub async fn delete(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    let _item = state.db.delete_library_item(ctx.user.id, id).await?;
    Ok(())
}

/// POST /api/v1/workfile/:id/send-email
pub async fn send_email(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let item = state.db.get_library_item(ctx.user.id, id).await?;
    let root_folder = state.db.get_root_folder(item.root_folder_id).await?;
    let abs_path = std::path::Path::new(&root_folder.path).join(&item.path);
    let cfg = state.db.get_email_config().await?;

    let ext = abs_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if !super::email::ACCEPTED_EXTENSIONS.contains(&ext.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "Format '.{ext}' not accepted. Supported: EPUB, PDF, DOCX, RTF, TXT, HTML."
        )));
    }

    if item.file_size > super::email::MAX_EMAIL_SIZE {
        return Err(ApiError::BadRequest(format!(
            "File exceeds the 50 MB email limit ({})",
            format_bytes(item.file_size)
        )));
    }

    let file_bytes = tokio::fs::read(&abs_path).await.map_err(|e| {
        ApiError::Internal(format!("Failed to read file {}: {e}", abs_path.display()))
    })?;

    let filename = abs_path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("book");

    super::email::send_file(&cfg, file_bytes, filename, &ext)
        .await
        .map_err(|e| {
            tracing::error!("Email send failed: {e}");
            ApiError::Internal(e)
        })?;

    tracing::info!(file = %item.path, "Email sent");
    Ok(Json(serde_json::json!({ "success": true })))
}

fn format_bytes(bytes: i64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Resolve a library item to an absolute, canonicalized file path.
/// Returns 403 if the resolved path escapes the root folder (path traversal protection).
pub async fn resolve_file_path(
    db: &(impl LibraryItemDb + RootFolderDb + Send + Sync),
    user_id: livrarr_domain::UserId,
    library_item_id: livrarr_domain::LibraryItemId,
) -> Result<std::path::PathBuf, ApiError> {
    let item = db.get_library_item(user_id, library_item_id).await?;
    let root_folder = db.get_root_folder(item.root_folder_id).await?;
    let root = std::path::Path::new(&root_folder.path);
    let abs_path = root.join(&item.path);

    // Canonicalize and verify containment.
    let canonical = abs_path.canonicalize().map_err(|_| ApiError::NotFound)?;
    let canonical_root = root
        .canonicalize()
        .map_err(|e| ApiError::Internal(format!("Root folder not accessible: {e}")))?;
    if !canonical.starts_with(&canonical_root) {
        return Err(ApiError::Forbidden);
    }

    Ok(canonical)
}

/// Map file extension to Content-Type.
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
    let path = resolve_file_path(&state.db, ctx.user.id, id).await?;

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let content_type = mime_for_ext(&ext);

    // Use tower_http::services::ServeFile for correct byte-range handling.
    use tower::Service;
    use tower_http::services::ServeFile;
    let mut svc = ServeFile::new(&path);
    let resp = svc
        .call(req)
        .await
        .map_err(|e| ApiError::Internal(format!("File serve error: {e}")))?;

    // Override content-type to be precise.
    let (mut parts, body) = resp.into_response().into_parts();
    parts.headers.insert(
        axum::http::header::CONTENT_TYPE,
        content_type.parse().unwrap(),
    );
    Ok(Response::from_parts(parts, body))
}

/// GET /api/v1/stream/:id?token=<bearer_token>
/// Token-authenticated streaming endpoint for HTML5 audio/video elements
/// which cannot send custom headers.
pub async fn stream(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(params): Query<StreamQuery>,
    req: axum::http::Request<axum::body::Body>,
) -> Result<Response, ApiError> {
    let token = params.token.as_deref().ok_or(ApiError::Unauthorized)?;

    // Authenticate via token (same as Bearer auth).
    use crate::auth_crypto::{AuthCryptoService, RealAuthCrypto};
    let crypto = RealAuthCrypto;
    let token_hash = crypto
        .hash_token(token)
        .await
        .map_err(|_| ApiError::Unauthorized)?;

    let session = state
        .db
        .get_session(&token_hash)
        .await
        .map_err(|_| ApiError::Unauthorized)?
        .ok_or(ApiError::Unauthorized)?;

    let path = resolve_file_path(&state.db, session.user_id, id).await?;

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
    let progress = state.db.get_progress(ctx.user.id, id).await?;
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
    // Validate the library item exists and belongs to the user.
    let _item = state.db.get_library_item(ctx.user.id, id).await?;

    let pct = body.progress_pct.clamp(0.0, 1.0);
    state
        .db
        .upsert_progress(ctx.user.id, id, &body.position, pct)
        .await?;
    Ok(Json(serde_json::json!({ "success": true })))
}
