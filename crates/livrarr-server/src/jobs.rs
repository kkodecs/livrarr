//! Background job runner and interval jobs.
//!
//! Satisfies: JOBS-001, JOBS-002, JOBS-003, JOBS-004

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};

use crate::state::AppState;
use livrarr_db::{
    ConfigDb, CreateHistoryEventDbRequest, CreateNotificationDbRequest, CreateWorkDbRequest,
    DownloadClientDb, GrabDb, HistoryDb, ListImportDb, NotificationDb, SessionDb, WorkDb,
};
use livrarr_domain::services::RssSyncWorkflow;
use livrarr_domain::{EventType, NotificationType};

// ---------------------------------------------------------------------------
// JobRunner
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct JobRunner {
    status: Arc<RwLock<Vec<JobStatus>>>,
    handles: Arc<tokio::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    cancel: CancellationToken,
}

#[derive(Clone)]
struct JobStatus {
    name: String,
    interval: Duration,
    last_run: Option<chrono::DateTime<chrono::Utc>>,
    running: bool,
    panic_notified: bool,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatusResponse {
    pub name: String,
    pub interval_seconds: u64,
    pub last_run: Option<String>,
    pub running: bool,
}

impl Default for JobRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl JobRunner {
    pub fn new() -> Self {
        Self {
            status: Arc::new(RwLock::new(Vec::new())),
            handles: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            cancel: CancellationToken::new(),
        }
    }

    /// Start all interval jobs. Call once after AppState is constructed.
    pub async fn start(&self, state: AppState) {
        self.spawn_job(
            "download_poller",
            Duration::from_secs(60),
            state.clone(),
            download_poller_tick,
        )
        .await;
        self.spawn_job(
            "session_cleanup",
            Duration::from_secs(3600),
            state.clone(),
            session_cleanup_tick,
        )
        .await;
        self.spawn_job(
            "author_monitor",
            Duration::from_secs(86400),
            state.clone(),
            author_monitor_tick,
        )
        .await;
        self.spawn_job(
            "enrichment_retry",
            Duration::from_secs(300),
            state.clone(),
            enrichment_retry_tick,
        )
        .await;
        self.spawn_job(
            "state_map_cleanup",
            Duration::from_secs(1800),
            state.clone(),
            state_map_cleanup_tick,
        )
        .await;
        self.spawn_job("rss_sync", Duration::from_secs(60), state, rss_sync_tick)
            .await;
    }

    async fn spawn_job<F, Fut>(&self, name: &str, interval: Duration, state: AppState, tick_fn: F)
    where
        F: Fn(AppState, CancellationToken) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), String>> + Send,
    {
        let status = self.status.clone();
        let cancel = self.cancel.clone();
        let job_name = name.to_string();

        // Register in status vec synchronously (no race condition).
        self.status.write().await.push(JobStatus {
            name: job_name.clone(),
            interval,
            last_run: None,
            running: false,
            panic_notified: false,
        });

        let tick_fn = Arc::new(tick_fn);

        let handle = tokio::spawn(async move {
            // Short stagger before first tick to avoid all jobs hitting DB/network at startup.
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(5)) => {},
                _ = cancel.cancelled() => {
                    debug!("job '{job_name}' cancelled during stagger");
                    return;
                },
            }

            loop {
                if cancel.is_cancelled() {
                    debug!("job '{job_name}' loop cancelled");
                    break;
                }

                set_job_running(&status, &job_name, true).await;

                // Spawn tick as child task for panic isolation (JOBS-002).
                let state_clone = state.clone();
                let cancel_clone = cancel.clone();
                let tf = tick_fn.clone();
                let tick_handle = tokio::spawn(async move { tf(state_clone, cancel_clone).await });

                match tick_handle.await {
                    Ok(Ok(())) => {
                        let mut statuses = status.write().await;
                        if let Some(s) = statuses.iter_mut().find(|s| s.name == job_name) {
                            s.last_run = Some(chrono::Utc::now());
                            s.running = false;
                            s.panic_notified = false;
                        }
                        debug!("job '{job_name}' tick completed");
                    }
                    Ok(Err(e)) => {
                        error!("job '{job_name}' error: {e}");
                        set_job_running(&status, &job_name, false).await;
                    }
                    Err(join_err) if join_err.is_panic() => {
                        // Extract panic payload for logging.
                        let payload = join_err.into_panic();
                        let msg = payload
                            .downcast_ref::<&str>()
                            .map(|s| s.to_string())
                            .or_else(|| payload.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "unknown panic".to_string());
                        error!("job '{job_name}' panicked: {msg}");

                        let mut statuses = status.write().await;
                        if let Some(s) = statuses.iter_mut().find(|s| s.name == job_name) {
                            s.running = false;
                            if !s.panic_notified {
                                s.panic_notified = true;
                                if let Err(e) = state
                                    .db
                                    .create_notification(CreateNotificationDbRequest {
                                        user_id: 1,
                                        notification_type: NotificationType::JobPanicked,
                                        ref_key: Some(job_name.clone()),
                                        message: format!("Job '{}' panicked: {}", job_name, msg),
                                        data: serde_json::Value::Null,
                                    })
                                    .await
                                {
                                    tracing::warn!("create_notification failed: {e}");
                                }
                            }
                        }
                    }
                    Err(join_err) => {
                        warn!("job '{job_name}' task cancelled: {join_err}");
                        set_job_running(&status, &job_name, false).await;
                    }
                }

                // Sleep at end of loop — first tick runs shortly after startup.
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {},
                    _ = cancel.cancelled() => {
                        debug!("job '{job_name}' cancelled during interval sleep");
                        break;
                    },
                }
            }
        });

        self.handles.lock().await.push(handle);
    }

    /// Signal graceful shutdown and wait for all jobs to exit.
    pub async fn shutdown(&self) {
        info!("signalling job shutdown");
        self.cancel.cancel();
        let handles = std::mem::take(&mut *self.handles.lock().await);
        for h in handles {
            let _ = h.await;
        }
        info!("all jobs stopped");
    }

    /// Get a clone of the cancellation token for external shutdown coordination.
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Get observable status for system endpoint.
    pub async fn job_statuses(&self) -> Vec<JobStatusResponse> {
        self.status
            .read()
            .await
            .iter()
            .map(|s| JobStatusResponse {
                name: s.name.clone(),
                interval_seconds: s.interval.as_secs(),
                last_run: s.last_run.map(|t| t.to_rfc3339()),
                running: s.running,
            })
            .collect()
    }
}

