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
    CreateHistoryEventDbRequest, CreateNotificationDbRequest, CreateWorkDbRequest,
    DownloadClientDb, EnrichmentRetryDb, GrabDb, HistoryDb, NotificationDb, SessionDb, WorkDb,
};
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
                    .set_grab_download_id(grab.user_id, grab.id, hash)
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
        let _permit = state.import_semaphore.acquire().await;
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
    let preview_count = sqlx::query("DELETE FROM list_import_previews WHERE created_at < ?")
        .bind(&cutoff)
        .execute(state.db.pool())
        .await
        .map(|r| r.rows_affected())
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
                        d.chars()
                            .filter(|c| c.is_ascii_digit())
                            .take(4)
                            .collect::<String>()
                            .parse::<i32>()
                            .ok()
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

                let work_title = entry
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("Unknown")
                    .to_string();

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
                            author_name: author.name.clone(),
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
                            monitor_ebook: false,
                            monitor_audiobook: false,
                        })
                        .await
                    {
                        Ok(new_work) => {
                            // Enrich the newly added work (30s timeout).
                            let enrich_result = tokio::time::timeout(
                                Duration::from_secs(30),
                                crate::handlers::enrichment::enrich_work(&state, &new_work),
                            )
                            .await;
                            let outcome = match enrich_result {
                                Ok(o) => o,
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
                                .update_work_enrichment(
                                    author.user_id,
                                    new_work.id,
                                    outcome.request,
                                )
                                .await
                            {
                                tracing::warn!("update_work_enrichment failed: {e}");
                            }

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
    _cancel: CancellationToken,
) -> Result<(), String> {
    let works = state
        .db
        .list_works_for_retry()
        .await
        .map_err(|e| format!("list works for retry: {e}"))?;

    if works.is_empty() {
        return Ok(());
    }

    debug!("enrichment retry: {} works eligible", works.len());

    for work in &works {
        // Route to the correct enrichment function based on metadata source.
        let is_foreign =
            livrarr_metadata::language::is_foreign_source(work.metadata_source.as_deref());

        // Skip foreign works without a detail URL — nothing to enrich from.
        if is_foreign && work.detail_url.is_none() {
            continue;
        }

        // 30s timeout per spec.
        let enrich_result = tokio::time::timeout(Duration::from_secs(30), async {
            if is_foreign {
                crate::handlers::enrichment::enrich_foreign_work(&state, work).await
            } else {
                crate::handlers::enrichment::enrich_work(&state, work).await
            }
        })
        .await;

        let outcome = match enrich_result {
            Ok(outcome) => outcome,
            Err(_) => {
                warn!("enrichment retry: timeout for work {}", work.id);
                if let Err(e) = state.db.increment_retry_count(work.user_id, work.id).await {
                    tracing::warn!("increment_retry_count failed: {e}");
                }
                continue;
            }
        };

        match state
            .db
            .update_work_enrichment(work.user_id, work.id, outcome.request)
            .await
        {
            Ok(updated) => {
                if updated.enrichment_status == livrarr_domain::EnrichmentStatus::Enriched
                    || updated.enrichment_status == livrarr_domain::EnrichmentStatus::Partial
                {
                    debug!("enrichment retry: work {} enriched successfully", work.id);
                } else {
                    if let Err(e) = state.db.increment_retry_count(work.user_id, work.id).await {
                        tracing::warn!("increment_retry_count failed: {e}");
                    }
                    debug!(
                        "enrichment retry: work {} still failed, count incremented",
                        work.id
                    );
                }
            }
            Err(e) => {
                warn!(
                    "enrichment retry: DB update failed for work {}: {e}",
                    work.id
                );
                if let Err(e) = state.db.increment_retry_count(work.user_id, work.id).await {
                    tracing::warn!("increment_retry_count failed: {e}");
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// RSS Sync Job
// ---------------------------------------------------------------------------

use livrarr_db::{ConfigDb, IndexerDb, LibraryItemDb};
use livrarr_domain::MediaType;

/// Convert a Work to a MatchCandidate for M4 scoring.
///
/// Satisfies: RSS-MATCH-001
fn work_to_candidate(work: &livrarr_domain::Work) -> crate::matching::types::MatchCandidate {
    crate::matching::types::MatchCandidate {
        title: work.title.clone(),
        author: work.author_name.clone(),
        year: work.year,
        work_key: String::new(),
        author_key: None,
        cover_url: None,
        series: work.series_name.clone(),
        series_position: work.series_position,
        provider: crate::matching::types::MatchProvider::OpenLibrary,
        score: 0.0,
    }
}

/// Derive media type(s) from Torznab release categories.
///
/// Satisfies: RSS-FILTER-001
fn media_types_from_categories(categories: &[i32]) -> Vec<MediaType> {
    let mut types = Vec::new();
    if categories.contains(&7020) {
        types.push(MediaType::Ebook);
    }
    if categories.contains(&3030) {
        types.push(MediaType::Audiobook);
    }
    types
}

/// Fetch recent releases from a single indexer (no search query).
///
/// Satisfies: RSS-FETCH-001
async fn fetch_rss(
    http: &livrarr_http::HttpClient,
    indexer: &livrarr_domain::Indexer,
) -> Result<Vec<crate::ReleaseResponse>, String> {
    let t_val = if indexer.supports_book_search {
        "book"
    } else {
        "search"
    };
    let cats = indexer
        .categories
        .iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let url = crate::handlers::release::build_torznab_url(
        &indexer.url,
        &indexer.api_path,
        indexer.api_key.as_deref(),
        &[
            ("t", t_val),
            ("cat", &cats),
            ("limit", "100"),
            ("extended", "1"),
        ],
    );
    let mut releases = crate::handlers::release::fetch_and_parse(http, &url, &indexer.name).await?;
    // Sort: dated items first (by pubDate descending), then undated.
    releases.sort_by(|a, b| match (&b.publish_date, &a.publish_date) {
        (Some(bd), Some(ad)) => bd.cmp(ad),
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (None, None) => std::cmp::Ordering::Equal,
    });
    Ok(releases)
}

/// RSS sync background job tick (called by interval job runner).
pub async fn rss_sync_tick(state: AppState, cancel: CancellationToken) -> Result<(), String> {
    use std::sync::atomic::Ordering;

    // Step 1: Read config — return if disabled.
    let config = state
        .db
        .get_indexer_config()
        .await
        .map_err(|e| format!("get_indexer_config: {e}"))?;

    if config.rss_sync_interval_minutes == 0 {
        return Ok(());
    }

    // Step 2: Check if enough time has passed since last run.
    let now = chrono::Utc::now().timestamp();
    let last_run = state.rss_last_run.load(Ordering::Relaxed);
    let interval_secs = (config.rss_sync_interval_minutes as i64) * 60;
    if last_run > 0 && (now - last_run) < interval_secs {
        return Ok(());
    }

    rss_sync_run(state, cancel).await
}

/// RSS sync run — acquires the running guard and executes the full sync.
/// Called by both the scheduled tick (after interval check) and the trigger endpoint.
///
/// Returns Err("already running") if the CAS guard cannot be acquired.
pub async fn rss_sync_run(state: AppState, cancel: CancellationToken) -> Result<(), String> {
    use std::sync::atomic::Ordering;

    // Acquire running guard — prevents overlap.
    if state
        .rss_sync_running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("already running".into());
    }

    // Ensure guard is released on all exit paths.
    struct RunGuard(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for RunGuard {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    let _guard = RunGuard(state.rss_sync_running.clone());

    rss_sync_core(state, cancel).await
}

/// Core RSS sync logic — assumes guard is held by caller.
///
/// Satisfies: RSS-FETCH-001..002, RSS-MATCH-001..002, RSS-FILTER-001..003,
///            RSS-GRAB-001..003, RSS-GAP-001, RSS-JOB-001, RSS-LOG-001
pub(crate) async fn rss_sync_core(
    state: AppState,
    cancel: CancellationToken,
) -> Result<(), String> {
    use std::collections::{HashMap, HashSet};
    use std::sync::atomic::Ordering;

    let config = state
        .db
        .get_indexer_config()
        .await
        .map_err(|e| format!("get_indexer_config: {e}"))?;

    let media_mgmt = state
        .db
        .get_media_management_config()
        .await
        .map_err(|e| format!("get_media_management_config: {e}"))?;

    let now = chrono::Utc::now().timestamp();

    info!("RSS sync: starting");

    // Step 3: Fetch from all RSS-enabled indexers in parallel.
    let indexers = state
        .db
        .list_enabled_rss_indexers()
        .await
        .map_err(|e| format!("list_enabled_rss_indexers: {e}"))?;

    if indexers.is_empty() {
        debug!("RSS sync: no RSS-enabled indexers, nothing to do");
        state.rss_last_run.store(now, Ordering::Relaxed);
        return Ok(());
    }

    debug!("RSS sync: fetching from {} indexers", indexers.len());

    // Pre-query state for all indexers before any upserting (for first-sync detection).
    let mut pre_states: HashMap<i64, Option<livrarr_domain::IndexerRssState>> = HashMap::new();
    for idx in &indexers {
        let s = state
            .db
            .get_rss_state(idx.id)
            .await
            .map_err(|e| format!("get_rss_state: {e}"))?;
        pre_states.insert(idx.id, s);
    }

    let mut fetch_results: Vec<(livrarr_domain::Indexer, Vec<crate::ReleaseResponse>)> = Vec::new();
    {
        let mut join_set = tokio::task::JoinSet::new();
        for indexer in indexers {
            let http = state.http_client_safe.clone();
            let idx = indexer.clone();
            join_set.spawn(async move {
                let result =
                    tokio::time::timeout(Duration::from_secs(30), fetch_rss(&http, &idx)).await;
                (idx, result)
            });
        }
        while let Some(result) = join_set.join_next().await {
            if cancel.is_cancelled() {
                return Ok(());
            }
            match result {
                Ok((idx, Ok(Ok(releases)))) => {
                    debug!(
                        "RSS sync: fetched {} releases from {}",
                        releases.len(),
                        idx.name
                    );
                    fetch_results.push((idx, releases));
                }
                Ok((idx, Ok(Err(e)))) => {
                    warn!("RSS sync: fetch failed for {}: {e}", idx.name);
                }
                Ok((idx, Err(_))) => {
                    warn!("RSS sync: fetch timeout for {}", idx.name);
                }
                Err(e) => {
                    warn!("RSS sync: join error: {e}");
                }
            }
        }
    }

    // Counters for RSS-LOG-001.
    let mut n_fetched: usize = 0;
    let mut n_parsed: usize = 0;
    let mut n_unparsed: usize = 0;
    let mut n_grabbed: usize = 0;
    let mut n_skipped: usize = 0;

    // Collect first-sync indexer IDs.
    let first_sync_ids: HashSet<i64> = pre_states
        .iter()
        .filter(|(_, v)| v.is_none())
        .map(|(k, _)| *k)
        .collect();

    // Step 4: Per-indexer gap detection + state update.
    // Only consider items with non-empty GUIDs (RSS-FILTER-003).
    for (indexer, releases) in &fetch_results {
        n_fetched += releases.len();

        let existing_state = pre_states.get(&indexer.id).and_then(|s| s.as_ref());

        // Filter to items with non-empty GUIDs for gap/state tracking.
        let guidful: Vec<&crate::ReleaseResponse> =
            releases.iter().filter(|r| !r.guid.is_empty()).collect();

        // Find dated+guidful items sorted newest first.
        let mut dated_items: Vec<&crate::ReleaseResponse> = guidful
            .iter()
            .filter(|r| r.publish_date.is_some())
            .copied()
            .collect();
        dated_items.sort_by(|a, b| {
            b.publish_date
                .as_deref()
                .unwrap_or("")
                .cmp(a.publish_date.as_deref().unwrap_or(""))
        });

        // Gap detection (RSS-GAP-001).
        let mut n_gaps = 0usize;
        if let Some(es) = existing_state {
            if let (Some(ref stored_date), Some(oldest_dated)) =
                (&es.last_publish_date, dated_items.last())
            {
                if let Some(ref oldest_pub) = oldest_dated.publish_date {
                    if oldest_pub > stored_date {
                        let stored_guid = es.last_guid.as_deref().unwrap_or("");
                        let guid_in_batch = guidful.iter().any(|r| r.guid == stored_guid);
                        if !guid_in_batch {
                            n_gaps += 1;
                            warn!(
                                "RSS sync: gap detected for indexer {} — oldest item {} > stored {}",
                                indexer.name, oldest_pub, stored_date
                            );
                            trace!(
                                "RSS sync: gap detected for {} — oldest item {} > stored {}, stored GUID not in batch",
                                indexer.name, oldest_pub, stored_date
                            );
                        }
                    }
                }
            }
        }

        debug!(
            "RSS sync: gap detection complete for {}, {} gaps found",
            indexer.name, n_gaps
        );

        // Update state to newest dated item.
        if let Some(newest) = dated_items.first() {
            trace!(
                "RSS sync: state update for {} — newest guid={}, date={:?}",
                indexer.name,
                newest.guid,
                newest.publish_date
            );
            state
                .db
                .upsert_rss_state(indexer.id, newest.publish_date.as_deref(), &newest.guid)
                .await
                .map_err(|e| format!("upsert_rss_state: {e}"))?;
        } else if !guidful.is_empty() && existing_state.is_none() {
            // No dated items but have guidful releases — store guid only (RSS-JOB-001).
            trace!(
                "RSS sync: state update for {} — guid only={}, no dated items",
                indexer.name,
                guidful[0].guid
            );
            state
                .db
                .upsert_rss_state(indexer.id, None, &guidful[0].guid)
                .await
                .map_err(|e| format!("upsert_rss_state: {e}"))?;
        }

        if first_sync_ids.contains(&indexer.id) {
            info!(
                "RSS sync: first sync for indexer {} — recording state, no grabs",
                indexer.name
            );
            trace!(
                "RSS sync: first sync for {} — storing {} GUIDs, skipping grabs",
                indexer.name,
                guidful.len()
            );
        }
    }

    // Step 5: Load monitored works and pre-compute candidates.
    let monitored_works = state
        .db
        .list_monitored_works_all_users()
        .await
        .map_err(|e| format!("list_monitored_works_all_users: {e}"))?;

    if monitored_works.is_empty() {
        state.rss_last_run.store(now, Ordering::Relaxed);
        info!("RSS sync: {n_fetched} releases, 0 matched (no monitored works)");
        return Ok(());
    }

    debug!("RSS sync: parsing {} releases", n_fetched);
    debug!(
        "RSS sync: matching against {} monitored works (threshold {})",
        monitored_works.len(),
        config.rss_match_threshold
    );

    let candidates: Vec<(livrarr_domain::Work, crate::matching::types::MatchCandidate)> =
        monitored_works
            .iter()
            .map(|w| (w.clone(), work_to_candidate(w)))
            .collect();

    // Precompute protocol availability to avoid repeated DB calls (RSS-GRAB-003).
    let has_torrent_client = state
        .db
        .get_default_download_client("qbittorrent")
        .await
        .map(|c| c.is_some())
        .unwrap_or(false);
    let has_usenet_client = state
        .db
        .get_default_download_client("sabnzbd")
        .await
        .map(|c| c.is_some())
        .unwrap_or(false);

    // Step 6: Two-phase matching per spec.
    //
    // Phase 1 (RSS-MATCH-001): For each (release, user, media_type), find the
    //   best-scoring work. One release → at most one work per user per media_type.
    //   Tie on score: lower work_id wins.
    //
    // Phase 2 (RSS-MATCH-002): For each (user, work, media_type), select the
    //   best eligible release. Ranking: score desc, indexer priority asc,
    //   seeders desc, size asc, GUID asc.

    struct GrabCandidate {
        user_id: i64,
        work_id: i64,
        media_type: MediaType,
        score: f64,
        indexer_priority: i32,
        seeders: i32,
        size: i64,
        release: crate::ReleaseResponse,
        indexer_name: String,
    }

    let threshold = config.rss_match_threshold;
    let mut n_matched: usize = 0;

    // Phase 1: Per-release work selection.
    // Intermediate: Vec<(release_info, user_id, work_id, media_type, score)>
    struct ReleaseMatch {
        user_id: i64,
        work_id: i64,
        media_type: MediaType,
        score: f64,
        indexer_priority: i32,
        release: crate::ReleaseResponse,
        indexer_name: String,
    }
    let mut release_matches: Vec<ReleaseMatch> = Vec::new();

    for (indexer, releases) in &fetch_results {
        // Skip grabs for first-sync indexers (RSS-JOB-001).
        if first_sync_ids.contains(&indexer.id) {
            continue;
        }

        for release in releases {
            trace!(
                "RSS sync: [{}] release '{}' — guid={}, protocol={}, categories={:?}, size={}, seeders={:?}, published={:?}",
                indexer.name, release.title, release.guid, release.protocol,
                release.categories, release.size, release.seeders, release.publish_date
            );

            // RSS-FILTER-003: skip items without GUID.
            if release.guid.is_empty() {
                trace!("RSS sync: skip '{}' — no GUID", release.title);
                continue;
            }

            // Determine media types from categories (RSS-FILTER-001).
            let media_types = media_types_from_categories(&release.categories);
            if media_types.is_empty() {
                trace!(
                    "RSS sync: skip '{}' — no ebook/audiobook category (categories: {:?})",
                    release.title,
                    release.categories
                );
                continue;
            }
            trace!(
                "RSS sync: '{}' media types: {:?}",
                release.title,
                media_types
            );

            // RSS-GRAB-003: filter ineligible protocols before ranking.
            let protocol_eligible = match release.protocol.as_str() {
                "usenet" => has_usenet_client,
                _ => has_torrent_client,
            };
            if !protocol_eligible {
                trace!(
                    "RSS sync: skip '{}' — no {} client configured",
                    release.title,
                    release.protocol
                );
                continue;
            }

            // M3 parse (RSS-MATCH-001).
            let (extractions, side) = crate::matching::m3_string::parse_string(&release.title);
            if extractions.is_empty() {
                n_unparsed += 1;
                trace!("RSS sync: unparseable '{}'", release.title);
                continue;
            }
            n_parsed += 1;
            trace!(
                "RSS sync: '{}' parsed — {} extractions",
                release.title,
                extractions.len()
            );

            // RSS-FILTER-005: format preference check.
            // Filter per media type — a release may be eligible for one type but not another.
            let format_lower = side.format.as_deref().map(|f| f.to_lowercase());
            let mut format_eligible_types: Vec<MediaType> = Vec::new();
            for mt in &media_types {
                let prefs = match mt {
                    MediaType::Ebook => &media_mgmt.preferred_ebook_formats,
                    MediaType::Audiobook => &media_mgmt.preferred_audiobook_formats,
                };
                if let Some(ref fmt) = format_lower {
                    if !prefs.is_empty() && !prefs.iter().any(|p| p.eq_ignore_ascii_case(fmt)) {
                        trace!(
                            "RSS sync: skip '{}' for {:?} — format {} not in preferences {:?}",
                            release.title,
                            mt,
                            fmt,
                            prefs
                        );
                        continue;
                    }
                }
                format_eligible_types.push(*mt);
            }
            if format_eligible_types.is_empty() {
                n_skipped += 1;
                continue;
            }

            // For each media type, find the best work per user (Phase 1).
            for mt in &format_eligible_types {
                // Collect best work per user for this release+media_type.
                let mut best_per_user: HashMap<i64, (i64, f64)> = HashMap::new(); // user_id → (work_id, score)

                for (work, cand) in &candidates {
                    let monitored_for_type = match mt {
                        MediaType::Ebook => work.monitor_ebook,
                        MediaType::Audiobook => work.monitor_audiobook,
                    };
                    if !monitored_for_type {
                        trace!(
                            "RSS sync: '{}' vs work {} — not monitored for {:?}, skipping",
                            release.title,
                            work.id,
                            mt
                        );
                        continue;
                    }

                    // RSS-FILTER-004: skip releases published before work was added.
                    if let Some(ref pub_date_str) = release.publish_date {
                        let published = chrono::DateTime::parse_from_rfc2822(pub_date_str)
                            .or_else(|_| chrono::DateTime::parse_from_rfc3339(pub_date_str));
                        if let Ok(pub_dt) = published {
                            if pub_dt < work.added_at {
                                trace!(
                                    "RSS sync: skip '{}' for work {} — published {} before work added {}",
                                    release.title, work.id, pub_date_str, work.added_at
                                );
                                continue;
                            }
                        }
                    }

                    let best_score = extractions
                        .iter()
                        .map(|ext| crate::matching::m4_scoring::score_candidate(ext, cand))
                        .fold(0.0_f64, f64::max);

                    trace!(
                        "RSS sync: '{}' vs work {} '{}' — score {:.3} (threshold {})",
                        release.title,
                        work.id,
                        work.title,
                        best_score,
                        threshold
                    );

                    if best_score < threshold {
                        trace!(
                            "RSS sync: '{}' below threshold for work {} — score {:.3} < {}",
                            release.title,
                            work.id,
                            best_score,
                            threshold
                        );
                        continue;
                    }

                    // RSS-MATCH-001: best score wins. Tie: lower work_id.
                    let is_better = match best_per_user.get(&work.user_id) {
                        None => true,
                        Some(&(existing_wid, existing_score)) => {
                            (best_score, std::cmp::Reverse(work.id))
                                > (existing_score, std::cmp::Reverse(existing_wid))
                        }
                    };

                    if is_better {
                        best_per_user.insert(work.user_id, (work.id, best_score));
                    }
                }

                // Emit one match per user for this release.
                for (uid, (wid, score)) in &best_per_user {
                    n_matched += 1;
                    let mt_label = match mt {
                        MediaType::Ebook => "ebook",
                        MediaType::Audiobook => "audiobook",
                    };
                    trace!(
                        "RSS sync: match '{}' → work {} (user {}, {}, score {:.3})",
                        release.title,
                        wid,
                        uid,
                        mt_label,
                        score
                    );
                    release_matches.push(ReleaseMatch {
                        user_id: *uid,
                        work_id: *wid,
                        media_type: *mt,
                        score: *score,
                        indexer_priority: indexer.priority,
                        release: crate::ReleaseResponse {
                            title: release.title.clone(),
                            indexer: release.indexer.clone(),
                            size: release.size,
                            guid: release.guid.clone(),
                            download_url: release.download_url.clone(),
                            seeders: release.seeders,
                            leechers: release.leechers,
                            publish_date: release.publish_date.clone(),
                            protocol: release.protocol.clone(),
                            categories: release.categories.clone(),
                        },
                        indexer_name: indexer.name.clone(),
                    });
                }
            }
        }
    }

    debug!(
        "RSS sync: phase 1 complete — {} release-work matches",
        n_matched
    );

    // Phase 2: Best release per (user, work, media_type) (RSS-MATCH-002).
    let mut best_map: HashMap<(i64, i64, String), GrabCandidate> = HashMap::new();

    for rm in release_matches {
        let mt_str = match rm.media_type {
            MediaType::Ebook => "ebook",
            MediaType::Audiobook => "audiobook",
        };
        let key = (rm.user_id, rm.work_id, mt_str.to_string());
        let seeders = rm.release.seeders.unwrap_or(0);

        let is_better = match best_map.get(&key) {
            None => true,
            Some(existing) => {
                // RSS-MATCH-002: score desc, priority asc, seeders desc, size asc, guid asc.
                (
                    rm.score,
                    std::cmp::Reverse(rm.indexer_priority),
                    seeders,
                    std::cmp::Reverse(rm.release.size),
                    std::cmp::Reverse(rm.release.guid.as_str()),
                ) > (
                    existing.score,
                    std::cmp::Reverse(existing.indexer_priority),
                    existing.seeders,
                    std::cmp::Reverse(existing.size),
                    std::cmp::Reverse(existing.release.guid.as_str()),
                )
            }
        };

        if is_better {
            if let Some(existing) = best_map.get(&key) {
                trace!(
                    "RSS sync: phase 2 — '{}' beats '{}' for work {} (score {:.3} vs {:.3}, priority {} vs {})",
                    rm.release.title, existing.release.title, rm.work_id,
                    rm.score, existing.score, rm.indexer_priority, existing.indexer_priority
                );
            }
            best_map.insert(
                key,
                GrabCandidate {
                    user_id: rm.user_id,
                    work_id: rm.work_id,
                    media_type: rm.media_type,
                    score: rm.score,
                    indexer_priority: rm.indexer_priority,
                    seeders,
                    size: rm.release.size,
                    release: rm.release,
                    indexer_name: rm.indexer_name,
                },
            );
        }
    }

    debug!(
        "RSS sync: phase 2 complete — {} grab candidates after dedup",
        best_map.len()
    );

    // Step 7: Filter and grab.
    debug!(
        "RSS sync: filtering and grabbing {} candidates",
        best_map.len()
    );
    for gc in best_map.values() {
        if cancel.is_cancelled() {
            break;
        }

        // RSS-FILTER-002: active grab exists?
        match state
            .db
            .active_grab_exists(gc.user_id, gc.work_id, gc.media_type)
            .await
        {
            Ok(true) => {
                trace!(
                    "RSS sync: skip '{}' — active grab exists for work {}",
                    gc.release.title,
                    gc.work_id
                );
                n_skipped += 1;
                continue;
            }
            Err(e) => {
                warn!("RSS sync: active_grab_exists error: {e}");
                n_skipped += 1;
                continue;
            }
            Ok(false) => {}
        }

        // RSS-FILTER-002: already in library?
        match state
            .db
            .work_has_library_item(gc.user_id, gc.work_id, gc.media_type)
            .await
        {
            Ok(true) => {
                trace!(
                    "RSS sync: skip '{}' — already in library for work {}",
                    gc.release.title,
                    gc.work_id
                );
                n_skipped += 1;
                continue;
            }
            Err(e) => {
                warn!("RSS sync: work_has_library_item error: {e}");
                n_skipped += 1;
                continue;
            }
            Ok(false) => {}
        }

        // Step 8: Grab (RSS-GRAB-001).
        debug!(
            "RSS sync: grabbing '{}' for work {} via {}",
            gc.release.title, gc.work_id, gc.indexer_name
        );
        let grab_req = crate::handlers::release::InternalGrabRequest {
            user_id: gc.user_id,
            work_id: gc.work_id,
            download_url: gc.release.download_url.clone(),
            title: gc.release.title.clone(),
            indexer: gc.indexer_name.clone(),
            guid: gc.release.guid.clone(),
            size: gc.release.size,
            protocol: gc.release.protocol.clone(),
            categories: gc.release.categories.clone(),
            download_client_id: None,
            source: "rss".to_string(),
        };

        match crate::handlers::release::do_grab_internal(&state, grab_req).await {
            Ok(_grab) => {
                n_grabbed += 1;

                // RSS-GRAB-001: RssGrabbed notification.
                if let Err(e) = state
                    .db
                    .create_notification(CreateNotificationDbRequest {
                        user_id: gc.user_id,
                        notification_type: NotificationType::RssGrabbed,
                        ref_key: Some(format!("rss:{}", gc.release.guid)),
                        message: format!(
                            "RSS grabbed: {} (score {:.2})",
                            gc.release.title, gc.score
                        ),
                        data: serde_json::json!({
                            "title": gc.release.title,
                            "indexer": gc.indexer_name,
                            "score": gc.score,
                            "workId": gc.work_id,
                        }),
                    })
                    .await
                {
                    tracing::warn!("create_notification failed: {e}");
                }
            }
            Err(e) => {
                warn!("RSS sync: grab failed for '{}': {:?}", gc.release.title, e);

                // RSS-GRAB-001: RssGrabFailed notification (dedup via ref_key).
                if let Err(ne) = state
                    .db
                    .create_notification(CreateNotificationDbRequest {
                        user_id: gc.user_id,
                        notification_type: NotificationType::RssGrabFailed,
                        ref_key: Some(format!("rss-fail:{}", gc.release.guid)),
                        message: format!("RSS grab failed: {}", gc.release.title),
                        data: serde_json::json!({
                            "title": gc.release.title,
                            "indexer": gc.indexer_name,
                            "error": format!("{:?}", e),
                            "workId": gc.work_id,
                        }),
                    })
                    .await
                {
                    tracing::warn!("create_notification failed: {ne}");
                }

                n_skipped += 1;
            }
        }
    }

    // Step 9: Summary log (RSS-LOG-001).
    info!(
        "RSS sync: {n_fetched} releases, {n_parsed} parsed, {n_unparsed} unparseable, \
         {n_matched} matched, {n_grabbed} grabbed, {n_skipped} filtered"
    );

    // Update last-run timestamp.
    state
        .rss_last_run
        .store(chrono::Utc::now().timestamp(), Ordering::Relaxed);

    Ok(())
}
