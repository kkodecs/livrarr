use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

pub use crate::infra::cache::{
    cleanup_manual_import_scans, GrabSearchCache, ManualImportScanMap, ManualImportScanState,
    GRAB_CACHE_CLEANUP_INTERVAL_SECS, GRAB_CACHE_TTL_SECS, STATE_MAP_TTL,
};
pub use crate::infra::log_buffer::{LogBuffer, LogLevelHandle, MAX_LOG_LINES};
pub use crate::infra::rate_limiter::{
    GoodreadsRateLimiter, OlRateLimiter, GR_BURST, GR_RATE, OL_BURST, OL_RATE,
};

use livrarr_db::sqlite::SqliteDb;
use livrarr_http::HttpClient;
use tokio::sync::RwLock;

use crate::auth_crypto::RealAuthCrypto;
use crate::auth_service::ServerAuthService;
use crate::config::AppConfig;

/// Type alias for the production `DefaultProviderQueue` instance — the queue
/// that scatter-gathers HC / OL / Audnexus / GR for live enrichment dispatch.
pub type LiveProviderQueue = livrarr_metadata::DefaultProviderQueue<SqliteDb>;

/// Type alias for the production LLM validator — single struct that reads
/// credentials from the live metadata config per-call. Behaves as a no-op
/// when LLM is not configured; calls Gemini when llm_enabled + key are set.
pub type LiveLlmValidator = livrarr_metadata::llm_validator::LiveLlmValidator;

/// Type alias for the production `EnrichmentServiceImpl` instance — the IR-defined
/// enrichment service backed by the live `DefaultProviderQueue` + `DefaultMergeEngine`
/// + LLM validator (no-op or Gemini per `MetadataConfig.llm_*`).
pub type LiveEnrichmentService = livrarr_metadata::EnrichmentServiceImpl<
    SqliteDb,
    LiveProviderQueue,
    livrarr_metadata::DefaultMergeEngine,
    LiveLlmValidator,
>;

// =============================================================================
// Service layer type aliases — Phase 4 handler migration
// =============================================================================

pub type LiveEnrichmentWorkflow =
    livrarr_metadata::enrichment_workflow_service::EnrichmentWorkflowImpl<
        LiveEnrichmentService,
        SqliteDb,
    >;

pub type LiveAuthorService = livrarr_metadata::author_service::AuthorServiceImpl<
    SqliteDb,
    livrarr_http::fetcher::HttpFetcherImpl,
    livrarr_metadata::llm_caller_service::LlmCallerImpl,
>;
pub type LiveSeriesService = livrarr_metadata::series_service::SeriesServiceImpl<SqliteDb>;
pub type LiveSeriesQueryService = livrarr_metadata::series_query_service::SeriesQueryServiceImpl<
    SqliteDb,
    livrarr_http::fetcher::HttpFetcherImpl,
    LiveEnrichmentWorkflow,
    livrarr_metadata::llm_caller_service::LlmCallerImpl,
>;
pub type LiveWorkService = livrarr_metadata::work_service::WorkServiceImpl<
    SqliteDb,
    LiveEnrichmentWorkflow,
    livrarr_http::fetcher::HttpFetcherImpl,
    livrarr_metadata::llm_caller_service::LlmCallerImpl,
>;
pub type LiveGrabService = livrarr_download::grab_service::GrabServiceImpl<SqliteDb>;
pub type LiveReleaseService = livrarr_download::release_service::ReleaseServiceImpl<
    SqliteDb,
    livrarr_http::fetcher::HttpFetcherImpl,
>;
pub type LiveFileService = livrarr_library::file_service::FileServiceImpl<SqliteDb>;
pub type LiveImportWorkflow = livrarr_library::import_workflow::ImportWorkflowImpl<SqliteDb>;
pub type LiveListService = livrarr_metadata::list_service::ListServiceImpl<
    SqliteDb,
    LiveWorkService,
    livrarr_http::fetcher::HttpFetcherImpl,
    livrarr_metadata::list_service::NoOpBibliographyTrigger,
>;
pub type LiveAuthorMonitorWorkflow =
    livrarr_metadata::author_monitor_workflow::AuthorMonitorWorkflowImpl<
        SqliteDb,
        LiveWorkService,
        livrarr_http::fetcher::HttpFetcherImpl,
    >;
pub type ReadarrImportServiceImpl =
    crate::readarr_import_service::LiveReadarrImportService<SqliteDb>;
