//! Background job runner and interval jobs.
//!
//! Satisfies: JOBS-001, JOBS-002, JOBS-003, JOBS-004

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::state::AppState;
use livrarr_db::{
    CreateHistoryEventDbRequest, CreateNotificationDbRequest, CreateWorkDbRequest,
    DownloadClientDb, EnrichmentRetryDb, GrabDb, HistoryDb, NotificationDb, SessionDb, WorkDb,
};
use livrarr_domain::{EventType, NotificationType};

// ---------------------------------------------------------------------------
// JobRunner
// ---------------------------------------------------------------------------

/// Fix #1: Derive Clone so it can be added to AppState.
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
    /// Fix #5: start is now async so we can register status synchronously.
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
            state,
            enrichment_retry_tick,
        )
        .await;
    }

    /// Fix #5: async fn — register status directly, no detached spawn.
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
                        // Fix #6: Extract panic payload for logging.
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
// Fix #7: Use DB trait methods instead of raw SQL.
// ---------------------------------------------------------------------------

/// Reset stale state from unclean shutdown. Run once before starting jobs.
pub async fn recover_interrupted_state(state: &AppState) {
    // Reset importing grabs → confirmed.
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
        match client.client_type.as_str() {
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

    Ok(())
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
            let local_path = match crate::handlers::import::apply_remote_path_mapping(
                state,
                &client.host,
                &content_path,
            )
            .await
            {
                Ok(p) => p,
                Err(e) => {
                    warn!("poller: path mapping failed for grab {}: {e}", grab.id);
                    continue;
                }
            };
            if !std::path::Path::new(&local_path).exists() {
                tracing::debug!(
                    "poller: source not yet available for grab {}, will retry: {local_path}",
                    grab.id
                );
                continue;
            }

            let transitioned = match state.db.try_set_importing(grab.user_id, grab.id).await {
                Ok(t) => t,
                Err(e) => {
                    warn!("poller: try_set_importing failed for grab {}: {e}", grab.id);
                    continue;
                }
            };

            if transitioned {
                match crate::handlers::import::import_grab(state, grab.user_id, grab.id).await {
                    Ok(result) => {
                        info!(
                            "poller: imported grab {} — {} files imported",
                            grab.id, result.imported_count
                        );
                    }
                    Err(e) => {
                        warn!("poller: import failed for grab {}: {e}", grab.id);
                    }
                }
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
                    let local_path = match crate::handlers::import::apply_remote_path_mapping(
                        state,
                        &client.host,
                        storage,
                    )
                    .await
                    {
                        Ok(p) => p,
                        Err(e) => {
                            warn!("poller: path mapping failed for grab {}: {e}", grab.id);
                            continue;
                        }
                    };
                    if !std::path::Path::new(&local_path).exists() {
                        tracing::debug!(
                            "poller: source not yet available for grab {}, will retry: {local_path}",
                            grab.id
                        );
                        continue;
                    }

                    info!(
                        "poller: SABnzbd completed grab {}, storage={storage}",
                        grab.id
                    );

                    let transitioned = match state.db.try_set_importing(grab.user_id, grab.id).await
                    {
                        Ok(t) => t,
                        Err(e) => {
                            warn!("poller: try_set_importing failed for grab {}: {e}", grab.id);
                            continue;
                        }
                    };

                    if transitioned {
                        match crate::handlers::import::import_grab(state, grab.user_id, grab.id)
                            .await
                        {
                            Ok(result) => {
                                info!(
                                    "poller: imported grab {} — {} files imported",
                                    grab.id, result.imported_count
                                );
                            }
                            Err(e) => {
                                warn!("poller: import failed for grab {}: {e}", grab.id);
                            }
                        }
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

    // Fix #3: index-based loop so we can retry on 429 without skipping.
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
                    "author monitor: new work detected — '{}' by {} ({})",
                    work_title, author.name, year
                );

                if author.monitor_new_items {
                    // Fix #2: Actually create the work and enrich it (AUTHOR-004).
                    match state
                        .db
                        .create_work(CreateWorkDbRequest {
                            user_id: author.user_id,
                            title: work_title.clone(),
                            author_name: author.name.clone(),
                            author_id: Some(author.id),
                            ol_key: Some(work_ol_key.clone()),
                            year: publish_year,
                            cover_url: None,
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
                                        "author monitor: enrichment timeout for '{}'",
                                        work_title
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
                            warn!("author monitor: failed to auto-add '{}': {e}", work_title);
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
// Fix #8: Explicit 30s timeout on enrichment calls.
// ---------------------------------------------------------------------------

async fn enrichment_retry_tick(state: AppState, _cancel: CancellationToken) -> Result<(), String> {
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
        // Fix #8: 30s timeout per spec.
        let enrich_result = tokio::time::timeout(
            Duration::from_secs(30),
            crate::handlers::enrichment::enrich_work(&state, work),
        )
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
