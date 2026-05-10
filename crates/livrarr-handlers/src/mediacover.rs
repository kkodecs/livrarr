use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::context::HasDataDir;

pub async fn get_cover<S: HasDataDir>(
    State(state): State<S>,
    Path(id): Path<i64>,
    req_headers: HeaderMap,
) -> Response {
    let data_dir = state.data_dir().to_path_buf();
    let cover_path = tokio::task::spawn_blocking(move || resolve_cover_path(&data_dir, id, ""))
        .await
        .unwrap_or_else(|_| state.data_dir().join("covers").join(format!("{id}.jpg")));
    serve_image(&cover_path, id, &req_headers).await
}

pub async fn get_thumb<S: HasDataDir>(
    State(state): State<S>,
    Path(id): Path<i64>,
    req_headers: HeaderMap,
) -> Response {
    let data_dir = state.data_dir().to_path_buf();
    let (full_path, thumb_path) = tokio::task::spawn_blocking(move || {
        let full = resolve_cover_path(&data_dir, id, "");
        let thumb = resolve_cover_path(&data_dir, id, "_thumb");
        (full, thumb)
    })
    .await
    .unwrap_or_else(|_| {
        let dir = state.data_dir().join("covers");
        (
            dir.join(format!("{id}.jpg")),
            dir.join(format!("{id}_thumb.jpg")),
        )
    });

    if !thumb_path.exists() {
        if !full_path.exists() {
            return placeholder_response();
        }
        match tokio::fs::read(&full_path).await {
            Ok(bytes) => {
                let thumb_path_clone = thumb_path.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    match generate_thumbnail_jpeg(&bytes, 300) {
                        Ok(thumb_bytes) => {
                            if let Err(e) = std::fs::write(&thumb_path_clone, &thumb_bytes) {
                                tracing::warn!(id, error = %e, "failed to write thumbnail");
                            }
                        }
                        Err(e) => tracing::warn!(id, error = %e, "thumbnail generation failed"),
                    }
                })
                .await;
            }
            Err(_) => return placeholder_response(),
        }
    }

    if !thumb_path.exists() {
        return serve_image(&full_path, id, &req_headers).await;
    }

    serve_image(&thumb_path, id, &req_headers).await
}

/// Resolve the on-disk path for a cover image. Checks the new tenant-aware
/// layout `covers/{user_id}/{work_id}{suffix}.jpg` first (scanning user
/// subdirectories), then falls back to the old flat layout
/// `covers/{work_id}{suffix}.jpg`.
pub fn resolve_cover_path(
    data_dir: &std::path::Path,
    work_id: i64,
    suffix: &str,
) -> std::path::PathBuf {
    let covers_dir = data_dir.join("covers");
    let filename = format!("{work_id}{suffix}.jpg");

    // Check user subdirectories (new layout: covers/{user_id}/{work_id}.jpg)
    if let Ok(entries) = std::fs::read_dir(&covers_dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                let candidate = entry.path().join(&filename);
                if candidate.exists() {
                    return candidate;
                }
            }
        }
    }

    // Fallback to old flat layout (covers/{work_id}.jpg)
    covers_dir.join(&filename)
}

fn generate_thumbnail_jpeg(bytes: &[u8], max_width: u32) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let thumb = img.thumbnail(max_width, u32::MAX / 2);
    let mut out = Vec::new();
    thumb
        .write_to(
            &mut std::io::Cursor::new(&mut out),
            image::ImageFormat::Jpeg,
        )
        .map_err(|e| e.to_string())?;
    Ok(out)
}

fn placeholder_response() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
    )
        .into_response()
}

async fn serve_image(path: &std::path::Path, id: i64, req_headers: &HeaderMap) -> Response {
    if !path.exists() {
        return placeholder_response();
    }

    let etag = tokio::fs::metadata(path)
        .await
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|mtime| {
            let secs = mtime
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            format!("\"{id}-{secs}\"")
        });

    if let (Some(ref etag_val), Some(inm)) = (&etag, req_headers.get(header::IF_NONE_MATCH)) {
        if inm.as_bytes() == etag_val.as_bytes() {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, no-cache"),
            );
            if let Ok(val) = HeaderValue::from_str(etag_val) {
                headers.insert(header::ETAG, val);
            }
            return (StatusCode::NOT_MODIFIED, headers).into_response();
        }
    }

    match tokio::fs::read(path).await {
        Ok(bytes) => {
            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
            headers.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, no-cache"),
            );
            if let Some(etag_val) = etag {
                if let Ok(val) = HeaderValue::from_str(&etag_val) {
                    headers.insert(header::ETAG, val);
                }
            }
            (StatusCode::OK, headers, bytes).into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
