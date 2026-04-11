use axum::extract::{Path, Query, State};
use axum::Json;

use crate::state::AppState;
use crate::{ApiError, AuthContext, NotificationResponse, PaginatedResponse};
use livrarr_db::NotificationDb;

#[derive(serde::Deserialize)]
pub struct ListQuery {
    #[serde(rename = "unreadOnly")]
    pub unread_only: Option<bool>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

/// GET /api/v1/notification
pub async fn list(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(q): Query<ListQuery>,
) -> Result<Json<PaginatedResponse<NotificationResponse>>, ApiError> {
    let unread_only = q.unread_only.unwrap_or(false);
    let pq = crate::PaginationQuery {
        page: q.page,
        page_size: q.page_size,
    };
    let page = pq.page();
    let page_size = pq.page_size();

    let (notifs, total) = state
        .db
        .list_notifications_paginated(ctx.user.id, unread_only, page, page_size)
        .await?;

    Ok(Json(PaginatedResponse {
        items: notifs
            .iter()
            .map(|n| NotificationResponse {
                id: n.id,
                notification_type: n.notification_type,
                ref_key: n.ref_key.clone(),
                message: n.message.clone(),
                data: n.data.clone(),
                read: n.read,
                created_at: n.created_at.to_rfc3339(),
            })
            .collect(),
        total,
        page,
        page_size,
    }))
}

/// PUT /api/v1/notification/:id (mark read)
pub async fn mark_read(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    state.db.mark_notification_read(ctx.user.id, id).await?;
    Ok(())
}

/// DELETE /api/v1/notification/:id (dismiss)
pub async fn dismiss(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    state.db.dismiss_notification(ctx.user.id, id).await?;
    Ok(())
}

/// DELETE /api/v1/notification (dismiss all)
pub async fn dismiss_all(State(state): State<AppState>, ctx: AuthContext) -> Result<(), ApiError> {
    state.db.dismiss_all_notifications(ctx.user.id).await?;
    Ok(())
}
