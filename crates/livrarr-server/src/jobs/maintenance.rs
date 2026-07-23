use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};

use crate::state::AppState;
use livrarr_db::{GrabDb, ListImportDb, SessionDb};

/// Retention bounds for provider call records (REQ-001): 30 days / 100k rows.
const CALL_RECORD_RETENTION: livrarr_db::RetentionPolicy = livrarr_db::RetentionPolicy {
    max_age_days: 30,
    max_records: 100_000,
};

/// Evict provider call records past the retention bounds, oldest first
/// (REQ-001). Registered on a 6h interval.
pub async fn call_record_retention_tick(
    state: AppState,
    cancel: CancellationToken,
) -> Result<(), String> {
    use livrarr_db::ProviderCallRecordDb;

    if cancel.is_cancelled() {
        return Ok(());
    }

    match state.db.evict_call_records(CALL_RECORD_RETENTION).await {
        Ok(evicted) => {
            if evicted > 0 {
                info!("call-record retention: evicted {evicted} rows");
            }
            Ok(())
        }
        Err(e) => Err(format!("call-record retention sweep failed: {e}")),
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

    // Phase 7 will requeue stale Unenriched works via list_stale_unenriched_works.
    // No-op here until Phase 7 lands.

    // Reconcile import-intent crash-consistency records (Unit D2): complete
    // or roll back any file import interrupted by an unclean shutdown, then
    // sweep aged, unreferenced staging files. Must run before anything else
    // can start a new import — nothing is in-flight yet at this point.
    // recover_import_intents self-logs the reconciliation summary; here we
    // only need to surface a listing failure (never silently indistinguishable
    // from "nothing to recover") and escalate any anomalous intent left in
    // place. Startup always continues — a hard abort would take the whole
    // app down over one recovery pass (PO decision).
    match state.import_workflow.recover_import_intents().await {
        Ok(report) if report.anomalous > 0 => {
            error!(
                anomalous = report.anomalous,
                "import recovery: anomalous intents left in place — needs investigation"
            );
        }
        Ok(_) => {
            info!("import recovery: startup reconciliation complete, no anomalies");
        }
        Err(e) => {
            error!(
                error = %e,
                "import recovery FAILED to list intents — continuing startup; stale intents unreconciled"
            );
        }
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
// Session Cleanup Tick (JOBS-SESSION-001)
// ---------------------------------------------------------------------------

pub(super) async fn session_cleanup_tick(
    state: AppState,
    _cancel: CancellationToken,
) -> Result<(), String> {
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
// State Map TTL Cleanup Tick
// ---------------------------------------------------------------------------

/// Remove stale entries from `manual_import_scans`.
/// Runs every 30 minutes — evicts entries abandoned without explicit cleanup.
pub(super) async fn state_map_cleanup_tick(
    state: AppState,
    _cancel: CancellationToken,
) -> Result<(), String> {
    crate::state::cleanup_manual_import_scans(&state.manual_import_scans);
    trace!("state_map_cleanup: stale entries evicted");
    Ok(())
}
