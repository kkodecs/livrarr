use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::state::AppState;
use livrarr_db::ConfigDb;
use livrarr_domain::services::RssSyncWorkflow;

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
