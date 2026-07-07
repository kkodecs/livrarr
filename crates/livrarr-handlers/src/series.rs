use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;

use crate::context::{HasAuthorService, HasSeriesQueryService};
use crate::types::api_error::ApiError;
use crate::types::auth::AuthContext;
use crate::types::series::{
    GrAuthorCandidate, MonitorSeriesRequest, PromoteSeriesRequest, PromoteSeriesResponse,
    ResolveGrResponse, SeriesBookRowResponse, SeriesBooksResponse, SeriesDetailResponse,
    SeriesListResponse, SeriesResponse, SeriesWithAuthorResponse, UpdateSeriesRequest,
};
use crate::types::work::work_to_detail;
use crate::LibraryItemResponse;
use livrarr_domain::services::{
    AuthorService, MonitorSeriesServiceRequest, SeriesMonitorWorkerParams, SeriesQueryService,
    UpdateAuthorRequest,
};

pub async fn list_all<S: HasSeriesQueryService>(
    State(state): State<S>,
    ctx: AuthContext,
) -> Result<Json<Vec<SeriesWithAuthorResponse>>, ApiError> {
    let views = state
        .series_query_service()
        .list_enriched(ctx.user.id)
        .await?;
    let results = views
        .into_iter()
        .map(|v| {
            // Stub internals never leave the API: the "stub:" key reads as
            // keyless (UI hides GR links) and the sentinel count as 0 (UI
            // shows the FK-linked works_in_library instead).
            let is_stub = livrarr_domain::is_series_stub_key(&v.gr_key);
            SeriesWithAuthorResponse {
                id: v.id,
                name: v.name,
                gr_key: if is_stub { String::new() } else { v.gr_key },
                book_count: if is_stub { 0 } else { v.book_count },
                monitor_ebook: v.monitor_ebook,
                monitor_audiobook: v.monitor_audiobook,
                monitor_language: v.monitor_language,
                suggested_language: v.suggested_language,
                works_in_library: v.works_in_library,
                author_id: v.author_id,
                author_name: v.author_name,
                first_work_id: v.first_work_id,
            }
        })
        .collect();
    Ok(Json(results))
}

pub async fn get_detail<S: HasSeriesQueryService>(
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
                    progress_pct: None,
                    duration_seconds: li.duration_seconds,
                    finished_at: None,
                })
                .collect();
            detail
        })
        .collect();

    let is_stub = livrarr_domain::is_series_stub_key(&view.gr_key);
    Ok(Json(SeriesDetailResponse {
        id: view.id,
        name: view.name,
        gr_key: if is_stub { String::new() } else { view.gr_key },
        book_count: if is_stub { 0 } else { view.book_count },
        monitor_ebook: view.monitor_ebook,
        monitor_audiobook: view.monitor_audiobook,
        author_id: view.author_id,
        author_name: view.author_name,
        works,
    }))
}

pub async fn resolve_gr<S: HasSeriesQueryService + HasAuthorService>(
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
                            monitor_language: None,
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

#[derive(serde::Deserialize)]
pub struct SeriesListQuery {
    pub raw: Option<bool>,
}

pub async fn list_series<S: HasSeriesQueryService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Query(q): Query<SeriesListQuery>,
) -> Result<Json<SeriesListResponse>, ApiError> {
    let view = state
        .series_query_service()
        .list_author_series(ctx.user.id, id, q.raw.unwrap_or(false))
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
            language: s.language,
        })
        .collect();

    Ok(Json(SeriesListResponse {
        series,
        fetched_at: view.fetched_at,
        raw_available: view.raw_available,
        filtered_count: view.filtered_count,
        raw_count: view.raw_count,
    }))
}

pub async fn refresh_series<S: HasSeriesQueryService>(
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
            language: s.language,
        })
        .collect();

    Ok(Json(SeriesListResponse {
        series,
        fetched_at: view.fetched_at,
        raw_available: view.raw_available,
        filtered_count: view.filtered_count,
        raw_count: view.raw_count,
    }))
}

pub async fn monitor_series<S: HasSeriesQueryService>(
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
                language: req.language.clone(),
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
            language: view.language,
        }),
    ))
}

