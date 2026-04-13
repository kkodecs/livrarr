use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use crate::state::AppState;
use crate::{
    ApiError, AuthContext, GrAuthorCandidate, MonitorSeriesRequest, ResolveGrResponse,
    SeriesDetailResponse, SeriesListResponse, SeriesResponse, SeriesWithAuthorResponse,
    UpdateSeriesRequest, WorkDetailResponse,
};
use livrarr_db::{
    AuthorDb, CreateSeriesDbRequest, CreateWorkDbRequest, LibraryItemDb, LinkWorkToSeriesRequest,
    SeriesCacheDb, SeriesCacheEntry, SeriesDb, WorkDb,
};
use livrarr_metadata::goodreads;

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
    let all_series = state.db.list_all_series(ctx.user.id).await?;
    let authors = state.db.list_authors(ctx.user.id).await?;
    let works = state.db.list_works(ctx.user.id).await?;

    let results = all_series
        .iter()
        .map(|s| {
            let author_name = authors
                .iter()
                .find(|a| a.id == s.author_id)
                .map(|a| a.name.clone())
                .unwrap_or_default();
            let works_in_library =
                works.iter().filter(|w| w.series_id == Some(s.id)).count() as i64;
            // Find first work by position for cover image.
            let first_work_id = works
                .iter()
                .filter(|w| w.series_id == Some(s.id))
                .min_by(|a, b| {
                    a.series_position
                        .unwrap_or(f64::MAX)
                        .partial_cmp(&b.series_position.unwrap_or(f64::MAX))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|w| w.id);
            SeriesWithAuthorResponse {
                id: s.id,
                name: s.name.clone(),
                gr_key: s.gr_key.clone(),
                book_count: s.work_count,
                monitor_ebook: s.monitor_ebook,
                monitor_audiobook: s.monitor_audiobook,
                works_in_library,
                author_id: s.author_id,
                author_name,
                first_work_id,
            }
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
    let series = state.db.get_series(id).await?.ok_or(ApiError::NotFound)?;
    let author = state.db.get_author(ctx.user.id, series.author_id).await?;

    let all_works = state
        .db
        .list_works_by_author(ctx.user.id, series.author_id)
        .await?;

    let mut series_works: Vec<&livrarr_domain::Work> = all_works
        .iter()
        .filter(|w| w.series_id == Some(id))
        .collect();
    series_works.sort_by(|a, b| {
        a.series_position
            .unwrap_or(f64::MAX)
            .partial_cmp(&b.series_position.unwrap_or(f64::MAX))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let work_ids: Vec<i64> = series_works.iter().map(|w| w.id).collect();
    let items = state
        .db
        .list_library_items_by_work_ids(ctx.user.id, &work_ids)
        .await?;

    let mut works: Vec<WorkDetailResponse> =
        series_works.iter().map(|w| work_to_detail(w)).collect();
    for detail in &mut works {
        detail.library_items = items
            .iter()
            .filter(|li| li.work_id == detail.id)
            .map(|li| crate::LibraryItemResponse {
                id: li.id,
                path: li.path.clone(),
                media_type: li.media_type,
                file_size: li.file_size,
                imported_at: li.imported_at.to_rfc3339(),
            })
            .collect();
    }

    Ok(Json(SeriesDetailResponse {
        id: series.id,
        name: series.name,
        gr_key: series.gr_key,
        book_count: series.work_count,
        monitor_ebook: series.monitor_ebook,
        monitor_audiobook: series.monitor_audiobook,
        author_id: author.id,
        author_name: author.name,
        works,
    }))
}

/// POST /api/v1/author/{id}/resolve-gr — search GR for author candidates.
pub async fn resolve_gr(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<ResolveGrResponse>, ApiError> {
    let author = state.db.get_author(ctx.user.id, id).await?;

    let url = format!(
        "https://www.goodreads.com/search?q={}&search_type=authors",
        urlencoding::encode(&author.name)
    );

    let resp = state
        .http_client
        .get(&url)
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("GR request failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "GR returned {}",
            resp.status()
        )));
    }

    let html = resp
        .text()
        .await
        .map_err(|e| ApiError::BadGateway(format!("GR read body: {e}")))?;

    let candidates = goodreads::parse_author_search_html(&html)
        .into_iter()
        .map(|c| GrAuthorCandidate {
            gr_key: c.gr_key,
            name: c.name,
            profile_url: format!("https://www.goodreads.com{}", c.profile_url),
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
    let author = state.db.get_author(ctx.user.id, id).await?;
    let gr_key = author
        .gr_key
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest("Author has no Goodreads key".into()))?;

    // Try cache first.
    let cache = state.db.get_series_cache(id).await.unwrap_or(None);
    let (cache_entries, fetched_at) = if let Some(cached) = cache {
        (cached.entries, Some(cached.fetched_at))
    } else {
        // Cache miss — fetch from GR synchronously.
        let entries = fetch_author_series_from_gr(&state, gr_key).await?;
        let saved = state
            .db
            .save_series_cache(id, &entries)
            .await
            .map_err(|e| ApiError::Internal(format!("save cache: {e}")))?;
        (saved.entries, Some(saved.fetched_at))
    };

    // Merge with DB series for monitoring state.
    let db_series = state
        .db
        .list_series_for_author(ctx.user.id, id)
        .await
        .unwrap_or_default();

    let series = build_series_response(&state, ctx.user.id, id, &cache_entries, &db_series).await;

    Ok(Json(SeriesListResponse { series, fetched_at }))
}

/// POST /api/v1/author/{id}/series/refresh — force re-fetch from GR.
pub async fn refresh_series(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<SeriesListResponse>, ApiError> {
    let author = state.db.get_author(ctx.user.id, id).await?;
    let gr_key = author
        .gr_key
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest("Author has no Goodreads key".into()))?;

    // Clear cache and re-fetch.
    let _ = state.db.delete_series_cache(id).await;
    let entries = fetch_author_series_from_gr(&state, gr_key).await?;
    let saved = state
        .db
        .save_series_cache(id, &entries)
        .await
        .map_err(|e| ApiError::Internal(format!("save cache: {e}")))?;

    let db_series = state
        .db
        .list_series_for_author(ctx.user.id, id)
        .await
        .unwrap_or_default();

    let series = build_series_response(&state, ctx.user.id, id, &saved.entries, &db_series).await;

    Ok(Json(SeriesListResponse {
        series,
        fetched_at: Some(saved.fetched_at),
    }))
}

/// POST /api/v1/author/{id}/series/monitor — monitor a series.
pub async fn monitor_series(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(req): Json<MonitorSeriesRequest>,
) -> Result<(StatusCode, Json<SeriesResponse>), ApiError> {
    let author = state.db.get_author(ctx.user.id, id).await?;

    tracing::info!(
        author_id = id,
        author_name = %author.name,
        gr_key = %req.gr_key,
        monitor_ebook = req.monitor_ebook,
        monitor_audiobook = req.monitor_audiobook,
        "monitor_series request received"
    );

    // Validate grKey against cache.
    let cache = state
        .db
        .get_series_cache(id)
        .await
        .unwrap_or(None)
        .ok_or_else(|| ApiError::BadRequest("Fetch series list first".into()))?;

    tracing::info!(
        author_id = id,
        cache_entries = cache.entries.len(),
        cache_gr_keys = ?cache.entries.iter().map(|e| e.gr_key.as_str()).collect::<Vec<_>>(),
        requested_gr_key = %req.gr_key,
        "cache lookup"
    );

    let cache_entry = cache
        .entries
        .iter()
        .find(|e| e.gr_key == req.gr_key)
        .ok_or_else(|| {
            tracing::warn!(
                author_id = id,
                author_name = %author.name,
                requested_gr_key = %req.gr_key,
                available_gr_keys = ?cache.entries.iter().map(|e| format!("{}={}", e.gr_key, e.name)).collect::<Vec<_>>(),
                "grKey not found in cache"
            );
            ApiError::BadRequest(format!("Series {} not found in cache", req.gr_key))
        })?;

    // Upsert series row.
    let series = state
        .db
        .upsert_series(CreateSeriesDbRequest {
            user_id: ctx.user.id,
            author_id: id,
            name: cache_entry.name.clone(),
            gr_key: req.gr_key.clone(),
            monitor_ebook: req.monitor_ebook,
            monitor_audiobook: req.monitor_audiobook,
            work_count: cache_entry.book_count,
        })
        .await?;

    // Spawn background task to fetch series detail and create works.
    let bg_state = state.clone();
    let bg_author = author.clone();
    let bg_series_id = series.id;
    let bg_series_name = series.name.clone();
    let bg_gr_key = req.gr_key.clone();
    tokio::spawn(async move {
        if let Err(e) = series_monitor_worker(
            &bg_state,
            &bg_author,
            bg_series_id,
            &bg_series_name,
            &bg_gr_key,
        )
        .await
        {
            tracing::warn!(
                series = %bg_series_name,
                author = %bg_author.name,
                "series monitor worker failed: {e}"
            );
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(SeriesResponse {
            id: Some(series.id),
            name: series.name,
            gr_key: series.gr_key,
            book_count: series.work_count,
            monitor_ebook: series.monitor_ebook,
            monitor_audiobook: series.monitor_audiobook,
            works_in_library: 0, // Will be populated after background task completes.
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
    // Verify ownership.
    let series = state.db.get_series(id).await?.ok_or(ApiError::NotFound)?;
    let _author = state.db.get_author(ctx.user.id, series.author_id).await?;

    let updated = state
        .db
        .update_series_flags(id, req.monitor_ebook, req.monitor_audiobook)
        .await?;

    // Count works in library for this series.
    let works = state
        .db
        .list_works_by_author(ctx.user.id, series.author_id)
        .await
        .unwrap_or_default();
    let count = works.iter().filter(|w| w.series_id == Some(id)).count() as i64;

    Ok(Json(SeriesResponse {
        id: Some(updated.id),
        name: updated.name,
        gr_key: updated.gr_key,
        book_count: updated.work_count,
        monitor_ebook: updated.monitor_ebook,
        monitor_audiobook: updated.monitor_audiobook,
        works_in_library: count,
    }))
}

// =============================================================================
// Helpers
// =============================================================================

/// Fetch author series list from GR, handling pagination.
async fn fetch_author_series_from_gr(
    state: &AppState,
    gr_author_id: &str,
) -> Result<Vec<SeriesCacheEntry>, ApiError> {
    let mut all_entries = Vec::new();
    let mut page = 1;

    loop {
        let url = format!(
            "https://www.goodreads.com/series/list?id={}&page={}",
            gr_author_id, page
        );

        let resp = state
            .http_client
            .get(&url)
            .send()
            .await
            .map_err(|e| ApiError::BadGateway(format!("GR series list: {e}")))?;

        if !resp.status().is_success() {
            return Err(ApiError::BadGateway(format!(
                "GR series list returned {}",
                resp.status()
            )));
        }

        let html = resp
            .text()
            .await
            .map_err(|e| ApiError::BadGateway(format!("GR read body: {e}")))?;

        let (entries, has_next) = goodreads::parse_series_list_html(&html);

        if entries.is_empty() {
            break;
        }

        all_entries.extend(entries.into_iter().map(|e| SeriesCacheEntry {
            name: e.name,
            gr_key: e.gr_key,
            book_count: e.book_count,
        }));

        if !has_next || page >= 10 {
            break;
        }

        page += 1;
        // Rate limiting: 1s delay between pages.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    Ok(all_entries)
}

/// Build the merged series response from cache + DB series.
async fn build_series_response(
    state: &AppState,
    user_id: i64,
    author_id: i64,
    cache_entries: &[SeriesCacheEntry],
    db_series: &[livrarr_domain::Series],
) -> Vec<SeriesResponse> {
    // Get all works for this author to compute worksInLibrary.
    let works = state
        .db
        .list_works_by_author(user_id, author_id)
        .await
        .unwrap_or_default();

    cache_entries
        .iter()
        .map(|ce| {
            // Check if this series is tracked in DB.
            let db_match = db_series.iter().find(|s| s.gr_key == ce.gr_key);

            let (id, monitor_ebook, monitor_audiobook) = if let Some(s) = db_match {
                (Some(s.id), s.monitor_ebook, s.monitor_audiobook)
            } else {
                (None, false, false)
            };

            // Count works in library for this series.
            let works_in_library = if let Some(s) = db_match {
                works.iter().filter(|w| w.series_id == Some(s.id)).count() as i64
            } else {
                // Best-effort: match by series_name string.
                works
                    .iter()
                    .filter(|w| w.series_name.as_deref() == Some(&ce.name))
                    .count() as i64
            };

            SeriesResponse {
                id,
                name: ce.name.clone(),
                gr_key: ce.gr_key.clone(),
                book_count: ce.book_count,
                monitor_ebook,
                monitor_audiobook,
                works_in_library,
            }
        })
        .collect()
}

/// Normalize a string for matching: lowercase, strip non-alphanumeric.
fn normalize_for_match(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Background worker: fetch series detail from GR and create/link works.
pub async fn series_monitor_worker(
    state: &AppState,
    author: &livrarr_domain::Author,
    series_id: i64,
    series_name: &str,
    series_gr_key: &str,
) -> Result<(), String> {
    // Fetch series detail with pagination.
    let mut all_books = Vec::new();
    let mut page = 1;

    loop {
        let url = if page == 1 {
            format!("https://www.goodreads.com/series/{}", series_gr_key)
        } else {
            format!(
                "https://www.goodreads.com/series/{}?page={}",
                series_gr_key, page
            )
        };

        let resp = state
            .http_client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("GR series detail: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("GR series detail returned {}", resp.status()));
        }

        let html = resp
            .text()
            .await
            .map_err(|e| format!("GR read body: {e}"))?;

        let (books, has_next) = goodreads::parse_series_detail_html(&html);

        if books.is_empty() {
            break;
        }

        all_books.extend(books);

        if !has_next || page >= 10 {
            break;
        }

        page += 1;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    // Filter to primary works only: integer positions (1.0, 2.0, ...).
    // Excludes novellas (2.5), companions, part splits, and works with no position.
    let primary_books: Vec<_> = all_books
        .into_iter()
        .filter(|b| {
            b.position
                .map(|p| p > 0.0 && p.fract() == 0.0)
                .unwrap_or(false)
        })
        .collect();

    tracing::info!(
        series = %series_name,
        author = %author.name,
        books = primary_books.len(),
        "series detail fetched (primary works only)"
    );

    let all_books = primary_books;

    // Re-read current series flags (cancellation guard).
    let series = state
        .db
        .get_series(series_id)
        .await
        .map_err(|e| format!("get series: {e}"))?
        .ok_or("series not found")?;

    if !series.monitor_ebook && !series.monitor_audiobook {
        tracing::info!(series = %series_name, "series unmonitored — skipping work creation");
        return Ok(());
    }

    // Update work_count from actual GR data.
    let _ = state
        .db
        .update_series_work_count(series_id, all_books.len() as i32)
        .await;

    // Get all existing works by this author for matching.
    let existing_works = state
        .db
        .list_works_by_author(author.user_id, author.id)
        .await
        .map_err(|e| format!("list works: {e}"))?;

    let mut created = 0u32;
    let mut linked = 0u32;
    let mut skipped = 0u32;
    let max_works = 50;

    for book in &all_books {
        if created >= max_works {
            tracing::warn!(series = %series_name, "hit max works cap ({max_works})");
            break;
        }

        // Re-read flags for each work (guard against concurrent unmonitor).
        let current = state
            .db
            .get_series(series_id)
            .await
            .map_err(|e| format!("get series: {e}"))?;
        if let Some(s) = &current {
            if !s.monitor_ebook && !s.monitor_audiobook {
                tracing::info!(series = %series_name, "series unmonitored mid-task — stopping");
                break;
            }
        }

        // Match rule 1: exact gr_key.
        let matched = existing_works
            .iter()
            .find(|w| w.gr_key.as_deref() == Some(&book.gr_key));

        if let Some(existing) = matched {
            // Link existing work to series (with assignment guard).
            let _ = state
                .db
                .link_work_to_series(LinkWorkToSeriesRequest {
                    work_id: existing.id,
                    series_id,
                    series_work_count: series.work_count,
                    series_name: series_name.to_string(),
                    series_position: book.position,
                    monitor_ebook: series.monitor_ebook,
                    monitor_audiobook: series.monitor_audiobook,
                })
                .await;
            linked += 1;
            continue;
        }

        // Match rule 2: normalized title (scoped to author already).
        let norm_title = normalize_for_match(&book.title);
        let title_matched = existing_works
            .iter()
            .find(|w| normalize_for_match(&w.title) == norm_title);

        if let Some(existing) = title_matched {
            let _ = state
                .db
                .link_work_to_series(LinkWorkToSeriesRequest {
                    work_id: existing.id,
                    series_id,
                    series_work_count: series.work_count,
                    series_name: series_name.to_string(),
                    series_position: book.position,
                    monitor_ebook: series.monitor_ebook,
                    monitor_audiobook: series.monitor_audiobook,
                })
                .await;
            linked += 1;
            continue;
        }

        // No match — create new work.
        match state
            .db
            .create_work(CreateWorkDbRequest {
                user_id: author.user_id,
                title: book.title.clone(),
                author_name: author.name.clone(),
                author_id: Some(author.id),
                ol_key: None,
                gr_key: Some(book.gr_key.clone()),
                year: book.year,
                cover_url: None,
                metadata_source: None,
                detail_url: None,
                language: None,
                import_id: None,
                series_id: Some(series_id),
                series_name: Some(series_name.to_string()),
                series_position: book.position,
                monitor_ebook: series.monitor_ebook,
                monitor_audiobook: series.monitor_audiobook,
            })
            .await
        {
            Ok(work) => {
                created += 1;
                tracing::debug!(
                    work_id = work.id,
                    title = %book.title,
                    "created work from series"
                );
                // Enrich in background (best-effort, 30s timeout per work).
                let enrich_state = state.clone();
                let work_clone = work.clone();
                tokio::spawn(async move {
                    let outcome = tokio::time::timeout(
                        std::time::Duration::from_secs(30),
                        super::enrichment::enrich_work(&enrich_state, &work_clone),
                    )
                    .await;
                    match outcome {
                        Ok(o) => {
                            let _ = enrich_state
                                .db
                                .update_work_enrichment(
                                    work_clone.user_id,
                                    work_clone.id,
                                    o.request,
                                )
                                .await;
                        }
                        Err(_) => {
                            tracing::warn!(work_id = work_clone.id, "enrichment timed out");
                        }
                    }
                });
            }
            Err(e) => {
                tracing::warn!(title = %book.title, "failed to create work: {e}");
            }
        }
    }

    tracing::info!(
        series = %series_name,
        author = %author.name,
        created,
        linked,
        skipped,
        "series monitor worker complete"
    );

    Ok(())
}