pub type LiveSettingsService = crate::services::settings_service::LiveSettingsService<SqliteDb>;
pub type LiveRssSyncWorkflow = livrarr_metadata::rss_sync_workflow::RssSyncWorkflowImpl<
    SqliteDb,
    livrarr_http::fetcher::HttpFetcherImpl,
    LiveReleaseService,
>;
pub type LiveNotificationService = crate::notification_service::NotificationServiceImpl<SqliteDb>;
pub type LiveHistoryService = crate::history_service::HistoryServiceImpl<SqliteDb>;
pub type LiveQueueService = crate::queue_service::QueueServiceImpl<SqliteDb>;
pub type LiveImportIoService = crate::import_io_service::ImportIoServiceImpl<SqliteDb>;
pub type LiveManualImportDbService =
    crate::manual_import_service::ManualImportServiceImpl<SqliteDb>;

/// Shared application state — injected into all Axum handlers.
///
/// Satisfies: RUNTIME-COMPOSE-001
#[derive(Clone)]
pub struct AppState {
    pub db: SqliteDb,
    pub auth_service: Arc<ServerAuthService<RealAuthCrypto>>,
    pub http_client: HttpClient,
    /// SSRF-safe HTTP client — uses DNS resolver that rejects private IPs.
    /// Use for all user-supplied URLs (grab, fetch_and_extract_hash).
    pub http_client_safe: HttpClient,
    pub config: Arc<AppConfig>,
    pub data_dir: Arc<std::path::PathBuf>,
    pub startup_time: chrono::DateTime<chrono::Utc>,
    pub job_runner: Option<crate::jobs::JobRunner>,
    pub provider_health: Arc<ProviderHealthState>,
    pub cover_proxy_cache: Arc<crate::infra::cover_cache::CoverProxyCache>,
    pub goodreads_rate_limiter: Arc<GoodreadsRateLimiter>,
    /// Shared, mutable snapshot of `MetadataConfig`. The
    /// `update_metadata_config` handlers call `.replace()` after persisting
    /// to the DB so the new credentials are live on the next enrichment
    /// without a restart. All credential-dependent components
    /// (LiveLlmValidator, HardcoverClient, GoodreadsClient LLM fallback)
    /// hold a clone and read fresh per call.
    pub live_metadata_config: livrarr_metadata::live_config::LiveMetadataConfig,
    pub log_buffer: Arc<LogBuffer>,
    pub log_level_handle: Arc<LogLevelHandle>,
    pub refresh_in_progress: Arc<std::sync::Mutex<HashSet<livrarr_db::UserId>>>,
    /// Limits concurrent imports to avoid blocking poller and exhausting I/O.
    pub import_semaphore: Arc<tokio::sync::Semaphore>,
    pub grab_search_cache: Arc<GrabSearchCache>,
    /// Last RSS sync completion timestamp (unix seconds, 0 = never).
    pub rss_last_run: Arc<std::sync::atomic::AtomicI64>,
    /// Guard against concurrent RSS sync runs.
    pub rss_sync_running: Arc<std::sync::atomic::AtomicBool>,
    /// Readarr import progress — polled by frontend.
    pub readarr_import_progress:
        Arc<tokio::sync::Mutex<crate::readarr_import_service::ReadarrImportProgress>>,
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

    // --- Service layer (Phase 4) ---
    pub author_service: Arc<LiveAuthorService>,
    pub series_service: Arc<LiveSeriesService>,
    pub series_query_service: Arc<LiveSeriesQueryService>,
    pub work_service: Arc<LiveWorkService>,
    pub grab_service: Arc<LiveGrabService>,
    pub release_service: Arc<LiveReleaseService>,
    pub file_service: Arc<LiveFileService>,
    pub import_workflow: Arc<LiveImportWorkflow>,
    pub list_service: Arc<LiveListService>,
    pub rss_sync_workflow: Arc<LiveRssSyncWorkflow>,
    pub author_monitor_workflow: Arc<LiveAuthorMonitorWorkflow>,
    pub enrichment_workflow: Arc<LiveEnrichmentWorkflow>,
    pub readarr_import_service: Arc<ReadarrImportServiceImpl>,
    pub settings_service: Arc<LiveSettingsService>,
    pub notification_service: Arc<LiveNotificationService>,
    pub history_service: Arc<LiveHistoryService>,
    pub queue_service: Arc<LiveQueueService>,
    pub import_io_service: Arc<LiveImportIoService>,
    pub manual_import_db_service: Arc<LiveManualImportDbService>,

