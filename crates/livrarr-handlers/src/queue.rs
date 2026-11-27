use axum::extract::{Path, Query, State};
use axum::Json;

use futures::stream::{self, StreamExt};

use crate::context::{HasGrabService, HasImportService, HasQueueService};
use crate::{ApiError, AuthContext, QueueItemResponse, QueueListResponse};
use livrarr_domain::services::{GrabService, ImportService, QueueService};
use livrarr_domain::{DownloadClient, Grab, GrabStatus, QueueProgress};

const DEFAULT_PER_PAGE: u32 = 25;

#[derive(Debug, serde::Deserialize)]
pub struct QueueQuery {
    pub page: Option<u32>,
}

pub async fn list<S: HasQueueService>(
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

    let items = grabs
        .iter()
        .zip(progress_results)
        .map(|(grab, progress)| {
            let client = clients.iter().find(|c| c.id == grab.download_client_id);
            queue_item(grab, client, progress)
        })
        .collect();

    Ok(Json(QueueListResponse {
        items,
        total,
        page,
        per_page: DEFAULT_PER_PAGE,
    }))
}

fn queue_item(
    grab: &Grab,
    client: Option<&DownloadClient>,
    progress: Option<QueueProgress>,
) -> QueueItemResponse {
    let download_client = client.map(|c| c.name.clone()).unwrap_or_default();
    let protocol = client
        .map(|c| c.implementation.protocol().to_string())
        .unwrap_or_else(|| "torrent".to_string());

    QueueItemResponse {
        id: grab.id,
        title: grab.title.clone(),
        status: grab.status,
        size: grab.size,
        media_type: grab.media_type,
        indexer: grab.indexer.clone(),
        download_client,
        download_id: grab.download_id.clone(),
        work_id: grab.work_id,
        protocol,
        error: grab.import_error.clone(),
        grabbed_at: grab.grabbed_at.to_rfc3339(),
        progress,
    }
}

pub async fn remove<S: HasGrabService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    state.grab_service().remove(ctx.user.id, id).await?;
    Ok(())
}

pub async fn retry_import<S: HasQueueService + HasImportService>(
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

pub async fn summary<S: HasQueueService>(
    State(state): State<S>,
    ctx: AuthContext,
) -> Result<Json<livrarr_domain::QueueSummary>, ApiError> {
    let summary = state.queue_service().summary(ctx.user.id).await?;
    Ok(Json(summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn grab(download_id: Option<&str>) -> Grab {
        Grab {
            id: 7,
            user_id: 1,
            work_id: 42,
            download_client_id: 3,
            title: "Some Book".to_string(),
            indexer: "MyIndexer".to_string(),
            guid: "guid-7".to_string(),
            size: Some(1024),
            download_url: "http://indexer/7.torrent".to_string(),
            download_id: download_id.map(str::to_string),
            status: GrabStatus::Imported,
            import_error: None,
            media_type: None,
            content_path: None,
            grabbed_at: Utc.with_ymd_and_hms(2026, 11, 27, 12, 0, 0).unwrap(),
            import_retry_count: 0,
            import_failed_at: None,
        }
    }

    #[test]
    fn queue_item_carries_the_download_clients_id() {
        let item = queue_item(&grab(Some("4f2a9c")), None, None);
        assert_eq!(item.download_id.as_deref(), Some("4f2a9c"));

        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["downloadId"], "4f2a9c");
    }

    #[test]
    fn queue_item_without_a_download_id_serializes_null() {
        let json = serde_json::to_value(queue_item(&grab(None), None, None)).unwrap();
        assert!(json["downloadId"].is_null());
    }
}
