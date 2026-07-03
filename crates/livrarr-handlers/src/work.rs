use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;

use axum::response::{IntoResponse, Response};

use crate::context::{
    HasAppConfigService, HasAuthService, HasAuthorMonitorWorkflow, HasAuthorService,
    HasEmailService, HasEnrichmentWorkflow, HasFileService, HasIdentityResolver,
    HasNotificationService, HasSeriesQueryService, HasTagService, HasWorkIdentityRepository,
    HasWorkService,
};

use crate::middleware::RequireAdmin;
use crate::types::work::work_to_detail;
use crate::{
    AddWorkRequest, AddWorkResponse, ApiError, AuthContext, DeleteWorkResponse, LookupApiResponse,
    RefreshWorkResponse, UpdateWorkRequest, WorkDetailResponse, WorkSearchResult,
};
use livrarr_domain::identity::{AnchorConfidence, AnchorSetter, AnchorType};
use livrarr_domain::services::{
    AppConfigService, AuthorService, CreateNotificationRequest, EmailService, FileService,
    NotificationService, RefreshSurface, SeriesQueryService, WorkIdentityRepository, WorkService,
    WorkServiceError,
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
pub struct StreamQuery {
    pub token: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct PreaddCoverQuery {
    pub title: String,
    pub author: Option<String>,
    pub lang: Option<String>,
    pub isbn_13: Option<String>,
}

pub async fn preadd_cover_alternatives<S>(
    State(state): State<S>,
    ctx: AuthContext,
    Query(q): Query<PreaddCoverQuery>,
) -> Result<Json<Vec<livrarr_domain::services::PreaddCoverCandidate>>, ApiError>
where
    S: crate::context::HasPreaddCoverService,
{
    if q.title.trim().is_empty() {
        return Err(ApiError::BadRequest("title is required".into()));
    }
    let author = q.author.as_deref().unwrap_or("");
    let lang = q.lang.as_deref().unwrap_or("en");
    let svc = state.preadd_cover_service();
    let candidates = livrarr_domain::services::PreaddCoverService::fetch_cover_alternatives(
        svc,
        ctx.user.id,
        q.title.trim(),
        author,
        lang,
        q.isbn_13.as_deref(),
    )
    .await
    .map_err(|e| ApiError::Internal(format!("cover alternatives: {e}")))?;
    Ok(Json(candidates))
}

pub async fn lookup<S: HasWorkService>(
    State(state): State<S>,
    ctx: AuthContext,
    Query(q): Query<LookupQuery>,
) -> Result<Json<LookupApiResponse>, ApiError> {
    let req = livrarr_domain::services::LookupRequest {
        term: q.term.unwrap_or_default(),
        lang_override: q.lang,
    };
    let raw = q.raw.unwrap_or(false);

    let resp = state
        .work_service()
        .lookup_filtered(ctx.user.id, req, raw)
        .await?;

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
            candidate_id: r.candidate_id,
            isbn_13: r.isbn_13,
            hc_key: r.hc_key,
            gr_key: r.gr_key,
            asin: r.asin,
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
        + HasEnrichmentWorkflow
        + HasIdentityResolver
        + HasAppConfigService,
>(
    State(state): State<S>,
    ctx: AuthContext,
    Json(req): Json<AddWorkRequest>,
) -> Result<Json<AddWorkResponse>, ApiError> {
    let author_name_for_gr = req.author_name.clone();
    use livrarr_domain::identity::{LatencyTier, RawHarvest};
    use livrarr_domain::seed::{seed_add_box, SeedInput, SeedLanguage};

    let default_language = state.app_config_service().get_default_language().await?;
    let language = SeedLanguage::resolve(req.language.as_deref(), &default_language);

    // The UI hands back the picked cover in its proxied display form
    // (`/api/v1/coverproxy?url=<encoded>`); persist the real provider URL so it
    // is a usable cover source (a proxied, leading-`/` value is dropped by the
    // DB cover-URL backstop and the manual pick never sticks).
    let cover_url = req
        .cover_url
        .as_deref()
        .map(livrarr_domain::unproxy_cover_url);
    let cover_is_manual = req.cover_manual && cover_url.is_some();

    // Resolve identity through the shared resolver — the one place every door
    // turns raw anchors into a Confirmed/Pending/Conflict badge (P1). Boundary
    // sanitization (normalize + drop malformed anchors) happens inside
    // resolve_identity; an isbn/asin-only pick still fans out to find a work anchor.
    let resolved = state
        .work_service()
        .resolve_identity(
            ctx.user.id,
            RawHarvest {
                ol_key: req.ol_key.clone(),
                gr_key: req.gr_key.clone(),
                hc_key: req.hc_key.clone(),
                isbn: req.isbn_13.clone(),
                asin: req.asin.clone(),
                title: Some(req.title.clone()),
                author_name: Some(req.author_name.clone()),
                language: Some(language.as_str().to_string()),
                series_name: None,
                year: req.year,
                user_confirmed: true,
            },
            LatencyTier::Interactive,
        )
        .await
        .map_err(|e| ApiError::Internal(format!("identity resolve: {e}")))?;
    if let Some(conflict) = resolved.conflict {
        let work = state
            .work_service()
            .get(ctx.user.id, conflict.existing_work_id)
            .await?;
        let detail = crate::types::work::work_to_detail_with_cover_mtime(&work, None, None);
        return Ok(Json(AddWorkResponse {
            work: detail,
            author_created: false,
            messages: vec!["identity conflict: existing work has a different anchor".into()],
        }));
    }
    let identity = resolved.identity;

    // Funnel through the one road: enrichment + cover/tag materialization run
    // synchronously via the pipeline, reusing the candidate's cached discovery
    // payloads (zero-network when the search cache is still warm).
    let candidate = seed_add_box(
        SeedInput {
            title: req.title,
            author_name: req.author_name,
            language,
            author_ol_key: req.author_ol_key,
            year: req.year,
            cover_url,
            detail_url: req.detail_url,
            description: None,
            series_name: None,
            series_position: None,
        },
        identity,
        req.candidate_id,
        cover_is_manual,
    );

    let result = state.work_service().add(ctx.user.id, candidate).await?;

    // Background refresh: fill in anchors (GR/HC/ASIN) that initial enrichment
    // misses because they require the identity road to run first.
    if result.created {
        let s = state.clone();
        let uid = ctx.user.id;
        let wid = result.work.id;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let _ = s
                .work_service()
                .refresh(uid, wid, RefreshSurface::Interactive)
                .await;
        });
    }

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
                                            monitor_language: None,
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

    let detail = crate::types::work::work_to_detail_with_cover_mtime(
        &result.work,
        result.cover_mtime,
        result.audiobook_cover_mtime,
    );
    Ok(Json(AddWorkResponse {
        work: detail,
        author_created: result.author_created,
        messages: result.messages,
    }))
}

pub async fn list<S: HasWorkService + HasFileService>(
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
            pq.media_type,
            pq.language.as_deref(),
        )
        .await?;

    let all_item_ids: Vec<i64> = view
        .works
        .iter()
        .flat_map(|w| w.library_items.iter().map(|li| li.id))
        .collect();

    let progress_list = state
        .file_service()
        .get_progress_for_items(ctx.user.id, &all_item_ids)
        .await
        .unwrap_or_default();

    let progress_map: std::collections::HashMap<i64, &livrarr_domain::services::ItemProgress> =
        progress_list
            .iter()
            .map(|p| (p.library_item_id, p))
            .collect();

    let items = view
        .works
        .into_iter()
        .map(|wv| {
            let work_duration = wv.work.duration_seconds.map(|d| d as f64);
            let mut detail = crate::types::work::work_to_detail_with_cover_mtime(
                &wv.work,
                wv.cover_mtime,
                wv.audiobook_cover_mtime,
            );
            detail.library_items = wv
                .library_items
                .iter()
                .map(|li| {
                    let prog = progress_map.get(&li.id);
                    crate::LibraryItemResponse {
                        id: li.id,
                        path: li.path.clone(),
                        media_type: li.media_type,
                        file_size: li.file_size,
                        imported_at: li.imported_at.to_rfc3339(),
                        progress_pct: prog.map(|p| p.progress_pct),
                        duration_seconds: li.duration_seconds.or(work_duration),
                        finished_at: prog.and_then(|p| p.finished_at.map(|d| d.to_rfc3339())),
                    }
                })
                .collect();
            detail
        })
        .collect();

    Ok(Json(crate::PaginatedResponse {
        items,
        total: view.total,
        page: view.page,
        page_size: view.page_size,
    }))
}

