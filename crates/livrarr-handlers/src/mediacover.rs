use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::context::AppContext;

pub async fn get_cover<S: AppContext>(
    State(state): State<S>,
    Path(id): Path<i64>,
    req_headers: HeaderMap,
) -> Response {
    let cover_path = state.data_dir().join("covers").join(format!("{id}.jpg"));
    serve_image(&cover_path, id, &req_headers).await
}

pub async fn get_thumb<S: AppContext>(
    State(state): State<S>,
    Path(id): Path<i64>,
    req_headers: HeaderMap,
) -> Response {
    let covers_dir = state.data_dir().join("covers");
    let thumb_path = covers_dir.join(format!("{id}_thumb.jpg"));
    let full_path = covers_dir.join(format!("{id}.jpg"));

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

const PLACEHOLDER_SVG: &str = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 120 180'><rect width='120' height='180' rx='4' fill='rgb(39,39,42)'/></svg>";

fn placeholder_response() -> Response {
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("image/svg+xml"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=30"),
            ),
        ],
        PLACEHOLDER_SVG.to_owned().into_bytes(),
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
