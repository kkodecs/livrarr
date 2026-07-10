use std::sync::Arc;

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

use crate::auth_crypto::RealAuthCrypto;
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
    livrarr_external_data::llm_caller_service::LlmCallerImpl,
>;
pub type LiveSeriesService = livrarr_metadata::series_service::SeriesServiceImpl<SqliteDb>;
pub type LiveSeriesQueryService = livrarr_metadata::series_query_service::SeriesQueryServiceImpl<
    SqliteDb,
    livrarr_http::fetcher::HttpFetcherImpl,
    LiveWorkService,
    livrarr_external_data::llm_caller_service::LlmCallerImpl,
>;
pub type LiveTagServiceImpl = crate::tag_service::LiveTagService<LiveImportIoService>;
pub type LiveIdentityResolver =
    livrarr_metadata::english_identity_resolver::LiveEnglishIdentityResolver;

pub type LiveWorkService = livrarr_metadata::work_service::WorkServiceImpl<
    SqliteDb,
    LiveEnrichmentWorkflow,
    livrarr_http::fetcher::HttpFetcherImpl,
    livrarr_external_data::llm_caller_service::LlmCallerImpl,
>;
pub type LiveGrabService = livrarr_download::grab_service::GrabServiceImpl<SqliteDb>;
pub type LiveReleaseService = livrarr_download::release_service::ReleaseServiceImpl<
    SqliteDb,
    livrarr_http::fetcher::HttpFetcherImpl,
>;
pub type LiveFileService = livrarr_library::file_service::FileServiceImpl<SqliteDb>;
pub type LiveChapterService = livrarr_library::chapter_service::ChapterServiceImpl<SqliteDb>;
pub type LiveBookmarkService = livrarr_library::bookmark_service::BookmarkServiceImpl<SqliteDb>;
pub type LiveCrossFormatService =
    livrarr_library::cross_format_service::CrossFormatServiceImpl<SqliteDb, LiveFileService>;
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
pub type LiveCoverService = crate::cover_service::LiveCoverService;
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
    /// Shared `HttpFetcher` implementation — routes admin-triggered outbound
    /// requests through the process-global rate-limit queue.
    pub http_fetcher: livrarr_http::fetcher::HttpFetcherImpl,
    pub config: Arc<AppConfig>,
    pub data_dir: Arc<std::path::PathBuf>,
    pub startup_time: chrono::DateTime<chrono::Utc>,
    pub job_runner: Option<crate::jobs::JobRunner>,
    pub cover_proxy_cache: Arc<crate::infra::cover_cache::CoverProxyCache>,
    /// Shared, mutable snapshot of `MetadataConfig`. The
    /// `update_metadata_config` handlers call `.replace()` after persisting
    /// to the DB so the new credentials are live on the next enrichment
    /// without a restart. All credential-dependent components
    /// (HardcoverClient, GoodreadsClient LLM fallback)
    /// hold a clone and read fresh per call.
    pub live_metadata_config: livrarr_external_data::live_config::LiveMetadataConfig,
    pub log_buffer: Arc<LogBuffer>,
    pub log_level_handle: Arc<LogLevelHandle>,
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
    /// In-progress manual import scan results — OL matches stream in via polling.
    pub manual_import_scans: Arc<ManualImportScanMap>,
    /// Live `DefaultProviderQueue` constructed at startup from the persisted
    /// `MetadataConfig` snapshot — the provider dispatch layer behind the live
    /// enrichment path (work service / unified enrichment).
    pub provider_queue: Arc<LiveProviderQueue>,
    /// Live `EnrichmentServiceImpl` wrapping `provider_queue` + the merge
    /// engine — drives live enrichment through the work service.
    pub enrichment_service: Arc<LiveEnrichmentService>,

    // --- Service layer (Phase 4) ---
    pub author_service: Arc<LiveAuthorService>,
    pub series_service: Arc<LiveSeriesService>,
    pub series_query_service: Arc<LiveSeriesQueryService>,
    pub work_service: Arc<LiveWorkService>,
    pub grab_service: Arc<LiveGrabService>,
    pub release_service: Arc<LiveReleaseService>,
    pub file_service: Arc<LiveFileService>,
    pub chapter_service: Arc<LiveChapterService>,
    pub bookmark_service: Arc<LiveBookmarkService>,
    pub cross_format_service: Arc<LiveCrossFormatService>,
    pub import_workflow: Arc<LiveImportWorkflow>,
    pub list_service: Arc<LiveListService>,
    pub identity_conflict_service:
        Arc<crate::services::identity_conflict_service::LiveIdentityConflictService>,
    pub identity_resolver: Arc<LiveIdentityResolver>,
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
    /// Record-fed provider panel stats (REQ-002).
    pub provider_stats_service: Arc<LiveProviderStatsService>,
    /// Truthful log surface for the status page (REQ-003).
    pub log_surface_accessor: LogSurfaceAccessorImpl,
    pub live_metadata_config_accessor: LiveMetadataConfigAccessorImpl,
    pub cover_proxy_cache_accessor: CoverProxyCacheAccessorImpl,
    pub tag_service: Arc<crate::tag_service::LiveTagService<LiveImportIoService>>,
    pub email_svc: Arc<crate::email_service::LiveEmailService<livrarr_db::sqlite::SqliteDb>>,
    pub import_svc: Arc<crate::import_service::LiveImportService>,
    pub matching_svc: crate::matching_service::LiveMatchingService,
    pub manual_import_scan_svc: crate::manual_import_scan_service::LiveManualImportScanService,
    pub readarr_import_wf: Arc<crate::readarr_import_workflow::LiveReadarrImportWorkflow>,
    pub cover_service: Arc<LiveCoverService>,
    pub preadd_cover_service: Arc<livrarr_metadata::preadd_cover_service::LivePreaddCoverService>,
    pub hmac_key: Vec<u8>,
    pub trusted_origins_rebuilder: TrustedOriginsRebuilderImpl,
}

