use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};

use crate::state::AppState;
use livrarr_db::{GrabDb, ListImportDb, SessionDb, WorkDb};

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
