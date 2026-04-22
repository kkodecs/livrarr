use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

// =============================================================================
// Grab Search Cache — avoids hammering indexers for repeated searches
// =============================================================================

pub const GRAB_CACHE_TTL_SECS: u64 = 86400; // 24 hours
pub const GRAB_CACHE_CLEANUP_INTERVAL_SECS: u64 = 300; // 5 minutes

type GrabCacheMap = HashMap<(String, String, i64), (Instant, Vec<crate::ReleaseResponse>)>;

/// In-memory cache for grab search results, keyed by (title, author, indexer_id).
pub struct GrabSearchCache {
    entries: RwLock<GrabCacheMap>,
    last_cleanup: RwLock<Instant>,
}

impl Default for GrabSearchCache {
    fn default() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            last_cleanup: RwLock::new(Instant::now()),
        }
    }
}

impl GrabSearchCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up cached results. Returns None if missing or expired.
    /// On hit, returns (results, age_in_seconds).
    pub async fn get(
        &self,
        title: &str,
        author: &str,
        indexer_id: i64,
    ) -> Option<(Vec<crate::ReleaseResponse>, u64)> {
        let entries = self.entries.read().await;
        let key = (title.to_string(), author.to_string(), indexer_id);
        let (ts, results) = entries.get(&key)?;
        let age = ts.elapsed().as_secs();
        if age < GRAB_CACHE_TTL_SECS {
            Some((results.clone(), age))
        } else {
            None
        }
    }

    /// Store results for a (title, author, indexer_id) tuple.
    /// Periodically evicts expired entries (at most once per 5 minutes).
    pub async fn put(
        &self,
        title: &str,
        author: &str,
        indexer_id: i64,
        results: Vec<crate::ReleaseResponse>,
    ) {
        let mut entries = self.entries.write().await;
        let should_cleanup =
            self.last_cleanup.read().await.elapsed().as_secs() >= GRAB_CACHE_CLEANUP_INTERVAL_SECS;
        if should_cleanup {
            entries.retain(|_, (ts, _)| ts.elapsed().as_secs() < GRAB_CACHE_TTL_SECS);
            *self.last_cleanup.write().await = Instant::now();
        }
        entries.insert(
            (title.to_string(), author.to_string(), indexer_id),
            (Instant::now(), results),
        );
    }
}

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
