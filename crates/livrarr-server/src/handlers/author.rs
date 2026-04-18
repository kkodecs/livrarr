use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use tokio_util::sync::CancellationToken;

use crate::middleware::RequireAdmin;
use crate::state::AppState;
use crate::{
    AddAuthorApiRequest, ApiError, AuthContext, AuthorDetailResponse, AuthorResponse,
    AuthorSearchResult, UpdateAuthorApiRequest, WorkDetailResponse,
};
use livrarr_domain::services::{
    AddAuthorRequest, AuthorService, UpdateAuthorRequest, WorkFilter, WorkService,
};
use livrarr_domain::{Author, Work};

fn author_to_response(a: &Author) -> AuthorResponse {
    AuthorResponse {
        id: a.id,
        name: a.name.clone(),
        sort_name: a.sort_name.clone(),
        ol_key: a.ol_key.clone(),
        gr_key: a.gr_key.clone(),
        monitored: a.monitored,
        monitor_new_items: a.monitor_new_items,
        added_at: a.added_at.to_rfc3339(),
    }
}

fn work_to_detail(w: &Work) -> WorkDetailResponse {
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

#[derive(serde::Deserialize)]
pub struct LookupQuery {
    pub term: Option<String>,
}

/// Search OpenLibrary for authors by name. Reusable by handlers and manual import.
pub async fn lookup_ol_authors(
    http: &livrarr_http::HttpClient,
    term: &str,
    limit: u32,
) -> Result<Vec<AuthorSearchResult>, String> {
    let resp = http
        .get("https://openlibrary.org/search/authors.json")
        .query(&[("q", term), ("limit", &limit.to_string())])
        .send()
        .await
        .map_err(|e| format!("OpenLibrary request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("OpenLibrary returned {}", resp.status()));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("OpenLibrary parse error: {e}"))?;

    let docs = data
        .get("docs")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(docs
        .iter()
        .filter_map(|doc| {
            let key = doc.get("key")?.as_str()?;
            let name = doc.get("name")?.as_str()?;
            let ol_key = key.trim_start_matches("/authors/").to_string();

            Some(AuthorSearchResult {
                ol_key,
                name: name.to_string(),
                sort_name: None,
            })
        })
        .collect())
}

/// GET /api/v1/author/lookup?term=...  — searches OpenLibrary
pub async fn lookup(
    State(state): State<AppState>,
    _ctx: AuthContext,
    Query(q): Query<LookupQuery>,
) -> Result<Json<Vec<AuthorSearchResult>>, ApiError> {
    let term = q.term.unwrap_or_default();
    if term.is_empty() {
        return Ok(Json(vec![]));
    }

    let results = state.author_service.lookup(&term, 20).await?;
    Ok(Json(
        results
            .into_iter()
            .map(|r| AuthorSearchResult {
                ol_key: r.ol_key,
                name: r.name,
                sort_name: r.sort_name,
            })
            .collect(),
    ))
}

/// POST /api/v1/author
pub async fn add(
    State(state): State<AppState>,
    ctx: AuthContext,
    Json(req): Json<AddAuthorApiRequest>,
) -> Result<Json<AuthorResponse>, ApiError> {
    let user_id = ctx.user.id;

    let result = state
        .author_service
        .add(
            user_id,
            AddAuthorRequest {
                name: req.name,
                sort_name: req.sort_name,
                ol_key: Some(req.ol_key),
                monitored: false,
            },
        )
        .await?;

    if result.is_created() {
        spawn_bibliography_fetch(state.clone(), result.author().id, user_id);
    }

    Ok(Json(author_to_response(result.author())))
}

/// GET /api/v1/author
pub async fn list(
    State(state): State<AppState>,
    ctx: AuthContext,
) -> Result<Json<Vec<AuthorResponse>>, ApiError> {
    let authors = state.author_service.list(ctx.user.id).await?;
    Ok(Json(authors.iter().map(author_to_response).collect()))
}

/// GET /api/v1/author/:id
pub async fn get(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<AuthorDetailResponse>, ApiError> {
    let user_id = ctx.user.id;
    let author = state.author_service.get(user_id, id).await?;
    let works = state
        .work_service
        .list(
            user_id,
            WorkFilter {
                author_id: Some(id),
                monitored: None,
                enrichment_status: None,
                media_type: None,
                sort_by: None,
                sort_dir: None,
            },
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let author_works: Vec<WorkDetailResponse> = works.iter().map(work_to_detail).collect();

    Ok(Json(AuthorDetailResponse {
        author: author_to_response(&author),
        works: author_works,
    }))
}

/// PUT /api/v1/author/:id
pub async fn update(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(req): Json<UpdateAuthorApiRequest>,
) -> Result<Json<AuthorResponse>, ApiError> {
    let updated = state
        .author_service
        .update(
            ctx.user.id,
            id,
            UpdateAuthorRequest {
                name: None,
                sort_name: None,
                ol_key: None,
                gr_key: req.gr_key,
                monitored: req.monitored,
                monitor_new_items: req.monitor_new_items,
            },
        )
        .await?;

    Ok(Json(author_to_response(&updated)))
}

/// DELETE /api/v1/author/:id
pub async fn delete(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    state.author_service.delete(ctx.user.id, id).await?;
    Ok(())
}

/// POST /api/v1/author/search — trigger author monitor check for all monitored authors.
/// Returns 202 Accepted; work runs in the background.
pub async fn search(State(state): State<AppState>, _admin: RequireAdmin) -> StatusCode {
    tokio::spawn(async move {
        let cancel = CancellationToken::new();
        if let Err(e) = crate::jobs::author_monitor_tick(state, cancel).await {
            tracing::error!("manual author search failed: {e}");
        }
    });
    StatusCode::ACCEPTED
}

/// GET /api/v1/author/{id}/bibliography — cached author bibliography from OL + LLM cleanup.
pub async fn bibliography(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<Vec<livrarr_domain::services::BibliographyEntry>>, ApiError> {
    let entries = state.author_service.bibliography(ctx.user.id, id).await?;
    Ok(Json(entries))
}

/// POST /api/v1/author/{id}/bibliography/refresh — force re-fetch.
pub async fn refresh_bibliography(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<Vec<livrarr_domain::services::BibliographyEntry>>, ApiError> {
    let entries = state
        .author_service
        .refresh_bibliography(ctx.user.id, id)
        .await?;
    Ok(Json(entries))
}

/// Spawn a background task to fetch and cache an author's bibliography.
/// Fire-and-forget — errors are logged, never block the caller.
pub fn spawn_bibliography_fetch(state: AppState, author_id: i64, user_id: i64) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        match state
            .author_service
            .refresh_bibliography(user_id, author_id)
            .await
        {
            Ok(entries) => {
                tracing::info!(
                    author_id,
                    entries = entries.len(),
                    "background bibliography fetch complete"
                );
            }
            Err(e) => {
                tracing::debug!(author_id, "background bibliography fetch skipped: {e}");
            }
        }
    });
}
