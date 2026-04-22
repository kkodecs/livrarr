use axum::extract::{Path, Query, State};
use axum::Json;

use futures::stream::{self, StreamExt};

use crate::context::AppContext;
use crate::{ApiError, AuthContext, QueueItemResponse, QueueListResponse};
use livrarr_domain::services::{GrabService, ImportService, QueueService};
use livrarr_domain::{GrabStatus, QueueProgress};

const DEFAULT_PER_PAGE: u32 = 25;

#[derive(Debug, serde::Deserialize)]
pub struct QueueQuery {
    pub page: Option<u32>,
}

pub async fn list<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Query(q): Query<QueueQuery>,
) -> Result<Json<QueueListResponse>, ApiError> {
    let page = q.page.unwrap_or(1).max(1);
    let (grabs, total) = state
        .queue_service()
        .list_grabs_paginated(ctx.user.id, page, DEFAULT_PER_PAGE)
        .await?;

    let clients = state.queue_service().list_download_clients().await?;

    const PROGRESS_CONCURRENCY: usize = 10;
    let progress_futures: Vec<_> = grabs
        .iter()
        .map(|grab| {
            let state = state.clone();
            let client = clients
                .iter()
                .find(|c| c.id == grab.download_client_id)
                .cloned();
            let download_id = grab.download_id.clone();
            let status = grab.status;
            async move {
                if matches!(status, GrabStatus::Sent | GrabStatus::Confirmed) {
                    if let (Some(client), Some(download_id)) = (client, download_id) {
                        return state
                            .queue_service()
                            .fetch_download_progress(&client, &download_id)
                            .await;
                    }
                }
                None
            }
        })
        .collect();

    let progress_results: Vec<Option<QueueProgress>> = stream::iter(progress_futures)
        .buffered(PROGRESS_CONCURRENCY)
        .collect()
        .await;

    let mut items = Vec::with_capacity(grabs.len());
    for (grab, progress) in grabs.iter().zip(progress_results) {
        let client = clients.iter().find(|c| c.id == grab.download_client_id);
        let client_name = client.map(|c| c.name.clone()).unwrap_or_default();
        let protocol = client
            .map(|c| c.implementation.protocol().to_string())
            .unwrap_or_else(|| "torrent".to_string());

        items.push(QueueItemResponse {
            id: grab.id,
            title: grab.title.clone(),
            status: grab.status,
            size: grab.size,
            media_type: grab.media_type,
            indexer: grab.indexer.clone(),
            download_client: client_name,
            work_id: grab.work_id,
            protocol,
            error: grab.import_error.clone(),
            grabbed_at: grab.grabbed_at.to_rfc3339(),
            progress,
        });
    }

    Ok(Json(QueueListResponse {
        items,
        total,
        page,
        per_page: DEFAULT_PER_PAGE,
    }))
}

pub async fn remove<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    state.grab_service().remove(ctx.user.id, id).await?;
    Ok(())
}

pub async fn retry_import<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<ImportRetryResponse>, ApiError> {
    let transitioned = state
        .queue_service()
        .try_set_importing(ctx.user.id, id)
        .await?;
    if !transitioned {
        return Err(ApiError::Conflict {
            reason: "grab is not in an importable state".into(),
        });
    }

    let result = match state.import_service().import_grab(ctx.user.id, id).await {
        Ok(r) => r,
        Err(e) => {
            if let Err(e2) = state
                .queue_service()
                .update_grab_status(
                    ctx.user.id,
                    id,
                    GrabStatus::ImportFailed,
                    Some(&e.to_string()),
                )
                .await
            {
                tracing::warn!("update_grab_status failed: {e2}");
            }
            return Err(ApiError::Internal(e.to_string()));
        }
    };

    Ok(Json(ImportRetryResponse {
        status: result.final_status,
        imported: result.imported_count as i64,
        failed: result.failed_count as i64,
        skipped: result.skipped_count as i64,
        warnings: result.warnings,
        error: result.error,
    }))
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRetryResponse {
    pub status: GrabStatus,
    pub imported: i64,
    pub failed: i64,
    pub skipped: i64,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

pub async fn summary<S: AppContext>(
    State(state): State<S>,
    ctx: AuthContext,
) -> Result<Json<livrarr_domain::QueueSummary>, ApiError> {
    let summary = state.queue_service().summary(ctx.user.id).await?;
    Ok(Json(summary))
}
