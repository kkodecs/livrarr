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
        .ok()
        .flatten();
    match cover_path {
        Some(path) => serve_image(&path, id, &req_headers).await,
        None => placeholder_response(),
    }
}

pub async fn get_thumb<S: HasDataDir>(
    State(state): State<S>,
    Path(id): Path<i64>,
    req_headers: HeaderMap,
) -> Response {
    let data_dir = state.data_dir().to_path_buf();
    let full_path = tokio::task::spawn_blocking(move || resolve_cover_path(&data_dir, id, ""))
        .await
        .ok()
        .flatten();
    let Some(full_path) = full_path else {
        return placeholder_response();
    };

    // The thumbnail lives next to the cover it renders — same user directory.
    let thumb_path = full_path
        .parent()
        .map(|dir| dir.join(format!("{id}_thumb.jpg")))
        .unwrap_or_else(|| full_path.with_file_name(format!("{id}_thumb.jpg")));

    if !thumb_path.exists() {
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

/// Resolve the on-disk path for a cover image in the tenant-aware layout,
/// `covers/{user_id}/{work_id}{suffix}.jpg` — the only layout a cover write
/// can land in (the startup migration adopts every legacy root-level file
/// into its owning user's directory before any request reaches this code).
///
/// This handler has no user context (the route is deliberately
/// unauthenticated — images loaded by `<img>` tags directly — see
/// `router.rs`), so the lookup scans user subdirectories rather than joining
/// a known user id. `None` when no user directory holds the file. There is
/// deliberately no flat-root fallback: post-migration, the only file that
/// can sit at the covers root is an orphan with no matching work row, and an
/// orphan must never be served.
pub fn resolve_cover_path(
    data_dir: &std::path::Path,
    work_id: i64,
    suffix: &str,
) -> Option<std::path::PathBuf> {
    let covers_dir = data_dir.join("covers");
    let filename = format!("{work_id}{suffix}.jpg");

    if let Ok(entries) = std::fs::read_dir(&covers_dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                let candidate = entry.path().join(&filename);
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }

    None
}

pub fn generate_thumbnail_jpeg(bytes: &[u8], max_width: u32) -> Result<Vec<u8>, String> {
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

pub fn placeholder_response() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
    )
        .into_response()
}

pub async fn serve_image(path: &std::path::Path, id: i64, req_headers: &HeaderMap) -> Response {
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

#[cfg(test)]
mod resolve_tests {
    use super::*;

    #[test]
    fn orphan_root_level_cover_is_never_resolved() {
        // A root-level covers/{id}.jpg with NO per-user copy is an orphan
        // (no matching work adopted it at migration) — it must not resolve
        // to a servable path.
        let data_dir = tempfile::tempdir().expect("tempdir");
        let covers = data_dir.path().join("covers");
        std::fs::create_dir_all(&covers).expect("mkdir covers");
        std::fs::write(covers.join("42.jpg"), b"orphan-bytes").expect("write orphan");

        let resolved = resolve_cover_path(data_dir.path(), 42, "");
        assert!(
            resolved.is_none(),
            "an orphan root-level cover must never resolve to a servable \
             path — got {resolved:?}"
        );
    }

    #[test]
    fn per_user_cover_resolves() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let user_dir = data_dir.path().join("covers").join("7");
        std::fs::create_dir_all(&user_dir).expect("mkdir user dir");
        std::fs::write(user_dir.join("42.jpg"), b"real-bytes").expect("write cover");

        let resolved = resolve_cover_path(data_dir.path(), 42, "").expect("must resolve");
        assert!(resolved.exists());
        assert!(resolved.ends_with("7/42.jpg"));
    }

    #[test]
    fn missing_cover_resolves_to_none() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(data_dir.path().join("covers")).expect("mkdir covers");
        assert!(resolve_cover_path(data_dir.path(), 42, "").is_none());
    }
}
