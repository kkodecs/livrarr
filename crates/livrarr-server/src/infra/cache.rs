use std::time::Duration;

// =============================================================================
// Manual Import Scan State — progressive OL lookup results
// =============================================================================

pub const STATE_MAP_TTL: Duration = Duration::from_secs(30 * 60); // 30 minutes

pub struct ManualImportScanState {
    pub files: std::sync::RwLock<Vec<livrarr_handlers::manual_import::ScannedFile>>,
    pub warnings: Vec<String>,
    pub ol_total: usize,
    pub ol_completed: std::sync::atomic::AtomicUsize,
    pub user_id: i64,
    pub created_at: std::time::Instant,
}

pub type ManualImportScanMap = dashmap::DashMap<String, ManualImportScanState>;

/// Remove entries from `manual_import_scans` that were created more than 30 minutes ago.
pub fn cleanup_manual_import_scans(map: &ManualImportScanMap) {
    let cutoff = std::time::Instant::now()
        .checked_sub(STATE_MAP_TTL)
        .unwrap_or_else(std::time::Instant::now);
    map.retain(|_, scan| scan.created_at > cutoff);
}
