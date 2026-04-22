use livrarr_domain::services::{
    AppConfigService, AuthorMonitorWorkflow, AuthorService, DownloadClientSettingsService,
    EmailService, EnrichmentWorkflow, FileService, GrabService, HistoryService, ImportIoService,
    ImportService, ImportWorkflow, IndexerSettingsService, ListService, ManualImportService,
    MatchingService, NotificationService, QueueService, ReadarrImportWorkflow, ReleaseService,
    RemotePathMappingService, RootFolderService, RssSyncWorkflow, SeriesQueryService,
    SeriesService, TagService, WorkService,
};
use livrarr_http::HttpClient;

use crate::accessors::{
    CoverProxyCacheAccessor, LiveMetadataConfigAccessor, ManualImportScanAccessor,
    ProviderHealthAccessor, RssSyncAccessor, SystemAccessor,
};
use crate::types::auth::AuthService as AuthServiceTrait;

// =============================================================================
// Capability sub-traits — one per service/infrastructure accessor
// =============================================================================

// --- Domain services ---

pub trait HasWorkService: Clone + Send + Sync + 'static {
    type WorkSvc: WorkService + Send + Sync + 'static;
    fn work_service(&self) -> &Self::WorkSvc;
}

pub trait HasFileService: Clone + Send + Sync + 'static {
    type FileSvc: FileService + Send + Sync + 'static;
    fn file_service(&self) -> &Self::FileSvc;
}

pub trait HasAuthorService: Clone + Send + Sync + 'static {
    type AuthorSvc: AuthorService + Send + Sync + 'static;
    fn author_service(&self) -> &Self::AuthorSvc;
}

pub trait HasSeriesService: Clone + Send + Sync + 'static {
    type SeriesSvc: SeriesService + Send + Sync + 'static;
    fn series_service(&self) -> &Self::SeriesSvc;
}

pub trait HasSeriesQueryService: Clone + Send + Sync + 'static {
    type SeriesQuerySvc: SeriesQueryService + Send + Sync + 'static;
    fn series_query_service(&self) -> &Self::SeriesQuerySvc;
}

pub trait HasGrabService: Clone + Send + Sync + 'static {
    type GrabSvc: GrabService + Send + Sync + 'static;
    fn grab_service(&self) -> &Self::GrabSvc;
}

pub trait HasReleaseService: Clone + Send + Sync + 'static {
    type ReleaseSvc: ReleaseService + Send + Sync + 'static;
    fn release_service(&self) -> &Self::ReleaseSvc;
}

pub trait HasListService: Clone + Send + Sync + 'static {
    type ListSvc: ListService + Send + Sync + 'static;
    fn list_service(&self) -> &Self::ListSvc;
}

pub trait HasAppConfigService: Clone + Send + Sync + 'static {
    type AppConfigSvc: AppConfigService + Send + Sync + 'static;
    fn app_config_service(&self) -> &Self::AppConfigSvc;
}

pub trait HasDownloadClientSettingsService: Clone + Send + Sync + 'static {
    type DownloadClientSettingsSvc: DownloadClientSettingsService + Send + Sync + 'static;
    fn download_client_settings_service(&self) -> &Self::DownloadClientSettingsSvc;
}

pub trait HasIndexerSettingsService: Clone + Send + Sync + 'static {
    type IndexerSettingsSvc: IndexerSettingsService + Send + Sync + 'static;
    fn indexer_settings_service(&self) -> &Self::IndexerSettingsSvc;
}

pub trait HasRootFolderService: Clone + Send + Sync + 'static {
    type RootFolderSvc: RootFolderService + Send + Sync + 'static;
    fn root_folder_service(&self) -> &Self::RootFolderSvc;
}

pub trait HasRemotePathMappingService: Clone + Send + Sync + 'static {
    type RemotePathMappingSvc: RemotePathMappingService + Send + Sync + 'static;
    fn remote_path_mapping_service(&self) -> &Self::RemotePathMappingSvc;
}

pub trait HasNotificationService: Clone + Send + Sync + 'static {
    type NotificationSvc: NotificationService + Send + Sync + 'static;
    fn notification_service(&self) -> &Self::NotificationSvc;
}

pub trait HasQueueService: Clone + Send + Sync + 'static {
    type QueueSvc: QueueService + Send + Sync + 'static;
    fn queue_service(&self) -> &Self::QueueSvc;
}

pub trait HasImportIoService: Clone + Send + Sync + 'static {
    type ImportIoSvc: ImportIoService + Send + Sync + 'static;
    fn import_io_service(&self) -> &Self::ImportIoSvc;
}

pub trait HasManualImportService: Clone + Send + Sync + 'static {
    type ManualImportSvc: ManualImportService + Send + Sync + 'static;
    fn manual_import_service(&self) -> &Self::ManualImportSvc;
}

pub trait HasHistoryService: Clone + Send + Sync + 'static {
    type HistorySvc: HistoryService + Send + Sync + 'static;
    fn history_service(&self) -> &Self::HistorySvc;
}

pub trait HasAuthService: Clone + Send + Sync + 'static {
    type AuthSvc: AuthServiceTrait + Send + Sync + 'static;
    fn auth_service(&self) -> &Self::AuthSvc;
}

pub trait HasImportWorkflow: Clone + Send + Sync + 'static {
    type ImportWf: ImportWorkflow + Send + Sync + 'static;
    fn import_workflow(&self) -> &Self::ImportWf;
}

pub trait HasEnrichmentWorkflow: Clone + Send + Sync + 'static {
    type EnrichmentWf: EnrichmentWorkflow + Send + Sync + 'static;
    fn enrichment_workflow(&self) -> &Self::EnrichmentWf;
}

