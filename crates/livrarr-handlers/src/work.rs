use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::Json;

use axum::response::{IntoResponse, Response};

use crate::context::{
    HasAuthService, HasAuthorMonitorWorkflow, HasAuthorService, HasEmailService,
    HasEnrichmentNotify, HasEnrichmentWorkflow, HasFileService, HasNotificationService,
    HasSeriesQueryService, HasTagService, HasWorkService,
};

use crate::middleware::RequireAdmin;
use crate::types::work::work_to_detail;
use crate::{
    AddWorkRequest, AddWorkResponse, ApiError, AuthContext, DeleteWorkResponse, LookupApiResponse,
    RefreshWorkResponse, UpdateWorkRequest, WorkDetailResponse, WorkSearchResult,
};
use livrarr_domain::services::{
    AuthorService, CreateNotificationRequest, EmailService, EnrichmentWorkflow, FileService,
    NotificationService, SeriesQueryService, TagService, WorkDetailView, WorkService,
};

fn proxy_cover_url(url: String) -> String {
    if url.starts_with('/') {
        return url;
    }
    format!("/api/v1/coverproxy?url={}", urlencoding::encode(&url))
}

/// Validate that image data begins with a recognized magic byte signature.
pub fn validate_image_magic_bytes(data: &[u8]) -> Result<(), ApiError> {
    const JPEG_MAGIC: &[u8] = &[0xFF, 0xD8, 0xFF];
    const PNG_MAGIC: &[u8] = &[0x89, 0x50, 0x4E, 0x47];
    const WEBP_RIFF: &[u8] = b"RIFF";
    const WEBP_WEBP: &[u8] = b"WEBP";

    if data.len() < 12 {
        return Err(ApiError::BadRequest(
            "image data too small to identify format".into(),
        ));
    }
    if data.starts_with(JPEG_MAGIC) || data.starts_with(PNG_MAGIC) {
        return Ok(());
    }
    if data.starts_with(WEBP_RIFF) && data[8..12] == *WEBP_WEBP {
        return Ok(());
    }
    Err(ApiError::BadRequest(
        "unsupported image format: expected JPEG, PNG, or WebP".into(),
    ))
}