pub async fn get<S: HasWorkService + HasFileService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<WorkDetailResponse>, ApiError> {
    let view = state.work_service().get_detail(ctx.user.id, id).await?;

    let item_ids: Vec<i64> = view.library_items.iter().map(|li| li.id).collect();
    let progress_list = state
        .file_service()
        .get_progress_for_items(ctx.user.id, &item_ids)
        .await
        .unwrap_or_default();
    let progress_map: std::collections::HashMap<i64, &livrarr_domain::services::ItemProgress> =
        progress_list
            .iter()
            .map(|p| (p.library_item_id, p))
            .collect();

    let work_duration = view.work.duration_seconds.map(|d| d as f64);
    let mut detail = crate::types::work::work_to_detail_with_cover_mtime(
        &view.work,
        view.cover_mtime,
        view.audiobook_cover_mtime,
    );
    detail.library_items = view
        .library_items
        .iter()
        .map(|li| {
            let prog = progress_map.get(&li.id);
            crate::LibraryItemResponse {
                id: li.id,
                path: li.path.clone(),
                media_type: li.media_type,
                file_size: li.file_size,
                imported_at: li.imported_at.to_rfc3339(),
                progress_pct: prog.map(|p| p.progress_pct),
                duration_seconds: li.duration_seconds.or(work_duration),
                finished_at: prog.and_then(|p| p.finished_at.map(|d| d.to_rfc3339())),
            }
        })
        .collect();

    Ok(Json(detail))
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
) -> Result<Json<DeleteWorkResponse>, ApiError> {
    state.work_service().delete(ctx.user.id, id).await?;
    Ok(Json(DeleteWorkResponse { warnings: vec![] }))
}

