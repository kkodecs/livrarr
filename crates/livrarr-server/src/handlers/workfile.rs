use axum::extract::{Path, State};
use axum::Json;

use crate::state::AppState;
use crate::{ApiError, AuthContext, LibraryItemResponse};
use livrarr_db::{ConfigDb, LibraryItemDb, RootFolderDb};

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
) -> Result<Json<Vec<LibraryItemResponse>>, ApiError> {
    let items = state.db.list_library_items(ctx.user.id).await?;
    Ok(Json(items.iter().map(to_response).collect()))
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