async fn set_job_running(status: &Arc<RwLock<Vec<JobStatus>>>, name: &str, running: bool) {
    let mut statuses = status.write().await;
    if let Some(s) = statuses.iter_mut().find(|s| s.name == name) {
        s.running = running;
    }
}

// ---------------------------------------------------------------------------
// Startup Recovery (JOBS-003)
// ---------------------------------------------------------------------------

/// Reset stale state from unclean shutdown. Run once before starting jobs.
pub async fn recover_interrupted_state(state: &AppState) {
    // Reset importing grabs → importFailed (retryable via H-1).
    match state.db.reset_importing_grabs().await {
        Ok(count) if count > 0 => {
            warn!("recovered {count} grabs from importing → confirmed");
        }
        Ok(_) => {}
        Err(e) => error!("startup recovery (grabs) failed: {e}"),
    }

    // Reset pending enrichments → failed (retry queue will pick them up).
    match state.db.reset_pending_enrichments().await {
        Ok(count) if count > 0 => {
            warn!("recovered {count} works from pending → failed");
        }
        Ok(_) => {}
        Err(e) => error!("startup recovery (enrichments) failed: {e}"),
    }

    // Sweep stale temp files from root folders (crashed imports).
    sweep_stale_temp_files(state).await;
}

/// Remove app-owned temp files older than 1 hour from root folders.
/// Only matches patterns created by the import pipeline:
/// - `*.fallback.tmp` (H-2 atomic fallback)
/// - `*.epub.tagwrite.*.tmp` (EPUB tag writer)
/// - `*.tmp` where a corresponding final file does NOT exist (import .tmp)
async fn sweep_stale_temp_files(state: &AppState) {
    use livrarr_db::RootFolderDb;

    let root_folders = match state.db.list_root_folders().await {
        Ok(rf) => rf,
        Err(e) => {
            warn!("startup sweep: failed to list root folders: {e}");
            return;
        }
    };

    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    let mut removed = 0usize;

    for rf in &root_folders {
        let root = std::path::PathBuf::from(&rf.path);
        if !root.is_dir() {
            continue;
        }
        let root_clone = root.clone();
        let result =
            tokio::task::spawn_blocking(move || sweep_dir_recursive(&root_clone, cutoff)).await;
        match result {
            Ok(count) => removed += count,
            Err(e) => warn!("startup sweep: spawn error for {}: {e}", rf.path),
        }
    }

    if removed > 0 {
        info!("startup sweep: removed {removed} stale temp file(s)");
    }
}