pub async fn refresh<S: HasWorkService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<RefreshWorkResponse>, ApiError> {
    // WorkService::refresh() runs the unified enrichment pipeline:
    // provider dispatch, merge, cover download, and tag sync are all handled inside.
    let result = state
        .work_service()
        .refresh(ctx.user.id, id, RefreshSurface::Interactive)
        .await?;

    Ok(Json(RefreshWorkResponse {
        work: work_to_detail(&result.work),
        messages: result.messages,
    }))
}

/// Active library filters carried into Refresh All (REQ-015): the sweep
/// refreshes what the user sees. No params = all works (AC-017).
#[derive(serde::Deserialize)]
pub struct RefreshAllParams {
    pub language: Option<String>,
    pub monitored: Option<bool>,
    pub enrichment_status: Option<livrarr_domain::EnrichmentStatus>,
    pub media_type: Option<livrarr_domain::MediaType>,
}

pub async fn refresh_all<S: HasWorkService + HasTagService + HasNotificationService>(
    State(state): State<S>,
    ctx: AuthContext,
    Query(params): Query<RefreshAllParams>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user_id = ctx.user.id;

    let Some(bulk_guard) = state.work_service().try_start_bulk_refresh(user_id) else {
        return Err(ApiError::Conflict {
            reason: "Refresh already in progress".to_string(),
        });
    };

    let works = state
        .work_service()
        .list(
            user_id,
            livrarr_domain::services::WorkFilter {
                author_id: None,
                monitored: params.monitored,
                enrichment_status: params.enrichment_status,
                media_type: params.media_type,
                language: params.language,
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
    let s = state.clone();
    tokio::spawn(async move {
        // Owns the slot: completion, error, panic unwind, and abort all
        // release via Drop (REQ-016).
        let _bulk_guard = bulk_guard;
        let mut enriched = 0usize;
        let mut failed = 0usize;

        for work in &works {
            // refresh() funnels through run_unified, which materializes covers and
            // tags itself — the handler does not re-download or re-tag (REQ-001).
            // Low: unattended refresh-all sweep (B4 table).
            match s
                .work_service()
                .refresh(user_id, work.id, RefreshSurface::Bulk)
                .await
            {
                Ok(_) => {
                    enriched += 1;
                }
                Err(e) => {
                    tracing::warn!(work_id = work.id, "refresh_all: refresh failed: {e}");
                    failed += 1;
                }
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
    });

    Ok(axum::http::StatusCode::ACCEPTED)
}

/// User-triggered bulk recovery (REQ-011): sweep every incomplete work — Failed,
/// Unenriched, or identity-Pending — and re-run each through the one road in a
/// single pass. Replaces the deleted background `enrichment_retry` job (REQ-001):
/// recovery is now an explicit user action, not a recurring loop. Shares the
/// `try_start_bulk_refresh` guard with `refresh_all` so only one bulk sweep runs
/// per user at a time; the work happens in a spawned one-shot and the response is
/// an immediate 202.
pub async fn retry_all_incomplete<S: HasWorkService + HasNotificationService>(
    State(state): State<S>,
    ctx: AuthContext,
) -> Result<axum::http::StatusCode, ApiError> {
    let user_id = ctx.user.id;

    let Some(bulk_guard) = state.work_service().try_start_bulk_refresh(user_id) else {
        return Err(ApiError::Conflict {
            reason: "Bulk operation already in progress".to_string(),
        });
    };

    let s = state.clone();
    tokio::spawn(async move {
        // Owns the slot: every exit path releases via Drop (REQ-016).
        let _bulk_guard = bulk_guard;
        match s.work_service().retry_all_incomplete(user_id).await {
            Ok(summary) => {
                if let Err(e) = s
                    .notification_service()
                    .create(CreateNotificationRequest {
                        user_id,
                        notification_type: livrarr_domain::NotificationType::BulkEnrichmentComplete,
                        ref_key: None,
                        message: format!(
                            "Retry complete: {}/{} recovered, {} still incomplete",
                            summary.recovered, summary.total, summary.still_incomplete
                        ),
                        data: serde_json::json!({
                            "total": summary.total,
                            "recovered": summary.recovered,
                            "still_incomplete": summary.still_incomplete,
                        }),
                    })
                    .await
                {
                    tracing::warn!("create_notification failed: {e}");
                }
            }
            Err(e) => {
                tracing::warn!("retry_all_incomplete failed: {e}");
            }
        }
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

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingAnchorDto {
    pub anchor_type: String,
    pub value: String,
    pub setter: String,
}

/// Canonical string form of an [`AnchorSetter`] for the DTO (matches the
/// snake_case ledger values, e.g. `auto_search`).
fn anchor_setter_str(setter: AnchorSetter) -> &'static str {
    match setter {
        AnchorSetter::User => "user",
        AnchorSetter::AutoIsbn => "auto_isbn",
        AnchorSetter::AutoSearch => "auto_search",
        AnchorSetter::Import => "import",
        AnchorSetter::Redirect => "redirect",
    }
}

/// List a work's pending (unaffirmed) identity guesses (REQ-005).
pub async fn list_pending_anchors<S: HasWorkIdentityRepository + HasWorkService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(work_id): Path<i64>,
) -> Result<Json<Vec<PendingAnchorDto>>, ApiError> {
    // R-3: the repo methods are work-id-only (no user scope), so verify ownership
    // via the user-scoped service first — another user's work must read as 404,
    // and a real service error must surface as 500, not be masked as not-found.
    state
        .work_service()
        .get(ctx.user.id, work_id)
        .await
        .map_err(|e| match e {
            WorkServiceError::NotFound => ApiError::NotFound,
            other => ApiError::Internal(other.to_string()),
        })?;

    let anchors = state
        .work_identity_repo()
        .list_anchors(work_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let dtos = anchors
        .into_iter()
        .filter(|a| a.confidence == AnchorConfidence::Pending && !a.anchor_value.is_empty())
        .map(|a| PendingAnchorDto {
            anchor_type: a.anchor_type.as_str().to_string(),
            value: a.anchor_value,
            setter: anchor_setter_str(a.setter).to_string(),
        })
        .collect();

    Ok(Json(dtos))
}

/// Affirm a pending identity guess: promote it to a confirmed anchor (synced into
/// `works.*`) and kick a background enrichment for the now-unlocked provider
/// (REQ-005).
pub async fn affirm_pending_anchor<S: HasWorkIdentityRepository + HasWorkService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path((work_id, anchor_type)): Path<(i64, String)>,
) -> Result<StatusCode, ApiError> {
    let user_id = ctx.user.id;

    // R-3: confirm_anchor mutates works.* with no user scope — verify ownership
    // before any mutation so a cross-user affirm cannot touch another's work.
    state
        .work_service()
        .get(user_id, work_id)
        .await
        .map_err(|e| match e {
            WorkServiceError::NotFound => ApiError::NotFound,
            other => ApiError::Internal(other.to_string()),
        })?;

    let anchor_type = AnchorType::new(anchor_type);

    // Resolve the pending guess of this type to its value; 404 if none to affirm.
    let anchors = state
        .work_identity_repo()
        .list_anchors(work_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let value = anchors
        .into_iter()
        .find(|a| a.confidence == AnchorConfidence::Pending && a.anchor_type == anchor_type)
        .map(|a| a.anchor_value)
        .ok_or(ApiError::NotFound)?;

    // The user verified it: promote pending→confirmed, sync works.*, and
    // immediately recompute + write the identity_status badge in one
    // atomic transaction (M-020 fix — badge must not wait for bg refresh).
    state
        .work_identity_repo()
        .confirm_anchor_and_recompute_badge(work_id, anchor_type, &value, AnchorSetter::User)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Fire-and-forget the enrichment the newly-confirmed anchor unlocks.
    let s = state.clone();
    tokio::spawn(async move {
        if let Err(e) = s
            .work_service()
            .refresh(user_id, work_id, RefreshSurface::Interactive)
            .await
        {
            tracing::debug!(work_id, "post-affirm background enrichment skipped: {e}");
        }
    });

    Ok(StatusCode::NO_CONTENT)
}
