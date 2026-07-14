use std::collections::HashMap;
use std::time::Instant;

use livrarr_domain::services::ReleaseResult;
use tokio::sync::RwLock;

// =============================================================================
// Release Search Cache — avoids hammering indexers for repeated searches
// =============================================================================
//
// Ported from the deleted `livrarr-server::infra::cache::GrabSearchCache`:
// same key shape, same 24h TTL, same lazy 5-minute eviction — moved into the
// seam that owns search policy (livrarr-download's release service) and
// holding domain results (`ReleaseResult`) instead of a handler DTO.

pub(crate) const RELEASE_CACHE_TTL_SECS: u64 = 86400; // 24 hours
pub(crate) const RELEASE_CACHE_CLEANUP_INTERVAL_SECS: u64 = 300; // 5 minutes

type ReleaseCacheMap = HashMap<(String, String, i64), (Instant, Vec<ReleaseResult>)>;

/// In-memory cache for indexer search results, keyed by (title, author, indexer_id).
pub(crate) struct ReleaseSearchCache {
    entries: RwLock<ReleaseCacheMap>,
    last_cleanup: RwLock<Instant>,
}

impl Default for ReleaseSearchCache {
    fn default() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            last_cleanup: RwLock::new(Instant::now()),
        }
    }
}

impl ReleaseSearchCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Look up cached results. Returns `None` if missing or expired.
    /// On hit, returns (results, age_in_seconds).
    pub(crate) async fn get(
        &self,
        title: &str,
        author: &str,
        indexer_id: i64,
    ) -> Option<(Vec<ReleaseResult>, u64)> {
        let entries = self.entries.read().await;
        let key = (title.to_string(), author.to_string(), indexer_id);
        let (ts, results) = entries.get(&key)?;
        let age = ts.elapsed().as_secs();
        if age < RELEASE_CACHE_TTL_SECS {
            Some((results.clone(), age))
        } else {
            None
        }
    }

    /// Store results for a (title, author, indexer_id) tuple.
    /// Periodically evicts expired entries (at most once per 5 minutes).
    pub(crate) async fn put(
        &self,
        title: &str,
        author: &str,
        indexer_id: i64,
        results: Vec<ReleaseResult>,
    ) {
        let mut entries = self.entries.write().await;
        let should_cleanup = self.last_cleanup.read().await.elapsed().as_secs()
            >= RELEASE_CACHE_CLEANUP_INTERVAL_SECS;
        if should_cleanup {
            entries.retain(|_, (ts, _)| ts.elapsed().as_secs() < RELEASE_CACHE_TTL_SECS);
            *self.last_cleanup.write().await = Instant::now();
        }
        entries.insert(
            (title.to_string(), author.to_string(), indexer_id),
            (Instant::now(), results),
        );
    }
}
