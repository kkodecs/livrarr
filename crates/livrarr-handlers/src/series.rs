use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use crate::context::AppContext;
use crate::types::api_error::ApiError;
use crate::types::auth::AuthContext;
use crate::types::series::{
    GrAuthorCandidate, MonitorSeriesRequest, ResolveGrResponse, SeriesDetailResponse,
    SeriesListResponse, SeriesResponse, SeriesWithAuthorResponse, UpdateSeriesRequest,
};
use crate::types::work::work_to_detail;
use crate::LibraryItemResponse;
use livrarr_domain::services::{
    AuthorService, MonitorSeriesServiceRequest, SeriesMonitorWorkerParams, SeriesQueryService,
    UpdateAuthorRequest,
};

pub async fn list_all<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
) -> Result<Json<Vec<SeriesWithAuthorResponse>>, ApiError> {
    let views = state
        .series_query_service()
        .list_enriched(ctx.user.id)
        .await?;
    let results = views
        .into_iter()
        .map(|v| SeriesWithAuthorResponse {
            id: v.id,
            name: v.name,
            gr_key: v.gr_key,
            book_count: v.book_count,
            monitor_ebook: v.monitor_ebook,
            monitor_audiobook: v.monitor_audiobook,
            works_in_library: v.works_in_library,
            author_id: v.author_id,
            author_name: v.author_name,
            first_work_id: v.first_work_id,
        })
        .collect();
    Ok(Json(results))
}

pub async fn get_detail<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<SeriesDetailResponse>, ApiError> {
    let view = state
        .series_query_service()
        .get_detail(ctx.user.id, id)
        .await?;

    let works = view
        .works
        .iter()
        .map(|sw| {
            let mut detail = work_to_detail(&sw.work);
            detail.library_items = sw
                .library_items
                .iter()
                .map(|li| LibraryItemResponse {
                    id: li.id,
                    path: li.path.clone(),
                    media_type: li.media_type,
                    file_size: li.file_size,
                    imported_at: li.imported_at.to_rfc3339(),
                })
                .collect();
            detail
        })
        .collect();

    Ok(Json(SeriesDetailResponse {
        id: view.id,
        name: view.name,
        gr_key: view.gr_key,
        book_count: view.book_count,
        monitor_ebook: view.monitor_ebook,
        monitor_audiobook: view.monitor_audiobook,
        author_id: view.author_id,
        author_name: view.author_name,
        works,
    }))
}

pub async fn resolve_gr<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<ResolveGrResponse>, ApiError> {
    let author = state.author_service().get(ctx.user.id, id).await?;
    let had_gr_key = author.gr_key.is_some();

    let views = state
        .series_query_service()
        .resolve_gr_candidates(ctx.user.id, id)
        .await?;

    // Auto-link if the first candidate is a strong name match (handler-level side effect).
    let mut auto_linked = false;
    if !had_gr_key {
        if let Some(first) = views.first() {
            let sim = livrarr_matching::author_similarity(&author.name, &first.name);
            if sim >= 0.90 {
                tracing::info!(
                    author = %author.name,
                    gr_candidate = %first.name,
                    similarity = %sim,
                    "auto-linking Goodreads author"
                );
                state
                    .author_service()
                    .update(
                        ctx.user.id,
                        id,
                        UpdateAuthorRequest {
                            name: None,
                            sort_name: None,
                            ol_key: None,
                            gr_key: Some(Some(first.gr_key.clone())),
                            monitored: None,
                            monitor_new_items: None,
                        },
                    )
                    .await?;
                auto_linked = true;
            }
        }
    }

    let candidates = views
        .into_iter()
        .map(|c| GrAuthorCandidate {
            gr_key: c.gr_key,
            name: c.name,
            profile_url: c.profile_url,
        })
        .collect();

    Ok(Json(ResolveGrResponse {
        candidates,
        auto_linked,
    }))
}

pub async fn list_series<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<SeriesListResponse>, ApiError> {
    let view = state
        .series_query_service()
        .list_author_series(ctx.user.id, id)
        .await?;

    let series = view
        .series
        .into_iter()
        .map(|s| SeriesResponse {
            id: s.id,
            name: s.name,
            gr_key: s.gr_key,
            book_count: s.book_count,
            monitor_ebook: s.monitor_ebook,
            monitor_audiobook: s.monitor_audiobook,
            works_in_library: s.works_in_library,
        })
        .collect();

    Ok(Json(SeriesListResponse {
        series,
        fetched_at: view.fetched_at,
    }))
}

pub async fn refresh_series<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<SeriesListResponse>, ApiError> {
    let view = state
        .series_query_service()
        .refresh_author_series(ctx.user.id, id)
        .await?;

    let series = view
        .series
        .into_iter()
        .map(|s| SeriesResponse {
            id: s.id,
            name: s.name,
            gr_key: s.gr_key,
            book_count: s.book_count,
            monitor_ebook: s.monitor_ebook,
            monitor_audiobook: s.monitor_audiobook,
            works_in_library: s.works_in_library,
        })
        .collect();

    Ok(Json(SeriesListResponse {
        series,
        fetched_at: view.fetched_at,
    }))
}

pub async fn monitor_series<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(req): Json<MonitorSeriesRequest>,
) -> Result<(StatusCode, Json<SeriesResponse>), ApiError> {
    let view = state
        .series_query_service()
        .monitor_series(
            ctx.user.id,
            id,
            MonitorSeriesServiceRequest {
                gr_key: req.gr_key.clone(),
                monitor_ebook: req.monitor_ebook,
                monitor_audiobook: req.monitor_audiobook,
            },
        )
        .await?;

    let series_id = view.id;
    let series_name = view.name.clone();
    let gr_key = view.gr_key.clone();
    let monitor_ebook = view.monitor_ebook;
    let monitor_audiobook = view.monitor_audiobook;

    let bg_state = state.clone();
    let user_id = ctx.user.id;
    tokio::spawn(async move {
        if let Err(e) = bg_state
            .series_query_service()
            .run_series_monitor_worker(SeriesMonitorWorkerParams {
                user_id,
                author_id: id,
                series_id,
                series_name: series_name.clone(),
                series_gr_key: gr_key,
                monitor_ebook,
                monitor_audiobook,
            })
            .await
        {
            tracing::warn!(
                series = %series_name,
                "series monitor worker failed: {e}"
            );
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(SeriesResponse {
            id: Some(view.id),
            name: view.name,
            gr_key: view.gr_key,
            book_count: view.book_count,
            monitor_ebook: view.monitor_ebook,
            monitor_audiobook: view.monitor_audiobook,
            works_in_library: 0,
        }),
    ))
}

pub async fn update_series<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(req): Json<UpdateSeriesRequest>,
) -> Result<Json<SeriesResponse>, ApiError> {
    let view = state
        .series_query_service()
        .update_flags(ctx.user.id, id, req.monitor_ebook, req.monitor_audiobook)
        .await?;

    Ok(Json(SeriesResponse {
        id: Some(view.id),
        name: view.name,
        gr_key: view.gr_key,
        book_count: view.book_count,
        monitor_ebook: view.monitor_ebook,
        monitor_audiobook: view.monitor_audiobook,
        works_in_library: view.works_in_library,
    }))
}