/// POST /series/{id}/promote — stub promotion (REQ-009). Resolves the stub's
/// gr_key (exact-match silent, picker on ambiguity, author resolution first
/// when the author has no key), then runs the existing monitor flow on the
/// surviving row. Never enables monitoring without a resolved gr_key.
pub async fn promote_series<S: HasSeriesQueryService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(req): Json<PromoteSeriesRequest>,
) -> Result<(StatusCode, Json<PromoteSeriesResponse>), ApiError> {
    use livrarr_domain::services::PromoteStubOutcome;

    let outcome = state
        .series_query_service()
        .promote_stub(ctx.user.id, id, req.gr_key.clone())
        .await?;

    let (author_id, gr_key, series_name) = match outcome {
        PromoteStubOutcome::NeedsAuthorResolution { author_id } => {
            // 200 with a tagged status (not 4xx): the API client treats
            // non-2xx as thrown errors, and these are flow outcomes.
            return Ok((
                StatusCode::OK,
                Json(PromoteSeriesResponse {
                    status: "needsAuthorResolution".into(),
                    author_id,
                    series: None,
                    candidates: Vec::new(),
                }),
            ));
        }
        PromoteStubOutcome::NeedsPicker {
            author_id,
            candidates,
        } => {
            return Ok((
                StatusCode::OK,
                Json(PromoteSeriesResponse {
                    status: "needsPicker".into(),
                    author_id,
                    series: None,
                    candidates: candidates
                        .into_iter()
                        .map(|c| SeriesResponse {
                            id: c.id,
                            name: c.name,
                            gr_key: c.gr_key,
                            book_count: c.book_count,
                            monitor_ebook: c.monitor_ebook,
                            monitor_audiobook: c.monitor_audiobook,
                            works_in_library: c.works_in_library,
                            language: c.language,
                        })
                        .collect(),
                }),
            ));
        }
        PromoteStubOutcome::Resolved {
            author_id,
            gr_key,
            name,
            ..
        } => (author_id, gr_key, name),
    };

    // Monitor flow on the resolved row — same road as monitor_series: the
    // upsert keys on (user, author, gr_key), so it lands on the surviving
    // row; then the existing async discovery worker (202, unchanged).
    let view = state
        .series_query_service()
        .monitor_series(
            ctx.user.id,
            author_id,
            MonitorSeriesServiceRequest {
                gr_key: gr_key.clone(),
                monitor_ebook: req.monitor_ebook,
                monitor_audiobook: req.monitor_audiobook,
                language: req.language.clone(),
            },
        )
        .await?;

    let series_id = view.id;
    let monitor_ebook = view.monitor_ebook;
    let monitor_audiobook = view.monitor_audiobook;

    let bg_state = state.clone();
    let user_id = ctx.user.id;
    let worker_name = series_name.clone();
    tokio::spawn(async move {
        if let Err(e) = bg_state
            .series_query_service()
            .run_series_monitor_worker(SeriesMonitorWorkerParams {
                user_id,
                author_id,
                series_id,
                series_name: worker_name.clone(),
                series_gr_key: gr_key,
                monitor_ebook,
                monitor_audiobook,
            })
            .await
        {
            tracing::warn!(
                series = %worker_name,
                "series monitor worker failed after promotion: {e}"
            );
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(PromoteSeriesResponse {
            status: "monitoring".into(),
            author_id,
            series: Some(SeriesResponse {
                id: Some(view.id),
                name: view.name,
                gr_key: view.gr_key,
                book_count: view.book_count,
                monitor_ebook: view.monitor_ebook,
                monitor_audiobook: view.monitor_audiobook,
                works_in_library: view.works_in_library,
                language: view.language,
            }),
            candidates: Vec::new(),
        }),
    ))
}

/// GET /series/{id}/books — full-roster expansion (REQ-010): persisted GR
/// roster merged with the library's linked works. Display-only road.
pub async fn series_books<S: HasSeriesQueryService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<SeriesBooksResponse>, ApiError> {
    use livrarr_domain::services::SeriesBookRow;

    let view = state
        .series_query_service()
        .series_books(ctx.user.id, id)
        .await?;

    let rows = view
        .rows
        .into_iter()
        .map(|row| match row {
            SeriesBookRow::InLibrary { position, entry } => {
                let mut detail = work_to_detail(&entry.work);
                detail.library_items = entry
                    .library_items
                    .iter()
                    .map(|li| LibraryItemResponse {
                        id: li.id,
                        path: li.path.clone(),
                        media_type: li.media_type,
                        file_size: li.file_size,
                        imported_at: li.imported_at.to_rfc3339(),
                        progress_pct: None,
                        duration_seconds: li.duration_seconds,
                        finished_at: None,
                    })
                    .collect();
                SeriesBookRowResponse {
                    position,
                    in_library: true,
                    title: detail.title.clone(),
                    year: None,
                    work: Some(detail),
                }
            }
            SeriesBookRow::Missing {
                position,
                title,
                year,
            } => SeriesBookRowResponse {
                position,
                in_library: false,
                title,
                year,
                work: None,
            },
        })
        .collect();

    Ok(Json(SeriesBooksResponse {
        roster_available: view.roster_available,
        rows,
    }))
}

pub async fn update_series<S: HasSeriesQueryService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(req): Json<UpdateSeriesRequest>,
) -> Result<Json<SeriesResponse>, ApiError> {
    let view = state
        .series_query_service()
        .update_flags(
            ctx.user.id,
            id,
            req.monitor_ebook,
            req.monitor_audiobook,
            req.language.clone(),
        )
        .await?;

    let masked_gr_key = if livrarr_domain::is_series_stub_key(&view.gr_key) {
        String::new()
    } else {
        view.gr_key
    };
    Ok(Json(SeriesResponse {
        id: Some(view.id),
        name: view.name,
        gr_key: masked_gr_key,
        book_count: view.book_count,
        monitor_ebook: view.monitor_ebook,
        monitor_audiobook: view.monitor_audiobook,
        works_in_library: view.works_in_library,
        language: view.language,
    }))
}