fn detail_from_view(view: WorkDetailView) -> WorkDetailResponse {
    let mut detail =
        crate::types::work::work_to_detail_with_cover_mtime(&view.work, view.cover_mtime);
    detail.library_items = view
        .library_items
        .iter()
        .map(|li| crate::LibraryItemResponse {
            id: li.id,
            path: li.path.clone(),
            media_type: li.media_type,
            file_size: li.file_size,
            imported_at: li.imported_at.to_rfc3339(),
        })
        .collect();
    detail
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

#[derive(serde::Deserialize)]
pub struct LookupQuery {
    pub term: Option<String>,
    pub lang: Option<String>,
    pub raw: Option<bool>,
}

#[derive(serde::Deserialize)]
pub struct DeleteQuery {
    #[serde(rename = "deleteFiles")]
    pub delete_files: Option<bool>,
}

#[derive(serde::Deserialize)]
pub struct StreamQuery {
    pub token: Option<String>,
}

pub async fn lookup<S: HasWorkService>(
    State(state): State<S>,
    _ctx: AuthContext,
    Query(q): Query<LookupQuery>,
) -> Result<Json<LookupApiResponse>, ApiError> {
    let req = livrarr_domain::services::LookupRequest {
        term: q.term.unwrap_or_default(),
        lang_override: q.lang,
    };
    let raw = q.raw.unwrap_or(false);

    let resp = state.work_service().lookup_filtered(req, raw).await?;

    let results = resp
        .results
        .into_iter()
        .map(|r| WorkSearchResult {
            ol_key: r.ol_key,
            title: r.title,
            author_name: r.author_name,
            author_ol_key: r.author_ol_key,
            year: r.year,
            cover_url: r.cover_url.map(proxy_cover_url),
            description: r.description,
            series_name: r.series_name,
            series_position: r.series_position,
            source: r.source,
            source_type: r.source_type,
            language: r.language,
            detail_url: r.detail_url,
            rating: r.rating,
        })
        .collect();

    Ok(Json(LookupApiResponse {
        results,
        filtered_count: resp.filtered_count,
        raw_count: resp.raw_count,
        raw_available: resp.raw_available,
    }))
}

pub async fn add<
    S: HasWorkService
        + HasAuthorService
        + HasSeriesQueryService
        + HasEnrichmentNotify
        + HasEnrichmentWorkflow,
>(
    State(state): State<S>,
    ctx: AuthContext,
    Json(req): Json<AddWorkRequest>,
) -> Result<Json<AddWorkResponse>, ApiError> {
    let author_name_for_gr = req.author_name.clone();
    let svc_req = livrarr_domain::services::AddWorkRequest {
        title: req.title,
        author_name: req.author_name,
        author_ol_key: req.author_ol_key,
        ol_key: req.ol_key,
        gr_key: None,
        year: req.year,
        cover_url: req.cover_url,
        metadata_source: req.metadata_source,
        language: req.language,
        detail_url: req.detail_url,
        series_name: None,
        series_position: None,
        defer_enrichment: true,
        provenance_setter: None,
    };

    let result = state.work_service().add(ctx.user.id, svc_req).await?;

    if result.author_created {
        if let Some(author_id) = result.author_id {
            let s = state.clone();
            let user_id = ctx.user.id;
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                if let Err(e) = s
                    .author_service()
                    .refresh_bibliography(user_id, author_id)
                    .await
                {
                    tracing::debug!(author_id, "background bibliography fetch skipped: {e}");
                }
            });

            let s_gr = state.clone();
            let uid = ctx.user.id;
            let author_name = author_name_for_gr;
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                match s_gr
                    .series_query_service()
                    .resolve_gr_candidates(uid, author_id)
                    .await
                {
                    Ok(candidates) => {
                        if let Some(first) = candidates.first() {
                            let sim =
                                livrarr_matching::author_similarity(&author_name, &first.name);
                            if sim >= 0.90 {
                                tracing::info!(
                                    author = %author_name,
                                    gr_candidate = %first.name,
                                    similarity = %sim,
                                    "auto-linking Goodreads author (work add)"
                                );
                                let _ = s_gr
                                    .author_service()
                                    .update(
                                        uid,
                                        author_id,
                                        livrarr_domain::services::UpdateAuthorRequest {
                                            name: None,
                                            sort_name: None,
                                            ol_key: None,
                                            gr_key: Some(Some(first.gr_key.clone())),
                                            monitored: None,
                                            monitor_new_items: None,
                                        },
                                    )
                                    .await;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!(author_id, "background GR resolve skipped: {e}");
                    }
                }
            });
        }
    }

    // Phase 2: background enrichment for metadata + potential cover upgrade
    {
        let s = state.clone();
        let user_id = ctx.user.id;
        let work_id = result.work.id;
        let phase1_had_cover = result.cover_mtime.is_some();
        tokio::spawn(async move {
            if !phase1_had_cover {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            } else {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            match s
                .enrichment_workflow()
                .enrich_work(
                    user_id,
                    work_id,
                    livrarr_domain::services::EnrichmentMode::Background,
                )
                .await
            {
                Ok(r) => {
                    if !r.work.cover_manual {
                        if let Some(ref url) = r.work.cover_url {
                            let _ = s
                                .work_service()
                                .download_cover_from_url(user_id, work_id, url)
                                .await;
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(work_id, "background enrichment failed: {e}");
                }
            }
        });
    }

    let detail =
        crate::types::work::work_to_detail_with_cover_mtime(&result.work, result.cover_mtime);
    Ok(Json(AddWorkResponse {
        work: detail,
        author_created: result.author_created,
        messages: result.messages,
    }))
}

pub async fn list<S: HasWorkService>(
    State(state): State<S>,
    ctx: AuthContext,
    Query(pq): Query<crate::PaginationQuery>,
) -> Result<Json<crate::PaginatedResponse<WorkDetailResponse>>, ApiError> {
    let view = state
        .work_service()
        .list_paginated(
            ctx.user.id,
            pq.page(),
            pq.page_size(),
            pq.sort_by(),
            pq.sort_dir(),
        )
        .await?;

    let items = view.works.into_iter().map(detail_from_view).collect();
    Ok(Json(crate::PaginatedResponse {
        items,
        total: view.total,
        page: view.page,
        page_size: view.page_size,
    }))
}

pub async fn get<S: HasWorkService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<WorkDetailResponse>, ApiError> {
    let view = state.work_service().get_detail(ctx.user.id, id).await?;
    Ok(Json(detail_from_view(view)))
}

pub async fn update<S: HasWorkService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(req): Json<UpdateWorkRequest>,
) -> Result<Json<WorkDetailResponse>, ApiError> {
    use crate::types::api_error::FieldError;
    use livrarr_domain::services::UpdateWorkRequest as DomainUpdateWorkRequest;

    let mut errors = Vec::new();
    if matches!(req.title, Some(None)) {
        errors.push(FieldError {
            field: "title".into(),
            message: "cannot be null".into(),
        });
    }
    if matches!(req.author_name, Some(None)) {
        errors.push(FieldError {
            field: "authorName".into(),
            message: "cannot be null".into(),
        });
    }
    if matches!(req.monitor_ebook, Some(None)) {
        errors.push(FieldError {
            field: "monitorEbook".into(),
            message: "cannot be null".into(),
        });
    }
    if matches!(req.monitor_audiobook, Some(None)) {
        errors.push(FieldError {
            field: "monitorAudiobook".into(),
            message: "cannot be null".into(),
        });
    }
    if let Some(Some(ref t)) = req.title {
        if t.trim().is_empty() {
            errors.push(FieldError {
                field: "title".into(),
                message: "cannot be empty".into(),
            });
        }
    }
    if let Some(Some(ref a)) = req.author_name {
        if a.trim().is_empty() {
            errors.push(FieldError {
                field: "authorName".into(),
                message: "cannot be empty".into(),
            });
        }
    }
    if !errors.is_empty() {
        return Err(ApiError::Validation { errors });
    }

    let work = state
        .work_service()
        .update(
            ctx.user.id,
            id,
            DomainUpdateWorkRequest {
                title: req.title.flatten(),
                author_name: req.author_name.flatten(),
                series_name: req.series_name,
                series_position: req.series_position,
                monitor_ebook: req.monitor_ebook.flatten(),
                monitor_audiobook: req.monitor_audiobook.flatten(),
            },
        )
        .await?;

    Ok(Json(work_to_detail(&work)))
}

pub async fn upload_cover<S: HasWorkService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    mut multipart: axum::extract::Multipart,
) -> Result<(), ApiError> {
    let mut image_data: Option<Bytes> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("multipart error: {e}")))?
    {
        if field.name() == Some("image_data") {
            image_data = Some(
                field
                    .bytes()
                    .await
                    .map_err(|e| ApiError::BadRequest(format!("read field: {e}")))?,
            );
            break;
        }
    }
    let data = image_data.ok_or_else(|| ApiError::BadRequest("missing image_data field".into()))?;
    validate_image_magic_bytes(&data)?;
    state
        .work_service()
        .upload_cover(ctx.user.id, id, &data)
        .await?;
    Ok(())
}