fn sweep_dir_recursive(dir: &std::path::Path, cutoff: std::time::SystemTime) -> usize {
    let mut removed = 0;
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            removed += sweep_dir_recursive(&path, cutoff);
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Only remove app-owned patterns.
        let is_app_temp = name_str.ends_with(".fallback.tmp")
            || (name_str.contains(".tagwrite.") && name_str.ends_with(".tmp"));
        if !is_app_temp {
            continue;
        }
        // Only remove if older than cutoff.
        if let Ok(meta) = entry.metadata() {
            let mtime = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            if mtime < cutoff && std::fs::remove_file(&path).is_ok() {
                tracing::debug!("startup sweep: removed {}", path.display());
                removed += 1;
            }
        }
    }
    removed
}

// ---------------------------------------------------------------------------
// Download Poller Tick (JOBS-POLL-001)
// ---------------------------------------------------------------------------

async fn download_poller_tick(state: AppState, _cancel: CancellationToken) -> Result<(), String> {
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
    let base_url = crate::handlers::release::qbit_base_url(client);
    let sid = crate::handlers::release::qbit_login(state, &base_url, client)
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

        let grab = active_grabs
            .iter()
            .find(|g| {
                g.download_id
                    .as_deref()
                    .is_some_and(|id| id.eq_ignore_ascii_case(hash))
            })
            .or_else(|| {
                active_grabs
                    .iter()
                    .find(|g| g.title.eq_ignore_ascii_case(name))
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
                match crate::handlers::import::fetch_qbit_content_path(state, client, hash).await {
                    Ok(p) => p,
                    Err(e) => {
                        warn!(
                            "poller: could not get content_path for grab {}: {e}",
                            grab.id
                        );
                        continue;
                    }
                };
            let mapping_result = match crate::handlers::import::apply_remote_path_mapping(
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
            if !std::path::Path::new(&mapping_result.local_path).exists() {
                tracing::debug!(
                    "poller: source not yet available for <grab {}>, will retry",
                    grab.id
                );
                // Notify user once — file may need a remote path mapping.
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
    let base_url = crate::handlers::download_client::client_base_url(client);
    let api_key = client.api_key.as_deref().unwrap_or("");

    // Fetch queue (active downloads).
    let queue_url = format!("{base_url}/api?mode=queue&apikey={api_key}&output=json");
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
            return Err(format!("SABnzbd unreachable: {e}"));
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
        let history_url =
            format!("{base_url}/api?mode=history&apikey={api_key}&output=json&limit=200");
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
                warn!("poller: SABnzbd history request failed: {e}");
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
                    let mapping_result = match crate::handlers::import::apply_remote_path_mapping(
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
                    if !std::path::Path::new(&mapping_result.local_path).exists() {
                        tracing::debug!(
                            "poller: source not yet available for <grab {}>, will retry",
                            grab.id
                        );
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
                            livrarr_domain::GrabStatus::Failed,
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
                                livrarr_domain::GrabStatus::Failed,
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
        match crate::handlers::import::import_grab(&state, user_id, grab_id).await {
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
                    .update_grab_status(
                        user_id,
                        grab_id,
                        livrarr_domain::GrabStatus::ImportFailed,
                        Some(&err_msg),
                    )
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

// ---------------------------------------------------------------------------
// Session Cleanup Tick (JOBS-SESSION-001)
// ---------------------------------------------------------------------------

async fn session_cleanup_tick(state: AppState, _cancel: CancellationToken) -> Result<(), String> {
    let count = state
        .db
        .delete_expired_sessions()
        .await
        .map_err(|e| format!("session cleanup: {e}"))?;
    if count > 0 {
        debug!("session cleanup: deleted {count} expired sessions");
    }

    // Clean up stale list import preview rows (older than 1 hour).
    let cutoff = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
    let preview_count = state
        .db
        .delete_stale_list_import_previews(&cutoff)
        .await
        .unwrap_or(0);
    if preview_count > 0 {
        debug!("session cleanup: deleted {preview_count} stale list import previews");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Author Monitor Tick (JOBS-AUTHOR-001)
// ---------------------------------------------------------------------------

pub async fn author_monitor_tick(state: AppState, cancel: CancellationToken) -> Result<(), String> {
    use livrarr_db::AuthorDb;

    let authors = state
        .db
        .list_monitored_authors()
        .await
        .map_err(|e| format!("list authors: {e}"))?;

    // Index-based loop to retry on 429 without skipping.
    let mut i = 0;
    let mut retry_counts: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();
    while i < authors.len() {
        let author = &authors[i];
        let ol_key = match &author.ol_key {
            Some(k) => k.clone(),
            None => {
                i += 1;
                continue;
            }
        };

        let works_url = format!(
            "https://openlibrary.org/authors/{}/works.json?limit=100",
            ol_key
        );
        let resp = match state.http_client.get(&works_url).send().await {
            Ok(r) => r,
            Err(e) => {
                warn!("author monitor: OL request failed for {}: {e}", author.name);
                i += 1;
                continue;
            }
        };

        if resp.status().as_u16() == 429 {
            let retries = retry_counts.entry(i).or_insert(0);
            *retries += 1;
            if *retries > 3 {
                warn!(
                    "author monitor: OL 429 for {} — max retries exceeded, skipping",
                    author.name
                );
                i += 1;
                continue;
            }
            warn!(
                "author monitor: OL 429 for {} — backing off 60s (attempt {}/3)",
                author.name, retries
            );
            if let Err(e) = state
                .db
                .create_notification(CreateNotificationDbRequest {
                    user_id: 1,
                    notification_type: NotificationType::RateLimitHit,
                    ref_key: Some("author_monitor".into()),
                    message: "Open Library rate limit hit during author monitoring".into(),
                    data: serde_json::Value::Null,
                })
                .await
            {
                tracing::warn!("create_notification failed: {e}");
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(60)) => {},
                _ = cancel.cancelled() => { return Ok(()); },
            }
            // Retry same author (don't increment i).
            continue;
        }

        if !resp.status().is_success() {
            warn!(
                "author monitor: OL returned {} for {}",
                resp.status(),
                author.name
            );
            i += 1;
            continue;
        }

        let body: serde_json::Value = match resp.json().await {
            Ok(b) => b,
            Err(e) => {
                warn!("author monitor: OL parse error for {}: {e}", author.name);
                i += 1;
                continue;
            }
        };

        let monitor_since_year = author
            .monitor_since
            .map(|dt| dt.format("%Y").to_string().parse::<i32>().unwrap_or(0))
            .unwrap_or(0);

        let existing_ol_keys = state
            .db
            .list_works_by_author_ol_keys(author.user_id, &ol_key)
            .await
            .unwrap_or_default();

        if let Some(entries) = body.get("entries").and_then(|e| e.as_array()) {
            for entry in entries {
                let work_ol_key = entry
                    .get("key")
                    .and_then(|k| k.as_str())
                    .unwrap_or_default()
                    .to_string();

                if existing_ol_keys.contains(&work_ol_key) {
                    continue;
                }

                let publish_year = entry
                    .get("first_publish_date")
                    .and_then(|d| d.as_str())
                    .and_then(|d| {
                        d.split(|c: char| !c.is_ascii_digit())
                            .find(|tok| tok.len() == 4)
                            .and_then(|tok| tok.parse::<i32>().ok())
                    });

                let year = match publish_year {
                    Some(y) => y,
                    None => {
                        debug!(
                            "author monitor: skipping work {} — unparseable date",
                            work_ol_key
                        );
                        continue;
                    }
                };

                if year < monitor_since_year {
                    continue;
                }

                let raw_title = entry
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("Unknown")
                    .to_string();
                // Auto-add still benefits from cleanup so the stored title
                // is canonical even though it isn't user-validated.
                let work_title = livrarr_metadata::title_cleanup::clean_title(&raw_title);
                let cleaned_author = livrarr_metadata::title_cleanup::clean_author(&author.name);

                info!(
                    "author monitor: new work detected for <author {}> ({})",
                    author.id, year
                );

                if author.monitor_new_items {
                    // Create the work and trigger enrichment (AUTHOR-004).
                    match state
                        .db
                        .create_work(CreateWorkDbRequest {
                            user_id: author.user_id,
                            title: work_title.clone(),
                            author_name: cleaned_author,
                            author_id: Some(author.id),
                            ol_key: Some(work_ol_key.clone()),
                            gr_key: None,
                            year: publish_year,
                            cover_url: None,
                            metadata_source: None,
                            detail_url: None,
                            language: None,
                            import_id: None,
                            series_id: None,
                            series_name: None,
                            series_position: None,
                            monitor_ebook: author.monitor_new_items,
                            monitor_audiobook: author.monitor_new_items,
                        })
                        .await
                    {
                        Ok(new_work) => {
                            // AutoAdded provenance — honest about origin
                            // (system pulled from OL bibliography, user did
                            // not per-work validate). Not a lock anchor for
                            // LLM identity check until a future user-confirm
                            // UX transitions it to setter=User.
                            crate::handlers::work::write_addtime_provenance(
                                &state.db,
                                author.user_id,
                                &new_work,
                                livrarr_domain::ProvenanceSetter::AutoAdded,
                            )
                            .await;
                            // Enrich the newly added work (30s timeout).
                            // Routed through DefaultProviderQueue + EnrichmentServiceImpl
                            // (Phase 1.5 cutover). Service writes to DB internally.
                            let enrich_result = tokio::time::timeout(
                                Duration::from_secs(30),
                                livrarr_metadata::EnrichmentService::enrich_work(
                                    state.enrichment_service.as_ref(),
                                    author.user_id,
                                    new_work.id,
                                    livrarr_metadata::EnrichmentMode::Background,
                                ),
                            )
                            .await;
                            match enrich_result {
                                Ok(Ok(_)) => {}
                                Ok(Err(e)) => {
                                    warn!(
                                        "author monitor: enrichment failed for <work {}>: {e}",
                                        new_work.id
                                    );
                                }
                                Err(_) => {
                                    warn!(
                                        "author monitor: enrichment timeout for <work {}>",
                                        new_work.id
                                    );
                                    // Still create notification — work was added, just not enriched.
                                    if let Err(e) = state
                                        .db
                                        .create_notification(CreateNotificationDbRequest {
                                            user_id: author.user_id,
                                            notification_type: NotificationType::WorkAutoAdded,
                                            ref_key: Some(work_ol_key.clone()),
                                            message: format!(
                                                "New work '{}' by {} auto-added (enrichment timed out)",
                                                work_title, author.name
                                            ),
                                            data: serde_json::json!({
                                                "title": work_title,
                                                "author": author.name,
                                                "year": year,
                                                "ol_key": work_ol_key,
                                                "work_id": new_work.id,
                                            }),
                                        })
                                        .await
                                    {
                                        tracing::warn!("create_notification failed: {e}");
                                    }
                                    continue;
                                }
                            };

                            if let Err(e) = state
                                .db
                                .create_notification(CreateNotificationDbRequest {
                                    user_id: author.user_id,
                                    notification_type: NotificationType::WorkAutoAdded,
                                    ref_key: Some(work_ol_key.clone()),
                                    message: format!(
                                        "New work '{}' by {} auto-added to your library",
                                        work_title, author.name
                                    ),
                                    data: serde_json::json!({
                                        "title": work_title,
                                        "author": author.name,
                                        "year": year,
                                        "ol_key": work_ol_key,
                                        "work_id": new_work.id,
                                    }),
                                })
                                .await
                            {
                                tracing::warn!("create_notification failed: {e}");
                            }
                        }
                        Err(e) => {
                            warn!(
                                "author monitor: failed to auto-add work for <author {}>: {e}",
                                author.id
                            );
                        }
                    }
                } else {
                    if let Err(e) = state
                        .db
                        .create_notification(CreateNotificationDbRequest {
                            user_id: author.user_id,
                            notification_type: NotificationType::NewWorkDetected,
                            ref_key: Some(work_ol_key.clone()),
                            message: format!(
                                "New work '{}' by {} detected",
                                work_title, author.name
                            ),
                            data: serde_json::json!({
                                "title": work_title,
                                "author": author.name,
                                "year": year,
                                "ol_key": work_ol_key,
                            }),
                        })
                        .await
                    {
                        tracing::warn!("create_notification failed: {e}");
                    }
                }
            }
        }

        // Rate limit respect: 1s delay between authors.
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(1)) => {},
            _ = cancel.cancelled() => { return Ok(()); },
        }
        i += 1;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Enrichment Retry Tick (JOBS-ENRICH-001)
// ---------------------------------------------------------------------------

pub async fn enrichment_retry_tick(
    state: AppState,
    cancel: CancellationToken,
) -> Result<(), String> {
    // Queue-aware retry tick. For each user, asks the new
    // ProviderRetryStateDb which (work_id, provider) pairs have
    // next_attempt_at <= now and are in WillRetry or Suppressed state.
    // Dedups by work_id and dispatches one enrich_work call per due work
    // — the queue's restart-safety logic skips providers whose retry-state
    // row is already terminal, so only the actually-due providers run.
    // Circuit breaker, throttling, and merge-engine all apply automatically.
    use livrarr_db::{ProviderRetryStateDb, UserDb};
    use std::collections::HashSet;

    let users = match state.db.list_users().await {
        Ok(u) => u,
        Err(e) => return Err(format!("list_users: {e}")),
    };

    let now = chrono::Utc::now();
    let mut total_due = 0usize;
    let mut total_dispatched = 0usize;

    for user in &users {
        if cancel.is_cancelled() {
            return Ok(());
        }
        let due = match state.db.list_works_due_for_retry(user.id, now).await {
            Ok(d) => d,
            Err(e) => {
                warn!(
                    "enrichment_retry: list_works_due_for_retry({}): {e}",
                    user.id
                );
                continue;
            }
        };
        if due.is_empty() {
            continue;
        }
        total_due += due.len();

        // Dedup by work_id — one dispatch covers all due providers for that
        // work (queue skips already-terminal providers via restart-safety).
        let work_ids: HashSet<livrarr_domain::WorkId> = due.iter().map(|(w, _)| *w).collect();

        for work_id in work_ids {
            if cancel.is_cancelled() {
                return Ok(());
            }
            match tokio::time::timeout(
                Duration::from_secs(30),
                livrarr_domain::services::EnrichmentWorkflow::enrich_work(
                    state.enrichment_workflow.as_ref(),
                    user.id,
                    work_id,
                    livrarr_domain::services::EnrichmentMode::Background,
                ),
            )
            .await
            {
                Ok(Ok(result)) => {
                    total_dispatched += 1;
                    if let Some(ref cover_url) = result.work.cover_url {
                        crate::handlers::work::download_post_enrich_cover(
                            &state, work_id, cover_url,
                        )
                        .await;
                    }
                }
                Ok(Err(e)) => {
                    warn!(
                        "enrichment_retry: enrich_work({}, {}) failed: {e}",
                        user.id, work_id
                    );
                }
                Err(_) => {
                    warn!(
                        "enrichment_retry: enrich_work({}, {}) timed out",
                        user.id, work_id
                    );
                }
            }
        }
    }

    if total_due > 0 {
        debug!(
            "enrichment_retry: {} due (work,provider) pairs across users; dispatched {} works",
            total_due, total_dispatched,
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// RSS Sync Job
// ---------------------------------------------------------------------------

/// RSS sync background job tick (called by interval job runner).
pub async fn rss_sync_tick(state: AppState, _cancel: CancellationToken) -> Result<(), String> {
    use std::sync::atomic::Ordering;

    let config = state
        .db
        .get_indexer_config()
        .await
        .map_err(|e| format!("get_indexer_config: {e}"))?;

    if config.rss_sync_interval_minutes == 0 {
        return Ok(());
    }

    let now = chrono::Utc::now().timestamp();
    let last_run = state.rss_last_run.load(Ordering::Relaxed);
    let interval_secs = (config.rss_sync_interval_minutes as i64) * 60;
    if last_run > 0 && (now - last_run) < interval_secs {
        return Ok(());
    }

    rss_sync_run(state).await
}

/// RSS sync run — acquires the running guard and executes the full sync.
/// Called by both the scheduled tick (after interval check) and the trigger endpoint.
pub async fn rss_sync_run(state: AppState) -> Result<(), String> {
    use std::sync::atomic::Ordering;

    if state
        .rss_sync_running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("already running".into());
    }

    struct RunGuard(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for RunGuard {
        fn drop(&mut self) {
            self.0.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let _guard = RunGuard(state.rss_sync_running.clone());

    match state.rss_sync_workflow.run_sync().await {
        Ok(report) => {
            state
                .rss_last_run
                .store(chrono::Utc::now().timestamp(), Ordering::Relaxed);
            if !report.warnings.is_empty() {
                for w in &report.warnings {
                    warn!("RSS sync: {w}");
                }
            }
            Ok(())
        }
        Err(e) => Err(format!("RSS sync failed: {e}")),
    }
}

// ---------------------------------------------------------------------------
// State Map TTL Cleanup Tick
// ---------------------------------------------------------------------------

/// Remove stale entries from `import_locks` and `manual_import_scans`.
/// Runs every 30 minutes — evicts entries abandoned without explicit cleanup.
async fn state_map_cleanup_tick(state: AppState, _cancel: CancellationToken) -> Result<(), String> {
    crate::state::cleanup_import_locks(&state.import_locks);
    crate::state::cleanup_manual_import_scans(&state.manual_import_scans);
    trace!("state_map_cleanup: stale entries evicted");
    Ok(())
}
