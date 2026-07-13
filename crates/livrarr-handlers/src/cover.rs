use axum::extract::{Multipart, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;

use crate::ApiError;

use livrarr_domain::services::CoverService;
use livrarr_domain::{CoverCandidate, CoverMediaType, SelectCoverRequest};

use crate::context::{HasCoverService, HasDataDir};
use crate::mediacover::{placeholder_response, resolve_cover_path, serve_image};
use crate::AuthContext;

pub async fn get_cover_alternatives<S: HasCoverService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<Vec<CoverCandidate>>, ApiError> {
    let candidates = state
        .cover_service()
        .fetch_alternatives(ctx.user.id, id)
        .await
        .map_err(cover_service_error)?;
    Ok(Json(candidates))
}

fn cover_service_error(e: livrarr_domain::services::CoverServiceError) -> ApiError {
    use livrarr_domain::services::CoverServiceError;
    match e {
        CoverServiceError::NotFound => ApiError::NotFound,
        CoverServiceError::InvalidCandidate(m) => ApiError::BadRequest(m),
        CoverServiceError::UploadValidation(m) => ApiError::BadRequest(m),
        other => ApiError::Internal(format!("cover service: {other:?}")),
    }
}

pub async fn select_cover_handler<S: HasCoverService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(req): Json<SelectCoverRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .cover_service()
        .select_cover(ctx.user.id, id, &req.candidate_id, req.media_type)
        .await
        .map_err(cover_service_error)?;
    Ok(StatusCode::OK)
}

#[derive(serde::Deserialize)]
pub struct UploadCoverQuery {
    #[serde(default = "default_ebook")]
    pub media_type: CoverMediaType,
}

fn default_ebook() -> CoverMediaType {
    CoverMediaType::Ebook
}

pub async fn upload_cover_handler<S: HasCoverService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Query(params): Query<UploadCoverQuery>,
    mut multipart: Multipart,
) -> Result<StatusCode, ApiError> {
    let mut image_data: Option<Vec<u8>> = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() == Some("image_data") || field.name() == Some("file") {
            image_data = field.bytes().await.ok().map(|b| b.to_vec());
            break;
        }
    }
    let data = image_data
        .ok_or_else(|| ApiError::BadRequest("no image_data field in multipart".into()))?;

    state
        .cover_service()
        .upload_cover(ctx.user.id, id, &data, params.media_type)
        .await
        .map_err(cover_service_error)?;
    Ok(StatusCode::OK)
}

pub async fn get_audiobook_cover<S: HasDataDir>(
    State(state): State<S>,
    Path(id): Path<i64>,
    Query(q): Query<crate::mediacover::CoverQuery>,
    req_headers: HeaderMap,
) -> Response {
    let versioned = q.v.is_some();
    let data_dir = state.data_dir().to_path_buf();
    let audio_path = {
        let dd = data_dir.clone();
        tokio::task::spawn_blocking(move || resolve_cover_path(&dd, id, "_audio"))
            .await
            .ok()
            .flatten()
    };

    if let Some(path) = audio_path {
        return serve_image(&path, id, &req_headers, versioned).await;
    }

    let ebook_path = tokio::task::spawn_blocking(move || resolve_cover_path(&data_dir, id, ""))
        .await
        .ok()
        .flatten();

    match ebook_path {
        Some(path) => serve_image(&path, id, &req_headers, versioned).await,
        None => placeholder_response(),
    }
}

pub async fn get_audiobook_thumb<S: HasDataDir>(
    State(state): State<S>,
    Path(id): Path<i64>,
    Query(q): Query<crate::mediacover::CoverQuery>,
    req_headers: HeaderMap,
) -> Response {
    let versioned = q.v.is_some();
    let data_dir = state.data_dir().to_path_buf();
    let audio_full = {
        let dd = data_dir.clone();
        tokio::task::spawn_blocking(move || resolve_cover_path(&dd, id, "_audio"))
            .await
            .ok()
            .flatten()
    };

    let Some(audio_full) = audio_full else {
        return crate::mediacover::get_thumb(State(state), Path(id), Query(q), req_headers).await;
    };

    // The thumbnail lives next to the cover it renders — same user directory.
    let audio_thumb = audio_full
        .parent()
        .map(|dir| dir.join(format!("{id}_audio_thumb.jpg")))
        .unwrap_or_else(|| audio_full.with_file_name(format!("{id}_audio_thumb.jpg")));

    if audio_thumb.exists() {
        return serve_image(&audio_thumb, id, &req_headers, versioned).await;
    }

    if let Ok(bytes) = tokio::fs::read(&audio_full).await {
        let thumb_clone = audio_thumb.clone();
        let _ =
            tokio::task::spawn_blocking(move || {
                match crate::mediacover::generate_thumbnail_jpeg(&bytes, 300) {
                    Ok(thumb_bytes) => {
                        if let Err(e) = std::fs::write(&thumb_clone, &thumb_bytes) {
                            tracing::warn!(id, error = %e, "failed to write audio thumbnail");
                        }
                    }
                    Err(e) => tracing::warn!(id, error = %e, "audio thumbnail generation failed"),
                }
            })
            .await;
    }

    if audio_thumb.exists() {
        serve_image(&audio_thumb, id, &req_headers, versioned).await
    } else {
        serve_image(&audio_full, id, &req_headers, versioned).await
    }
}
