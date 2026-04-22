use axum::extract::{Path, Query, State};
use axum::Json;

use crate::context::{HasAuthorService, HasSeriesQueryService, HasWorkService};
use crate::types::api_error::ApiError;
use crate::types::auth::AuthContext;
use crate::types::author::{
    AddAuthorApiRequest, AuthorDetailResponse, AuthorResponse, AuthorSearchResult,
    UpdateAuthorApiRequest,
};
use crate::types::work::{work_to_detail, WorkDetailResponse};
use livrarr_domain::services::{
    AddAuthorRequest, AuthorService, SeriesQueryService, UpdateAuthorRequest, WorkFilter,
    WorkService,
};
use livrarr_domain::Author;

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

#[derive(serde::Deserialize)]
pub struct LookupQuery {
    pub term: Option<String>,
}

pub async fn lookup<S: HasAuthorService>(
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

pub async fn add<S: HasAuthorService + HasSeriesQueryService>(
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
                Ok(result) => {
                    tracing::info!(
                        author_id,
                        entries = result.entries.len(),
                        "background bibliography fetch complete"
                    );
                }
                Err(e) => {
                    tracing::debug!(author_id, "background bibliography fetch skipped: {e}");
                }
            }
        });

        let gr_state = state.clone();
        let author_name = result.author().name.clone();
        let author_has_gr_key = result.author().gr_key.is_some();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            match gr_state
                .series_query_service()
                .resolve_gr_candidates(user_id, author_id)
                .await
            {
                Ok(candidates) => {
                    // Auto-link if the first candidate is a strong name match.
                    if !author_has_gr_key {
                        if let Some(first) = candidates.first() {
                            let sim =
                                livrarr_matching::author_similarity(&author_name, &first.name);
                            if sim >= 0.90 {
                                tracing::info!(
                                    author = %author_name,
                                    gr_candidate = %first.name,
                                    similarity = %sim,
                                    "auto-linking Goodreads author (background)"
                                );
                                let _ = gr_state
                                    .author_service()
                                    .update(
                                        user_id,
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
                }
                Err(e) => {
                    tracing::debug!(author_id, "background GR resolve skipped: {e}");
                }
            }
        });
    }

    Ok(Json(author_to_response(result.author())))
}

pub async fn list<S: HasAuthorService>(
    State(state): State<S>,
    ctx: AuthContext,
) -> Result<Json<Vec<AuthorResponse>>, ApiError> {
    let authors = state.author_service().list(ctx.user.id).await?;
    Ok(Json(authors.iter().map(author_to_response).collect()))
}

pub async fn get<S: HasAuthorService + HasWorkService>(
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

pub async fn update<S: HasAuthorService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(req): Json<UpdateAuthorApiRequest>,
) -> Result<Json<AuthorResponse>, ApiError> {
    use crate::types::api_error::FieldError;

    let mut errors = Vec::new();
    if matches!(req.monitored, Some(None)) {
        errors.push(FieldError {
            field: "monitored".into(),
            message: "cannot be null".into(),
        });
    }
    if matches!(req.monitor_new_items, Some(None)) {
        errors.push(FieldError {
            field: "monitorNewItems".into(),
            message: "cannot be null".into(),
        });
    }
    if !errors.is_empty() {
        return Err(ApiError::Validation { errors });
    }

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
                monitored: req.monitored.flatten(),
                monitor_new_items: req.monitor_new_items.flatten(),
            },
        )
        .await?;

    Ok(Json(author_to_response(&updated)))
}

pub async fn delete<S: HasAuthorService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    state.author_service().delete(ctx.user.id, id).await?;
    Ok(())
}

#[derive(serde::Deserialize)]
pub struct BibliographyQuery {
    pub raw: Option<bool>,
}

pub async fn bibliography<S: HasAuthorService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Query(q): Query<BibliographyQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let result = state
        .author_service()
        .bibliography(ctx.user.id, id, q.raw.unwrap_or(false))
        .await?;
    Ok(Json(bibliography_to_json(id, result)))
}

pub async fn refresh_bibliography<S: HasAuthorService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let result = state
        .author_service()
        .refresh_bibliography(ctx.user.id, id)
        .await?;
    Ok(Json(bibliography_to_json(id, result)))
}

fn bibliography_to_json(
    author_id: i64,
    result: livrarr_domain::services::BibliographyResult,
) -> serde_json::Value {
    serde_json::json!({
        "authorId": author_id,
        "entries": result.entries.into_iter().map(|e| serde_json::json!({
            "olKey": e.ol_key.unwrap_or_default(),
            "title": e.title,
            "year": e.year,
            "seriesName": e.series_name,
            "seriesPosition": e.series_position,
        })).collect::<Vec<_>>(),
        "llmFiltered": !result.raw_available || result.filtered_count != result.raw_count,
        "rawAvailable": result.raw_available,
        "filteredCount": result.filtered_count,
        "rawCount": result.raw_count,
        "fetchedAt": result.fetched_at,
    })
}
