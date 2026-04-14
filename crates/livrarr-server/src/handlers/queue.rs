use axum::extract::{Path, Query, State};
use axum::Json;

use futures::stream::{self, StreamExt};

use crate::state::AppState;
use crate::{
    ApiError, AuthContext, GrabStatus, QueueItemResponse, QueueListResponse, QueueProgress,
};
use livrarr_db::{DownloadClientDb, GrabDb};

use super::import;

const DEFAULT_PER_PAGE: u32 = 25;

#[derive(Debug, serde::Deserialize)]
pub struct QueueQuery {
    pub page: Option<u32>,
}

/// GET /api/v1/queue — paginated list of grabs with optional live progress.
pub async fn list(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(q): Query<QueueQuery>,
) -> Result<Json<QueueListResponse>, ApiError> {
    let page = q.page.unwrap_or(1).max(1);
    let (grabs, total) = state
        .db
        .list_grabs_paginated(ctx.user.id, page, DEFAULT_PER_PAGE)
        .await?;

    // Pre-fetch download clients for enrichment.
    let clients = state.db.list_download_clients().await?;

    // For active grabs (sent/confirmed), fetch live progress with bounded
    // concurrency so we don't spawn N outbound calls for a large page.
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
                        return fetch_progress(&state, &client, &download_id).await;
                    }
                }
                None
            }
        })
        .collect();

    // Use `buffered` (order-preserving) so results align with the `grabs`
    // slice via zip below.
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

/// Fetch live progress from a download client.
async fn fetch_progress(
    state: &AppState,
    client: &livrarr_domain::DownloadClient,
    download_id: &str,
) -> Option<QueueProgress> {
    match client.client_type() {
        "sabnzbd" => fetch_sab_progress(state, client, download_id).await,
        _ => fetch_qbit_progress(state, client, download_id).await,
    }
}

async fn fetch_qbit_progress(
    state: &AppState,
    client: &livrarr_domain::DownloadClient,
    hash: &str,
) -> Option<QueueProgress> {
    let base_url = super::release::qbit_base_url(client);
    let sid = super::release::qbit_login(state, &base_url, client)
        .await
        .ok()?;

    let url = format!("{base_url}/api/v2/torrents/info");
    let resp = state
        .http_client
        .get(&url)
        .query(&[("hashes", hash)])
        .header("Cookie", format!("SID={sid}"))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .ok()?;

    let torrents: Vec<serde_json::Value> = resp.json().await.ok()?;
    let t = torrents.first()?;

    let progress = t.get("progress").and_then(|p| p.as_f64()).unwrap_or(0.0);
    let eta = t
        .get("eta")
        .and_then(|e| e.as_i64())
        .filter(|&e| e > 0 && e < 86400);
    let qstate = t.get("state").and_then(|s| s.as_str()).unwrap_or("unknown");

    Some(QueueProgress {
        percent: (progress * 100.0).round(),
        eta,
        download_status: qstate.to_string(),
    })
}

async fn fetch_sab_progress(
    state: &AppState,
    client: &livrarr_domain::DownloadClient,
    nzo_id: &str,
) -> Option<QueueProgress> {
    let base_url = super::download_client::client_base_url(client);
    let api_key = client.api_key.as_deref().unwrap_or("");

    let url = format!("{base_url}/api?mode=queue&apikey={api_key}&output=json");
    let resp = state
        .http_client
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .ok()?;

    let body: serde_json::Value = resp.json().await.ok()?;
    let slot = body
        .get("queue")
        .and_then(|q| q.get("slots"))
        .and_then(|s| s.as_array())
        .and_then(|slots| {
            slots
                .iter()
                .find(|s| s.get("nzo_id").and_then(|n| n.as_str()) == Some(nzo_id))
        })?;

    let pct = slot
        .get("percentage")
        .and_then(|p| p.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let status = slot
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("Queued");
    let timeleft = slot
        .get("timeleft")
        .and_then(|t| t.as_str())
        .and_then(parse_sab_timeleft);

    Some(QueueProgress {
        percent: pct,
        eta: timeleft,
        download_status: status.to_string(),
    })
}

fn parse_sab_timeleft(s: &str) -> Option<i64> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 3 {
        let h: i64 = parts[0].parse().ok()?;
        let m: i64 = parts[1].parse().ok()?;
        let s: i64 = parts[2].parse().ok()?;
        let total = h * 3600 + m * 60 + s;
        if total > 0 && total < 86400 {
            Some(total)
        } else {
            None
        }
    } else {
        None
    }
}

/// DELETE /api/v1/queue/:id — remove grab from queue
pub async fn remove(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    state
        .db
        .update_grab_status(ctx.user.id, id, GrabStatus::Removed, None)
        .await?;
    Ok(())
}

/// POST /api/v1/grab/:id/retry — run import inline.
pub async fn retry_import(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<ImportRetryResponse>, ApiError> {
    let transitioned = state.db.try_set_importing(ctx.user.id, id).await?;
    if !transitioned {
        return Err(ApiError::Conflict {
            reason: "grab is not in an importable state".into(),
        });
    }

    let result = match import::import_grab(&state, ctx.user.id, id).await {
        Ok(r) => r,
        Err(e) => {
            if let Err(e2) = state
                .db
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
            return Err(e);
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
