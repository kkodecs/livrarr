use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use livrarr_domain::services::CrossFormatService;

use crate::context::HasCrossFormatService;
use crate::types::api_error::ApiError;
use crate::types::auth::AuthContext;

#[derive(Debug, serde::Deserialize)]
pub struct CurrentTsQuery {
    pub current_ts: f64,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumePromptResponse {
    pub format: String,
    pub position: String,
    pub label: String,
}

#[derive(Debug, serde::Serialize)]
pub struct AnchorResponse {
    pub cfi: String,
    pub ts: f64,
}

fn guard_ts(ts: f64) -> Result<(), ApiError> {
    if !ts.is_finite() || ts < 0.0 {
        return Err(ApiError::BadRequest(
            "current_ts must be a finite non-negative number".into(),
        ));
    }
    Ok(())
}

/// GET /workfile/{id}/cross-format/prompt?current_ts= — `null` body when no
/// prompt applies (unlinked, invalid link, suppressed, or not ahead).
pub async fn get_resume_prompt<S: HasCrossFormatService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Query(q): Query<CurrentTsQuery>,
) -> Result<Json<Option<ResumePromptResponse>>, ApiError> {
    guard_ts(q.current_ts)?;
    let prompt = state
        .cross_format_service()
        .resume_prompt(ctx.user.id, id, q.current_ts)
        .await?;
    Ok(Json(prompt.map(|p| ResumePromptResponse {
        format: format!("{:?}", p.format).to_lowercase(),
        position: p.position,
        label: p.label,
    })))
}

/// GET /workfile/{id}/cross-format/anchors — 404 when unlinked or stale (the
/// reader treats any error as "no cross-format here").
pub async fn get_anchors<S: HasCrossFormatService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<Vec<AnchorResponse>>, ApiError> {
    let anchors = state
        .cross_format_service()
        .anchors_for_item(ctx.user.id, id)
        .await?;
    Ok(Json(
        anchors
            .into_iter()
            .map(|a| AnchorResponse {
                cfi: a.cfi,
                ts: a.ts,
            })
            .collect(),
    ))
}

/// POST /workfile/{id}/cross-format/decline
pub async fn post_decline<S: HasCrossFormatService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state
        .cross_format_service()
        .decline_resume(ctx.user.id, id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /workfile/{id}/cross-format/sync?current_ts=
pub async fn post_sync_to_here<S: HasCrossFormatService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Query(q): Query<CurrentTsQuery>,
) -> Result<StatusCode, ApiError> {
    guard_ts(q.current_ts)?;
    state
        .cross_format_service()
        .sync_to_here(ctx.user.id, id, q.current_ts)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
