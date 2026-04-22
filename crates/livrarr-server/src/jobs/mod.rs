//! Background job runner and interval jobs.
//!
//! Satisfies: JOBS-001, JOBS-002, JOBS-003, JOBS-004

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::state::AppState;
use livrarr_db::{CreateNotificationDbRequest, NotificationDb};
use livrarr_domain::NotificationType;

pub mod author_monitor;
pub mod download_poller;
pub mod enrichment;
pub mod maintenance;
pub mod rss_sync;

pub use maintenance::recover_interrupted_state;

use self::author_monitor::author_monitor_tick;
use self::download_poller::download_poller_tick;
use self::enrichment::enrichment_retry_tick;
use self::maintenance::{session_cleanup_tick, state_map_cleanup_tick};
use self::rss_sync::rss_sync_tick;

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
        {
            let status = self.status.clone();
            let cancel = self.cancel.clone();
            status.write().await.push(JobStatus {
                name: "enrichment_retry".to_string(),
                interval: Duration::from_secs(300),
                last_run: None,
                running: false,
                panic_notified: false,
            });
            let s = state.clone();
            let handle = tokio::spawn(async move {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {},
                    _ = cancel.cancelled() => return,
                }
                if let Err(e) = enrichment_retry_tick(s.clone(), cancel.clone()).await {
                    error!("enrichment_retry initial tick: {e}");
                }
                set_job_running(&status, "enrichment_retry", false).await;
                if let Some(st) = status
                    .write()
                    .await
                    .iter_mut()
                    .find(|s| s.name == "enrichment_retry")
                {
                    st.last_run = Some(chrono::Utc::now());
                }
                debug!("job 'enrichment_retry' tick completed");
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(300)) => {}
                        _ = s.enrichment_notify.notified() => {}
                        _ = cancel.cancelled() => {
                            debug!("job 'enrichment_retry' cancelled");
                            break;
                        }
                    }
                    if cancel.is_cancelled() {
                        break;
                    }
                    set_job_running(&status, "enrichment_retry", true).await;
                    match tokio::spawn({
                        let s2 = s.clone();
                        let c2 = cancel.clone();
                        async move { enrichment_retry_tick(s2, c2).await }
                    })
                    .await
                    {
                        Ok(Ok(())) => {
                            let mut statuses = status.write().await;
                            if let Some(st) =
                                statuses.iter_mut().find(|s| s.name == "enrichment_retry")
                            {
                                st.last_run = Some(chrono::Utc::now());
                                st.running = false;
                            }
                            debug!("job 'enrichment_retry' tick completed");
                        }
                        Ok(Err(e)) => {
                            error!("job 'enrichment_retry' error: {e}");
                            set_job_running(&status, "enrichment_retry", false).await;
                        }
                        Err(join_err) if join_err.is_panic() => {
                            let payload = join_err.into_panic();
                            let msg = payload
                                .downcast_ref::<&str>()
                                .map(|s| s.to_string())
                                .or_else(|| payload.downcast_ref::<String>().cloned())
                                .unwrap_or_else(|| "unknown panic".to_string());
                            error!("job 'enrichment_retry' panicked: {msg}");
                            let mut statuses = status.write().await;
                            if let Some(st) =
                                statuses.iter_mut().find(|st| st.name == "enrichment_retry")
                            {
                                st.running = false;
                                if !st.panic_notified {
                                    st.panic_notified = true;
                                    if let Err(e) =
                                        s.db.create_notification(CreateNotificationDbRequest {
                                            user_id: 1,
                                            notification_type: NotificationType::JobPanicked,
                                            ref_key: Some("enrichment_retry".to_string()),
                                            message: format!(
                                                "Job 'enrichment_retry' panicked: {msg}"
                                            ),
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
                            warn!("job 'enrichment_retry' task cancelled: {join_err}");
                            set_job_running(&status, "enrichment_retry", false).await;
                        }
                    }
                }
            });
            self.handles.lock().await.push(handle);
        }
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
