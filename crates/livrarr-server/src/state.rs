use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use livrarr_db::sqlite::SqliteDb;
use livrarr_http::HttpClient;
use tokio::sync::RwLock;

use crate::auth_service::ServerAuthService;
use crate::config::AppConfig;

/// Type alias for the production `DefaultProviderQueue` instance — the queue
/// that scatter-gathers HC / OL / Audnexus / GR for live enrichment dispatch.
pub type LiveProviderQueue = livrarr_metadata::DefaultProviderQueue<SqliteDb>;

/// Type alias for the production `EnrichmentServiceImpl` instance — the IR-defined
/// enrichment service backed by the live `DefaultProviderQueue` + `DefaultMergeEngine`.
pub type LiveEnrichmentService = livrarr_metadata::EnrichmentServiceImpl<
    SqliteDb,
    LiveProviderQueue,
    livrarr_metadata::DefaultMergeEngine,
>;

/// Shared application state — injected into all Axum handlers.
///
/// Satisfies: RUNTIME-COMPOSE-001
#[derive(Clone)]
pub struct AppState {
    pub db: SqliteDb,
    pub auth_service: Arc<ServerAuthService>,
    pub http_client: HttpClient,
    /// SSRF-safe HTTP client — uses DNS resolver that rejects private IPs.
    /// Use for all user-supplied URLs (grab, fetch_and_extract_hash).
    pub http_client_safe: HttpClient,
    pub config: Arc<AppConfig>,
    pub data_dir: Arc<std::path::PathBuf>,
    pub startup_time: chrono::DateTime<chrono::Utc>,
    pub job_runner: Option<crate::jobs::JobRunner>,
    pub provider_health: Arc<ProviderHealthState>,
    pub cover_proxy_cache: Arc<crate::handlers::coverproxy::CoverProxyCache>,
    pub goodreads_rate_limiter: Arc<GoodreadsRateLimiter>,
    pub log_buffer: Arc<LogBuffer>,
    pub log_level_handle: Arc<LogLevelHandle>,
    pub refresh_in_progress: Arc<std::sync::Mutex<HashSet<livrarr_db::UserId>>>,
    /// Limits concurrent imports to avoid blocking poller and exhausting I/O.
    pub import_semaphore: Arc<tokio::sync::Semaphore>,
    /// Per-(user, work) import locks to prevent concurrent imports of the same work.
    pub import_locks: Arc<ImportLockMap>,
    pub grab_search_cache: Arc<GrabSearchCache>,
    /// Last RSS sync completion timestamp (unix seconds, 0 = never).
    pub rss_last_run: Arc<std::sync::atomic::AtomicI64>,
    /// Guard against concurrent RSS sync runs.
    pub rss_sync_running: Arc<std::sync::atomic::AtomicBool>,
    /// Readarr import progress — polled by frontend.
    pub readarr_import_progress:
        Arc<tokio::sync::Mutex<crate::handlers::readarr_import::ImportProgress>>,
    /// OL rate limiter for manual import parallel lookups (3 req/sec, burst 10).
    pub ol_rate_limiter: Arc<OlRateLimiter>,
    /// In-progress manual import scan results — OL matches stream in via polling.
    pub manual_import_scans: Arc<ManualImportScanMap>,
    /// Phase 1.5 plumbing: live `DefaultProviderQueue` constructed at startup
    /// from the persisted `MetadataConfig` snapshot. Wired into `AppState` so
    /// call sites can begin migrating off the legacy `enrich_work` /
    /// `enrich_foreign_work` standalone functions one at a time. Not yet on
    /// the live enrichment path.
    pub provider_queue: Arc<LiveProviderQueue>,
    /// Phase 1.5 plumbing: live `EnrichmentServiceImpl` wrapping
    /// `provider_queue` + `DefaultMergeEngine`. Same status as `provider_queue`
    /// — wired but not yet driving live enrichment.
    pub enrichment_service: Arc<LiveEnrichmentService>,
}

// =============================================================================
// OpenLibrary Rate Limiter — 3 req/sec, burst of 10
// =============================================================================

pub struct OlRateLimiter {
    state: tokio::sync::Mutex<RateLimiterInner>,
}

const OL_RATE: f64 = 3.0; // 3 tokens per second
const OL_BURST: f64 = 10.0;

impl Default for OlRateLimiter {
    fn default() -> Self {
        Self {
            state: tokio::sync::Mutex::new(RateLimiterInner {
                tokens: OL_BURST,
                last_refill: std::time::Instant::now(),
            }),
        }
    }
}