    // --- Phase 5: infrastructure accessors ---
    pub rss_sync_state: RssSyncState,
    pub system_state: SystemState,
    pub provider_health_accessor: ProviderHealthAccessorImpl,
    pub live_metadata_config_accessor: LiveMetadataConfigAccessorImpl,
    pub cover_proxy_cache_accessor: CoverProxyCacheAccessorImpl,
    pub tag_service: Arc<crate::tag_service::LiveTagService<LiveImportIoService>>,
    pub email_svc: Arc<crate::email_service::LiveEmailService<livrarr_db::sqlite::SqliteDb>>,
    pub import_svc: Arc<crate::import_service::LiveImportService>,
    pub matching_svc: crate::matching_service::LiveMatchingService,
    pub manual_import_scan_svc: crate::manual_import_scan_service::LiveManualImportScanService,
    pub readarr_import_wf: Arc<crate::readarr_import_workflow::LiveReadarrImportWorkflow>,
    pub enrichment_notify: Arc<tokio::sync::Notify>,
}

// =============================================================================
// Accessor trait impls for AppContext infrastructure
// =============================================================================

/// Wrapper for provider health status — satisfies orphan rule.
#[derive(Clone)]
pub struct ProviderHealthAccessorImpl(pub Arc<ProviderHealthState>);

impl livrarr_handlers::accessors::ProviderHealthAccessor for ProviderHealthAccessorImpl {
    async fn statuses(&self) -> HashMap<String, String> {
        self.0.statuses().await
    }
}

/// Wrapper for live metadata config — satisfies orphan rule.
#[derive(Clone)]
pub struct LiveMetadataConfigAccessorImpl(pub livrarr_metadata::live_config::LiveMetadataConfig);

impl livrarr_handlers::accessors::LiveMetadataConfigAccessor for LiveMetadataConfigAccessorImpl {
    fn replace(&self, cfg: livrarr_domain::settings::MetadataConfig) {
        self.0.replace(cfg);
    }
}

/// Wrapper around the two RSS sync atomics.
#[derive(Clone)]
pub struct RssSyncState {
    pub running: Arc<std::sync::atomic::AtomicBool>,
    pub last_run: Arc<std::sync::atomic::AtomicI64>,
}

