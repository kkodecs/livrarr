use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::state::AppState;
use livrarr_db::{
    CreateHistoryEventDbRequest, CreateNotificationDbRequest, DownloadClientDb, GrabDb, HistoryDb,
    NotificationDb,
};
use livrarr_domain::{EventType, GrabStatus, NotificationType};

// ---------------------------------------------------------------------------
// Download Poller Tick (JOBS-POLL-001)
// ---------------------------------------------------------------------------

pub(super) async fn download_poller_tick(
    state: AppState,
    _cancel: CancellationToken,
) -> Result<(), String> {
    let clients = state
        .db
        .list_download_clients()
        .await
        .map_err(|e| format!("list clients: {e}"))?;

    for client in clients.iter().filter(|c| c.enabled) {
        match client.client_type() {
            "sabnzbd" => {
                if let Err(e) = poll_sabnzbd(&state, client).await {
                    warn!("poller: SABnzbd error for {}: {e}", client.name);
                }
            }
            _ => {
                if let Err(e) = poll_qbittorrent(&state, client).await {
                    warn!("poller: qBit error for {}: {e}", client.name);
                }
            }
        }
    }

    // Retry failed imports with exponential backoff (max 5 retries).
    retry_failed_imports(&state).await;

    Ok(())
}

/// Retry importFailed grabs whose backoff has expired.
async fn retry_failed_imports(state: &AppState) {
    const MAX_RETRIES: i32 = 5;

    let retriable = match state.db.list_retriable_grabs(MAX_RETRIES).await {
        Ok(grabs) => grabs,
        Err(e) => {
            warn!("poller: list_retriable_grabs failed: {e}");
            return;
        }
    };

    for grab in retriable {
        info!(
            "import retry: grab {} '{}' (attempt {})",
            grab.id,
            grab.title,
            grab.import_retry_count + 1
        );
        // Increment retry count before attempting (so backoff advances even on crash).
        let _ = state.db.increment_import_retry(grab.user_id, grab.id).await;
        // try_set_importing accepts importFailed — same atomic transition as normal imports.
        let ok = state
            .db
            .try_set_importing(grab.user_id, grab.id)
            .await
            .unwrap_or(false);
        if ok {
            spawn_import(state, grab.user_id, grab.id);
        }
    }
}