impl OlRateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn acquire(&self) {
        loop {
            let mut inner = self.state.lock().await;
            let now = std::time::Instant::now();
            let elapsed = now.duration_since(inner.last_refill).as_secs_f64();
            inner.tokens = (inner.tokens + elapsed * OL_RATE).min(OL_BURST);
            inner.last_refill = now;

            if inner.tokens >= 1.0 {
                inner.tokens -= 1.0;
                return;
            }

            let wait = (1.0 - inner.tokens) / OL_RATE;
            drop(inner);
            tokio::time::sleep(Duration::from_secs_f64(wait)).await;
        }
    }
}

// =============================================================================
// Manual Import Scan State — progressive OL lookup results
// =============================================================================

pub type ManualImportScanMap = dashmap::DashMap<String, crate::handlers::manual_import::ScanState>;

/// Per-(user, work) mutex map for serializing concurrent imports of the same work.
/// Value is `(mutex, insertion_time)` — the insertion time is used by TTL cleanup.
pub type ImportLockMap = dashmap::DashMap<
    (livrarr_db::UserId, livrarr_db::WorkId),
    (Arc<tokio::sync::Mutex<()>>, std::time::Instant),
>;

const STATE_MAP_TTL: Duration = Duration::from_secs(30 * 60); // 30 minutes

/// Remove entries from `import_locks` that were inserted more than 30 minutes ago.
/// Safe to call from any context — entries still held by an active guard are still
/// referenced via Arc, so the mutex itself is not dropped until the guard releases.
pub fn cleanup_import_locks(map: &ImportLockMap) {
    let cutoff = std::time::Instant::now()
        .checked_sub(STATE_MAP_TTL)
        .unwrap_or(std::time::Instant::now());
    map.retain(|_, (_, ts)| *ts > cutoff);
}

/// Remove entries from `manual_import_scans` that were created more than 30 minutes ago.
pub fn cleanup_manual_import_scans(map: &ManualImportScanMap) {
    let cutoff = std::time::Instant::now()
        .checked_sub(STATE_MAP_TTL)
        .unwrap_or(std::time::Instant::now());
    map.retain(|_, scan| scan.created_at > cutoff);
}

/// Handle for dynamically reloading the tracing EnvFilter at runtime.
pub struct LogLevelHandle {
    inner: tracing_subscriber::reload::Handle<
        tracing_subscriber::EnvFilter,
        tracing_subscriber::Registry,
    >,
    current_level: std::sync::Mutex<String>,
}

impl LogLevelHandle {
    pub fn new(
        handle: tracing_subscriber::reload::Handle<
            tracing_subscriber::EnvFilter,
            tracing_subscriber::Registry,
        >,
        initial_level: &str,
    ) -> Self {
        Self {
            inner: handle,
            current_level: std::sync::Mutex::new(initial_level.to_string()),
        }
    }

    pub fn set_level(&self, level: &str) -> Result<(), String> {
        let filter =
            tracing_subscriber::EnvFilter::try_new(format!("livrarr={level},tower_http={level}"))
                .map_err(|e| format!("invalid log level: {e}"))?;
        self.inner
            .reload(filter)
            .map_err(|e| format!("reload failed: {e}"))?;
        *self.current_level.lock().unwrap() = level.to_string();
        Ok(())
    }

    pub fn current_level(&self) -> String {
        self.current_level.lock().unwrap().clone()
    }
}

/// In-memory provider error tracking with 1-hour TTL.
/// "Not Responding" status for providers that had HTTP/network failures.
pub struct ProviderHealthState {
    errors: RwLock<HashMap<String, (String, Instant)>>,
}

const ERROR_TTL_SECS: u64 = 3600; // 1 hour

impl Default for ProviderHealthState {
    fn default() -> Self {
        Self {
            errors: RwLock::new(HashMap::new()),
        }
    }
}

