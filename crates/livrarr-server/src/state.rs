use std::collections::{HashMap, HashSet};
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
    pub config: Arc<AppConfig>,
    pub data_dir: Arc<std::path::PathBuf>,
    pub startup_time: chrono::DateTime<chrono::Utc>,
    pub job_runner: Option<crate::jobs::JobRunner>,
    pub provider_registry: Arc<RwLock<ProviderRegistry>>,
    pub provider_health: Arc<ProviderHealthState>,
    pub cover_proxy_cache: Arc<crate::handlers::coverproxy::CoverProxyCache>,
    pub detail_url_cache: Arc<DetailUrlCache>,
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
