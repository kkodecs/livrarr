use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::Json;

use axum::response::{IntoResponse, Response};
use tokio_util::sync::CancellationToken;

use crate::middleware::RequireAdmin;
use crate::services::settings_service::SettingsService;
use crate::state::AppState;
use crate::{
    AddWorkRequest, AddWorkResponse, ApiError, AuthContext, DeleteWorkResponse, LookupApiResponse,
    RefreshWorkResponse, UpdateWorkRequest, WorkDetailResponse, WorkSearchResult,
};
use livrarr_domain::services::{
    CreateNotificationRequest, NotificationService, WorkDetailView, WorkService,
};

use livrarr_handlers::types::work::work_to_detail;

fn detail_from_view(view: WorkDetailView) -> WorkDetailResponse {
    let mut detail = work_to_detail(&view.work);
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

/// GET /api/v1/work/lookup?term=...&lang=...&raw=...  — searches metadata providers by language.
pub async fn lookup(
    State(state): State<AppState>,
    _ctx: AuthContext,
    Query(q): Query<LookupQuery>,
) -> Result<Json<LookupApiResponse>, ApiError> {
    use livrarr_domain::services::WorkService;

    let req = livrarr_domain::services::LookupRequest {
        term: q.term.unwrap_or_default(),
        lang_override: q.lang,
    };
    let raw = q.raw.unwrap_or(false);

    let resp = state.work_service.lookup_filtered(req, raw).await?;

    let results = resp
        .results
        .into_iter()
        .map(|r| WorkSearchResult {
            ol_key: r.ol_key,
            title: r.title,
            author_name: r.author_name,
            author_ol_key: r.author_ol_key,
            year: r.year,
            cover_url: r.cover_url,
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

/// Internal work creation — shared by the HTTP handler and manual import.
pub async fn add_work_internal(
    state: &AppState,
    user_id: i64,
    req: AddWorkRequest,
) -> Result<AddWorkResponse, ApiError> {
    use livrarr_domain::services::WorkService;

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
        defer_enrichment: req.defer_enrichment,
        provenance_setter: None,
    };

    let result = state.work_service.add(user_id, svc_req).await?;

    if result.author_created {
        if let Some(author_id) = result.author_id {
            crate::handlers::author::spawn_bibliography_fetch((*state).clone(), author_id, user_id);
        }
    }

    Ok(AddWorkResponse {
        work: work_to_detail(&result.work),
        author_created: result.author_created,
        messages: result.messages,
    })
}

/// Fetch the merge-resolved cover URL and write it atomically to
/// `covers/{user_id}/{work_id}.jpg`. Best-effort — every failure is logged
/// but returns `()` so callers don't have to handle errors.
///
/// Delegates to `WorkService::download_cover_from_url` which uses the
/// SSRF-safe HTTP fetcher and atomic write. Single implementation for
/// all cover download paths (refresh, refresh_all, enrichment_retry).
pub(crate) async fn download_post_enrich_cover(
    state: &AppState,
    user_id: i64,
    work_id: i64,
    cover_url: &str,
) {
    use livrarr_domain::services::WorkService;
    if let Err(e) = state
        .work_service
        .download_cover_from_url(user_id, work_id, cover_url)
        .await
    {
        tracing::warn!(work_id, %e, "cover download failed");
    }
}

/// POST /api/v1/work
pub async fn add(
    State(state): State<AppState>,
    ctx: AuthContext,
    Json(req): Json<AddWorkRequest>,
) -> Result<Json<AddWorkResponse>, ApiError> {
    let resp = add_work_internal(&state, ctx.user.id, req).await?;
    Ok(Json(resp))
}

/// GET /api/v1/work
pub async fn list(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(pq): Query<crate::PaginationQuery>,
) -> Result<Json<crate::PaginatedResponse<WorkDetailResponse>>, ApiError> {
    let view = state
        .work_service
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

/// GET /api/v1/work/:id
pub async fn get(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<WorkDetailResponse>, ApiError> {
    let view = state.work_service.get_detail(ctx.user.id, id).await?;
    Ok(Json(detail_from_view(view)))
}

/// PUT /api/v1/work/:id
pub async fn update(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(req): Json<UpdateWorkRequest>,
) -> Result<Json<WorkDetailResponse>, ApiError> {
    use livrarr_domain::services::{UpdateWorkRequest as DomainUpdateWorkRequest, WorkService};

    let cleaned_title = req
        .title
        .as_deref()
        .map(livrarr_metadata::title_cleanup::clean_title);
    let cleaned_author = req
        .author_name
        .as_deref()
        .map(livrarr_metadata::title_cleanup::clean_author);

    let work = state
        .work_service
        .update(
            ctx.user.id,
            id,
            DomainUpdateWorkRequest {
                title: cleaned_title,
                author_name: cleaned_author,
                series_name: req.series_name,
                series_position: req.series_position,
                monitor_ebook: req.monitor_ebook,
                monitor_audiobook: req.monitor_audiobook,
            },
        )
        .await?;

    Ok(Json(work_to_detail(&work)))
}

/// POST /api/v1/work/:id/cover
pub async fn upload_cover(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    body: Bytes,
) -> Result<(), ApiError> {
    use livrarr_domain::services::WorkService;
    livrarr_handlers::work::validate_image_magic_bytes(&body)?;
    state
        .work_service
        .upload_cover(ctx.user.id, id, &body)
        .await?;
    Ok(())
}

/// DELETE /api/v1/work/:id?deleteFiles=...
pub async fn delete(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Query(_q): Query<DeleteQuery>,
) -> Result<Json<DeleteWorkResponse>, ApiError> {
    use livrarr_domain::services::WorkService;
    state.work_service.delete(ctx.user.id, id).await?;
    Ok(Json(DeleteWorkResponse { warnings: vec![] }))
}

/// POST /api/v1/work/:id/refresh — re-enrich from providers
pub async fn refresh(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<RefreshWorkResponse>, ApiError> {
    let result = state.work_service.refresh(ctx.user.id, id).await?;

    if let Some(ref cover_url) = result.work.cover_url {
        download_post_enrich_cover(&state, ctx.user.id, id, cover_url).await;
    }

    let mut messages = result.messages;
    if result.merge_deferred {
        messages.push("Merge deferred — retry pending".to_string());
    }

    if !result.taggable_items.is_empty() {
        use livrarr_domain::services::TagService;
        let tag_warnings = state
            .tag_service
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

/// RAII guard that removes user_id from refresh_in_progress on drop.
/// Panic-safe: uses `lock().ok()` to avoid double-panic during unwind.
struct RefreshGuard {
    user_id: i64,
    set: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<i64>>>,
}

impl Drop for RefreshGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = self.set.lock() {
            set.remove(&self.user_id);
        }
    }
}

/// POST /api/v1/work/refresh — refresh metadata for all user works.
/// Returns 202 immediately; enrichment runs in background.
pub async fn refresh_all(
    State(state): State<AppState>,
    ctx: AuthContext,
) -> Result<axum::http::StatusCode, ApiError> {
    let user_id = ctx.user.id;

    {
        let mut guard = state.refresh_in_progress.lock().unwrap();
        if !guard.insert(user_id) {
            return Err(ApiError::Conflict {
                reason: "Refresh already in progress".to_string(),
            });
        }
    }

    let refresh_guard = RefreshGuard {
        user_id,
        set: state.refresh_in_progress.clone(),
    };

    let works = state
        .work_service
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
        return Ok(axum::http::StatusCode::ACCEPTED);
    }

    let total = works.len();
    tokio::spawn(async move {
        let _guard = refresh_guard;
        let mut enriched = 0usize;
        let mut failed = 0usize;

        for work in &works {
            let result = match tokio::time::timeout(
                std::time::Duration::from_secs(30),
                state.work_service.refresh(user_id, work.id),
            )
            .await
            {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => {
                    tracing::warn!(work_id = work.id, "refresh_all: refresh failed: {e}");
                    failed += 1;
                    continue;
                }
                Err(_) => {
                    tracing::warn!(work_id = work.id, "refresh_all: refresh timed out");
                    failed += 1;
                    continue;
                }
            };

            if let Some(ref cover_url) = result.work.cover_url {
                download_post_enrich_cover(&state, user_id, work.id, cover_url).await;
            }

            enriched += 1;
            if !result.taggable_items.is_empty() {
                use livrarr_domain::services::TagService;
                let _ = state
                    .tag_service
                    .retag_library_items(&result.work, &result.taggable_items)
                    .await;
            }
        }

        if let Err(e) = state
            .notification_service
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
    });

    Ok(axum::http::StatusCode::ACCEPTED)
}

// ---------------------------------------------------------------------------
// Workfile handlers that remain in server (use AppState internals)
// ---------------------------------------------------------------------------

/// POST /api/v1/workfile/:id/send-email
pub async fn send_email(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    use livrarr_domain::services::FileService;

    let payload = state.file_service.prepare_email(ctx.user.id, id).await?;

    let cfg = state.settings_service.get_email_config().await?;

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
    use livrarr_domain::services::FileService;

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
    use livrarr_domain::services::FileService;

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

// ---------------------------------------------------------------------------
// Author search handler (stays in server — uses crate::jobs)
// ---------------------------------------------------------------------------

/// POST /api/v1/author/search — trigger author monitor check for all monitored authors.
pub async fn author_search(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> axum::http::StatusCode {
    tokio::spawn(async move {
        let cancel = CancellationToken::new();
        if let Err(e) = crate::jobs::author_monitor_tick(state, cancel).await {
            tracing::error!("manual author search failed: {e}");
        }
    });
    axum::http::StatusCode::ACCEPTED
}