pub trait HasRssSyncWorkflow: Clone + Send + Sync + 'static {
    type RssSyncWf: RssSyncWorkflow + Send + Sync + 'static;
    fn rss_sync_workflow(&self) -> &Self::RssSyncWf;
}

pub trait HasTagService: Clone + Send + Sync + 'static {
    type TagSvc: TagService + Send + Sync + 'static;
    fn tag_service(&self) -> &Self::TagSvc;
}

pub trait HasEmailService: Clone + Send + Sync + 'static {
    type EmailSvc: EmailService + Send + Sync + 'static;
    fn email_service(&self) -> &Self::EmailSvc;
}

pub trait HasAuthorMonitorWorkflow: Clone + Send + Sync + 'static {
    type AuthorMonitorWf: AuthorMonitorWorkflow + Send + Sync + 'static;
    fn author_monitor_workflow(&self) -> &Self::AuthorMonitorWf;
}

pub trait HasImportService: Clone + Send + Sync + 'static {
    type ImportSvc: ImportService + Send + Sync + 'static;
    fn import_service(&self) -> &Self::ImportSvc;
}

pub trait HasMatchingService: Clone + Send + Sync + 'static {
    type MatchingSvc: MatchingService + Send + Sync + 'static;
    fn matching_service(&self) -> &Self::MatchingSvc;
}

pub trait HasManualImportScan: Clone + Send + Sync + 'static {
    type ManualImportScan: ManualImportScanAccessor + Send + Sync + 'static;
    fn manual_import_scan(&self) -> &Self::ManualImportScan;
}

pub trait HasReadarrImportWorkflow: Clone + Send + Sync + 'static {
    type ReadarrImportWf: ReadarrImportWorkflow + Send + Sync + 'static;
    fn readarr_import_workflow(&self) -> &Self::ReadarrImportWf;
}

// --- Infrastructure ---

pub trait HasHttpClient: Clone + Send + Sync + 'static {
    fn http_client(&self) -> &HttpClient;
    fn http_client_safe(&self) -> &HttpClient;
}

pub trait HasDataDir: Clone + Send + Sync + 'static {
    fn data_dir(&self) -> &std::path::Path;
}

pub trait HasStartupTime: Clone + Send + Sync + 'static {
    fn startup_time(&self) -> chrono::DateTime<chrono::Utc>;
}

pub trait HasProviderHealth: Clone + Send + Sync + 'static {
    type ProviderHealth: ProviderHealthAccessor + Send + Sync + 'static;
    fn provider_health(&self) -> &Self::ProviderHealth;
}

pub trait HasLiveConfig: Clone + Send + Sync + 'static {
    type LiveConfig: LiveMetadataConfigAccessor + Send + Sync + 'static;
    fn live_metadata_config(&self) -> &Self::LiveConfig;
}

pub trait HasRssSync: Clone + Send + Sync + 'static {
    type RssSync: RssSyncAccessor + Send + Sync + 'static;
    fn rss_sync(&self) -> &Self::RssSync;
}

pub trait HasSystem: Clone + Send + Sync + 'static {
    type System: SystemAccessor + Send + Sync + 'static;
    fn system(&self) -> &Self::System;
}

pub trait HasCoverCache: Clone + Send + Sync + 'static {
    type CoverCache: CoverProxyCacheAccessor + Send + Sync + 'static;
    fn cover_proxy_cache(&self) -> &Self::CoverCache;
}

pub trait HasEnrichmentNotify: Clone + Send + Sync + 'static {
    fn enrichment_notify(&self) -> &tokio::sync::Notify;
}

// =============================================================================
// AppContext — supertrait union of all capability traits
// =============================================================================

pub trait AppContext:
    HasWorkService
    + HasFileService
    + HasAuthorService
    + HasSeriesService
    + HasSeriesQueryService
    + HasGrabService
    + HasReleaseService
    + HasListService
    + HasAppConfigService
    + HasDownloadClientSettingsService
    + HasIndexerSettingsService
    + HasRootFolderService
    + HasRemotePathMappingService
    + HasNotificationService
    + HasQueueService
    + HasImportIoService
    + HasManualImportService
    + HasHistoryService
    + HasAuthService
    + HasImportWorkflow
    + HasEnrichmentWorkflow
    + HasRssSyncWorkflow
    + HasTagService
    + HasEmailService
    + HasAuthorMonitorWorkflow
    + HasImportService
    + HasMatchingService
    + HasManualImportScan
    + HasReadarrImportWorkflow
    + HasHttpClient
    + HasDataDir
    + HasStartupTime
    + HasProviderHealth
    + HasLiveConfig
    + HasRssSync
    + HasSystem
    + HasCoverCache
    + HasEnrichmentNotify
{
}

impl<T> AppContext for T where
    T: HasWorkService
        + HasFileService
        + HasAuthorService
        + HasSeriesService
        + HasSeriesQueryService
        + HasGrabService
        + HasReleaseService
        + HasListService
        + HasAppConfigService
        + HasDownloadClientSettingsService
        + HasIndexerSettingsService
        + HasRootFolderService
        + HasRemotePathMappingService
        + HasNotificationService
        + HasQueueService
        + HasImportIoService
        + HasManualImportService
        + HasHistoryService
        + HasAuthService
        + HasImportWorkflow
        + HasEnrichmentWorkflow
        + HasRssSyncWorkflow
        + HasTagService
        + HasEmailService
        + HasAuthorMonitorWorkflow
        + HasImportService
        + HasMatchingService
        + HasManualImportScan
        + HasReadarrImportWorkflow
        + HasHttpClient
        + HasDataDir
        + HasStartupTime
        + HasProviderHealth
        + HasLiveConfig
        + HasRssSync
        + HasSystem
        + HasCoverCache
        + HasEnrichmentNotify
{
}
