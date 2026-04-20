use axum::extract::{Path, Query, State};
use axum::Json;

use crate::context::AppContext;
use crate::types::api_error::ApiError;
use crate::types::auth::AuthContext;
use crate::types::author::{
    AddAuthorApiRequest, AuthorDetailResponse, AuthorResponse, AuthorSearchResult,
    UpdateAuthorApiRequest,
};
use crate::types::work::WorkDetailResponse;
use livrarr_domain::services::{
    AddAuthorRequest, AuthorService, SeriesQueryService, UpdateAuthorRequest, WorkFilter,
    WorkService,
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

pub async fn lookup<S: AppContext>(
    State(state): State<S>,
    _ctx: AuthContext,
    Query(q): Query<LookupQuery>,
) -> Result<Json<Vec<AuthorSearchResult>>, ApiError> {
    let term = q.term.unwrap_or_default();
    if term.is_empty() {
        return Ok(Json(vec![]));
    }

    let results = state.author_service().lookup(&term, 20).await?;
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

pub async fn add<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Json(req): Json<AddAuthorApiRequest>,
) -> Result<Json<AuthorResponse>, ApiError> {
    let user_id = ctx.user.id;

    let result = state
        .author_service()
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
        let bg_state = state.clone();
        let author_id = result.author().id;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            match bg_state
                .author_service()
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

        let gr_state = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            if let Err(e) = gr_state
                .series_query_service()
                .resolve_gr_candidates(user_id, author_id)
                .await
            {
                tracing::debug!(author_id, "background GR resolve skipped: {e}");
            }
        });
    }

    Ok(Json(author_to_response(result.author())))
}

pub async fn list<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
) -> Result<Json<Vec<AuthorResponse>>, ApiError> {
    let authors = state.author_service().list(ctx.user.id).await?;
    Ok(Json(authors.iter().map(author_to_response).collect()))
}

pub async fn get<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<AuthorDetailResponse>, ApiError> {
    let user_id = ctx.user.id;
    let author = state.author_service().get(user_id, id).await?;
    let works = state
        .work_service()
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

pub async fn update<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(req): Json<UpdateAuthorApiRequest>,
) -> Result<Json<AuthorResponse>, ApiError> {
    let updated = state
        .author_service()
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

pub async fn delete<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    state.author_service().delete(ctx.user.id, id).await?;
    Ok(())
}

pub async fn bibliography<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let entries = state.author_service().bibliography(ctx.user.id, id).await?;
    Ok(Json(serde_json::json!({
        "authorId": id,
        "entries": entries.into_iter().map(|e| serde_json::json!({
            "olKey": e.ol_key.unwrap_or_default(),
            "title": e.title,
            "year": e.year,
            "seriesName": null,
            "seriesPosition": null,
        })).collect::<Vec<_>>(),
        "fetchedAt": chrono::Utc::now().to_rfc3339(),
    })))
}

pub async fn refresh_bibliography<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let entries = state
        .author_service()
        .refresh_bibliography(ctx.user.id, id)
        .await?;
    Ok(Json(serde_json::json!({
        "authorId": id,
        "entries": entries.into_iter().map(|e| serde_json::json!({
            "olKey": e.ol_key.unwrap_or_default(),
            "title": e.title,
            "year": e.year,
            "seriesName": null,
            "seriesPosition": null,
        })).collect::<Vec<_>>(),
        "fetchedAt": chrono::Utc::now().to_rfc3339(),
    })))
}