pub async fn delete<S: HasWorkService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Query(_q): Query<DeleteQuery>,
) -> Result<Json<DeleteWorkResponse>, ApiError> {
    state.work_service().delete(ctx.user.id, id).await?;
    Ok(Json(DeleteWorkResponse { warnings: vec![] }))
}

pub async fn refresh<S: HasWorkService + HasTagService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<RefreshWorkResponse>, ApiError> {
    let result = state.work_service().refresh(ctx.user.id, id).await?;

    if !result.work.cover_manual {
        if let Some(ref cover_url) = result.work.cover_url {
            if let Err(e) = state
                .work_service()
                .download_cover_from_url(ctx.user.id, id, cover_url)
                .await
            {
                tracing::warn!(work_id = id, %e, "cover download failed after refresh");
            }
        }
    }

    let mut messages = result.messages;
    if result.merge_deferred {
        messages.push("Merge deferred — retry pending".to_string());
    }

    if !result.taggable_items.is_empty() {
        let tag_warnings = state
            .tag_service()
            .retag_library_items(&result.work, &result.taggable_items)
            .await;
        for w in &tag_warnings {
            messages.push(format!("tag rewrite warning: {w}"));
        }
        if tag_warnings.is_empty() {
            messages.push(format!(
                "tags rewritten on {} file(s)",
                result.taggable_items.len()
            ));
        }
    }

    Ok(Json(RefreshWorkResponse {
        work: work_to_detail(&result.work),
        messages,
    }))
}