impl livrarr_handlers::context::HasWorkIdentityRepository for AppState {
    type WorkIdentityRepo = SqliteDb;
    fn work_identity_repo(&self) -> &Self::WorkIdentityRepo {
        &self.db
    }
}

// =============================================================================
// Accessor trait impls for AppContext infrastructure
// =============================================================================

/// Record-fed provider stats service (REQ-002): thin delegation to the db's
/// rolling-24h aggregate query.
pub struct LiveProviderStatsService {
    db: SqliteDb,
}

impl LiveProviderStatsService {
    pub fn new(db: SqliteDb) -> Self {
        Self { db }
    }
}

impl livrarr_domain::services::ProviderStatsService for LiveProviderStatsService {
    async fn provider_stats_24h(
        &self,
    ) -> Result<Vec<livrarr_domain::services::ProviderStats>, livrarr_domain::services::ServiceError>
    {
        use livrarr_db::ProviderCallRecordDb;
        self.db
            .query_provider_stats_24h()
            .await
            .map_err(livrarr_domain::services::ServiceError::from)
    }
}

/// Truthful log surface (REQ-003) — satisfies orphan rule. The dated path is
/// computed at read time so the answer stays correct across midnight
/// rollover; the init error is fixed at startup.
#[derive(Clone)]
pub struct LogSurfaceAccessorImpl {
    pub log_dir: std::path::PathBuf,
    pub init_error: Option<String>,
}

impl livrarr_handlers::accessors::LogSurfaceAccessor for LogSurfaceAccessorImpl {
    fn status(&self) -> livrarr_domain::LogSurfaceStatus {
        livrarr_domain::LogSurfaceStatus {
            active_path: crate::log_surface::active_log_path(&self.log_dir),
            init_error: self.init_error.clone(),
        }
    }
}

