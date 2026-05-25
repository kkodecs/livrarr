use axum::extract::{Path, State};
use axum::Json;
use livrarr_domain::services::BookmarkService;

use crate::context::HasBookmarkService;
use crate::types::api_error::ApiError;
use crate::types::auth::AuthContext;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkResponse {
    pub id: i64,
    pub library_item_id: i64,
    pub media_type: String,
    pub position: String,
    pub sort_key: f64,
    pub name: String,
    pub chapter_title: Option<String>,
    pub paired_bookmark_id: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBookmarkRequest {
    pub position: String,
    pub sort_key: f64,
    pub name: String,
    pub chapter_title: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameBookmarkRequest {
    pub name: String,
}

fn to_response(b: &livrarr_domain::Bookmark) -> BookmarkResponse {
    BookmarkResponse {
        id: b.id,
        library_item_id: b.library_item_id,
        media_type: format!("{:?}", b.media_type).to_lowercase(),
        position: b.position.clone(),
        sort_key: b.sort_key,
        name: b.name.clone(),
        chapter_title: b.chapter_title.clone(),
        paired_bookmark_id: b.paired_bookmark_id,
        created_at: b.created_at.to_rfc3339(),
    }
}

pub async fn list_bookmarks<S: HasBookmarkService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(library_item_id): Path<i64>,
) -> Result<Json<Vec<BookmarkResponse>>, ApiError> {
    let bookmarks = state
        .bookmark_service()
        .list_bookmarks(ctx.user.id, library_item_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(bookmarks.iter().map(to_response).collect()))
}

pub async fn create_bookmark<S: HasBookmarkService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(library_item_id): Path<i64>,
    Json(body): Json<CreateBookmarkRequest>,
) -> Result<Json<BookmarkResponse>, ApiError> {
    let bookmark = state
        .bookmark_service()
        .create_bookmark(
            ctx.user.id,
            library_item_id,
            &body.position,
            body.sort_key,
            &body.name,
            body.chapter_title.as_deref(),
        )
        .await
        .map_err(ApiError::from)?;

    Ok(Json(to_response(&bookmark)))
}

pub async fn rename_bookmark<S: HasBookmarkService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(bookmark_id): Path<i64>,
    Json(body): Json<RenameBookmarkRequest>,
) -> Result<Json<()>, ApiError> {
    state
        .bookmark_service()
        .rename_bookmark(ctx.user.id, bookmark_id, &body.name)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(()))
}

pub async fn delete_bookmark<S: HasBookmarkService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(bookmark_id): Path<i64>,
) -> Result<Json<()>, ApiError> {
    state
        .bookmark_service()
        .delete_bookmark(ctx.user.id, bookmark_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(()))
}
