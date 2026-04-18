use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use crate::state::AppState;
use crate::{
    ApiError, AuthContext, GrAuthorCandidate, MonitorSeriesRequest, ResolveGrResponse,
    SeriesDetailResponse, SeriesListResponse, SeriesResponse, SeriesWithAuthorResponse,
    UpdateSeriesRequest, WorkDetailResponse,
};
use livrarr_domain::services::{
    MonitorSeriesServiceRequest, SeriesMonitorWorkerParams, SeriesQueryService,
};

fn work_to_detail(w: &livrarr_domain::Work) -> WorkDetailResponse {
    WorkDetailResponse {
        id: w.id,
        title: w.title.clone(),
        sort_title: w.sort_title.clone(),
        subtitle: w.subtitle.clone(),
        original_title: w.original_title.clone(),
        author_name: w.author_name.clone(),
        author_id: w.author_id,
        description: w.description.clone(),
        year: w.year,
        series_id: w.series_id,
        series_name: w.series_name.clone(),
        series_position: w.series_position,
        genres: w.genres.clone(),
        language: w.language.clone(),
        page_count: w.page_count,
        duration_seconds: w.duration_seconds,
        publisher: w.publisher.clone(),
        publish_date: w.publish_date.clone(),
        ol_key: w.ol_key.clone(),
        hc_key: w.hc_key.clone(),
        gr_key: w.gr_key.clone(),
        isbn_13: w.isbn_13.clone(),
        asin: w.asin.clone(),
        narrator: w.narrator.clone(),
        narration_type: w.narration_type,
        abridged: w.abridged,
        rating: w.rating,
        rating_count: w.rating_count,
        enrichment_status: w.enrichment_status,
        enriched_at: w.enriched_at.map(|d| d.to_rfc3339()),
        enrichment_source: w.enrichment_source.clone(),
        cover_manual: w.cover_manual,
        monitor_ebook: w.monitor_ebook,
        monitor_audiobook: w.monitor_audiobook,
        added_at: w.added_at.to_rfc3339(),
        library_items: vec![],
        metadata_source: w.metadata_source.clone(),
    }
}

/// GET /api/v1/series — list all series for the current user.
pub async fn list_all(
    State(state): State<AppState>,
    ctx: AuthContext,
) -> Result<Json<Vec<SeriesWithAuthorResponse>>, ApiError> {
    let views = state
        .series_query_service
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

/// GET /api/v1/series/{id} — series detail with works.
pub async fn get_detail(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<SeriesDetailResponse>, ApiError> {
    let view = state
        .series_query_service
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
                .map(|li| crate::LibraryItemResponse {
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

/// POST /api/v1/author/{id}/resolve-gr — search GR for author candidates.
pub async fn resolve_gr(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<ResolveGrResponse>, ApiError> {
    let views = state
        .series_query_service
        .resolve_gr_candidates(ctx.user.id, id)
        .await?;

    let candidates = views
        .into_iter()
        .map(|c| GrAuthorCandidate {
            gr_key: c.gr_key,
            name: c.name,
            profile_url: c.profile_url,
        })
        .collect();

    Ok(Json(ResolveGrResponse { candidates }))
}

/// GET /api/v1/author/{id}/series — list series (from cache or GR fetch on miss).
pub async fn list_series(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<SeriesListResponse>, ApiError> {
    let view = state
        .series_query_service
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

/// POST /api/v1/author/{id}/series/refresh — force re-fetch from GR.
pub async fn refresh_series(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<SeriesListResponse>, ApiError> {
    let view = state
        .series_query_service
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

/// POST /api/v1/author/{id}/series/monitor — monitor a series.
pub async fn monitor_series(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(req): Json<MonitorSeriesRequest>,
) -> Result<(StatusCode, Json<SeriesResponse>), ApiError> {
    let view = state
        .series_query_service
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

    let svc = state.series_query_service.clone();
    let user_id = ctx.user.id;
    tokio::spawn(async move {
        if let Err(e) = svc
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

/// PUT /api/v1/series/{id} — update monitoring flags (also used to unmonitor).
pub async fn update_series(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(req): Json<UpdateSeriesRequest>,
) -> Result<Json<SeriesResponse>, ApiError> {
    let view = state
        .series_query_service
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