/// Wrapper for live metadata config — satisfies orphan rule.
#[derive(Clone)]
pub struct LiveMetadataConfigAccessorImpl(
    pub livrarr_external_data::live_config::LiveMetadataConfig,
);

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
    fn is_running(&self) -> bool {
        self.running.load(std::sync::atomic::Ordering::Relaxed)
    }
    fn last_run_at(&self) -> i64 {
        self.last_run.load(std::sync::atomic::Ordering::Relaxed)
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

/// Wrapper for trusted origins — satisfies orphan rule.
#[derive(Clone)]
pub struct TrustedOriginsRebuilderImpl(pub Arc<livrarr_http::ssrf::TrustedOrigins>);

impl livrarr_handlers::accessors::TrustedOriginsRebuilder for TrustedOriginsRebuilderImpl {
    fn rebuild(&self, urls: &[String]) {
        self.0.rebuild(urls);
    }
}

// =============================================================================
// AppContext impl — one Has* trait per capability
// =============================================================================

use livrarr_handlers::context::{
    HasAppConfigService, HasAuthService, HasAuthorMonitorWorkflow, HasAuthorService,
    HasBookmarkService, HasChapterService, HasCoverCache, HasCoverService, HasDataDir,
    HasDownloadClientCredentialService, HasDownloadClientSettingsService, HasEmailService,
    HasEnrichmentWorkflow, HasFileService, HasGrabService, HasHistoryService, HasHmacKey,
    HasHttpClient, HasHttpFetcher, HasIdentityConflictService, HasIdentityResolver,
    HasImportIoService, HasImportService, HasImportWorkflow, HasIndexerCredentialService,
    HasIndexerSettingsService, HasListService, HasLiveConfig, HasLogSurface, HasManualImportScan,
    HasManualImportService, HasMatchingService, HasNotificationService, HasPreaddCoverService,
    HasProviderStats, HasQueueService, HasReadarrImportWorkflow, HasReleaseService,
    HasRemotePathMappingService, HasRootFolderService, HasRssSync, HasRssSyncWorkflow,
    HasSeriesQueryService, HasSeriesService, HasStartupTime, HasSystem, HasTagService,
    HasTrustedOrigins, HasWorkService,
};

impl HasWorkService for AppState {
    type WorkSvc = LiveWorkService;
    fn work_service(&self) -> &Self::WorkSvc {
        &self.work_service
    }
}

impl HasFileService for AppState {
    type FileSvc = LiveFileService;
    fn file_service(&self) -> &Self::FileSvc {
        &self.file_service
    }
}

impl HasChapterService for AppState {
    type ChapterSvc = LiveChapterService;
    fn chapter_service(&self) -> &Self::ChapterSvc {
        &self.chapter_service
    }
}

impl HasBookmarkService for AppState {
    type BookmarkSvc = LiveBookmarkService;
    fn bookmark_service(&self) -> &Self::BookmarkSvc {
        &self.bookmark_service
    }
}

impl livrarr_handlers::context::HasCrossFormatService for AppState {
    type CrossFormatSvc = LiveCrossFormatService;
    fn cross_format_service(&self) -> &Self::CrossFormatSvc {
        &self.cross_format_service
    }
}

impl HasAuthorService for AppState {
    type AuthorSvc = LiveAuthorService;
    fn author_service(&self) -> &Self::AuthorSvc {
        &self.author_service
    }
}

impl HasSeriesService for AppState {
    type SeriesSvc = LiveSeriesService;
    fn series_service(&self) -> &Self::SeriesSvc {
        &self.series_service
    }
}

impl HasSeriesQueryService for AppState {
    type SeriesQuerySvc = LiveSeriesQueryService;
    fn series_query_service(&self) -> &Self::SeriesQuerySvc {
        &self.series_query_service
    }
}

impl HasGrabService for AppState {
    type GrabSvc = LiveGrabService;
    fn grab_service(&self) -> &Self::GrabSvc {
        &self.grab_service
    }
}

impl HasReleaseService for AppState {
    type ReleaseSvc = LiveReleaseService;
    fn release_service(&self) -> &Self::ReleaseSvc {
        &self.release_service
    }
}

impl HasListService for AppState {
    type ListSvc = LiveListService;
    fn list_service(&self) -> &Self::ListSvc {
        &self.list_service
    }
}

impl HasIdentityConflictService for AppState {
    type IdentityConflictSvc =
        crate::services::identity_conflict_service::LiveIdentityConflictService;
    fn identity_conflict_service(&self) -> &Self::IdentityConflictSvc {
        &self.identity_conflict_service
    }
}

impl HasIdentityResolver for AppState {
    type IdentityResolverSvc = LiveIdentityResolver;
    fn identity_resolver(&self) -> &Self::IdentityResolverSvc {
        &self.identity_resolver
    }
}

impl HasAppConfigService for AppState {
    type AppConfigSvc = LiveSettingsService;
    fn app_config_service(&self) -> &Self::AppConfigSvc {
        &self.settings_service
    }
}

impl HasDownloadClientSettingsService for AppState {
    type DownloadClientSettingsSvc = LiveSettingsService;
    fn download_client_settings_service(&self) -> &Self::DownloadClientSettingsSvc {
        &self.settings_service
    }
}

impl HasDownloadClientCredentialService for AppState {
    type DownloadClientCredentialSvc = LiveSettingsService;
    fn download_client_credential_service(&self) -> &Self::DownloadClientCredentialSvc {
        &self.settings_service
    }
}

impl HasIndexerSettingsService for AppState {
    type IndexerSettingsSvc = LiveSettingsService;
    fn indexer_settings_service(&self) -> &Self::IndexerSettingsSvc {
        &self.settings_service
    }
}

impl HasIndexerCredentialService for AppState {
    type IndexerCredentialSvc = LiveSettingsService;
    fn indexer_credential_service(&self) -> &Self::IndexerCredentialSvc {
        &self.settings_service
    }
}

impl HasRootFolderService for AppState {
    type RootFolderSvc = LiveSettingsService;
    fn root_folder_service(&self) -> &Self::RootFolderSvc {
        &self.settings_service
    }
}

impl HasRemotePathMappingService for AppState {
    type RemotePathMappingSvc = LiveSettingsService;
    fn remote_path_mapping_service(&self) -> &Self::RemotePathMappingSvc {
        &self.settings_service
    }
}

impl HasNotificationService for AppState {
    type NotificationSvc = LiveNotificationService;
    fn notification_service(&self) -> &Self::NotificationSvc {
        &self.notification_service
    }
}

impl HasQueueService for AppState {
    type QueueSvc = LiveQueueService;
    fn queue_service(&self) -> &Self::QueueSvc {
        &self.queue_service
    }
}

impl HasImportIoService for AppState {
    type ImportIoSvc = LiveImportIoService;
    fn import_io_service(&self) -> &Self::ImportIoSvc {
        &self.import_io_service
    }
}

impl HasManualImportService for AppState {
    type ManualImportSvc = LiveManualImportDbService;
    fn manual_import_service(&self) -> &Self::ManualImportSvc {
        &self.manual_import_db_service
    }
}

impl HasHistoryService for AppState {
    type HistorySvc = LiveHistoryService;
    fn history_service(&self) -> &Self::HistorySvc {
        &self.history_service
    }
}

impl HasAuthService for AppState {
    type AuthSvc = ServerAuthService<RealAuthCrypto>;
    fn auth_service(&self) -> &Self::AuthSvc {
        &self.auth_service
    }
}

impl HasImportWorkflow for AppState {
    type ImportWf = LiveImportWorkflow;
    fn import_workflow(&self) -> &Self::ImportWf {
        &self.import_workflow
    }
}

impl HasEnrichmentWorkflow for AppState {
    type EnrichmentWf = LiveEnrichmentWorkflow;
    fn enrichment_workflow(&self) -> &Self::EnrichmentWf {
        &self.enrichment_workflow
    }
}

impl HasRssSyncWorkflow for AppState {
    type RssSyncWf = LiveRssSyncWorkflow;
    fn rss_sync_workflow(&self) -> &Self::RssSyncWf {
        &self.rss_sync_workflow
    }
}

impl HasTagService for AppState {
    type TagSvc = crate::tag_service::LiveTagService<LiveImportIoService>;
    fn tag_service(&self) -> &Self::TagSvc {
        &self.tag_service
    }
}

impl HasEmailService for AppState {
    type EmailSvc = crate::email_service::LiveEmailService<livrarr_db::sqlite::SqliteDb>;
    fn email_service(&self) -> &Self::EmailSvc {
        &self.email_svc
    }
}

impl HasAuthorMonitorWorkflow for AppState {
    type AuthorMonitorWf = LiveAuthorMonitorWorkflow;
    fn author_monitor_workflow(&self) -> &Self::AuthorMonitorWf {
        &self.author_monitor_workflow
    }
}

impl HasImportService for AppState {
    type ImportSvc = crate::import_service::LiveImportService;
    fn import_service(&self) -> &Self::ImportSvc {
        &self.import_svc
    }
}

impl HasMatchingService for AppState {
    type MatchingSvc = crate::matching_service::LiveMatchingService;
    fn matching_service(&self) -> &Self::MatchingSvc {
        &self.matching_svc
    }
}

impl HasManualImportScan for AppState {
    type ManualImportScan = crate::manual_import_scan_service::LiveManualImportScanService;
    fn manual_import_scan(&self) -> &Self::ManualImportScan {
        &self.manual_import_scan_svc
    }
}

impl HasReadarrImportWorkflow for AppState {
    type ReadarrImportWf = crate::readarr_import_workflow::LiveReadarrImportWorkflow;
    fn readarr_import_workflow(&self) -> &Self::ReadarrImportWf {
        &self.readarr_import_wf
    }
}

impl HasHttpClient for AppState {
    fn http_client(&self) -> &livrarr_http::HttpClient {
        &self.http_client
    }
    fn http_client_safe(&self) -> &livrarr_http::HttpClient {
        &self.http_client_safe
    }
}

impl HasHttpFetcher for AppState {
    type Fetcher = livrarr_http::fetcher::HttpFetcherImpl;
    fn http_fetcher(&self) -> &Self::Fetcher {
        &self.http_fetcher
    }
}

impl HasDataDir for AppState {
    fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }
}

