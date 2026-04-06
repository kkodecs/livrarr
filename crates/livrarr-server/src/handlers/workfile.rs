use axum::extract::{Path, State};
use axum::Json;

use crate::state::AppState;
use crate::{ApiError, AuthContext, LibraryItemResponse};
use livrarr_db::LibraryItemDb;

fn to_response(li: &livrarr_domain::LibraryItem) -> LibraryItemResponse {
    LibraryItemResponse {
        id: li.id,
        path: li.path.clone(),
        media_type: li.media_type,
        file_size: li.file_size,
        imported_at: li.imported_at.to_rfc3339(),
    }
}

/// GET /api/v1/workfile
pub async fn list(
    State(state): State<AppState>,
    ctx: AuthContext,
) -> Result<Json<Vec<LibraryItemResponse>>, ApiError> {
    let items = state.db.list_library_items(ctx.user.id).await?;
    Ok(Json(items.iter().map(to_response).collect()))
}

/// GET /api/v1/workfile/:id
pub async fn get(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<LibraryItemResponse>, ApiError> {
    let item = state.db.get_library_item(ctx.user.id, id).await?;
    Ok(Json(to_response(&item)))
}

/// DELETE /api/v1/workfile/:id
pub async fn delete(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    let _item = state.db.delete_library_item(ctx.user.id, id).await?;
    Ok(())
}