impl livrarr_handlers::accessors::RssSyncAccessor for RssSyncState {
    fn try_acquire(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }
    fn release(&self) {
        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
    fn set_last_run(&self, ts: i64) {
        self.last_run
            .store(ts, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Wrapper combining LogBuffer + LogLevelHandle for the SystemAccessor trait.
#[derive(Clone)]
pub struct SystemState {
    pub log_buffer: Arc<LogBuffer>,
    pub log_level_handle: Arc<LogLevelHandle>,
}

impl livrarr_handlers::accessors::SystemAccessor for SystemState {
    fn log_tail(&self, n: usize) -> Vec<String> {
        self.log_buffer.tail(n)
    }
    fn current_log_level(&self) -> String {
        self.log_level_handle.current_level()
    }
    fn set_log_level(&self, level: &str) -> Result<(), String> {
        self.log_level_handle.set_level(level)
    }
}

/// Wrapper for cover proxy cache — satisfies orphan rule.
#[derive(Clone)]
pub struct CoverProxyCacheAccessorImpl(pub Arc<crate::infra::cover_cache::CoverProxyCache>);

impl livrarr_handlers::accessors::CoverProxyCacheAccessor for CoverProxyCacheAccessorImpl {
    async fn get(&self, url: &str) -> Option<(Vec<u8>, String)> {
        self.0.get(url).await
    }
    async fn put(&self, url: String, data: Vec<u8>, content_type: String) {
        self.0.put(url, data, content_type).await
    }
}

// =============================================================================
// AppContext impl
// =============================================================================

impl livrarr_handlers::context::AppContext for AppState {
    // --- Domain services ---
    type WorkSvc = LiveWorkService;
    type FileSvc = LiveFileService;
    type AuthorSvc = LiveAuthorService;
    type SeriesSvc = LiveSeriesService;
    type SeriesQuerySvc = LiveSeriesQueryService;
    type GrabSvc = LiveGrabService;
    type ReleaseSvc = LiveReleaseService;
    type ListSvc = LiveListService;
    type SettingsSvc = LiveSettingsService;
    type NotificationSvc = LiveNotificationService;
    type QueueSvc = LiveQueueService;
    type ImportIoSvc = LiveImportIoService;
    type ManualImportSvc = LiveManualImportDbService;
    type HistorySvc = LiveHistoryService;
    type AuthSvc = ServerAuthService<RealAuthCrypto>;
    type ImportWf = LiveImportWorkflow;
    type EnrichmentWf = LiveEnrichmentWorkflow;
    type RssSyncWf = LiveRssSyncWorkflow;
    type TagSvc = crate::tag_service::LiveTagService<LiveImportIoService>;
    type EmailSvc = crate::email_service::LiveEmailService<livrarr_db::sqlite::SqliteDb>;
    type AuthorMonitorWf = LiveAuthorMonitorWorkflow;
    type ImportSvc = crate::import_service::LiveImportService;
    type MatchingSvc = crate::matching_service::LiveMatchingService;
    type ManualImportScan = crate::manual_import_scan_service::LiveManualImportScanService;
    type ReadarrImportWf = crate::readarr_import_workflow::LiveReadarrImportWorkflow;

    // --- Infrastructure ---
    type ProviderHealth = ProviderHealthAccessorImpl;
    type LiveConfig = LiveMetadataConfigAccessorImpl;
    type RssSync = RssSyncState;
    type System = SystemState;
    type CoverCache = CoverProxyCacheAccessorImpl;

    fn work_service(&self) -> &Self::WorkSvc {
        &self.work_service
    }
    fn file_service(&self) -> &Self::FileSvc {
        &self.file_service
    }
    fn author_service(&self) -> &Self::AuthorSvc {
        &self.author_service
    }
    fn series_service(&self) -> &Self::SeriesSvc {
        &self.series_service
    }
    fn series_query_service(&self) -> &Self::SeriesQuerySvc {
        &self.series_query_service
    }
    fn grab_service(&self) -> &Self::GrabSvc {
        &self.grab_service
    }
    fn release_service(&self) -> &Self::ReleaseSvc {
        &self.release_service
    }
    fn list_service(&self) -> &Self::ListSvc {
        &self.list_service
    }
    fn settings_service(&self) -> &Self::SettingsSvc {
        &self.settings_service
    }
    fn notification_service(&self) -> &Self::NotificationSvc {
        &self.notification_service
    }
    fn queue_service(&self) -> &Self::QueueSvc {
        &self.queue_service
    }
    fn import_io_service(&self) -> &Self::ImportIoSvc {
        &self.import_io_service
    }
    fn manual_import_service(&self) -> &Self::ManualImportSvc {
        &self.manual_import_db_service
    }
    fn history_service(&self) -> &Self::HistorySvc {
        &self.history_service
    }
    fn auth_service(&self) -> &Self::AuthSvc {
        &self.auth_service
    }
    fn import_workflow(&self) -> &Self::ImportWf {
        &self.import_workflow
    }
    fn enrichment_workflow(&self) -> &Self::EnrichmentWf {
        &self.enrichment_workflow
    }
    fn rss_sync_workflow(&self) -> &Self::RssSyncWf {
        &self.rss_sync_workflow
    }
    fn tag_service(&self) -> &Self::TagSvc {
        &self.tag_service
    }
    fn email_service(&self) -> &Self::EmailSvc {
        &self.email_svc
    }
    fn author_monitor_workflow(&self) -> &Self::AuthorMonitorWf {
        &self.author_monitor_workflow
    }
    fn import_service(&self) -> &Self::ImportSvc {
        &self.import_svc
    }
    fn matching_service(&self) -> &Self::MatchingSvc {
        &self.matching_svc
    }
    fn manual_import_scan(&self) -> &Self::ManualImportScan {
        &self.manual_import_scan_svc
    }
    fn readarr_import_workflow(&self) -> &Self::ReadarrImportWf {
        &self.readarr_import_wf
    }
    fn http_client(&self) -> &livrarr_http::HttpClient {
        &self.http_client
    }
    fn http_client_safe(&self) -> &livrarr_http::HttpClient {
        &self.http_client_safe
    }
    fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }
    fn startup_time(&self) -> chrono::DateTime<chrono::Utc> {
        self.startup_time
    }
    fn provider_health(&self) -> &Self::ProviderHealth {
        &self.provider_health_accessor
    }
    fn live_metadata_config(&self) -> &Self::LiveConfig {
        &self.live_metadata_config_accessor
    }
    fn rss_sync(&self) -> &Self::RssSync {
        &self.rss_sync_state
    }
    fn system(&self) -> &Self::System {
        &self.system_state
    }
    fn cover_proxy_cache(&self) -> &Self::CoverCache {
        &self.cover_proxy_cache_accessor
    }
    fn enrichment_notify(&self) -> &tokio::sync::Notify {
        &self.enrichment_notify
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
