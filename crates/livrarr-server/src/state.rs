use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use livrarr_db::sqlite::SqliteDb;
use livrarr_http::HttpClient;
use livrarr_metadata::registry::ProviderRegistry;
use tokio::sync::RwLock;

use crate::auth_service::ServerAuthService;
use crate::config::AppConfig;

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
    pub provider_registry: Arc<RwLock<ProviderRegistry>>,
    pub provider_health: Arc<ProviderHealthState>,
    pub cover_proxy_cache: Arc<crate::handlers::coverproxy::CoverProxyCache>,
    pub detail_url_cache: Arc<DetailUrlCache>,
    pub log_buffer: Arc<LogBuffer>,
    pub log_level_handle: Arc<LogLevelHandle>,
    pub refresh_in_progress: Arc<std::sync::Mutex<HashSet<livrarr_db::UserId>>>,
    /// Limits concurrent imports to avoid blocking poller and exhausting I/O.
    pub import_semaphore: Arc<tokio::sync::Semaphore>,
    /// Per-(user, work) import locks to prevent concurrent imports of the same work.
    pub import_locks: Arc<ImportLockMap>,
}

/// Per-(user, work) mutex map for serializing concurrent imports of the same work.
pub type ImportLockMap =
    dashmap::DashMap<(livrarr_db::UserId, livrarr_db::WorkId), Arc<tokio::sync::Mutex<()>>>;

/// Handle for dynamically reloading the tracing EnvFilter at runtime.
pub struct LogLevelHandle {
    inner: tracing_subscriber::reload::Handle<
        tracing_subscriber::EnvFilter,
        tracing_subscriber::Registry,
    >,
}

impl LogLevelHandle {
    pub fn new(
        handle: tracing_subscriber::reload::Handle<
            tracing_subscriber::EnvFilter,
            tracing_subscriber::Registry,
        >,
    ) -> Self {
        Self { inner: handle }
    }

    pub fn set_level(&self, level: &str) -> Result<(), String> {
        let filter =
            tracing_subscriber::EnvFilter::try_new(format!("livrarr={level},tower_http={level}"))
                .map_err(|e| format!("invalid log level: {e}"))?;
        self.inner
            .reload(filter)
            .map_err(|e| format!("reload failed: {e}"))
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
// Detail URL Cache — maps search results to their detail page URLs server-side.
// Goodreads URLs never reach the frontend; this cache bridges search → add.
// =============================================================================

const DETAIL_URL_TTL: Duration = Duration::from_secs(30 * 60); // 30 minutes
const MAX_DETAIL_URL_ENTRIES: usize = 500;

/// Server-side cache for detail page URLs extracted during search.
/// Keyed by a normalized (title, author) pair so the add flow can look up
/// the detail URL without the frontend ever seeing it.
pub struct DetailUrlCache {
    entries: RwLock<HashMap<String, (String, Instant)>>,
}

impl Default for DetailUrlCache {
    fn default() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }
}

impl DetailUrlCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a cache key from title and author.
    pub fn cache_key(title: &str, author: &str) -> String {
        format!(
            "{}::{}",
            title.trim().to_lowercase(),
            author.trim().to_lowercase()
        )
    }

    /// Store a detail URL for a search result.
    pub async fn put(&self, key: String, url: String) {
        let mut entries = self.entries.write().await;
        // Evict expired entries if at capacity.
        if entries.len() >= MAX_DETAIL_URL_ENTRIES {
            let cutoff = Instant::now() - DETAIL_URL_TTL;
            entries.retain(|_, (_, ts)| *ts > cutoff);
        }
        entries.insert(key, (url, Instant::now()));
    }

    /// Look up a detail URL by cache key. Returns None if missing or expired.
    pub async fn get(&self, key: &str) -> Option<String> {
        let entries = self.entries.read().await;
        entries.get(key).and_then(|(url, ts)| {
            if ts.elapsed() < DETAIL_URL_TTL {
                Some(url.clone())
            } else {
                None
            }
        })
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