impl ProviderHealthState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a provider failure.
    pub async fn set_error(&self, provider: &str, message: String) {
        self.errors
            .write()
            .await
            .insert(provider.to_string(), (message, Instant::now()));
    }

    /// Clear error for a provider (on successful query).
    pub async fn clear_error(&self, provider: &str) {
        self.errors.write().await.remove(provider);
    }

    /// Purge errors for providers not in the given set (on registry rebuild).
    pub async fn purge_stale(&self, active_providers: &HashSet<String>) {
        self.errors
            .write()
            .await
            .retain(|k, _| active_providers.contains(k));
    }

    /// Get current error statuses, excluding expired (>1 hour) entries.
    pub async fn statuses(&self) -> HashMap<String, String> {
        let mut errors = self.errors.write().await;
        let cutoff = Instant::now() - std::time::Duration::from_secs(ERROR_TTL_SECS);
        errors.retain(|_, (_, ts)| *ts > cutoff);
        errors
            .iter()
            .map(|(k, (msg, _))| (k.clone(), msg.clone()))
            .collect()
    }
}

// =============================================================================
// Goodreads Rate Limiter — async-safe token bucket for outbound requests
// =============================================================================

/// Outbound rate limiter for Goodreads requests.
/// Token bucket: 1 token/second, burst of 5.
pub struct GoodreadsRateLimiter {
    state: tokio::sync::Mutex<RateLimiterInner>,
}

struct RateLimiterInner {
    tokens: f64,
    last_refill: std::time::Instant,
}

const GR_RATE: f64 = 1.0; // 1 token per second
const GR_BURST: f64 = 5.0; // max burst of 5

impl Default for GoodreadsRateLimiter {
    fn default() -> Self {
        Self {
            state: tokio::sync::Mutex::new(RateLimiterInner {
                tokens: GR_BURST,
                last_refill: std::time::Instant::now(),
            }),
        }
    }
}

impl GoodreadsRateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire a token, waiting if necessary. Never blocks the tokio runtime.
    pub async fn acquire(&self) {
        loop {
            let mut inner = self.state.lock().await;
            let now = std::time::Instant::now();
            let elapsed = now.duration_since(inner.last_refill).as_secs_f64();
            inner.tokens = (inner.tokens + elapsed * GR_RATE).min(GR_BURST);
            inner.last_refill = now;

            if inner.tokens >= 1.0 {
                inner.tokens -= 1.0;
                return;
            }

            let wait = (1.0 - inner.tokens) / GR_RATE;
            drop(inner);
            tracing::debug!(wait_secs = %format!("{wait:.2}"), "Goodreads rate limiter: waiting");
            tokio::time::sleep(Duration::from_secs_f64(wait)).await;
        }
    }
}

// =============================================================================
// Log Buffer — in-memory ring buffer for recent log lines
// =============================================================================

const MAX_LOG_LINES: usize = 200;

/// Thread-safe ring buffer that stores recent log lines for the help page.
pub struct LogBuffer {
    lines: std::sync::Mutex<VecDeque<String>>,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self {
            lines: std::sync::Mutex::new(VecDeque::with_capacity(MAX_LOG_LINES)),
        }
    }
}

impl LogBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a log line. Drops oldest if at capacity.
    pub fn push(&self, line: String) {
        let mut buf = self.lines.lock().unwrap();
        if buf.len() >= MAX_LOG_LINES {
            buf.pop_front();
        }
        buf.push_back(line);
    }

    /// Get the last `n` log lines.
    pub fn tail(&self, n: usize) -> Vec<String> {
        let buf = self.lines.lock().unwrap();
        buf.iter().rev().take(n).rev().cloned().collect()
    }
}

// =============================================================================
// Grab Search Cache — avoids hammering indexers for repeated searches
// =============================================================================

const GRAB_CACHE_TTL_SECS: u64 = 86400; // 24 hours

type GrabCacheMap = HashMap<(String, String, i64), (Instant, Vec<crate::ReleaseResponse>)>;

/// Evict expired entries at most once per this interval.
const GRAB_CACHE_CLEANUP_INTERVAL_SECS: u64 = 300; // 5 minutes

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
        // Throttled cleanup: only scan the full map every 5 minutes.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rate_limiter_allows_burst() {
        let limiter = GoodreadsRateLimiter::new();

        let start = std::time::Instant::now();
        for _ in 0..5 {
            limiter.acquire().await;
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 100,
            "Burst of 5 took {}ms, expected <100ms",
            elapsed.as_millis()
        );
    }

    #[tokio::test]
    async fn rate_limiter_throttles_after_burst() {
        let limiter = GoodreadsRateLimiter::new();

        for _ in 0..5 {
            limiter.acquire().await;
        }

        let start = std::time::Instant::now();
        limiter.acquire().await;
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() >= 800,
            "6th acquire took only {}ms, expected >=800ms",
            elapsed.as_millis()
        );
    }
}