/// Poll qBittorrent for completed torrents — existing logic extracted.
async fn poll_qbittorrent(
    state: &AppState,
    client: &livrarr_domain::DownloadClient,
) -> Result<(), String> {
    let base_url = crate::infra::release_helpers::qbit_base_url(client);
    let sid = crate::infra::release_helpers::qbit_login(state, &base_url, client)
        .await
        .map_err(|e| format!("qBit login: {e}"))?;

    let info_url = format!("{base_url}/api/v2/torrents/info");
    let resp = state
        .http_client
        .get(&info_url)
        .query(&[("filter", "all"), ("category", client.category.as_str())])
        .header("Cookie", format!("SID={sid}"))
        .send()
        .await
        .map_err(|e| format!("qBit request: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("qBit returned {}", resp.status()));
    }

    let torrents: Vec<serde_json::Value> =
        resp.json().await.map_err(|e| format!("qBit parse: {e}"))?;

    let active_grabs = state
        .db
        .list_active_grabs()
        .await
        .map_err(|e| format!("list grabs: {e}"))?;

    // Filter grabs to this download client only, then pre-index.
    let client_grabs: Vec<_> = active_grabs
        .iter()
        .filter(|g| g.download_client_id == client.id)
        .collect();

    let grabs_by_download_id: std::collections::HashMap<String, usize> = client_grabs
        .iter()
        .enumerate()
        .filter_map(|(idx, g)| {
            g.download_id
                .as_deref()
                .filter(|id| *id != "pending" && !id.is_empty())
                .map(|id| (id.to_ascii_lowercase(), idx))
        })
        .collect();

    let grabs_by_title: std::collections::HashMap<String, usize> = client_grabs
        .iter()
        .enumerate()
        .map(|(idx, g)| (g.title.to_ascii_lowercase(), idx))
        .collect();

    for torrent in &torrents {
        let hash = torrent
            .get("hash")
            .and_then(|h| h.as_str())
            .unwrap_or_default();
        let qbit_state = torrent
            .get("state")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown");

        if !is_completed_state(qbit_state) {
            continue;
        }

        let name = torrent
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or_default();

        let grab = grabs_by_download_id
            .get(&hash.to_ascii_lowercase())
            .map(|&idx| client_grabs[idx])
            .or_else(|| {
                grabs_by_title
                    .get(&name.to_ascii_lowercase())
                    .map(|&idx| client_grabs[idx])
            })
            .or_else(|| {
                client_grabs.iter().copied().find(|g| {
                    g.download_id.as_deref() == Some("pending")
                        && livrarr_matching::string_similarity(&g.title, name) >= 0.6
                })
            });

        if let Some(grab) = grab {
            // Backfill download_id if missing.
            if grab.download_id.is_none() && !hash.is_empty() {
                if let Err(e) = state
                    .db
                    .update_grab_download_id(grab.user_id, grab.id, hash)
                    .await
                {
                    warn!(
                        "poller: failed to backfill download_id for grab {}: {e}",
                        grab.id
                    );
                }
            }

            // Resolve source path and verify it exists before attempting import.
            // If files aren't available yet (rsync delay), skip and retry next tick.
            let content_path =
                match crate::infra::import_pipeline::fetch_qbit_content_path(state, client, hash)
                    .await
                {
                    Ok(p) => p,
                    Err(e) => {
                        warn!(
                            "poller: could not get content_path for grab {}: {e}",
                            grab.id
                        );
                        continue;
                    }
                };
            let mapping_result = match crate::infra::import_pipeline::apply_remote_path_mapping(
                state,
                &client.host,
                &content_path,
            )
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!("poller: path mapping failed for grab {}: {e}", grab.id);
                    continue;
                }
            };
            if !tokio::fs::try_exists(&mapping_result.local_path)
                .await
                .unwrap_or(false)
            {
                tracing::debug!(
                    "poller: source not yet available for <grab {}>, will retry",
                    grab.id
                );
                let age = chrono::Utc::now() - grab.grabbed_at;
                if age > chrono::Duration::minutes(5) {
                    let _ = state
                        .db
                        .create_notification(CreateNotificationDbRequest {
                            user_id: grab.user_id,
                            notification_type: NotificationType::PathNotFound,
                            ref_key: Some(format!("path_not_found:{}", grab.id)),
                            message: format!(
                                "{} reports that {} (grab {}) has downloaded, but it does not seem to be available locally. You may need a remote path mapping.",
                                client.name, grab.title, grab.id
                            ),
                            data: {
                                let content_dir = std::path::Path::new(&content_path)
                                    .parent()
                                    .map(|d| d.display().to_string())
                                    .unwrap_or_default();
                                serde_json::json!({
                                    "grabId": grab.id,
                                    "title": grab.title,
                                    "clientName": client.name,
                                    "clientHost": client.host,
                                    "contentDir": content_dir,
                                    "configuredRemotePath": mapping_result.configured_remote_path,
                                    "configuredLocalPath": mapping_result.configured_local_path,
                                })
                            },
                        })
                        .await;
                }
                continue;
            }

            // Persist the raw remote path so import_grab doesn't need to re-query.
            if grab.content_path.is_none() {
                if let Err(e) = state
                    .db
                    .set_grab_content_path(grab.user_id, grab.id, &content_path)
                    .await
                {
                    warn!(
                        "poller: failed to persist content_path for grab {}: {e}",
                        grab.id
                    );
                }
            }

            let transitioned = match state.db.try_set_importing(grab.user_id, grab.id).await {
                Ok(t) => t,
                Err(e) => {
                    warn!("poller: try_set_importing failed for grab {}: {e}", grab.id);
                    continue;
                }
            };

            if transitioned {
                spawn_import(state, grab.user_id, grab.id);
            }
        }
    }

    Ok(())
}

