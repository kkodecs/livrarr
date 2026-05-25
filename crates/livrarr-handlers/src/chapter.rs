use axum::extract::{Path, State};
use axum::Json;
use livrarr_domain::services::ChapterService;

use crate::context::HasChapterService;
use crate::types::api_error::ApiError;
use crate::types::auth::AuthContext;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterResponse {
    pub id: i64,
    pub chapter_index: i32,
    pub title: String,
    pub start_time_secs: f64,
    pub end_time_secs: f64,
}

pub async fn get_chapters<S: HasChapterService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(library_item_id): Path<i64>,
) -> Result<Json<Vec<ChapterResponse>>, ApiError> {
    let chapters = state
        .chapter_service()
        .get_chapters(ctx.user.id, library_item_id)
        .await
        .map_err(ApiError::from)?;

    let response: Vec<ChapterResponse> = chapters
        .iter()
        .map(|ch| ChapterResponse {
            id: ch.id,
            chapter_index: ch.chapter_index,
            title: ch.title.clone(),
            start_time_secs: ch.start_time_secs,
            end_time_secs: ch.end_time_secs,
        })
        .collect();

    Ok(Json(response))
}