pub async fn refresh_all<S: HasWorkService + HasTagService + HasNotificationService>(
    State(state): State<S>,
    ctx: AuthContext,
) -> Result<axum::http::StatusCode, ApiError> {
    let user_id = ctx.user.id;

    if !state.work_service().try_start_bulk_refresh(user_id) {
        return Err(ApiError::Conflict {
            reason: "Refresh already in progress".to_string(),
        });
    }

    let works = state
        .work_service()
        .list(
            user_id,
            livrarr_domain::services::WorkFilter {
                author_id: None,
                monitored: None,
                enrichment_status: None,
                media_type: None,
                sort_by: None,
                sort_dir: None,
            },
        )
        .await
        .map_err(ApiError::from)?;

    if works.is_empty() {
        state.work_service().finish_bulk_refresh(user_id);
        return Ok(axum::http::StatusCode::ACCEPTED);
    }

    let total = works.len();
    let s = state.clone();
    tokio::spawn(async move {
        let mut enriched = 0usize;
        let mut failed = 0usize;

        for work in &works {
            let result = match s.work_service().refresh(user_id, work.id).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(work_id = work.id, "refresh_all: refresh failed: {e}");
                    failed += 1;
                    continue;
                }
            };

            if !result.work.cover_manual {
                if let Some(ref cover_url) = result.work.cover_url {
                    let _ = s
                        .work_service()
                        .download_cover_from_url(user_id, work.id, cover_url)
                        .await;
                }
            }

            enriched += 1;
            if !result.taggable_items.is_empty() {
                let _ = s
                    .tag_service()
                    .retag_library_items(&result.work, &result.taggable_items)
                    .await;
            }
        }

        if let Err(e) = s
            .notification_service()
            .create(CreateNotificationRequest {
                user_id,
                notification_type: livrarr_domain::NotificationType::BulkEnrichmentComplete,
                ref_key: None,
                message: format!(
                    "Bulk refresh complete: {enriched}/{total} enriched, {failed} failed"
                ),
                data: serde_json::json!({
                    "total": total,
                    "enriched": enriched,
                    "failed": failed,
                }),
            })
            .await
        {
            tracing::warn!("create_notification failed: {e}");
        }

        s.work_service().finish_bulk_refresh(user_id);
    });

    Ok(axum::http::StatusCode::ACCEPTED)
}

pub async fn send_email<S: HasFileService + HasEmailService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let payload = state.file_service().prepare_email(ctx.user.id, id).await?;

    state
        .email_service()
        .send_file(payload.file_bytes, &payload.filename, &payload.extension)
        .await
        .map_err(|e| {
            tracing::error!("Email send failed: {e}");
            ApiError::Internal(e.to_string())
        })?;

    tracing::info!(file = %payload.filename, "Email sent");
    Ok(Json(serde_json::json!({ "success": true })))
}

pub async fn download<S: HasFileService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    req: axum::http::Request<axum::body::Body>,
) -> Result<Response, ApiError> {
    let path = state.file_service().resolve_path(ctx.user.id, id).await?;

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

pub async fn stream<S: HasAuthService + HasFileService>(
    State(state): State<S>,
    Path(id): Path<i64>,
    Query(params): Query<StreamQuery>,
    req: axum::http::Request<axum::body::Body>,
) -> Result<Response, ApiError> {
    use crate::types::auth::AuthService;

    let token = params.token.as_deref().ok_or(ApiError::Unauthorized)?;
    let user_id = state
        .auth_service()
        .verify_token(token)
        .await
        .map_err(|_| ApiError::Unauthorized)?;

    let path = state.file_service().resolve_path(user_id, id).await?;

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

pub async fn author_search<S: HasAuthorMonitorWorkflow>(
    State(state): State<S>,
    ctx: AuthContext,
    _admin: RequireAdmin,
) -> axum::http::StatusCode {
    let s = state.clone();
    let user_id = ctx.user.id;
    tokio::spawn(async move {
        use livrarr_domain::services::{AuthorMonitorWorkflow, MonitorError};
        let cancel = tokio_util::sync::CancellationToken::new();
        match s
            .author_monitor_workflow()
            .run_monitor(user_id, cancel)
            .await
        {
            Ok(_) => {}
            Err(MonitorError::AlreadyRunning) => {}
            Err(e) => tracing::error!("manual author search failed for user {user_id}: {e}"),
        }
    });
    axum::http::StatusCode::ACCEPTED
}
