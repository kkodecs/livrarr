use axum::extract::{Path, Query, State};
use axum::Json;

use crate::context::AppContext;
use crate::types::api_error::ApiError;
use crate::types::auth::AuthContext;
use crate::types::notification::NotificationResponse;
use crate::types::pagination::{PaginatedResponse, PaginationQuery};
use livrarr_domain::services::NotificationService;

#[derive(serde::Deserialize)]
pub struct ListQuery {
    #[serde(rename = "unreadOnly")]
    pub unread_only: Option<bool>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

pub async fn list<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Query(q): Query<ListQuery>,
) -> Result<Json<PaginatedResponse<NotificationResponse>>, ApiError> {
    let unread_only = q.unread_only.unwrap_or(false);
    let pq = PaginationQuery {
        page: q.page,
        page_size: q.page_size,
        sort_by: None,
        sort_dir: None,
    };
    let page = pq.page();
    let page_size = pq.page_size();

    let (notifs, total) = state
        .notification_service()
        .list_paginated(ctx.user.id, unread_only, page, page_size)
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

pub async fn mark_read<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    state
        .notification_service()
        .mark_read(ctx.user.id, id)
        .await?;
    Ok(())
}

pub async fn dismiss<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    state
        .notification_service()
        .dismiss(ctx.user.id, id)
        .await?;
    Ok(())
}

pub async fn dismiss_all<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
) -> Result<(), ApiError> {
    state
        .notification_service()
        .dismiss_all(ctx.user.id)
        .await?;
    Ok(())
}