/// USE-POLL-001 through USE-POLL-005: Poll SABnzbd queue + history.
async fn poll_sabnzbd(
    state: &AppState,
    client: &livrarr_domain::DownloadClient,
) -> Result<(), String> {
    let base_url = livrarr_handlers::download_client::client_base_url(client);
    let api_key = client.api_key.as_deref().unwrap_or("");

    // Fetch queue (active downloads).
    let queue_url = format!(
        "{base_url}/api?mode=queue&apikey={}&output=json",
        urlencoding::encode(api_key)
    );
    let queue_resp = state
        .http_client
        .get(&queue_url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    let sab_reachable = queue_resp.is_ok();

    let queue_nzo_ids: std::collections::HashSet<String> = match queue_resp {
        Ok(resp) if resp.status().is_success() => {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            body.get("queue")
                .and_then(|q| q.get("slots"))
                .and_then(|s| s.as_array())
                .map(|slots| {
                    slots
                        .iter()
                        .filter_map(|s| s.get("nzo_id").and_then(|n| n.as_str()).map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        }
        Ok(resp) => {
            return Err(format!("SABnzbd queue returned {}", resp.status()));
        }
        Err(e) => {
            return Err(format!("SABnzbd unreachable: {}", e.without_url()));
        }
    };

    // Get active grabs for this client.
    let active_grabs = state
        .db
        .list_active_grabs()
        .await
        .map_err(|e| format!("list grabs: {e}"))?;

    let sab_grabs: Vec<_> = active_grabs
        .iter()
        .filter(|g| g.download_client_id == client.id)
        .collect();

    // Fetch history once (SABnzbd search param searches by name, not nzo_id).
    // Fetch enough entries to cover recent completions and match by nzo_id client-side.
    let history_slots: Vec<serde_json::Value> = if sab_grabs.iter().any(|g| {
        g.download_id
            .as_ref()
            .is_some_and(|id| !queue_nzo_ids.contains(id))
    }) {
        let history_url = format!(
            "{base_url}/api?mode=history&apikey={}&output=json&limit=200",
            urlencoding::encode(api_key)
        );
        match state
            .http_client
            .get(&history_url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                body.get("history")
                    .and_then(|h| h.get("slots"))
                    .and_then(|s| s.as_array())
                    .cloned()
                    .unwrap_or_default()
            }
            Ok(resp) => {
                warn!("poller: SABnzbd history returned {}", resp.status());
                vec![]
            }
            Err(e) => {
                warn!(
                    "poller: SABnzbd history request failed: {}",
                    e.without_url()
                );
                vec![]
            }
        }
    } else {
        vec![]
    };

    for grab in &sab_grabs {
        let nzo_id = match &grab.download_id {
            Some(id) => id,
            None => continue,
        };

        // If nzo_id is in the queue, it's still active — skip.
        if queue_nzo_ids.contains(nzo_id) {
            continue;
        }

        // Match against fetched history by nzo_id.
        let entry = history_slots.iter().find(|e| {
            e.get("nzo_id")
                .and_then(|n| n.as_str())
                .is_some_and(|n| n == nzo_id)
        });

        if let Some(entry) = entry {
            let status = entry
                .get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown");

            match status {
                "Completed" => {
                    let storage = entry
                        .get("storage")
                        .and_then(|s| s.as_str())
                        .unwrap_or_default();

                    // Resolve local path and verify it exists before attempting import.
                    let mapping_result =
                        match crate::infra::import_pipeline::apply_remote_path_mapping(
                            state,
                            &client.host,
                            storage,
                        )
                        .await
                        {
                            Ok(r) => r,
                            Err(e) => {
                                warn!("poller: path mapping failed for grab {}: {e}", grab.id);
                                continue;
                            }
                        };
                    if !tokio::fs::try_exists(&mapping_result.local_path)
                        .await
                        .unwrap_or(false)
                    {
                        tracing::debug!(
                            "poller: source not yet available for <grab {}>, will retry",
                            grab.id
                        );
                        let age = chrono::Utc::now() - grab.grabbed_at;
                        if age > chrono::Duration::minutes(5) {
                            let _ = state
                                .db
                                .create_notification(CreateNotificationDbRequest {
                                    user_id: grab.user_id,
                                    notification_type: NotificationType::PathNotFound,
                                    ref_key: Some(format!("path_not_found:{}", grab.id)),
                                    message: format!(
                                        "{} reports that {} (grab {}) has downloaded, but it does not seem to be available locally. You may need a remote path mapping.",
                                        client.name, grab.title, grab.id
                                    ),
                                    data: {
                                        let content_dir = std::path::Path::new(storage)
                                            .parent()
                                            .map(|d| d.display().to_string())
                                            .unwrap_or_default();
                                        serde_json::json!({
                                            "grabId": grab.id,
                                            "title": grab.title,
                                            "clientName": client.name,
                                            "clientHost": client.host,
                                            "contentDir": content_dir,
                                            "configuredRemotePath": mapping_result.configured_remote_path,
                                            "configuredLocalPath": mapping_result.configured_local_path,
                                        })
                                    },
                                })
                                .await;
                        }
                        continue;
                    }

                    info!(
                        "poller: SABnzbd completed grab {}, storage={storage}",
                        grab.id
                    );

                    // Persist the raw remote path so import_grab doesn't need to re-query.
                    if grab.content_path.is_none() {
                        if let Err(e) = state
                            .db
                            .set_grab_content_path(grab.user_id, grab.id, storage)
                            .await
                        {
                            warn!(
                                "poller: failed to persist content_path for grab {}: {e}",
                                grab.id
                            );
                        }
                    }

                    let transitioned = match state.db.try_set_importing(grab.user_id, grab.id).await
                    {
                        Ok(t) => t,
                        Err(e) => {
                            warn!("poller: try_set_importing failed for grab {}: {e}", grab.id);
                            continue;
                        }
                    };

                    if transitioned {
                        spawn_import(state, grab.user_id, grab.id);
                    }
                }
                "Failed" => {
                    let fail_msg = entry
                        .get("fail_message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown failure");
                    warn!("poller: SABnzbd failed grab {}: {fail_msg}", grab.id);
                    if let Err(e) = state
                        .db
                        .update_grab_status(
                            grab.user_id,
                            grab.id,
                            GrabStatus::Failed,
                            Some(fail_msg),
                        )
                        .await
                    {
                        warn!("poller: failed to update grab {} status: {e}", grab.id);
                    }
                    if let Err(e) = state
                        .db
                        .create_history_event(CreateHistoryEventDbRequest {
                            user_id: grab.user_id,
                            work_id: Some(grab.work_id),
                            event_type: EventType::DownloadFailed,
                            data: serde_json::json!({
                                "title": grab.title,
                                "error": fail_msg,
                            }),
                        })
                        .await
                    {
                        tracing::warn!("create_history_event failed: {e}");
                    }
                }
                _ => {
                    // Still processing (e.g., Extracting, Verifying).
                }
            }
        } else {
            // USE-POLL-005: Orphan detection — not in queue or history.
            if sab_reachable {
                {
                    let age = chrono::Utc::now() - grab.grabbed_at;
                    if age > chrono::Duration::hours(24) {
                        warn!(
                            "poller: orphaned SABnzbd grab {} (nzo_id={nzo_id}, age={}h)",
                            grab.id,
                            age.num_hours()
                        );
                        if let Err(e) = state
                            .db
                            .update_grab_status(
                                grab.user_id,
                                grab.id,
                                GrabStatus::Failed,
                                Some("download not found in SABnzbd"),
                            )
                            .await
                        {
                            warn!("poller: failed to mark grab {} as orphaned: {e}", grab.id);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Spawn an import task with concurrency semaphore. Poller continues immediately.
fn spawn_import(state: &AppState, user_id: i64, grab_id: i64) {
    let state = state.clone();
    tokio::spawn(async move {
        // Acquire permit inside the spawned task so poller doesn't block.
        // Bail out if the semaphore has been closed — we must not proceed without it.
        let _permit = match state.import_semaphore.acquire().await {
            Ok(p) => p,
            Err(e) => {
                warn!("import: semaphore acquisition failed for grab {grab_id}: {e} — aborting");
                return;
            }
        };
        match crate::infra::import_pipeline::import_grab(&state, user_id, grab_id).await {
            Ok(result) => {
                info!(
                    "import: grab {} — {} files imported",
                    grab_id, result.imported_count
                );
            }
            Err(e) => {
                warn!("import: failed for grab {}: {e}", grab_id);
                let err_msg = e.to_string();
                if let Err(e2) = state
                    .db
                    .update_grab_status(user_id, grab_id, GrabStatus::ImportFailed, Some(&err_msg))
                    .await
                {
                    warn!(
                        "import: failed to set ImportFailed for grab {}: {e2}",
                        grab_id
                    );
                }
            }
        }
    });
}

fn is_completed_state(state: &str) -> bool {
    matches!(
        state,
        "pausedUP"
            | "stoppedUP"
            | "uploading"
            | "stalledUP"
            | "forcedUP"
            | "queuedUP"
            | "checkingResumeData"
    )
}