impl HasStartupTime for AppState {
    fn startup_time(&self) -> chrono::DateTime<chrono::Utc> {
        self.startup_time
    }
}

impl HasProviderStats for AppState {
    type ProviderStatsSvc = LiveProviderStatsService;
    fn provider_stats(&self) -> &Self::ProviderStatsSvc {
        &self.provider_stats_service
    }
}

impl HasLogSurface for AppState {
    type LogSurface = LogSurfaceAccessorImpl;
    fn log_surface(&self) -> &Self::LogSurface {
        &self.log_surface_accessor
    }
}

impl HasLiveConfig for AppState {
    type LiveConfig = LiveMetadataConfigAccessorImpl;
    fn live_metadata_config(&self) -> &Self::LiveConfig {
        &self.live_metadata_config_accessor
    }
}

impl HasRssSync for AppState {
    type RssSync = RssSyncState;
    fn rss_sync(&self) -> &Self::RssSync {
        &self.rss_sync_state
    }
}

impl HasSystem for AppState {
    type System = SystemState;
    fn system(&self) -> &Self::System {
        &self.system_state
    }
}

impl HasCoverCache for AppState {
    type CoverCache = CoverProxyCacheAccessorImpl;
    fn cover_proxy_cache(&self) -> &Self::CoverCache {
        &self.cover_proxy_cache_accessor
    }
}

impl HasCoverService for AppState {
    type CoverSvc = LiveCoverService;
    fn cover_service(&self) -> &Self::CoverSvc {
        &self.cover_service
    }
}

impl HasPreaddCoverService for AppState {
    type PreaddCoverSvc = livrarr_metadata::preadd_cover_service::LivePreaddCoverService;
    fn preadd_cover_service(&self) -> &Self::PreaddCoverSvc {
        &self.preadd_cover_service
    }
}

impl HasHmacKey for AppState {
    fn hmac_key(&self) -> &[u8] {
        &self.hmac_key
    }
}

impl HasTrustedOrigins for AppState {
    type TrustedOrigins = TrustedOriginsRebuilderImpl;
    fn trusted_origins(&self) -> &Self::TrustedOrigins {
        &self.trusted_origins_rebuilder
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
