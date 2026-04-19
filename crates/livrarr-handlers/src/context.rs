use livrarr_domain::services::{
    AuthorMonitorWorkflow, AuthorService, EmailService, EnrichmentWorkflow, FileService,
    GrabService, HistoryService, ImportIoService, ImportService, ImportWorkflow, ListService,
    ManualImportService, MatchingService, NotificationService, QueueService, ReadarrImportWorkflow,
    ReleaseService, RssSyncWorkflow, SeriesQueryService, SeriesService, SettingsService,
    TagService, WorkService,
};
use livrarr_http::HttpClient;

use crate::accessors::{
    CoverProxyCacheAccessor, LiveMetadataConfigAccessor, ManualImportScanAccessor,
    ProviderHealthAccessor, RssSyncAccessor, SystemAccessor,
};
use crate::types::auth::AuthService as AuthServiceTrait;

pub trait AppContext: Clone + Send + Sync + 'static {
    // --- Domain services ---
    type WorkSvc: WorkService + Send + Sync + 'static;
    type FileSvc: FileService + Send + Sync + 'static;
    type AuthorSvc: AuthorService + Send + Sync + 'static;
    type SeriesSvc: SeriesService + Send + Sync + 'static;
    type SeriesQuerySvc: SeriesQueryService + Send + Sync + 'static;
    type GrabSvc: GrabService + Send + Sync + 'static;
    type ReleaseSvc: ReleaseService + Send + Sync + 'static;
    type ListSvc: ListService + Send + Sync + 'static;
    type SettingsSvc: SettingsService + Send + Sync + 'static;
    type NotificationSvc: NotificationService + Send + Sync + 'static;
    type QueueSvc: QueueService + Send + Sync + 'static;
    type ImportIoSvc: ImportIoService + Send + Sync + 'static;
    type ManualImportSvc: ManualImportService + Send + Sync + 'static;
    type HistorySvc: HistoryService + Send + Sync + 'static;
    type AuthSvc: AuthServiceTrait + Send + Sync + 'static;
    type ImportWf: ImportWorkflow + Send + Sync + 'static;
    type EnrichmentWf: EnrichmentWorkflow + Send + Sync + 'static;
    type RssSyncWf: RssSyncWorkflow + Send + Sync + 'static;
    type TagSvc: TagService + Send + Sync + 'static;
    type EmailSvc: EmailService + Send + Sync + 'static;
    type AuthorMonitorWf: AuthorMonitorWorkflow + Send + Sync + 'static;
    type ImportSvc: ImportService + Send + Sync + 'static;
    type MatchingSvc: MatchingService + Send + Sync + 'static;
    type ManualImportScan: ManualImportScanAccessor + Send + Sync + 'static;
    type ReadarrImportWf: ReadarrImportWorkflow + Send + Sync + 'static;

    // --- Infrastructure ---
    type ProviderHealth: ProviderHealthAccessor + Send + Sync + 'static;
    type LiveConfig: LiveMetadataConfigAccessor + Send + Sync + 'static;
    type RssSync: RssSyncAccessor + Send + Sync + 'static;
    type System: SystemAccessor + Send + Sync + 'static;
    type CoverCache: CoverProxyCacheAccessor + Send + Sync + 'static;

    // --- Domain service accessors ---
    fn work_service(&self) -> &Self::WorkSvc;
    fn file_service(&self) -> &Self::FileSvc;
    fn author_service(&self) -> &Self::AuthorSvc;
    fn series_service(&self) -> &Self::SeriesSvc;
    fn series_query_service(&self) -> &Self::SeriesQuerySvc;
    fn grab_service(&self) -> &Self::GrabSvc;
    fn release_service(&self) -> &Self::ReleaseSvc;
    fn list_service(&self) -> &Self::ListSvc;
    fn settings_service(&self) -> &Self::SettingsSvc;
    fn notification_service(&self) -> &Self::NotificationSvc;
    fn queue_service(&self) -> &Self::QueueSvc;
    fn import_io_service(&self) -> &Self::ImportIoSvc;
    fn manual_import_service(&self) -> &Self::ManualImportSvc;
    fn history_service(&self) -> &Self::HistorySvc;
    fn auth_service(&self) -> &Self::AuthSvc;
    fn import_workflow(&self) -> &Self::ImportWf;
    fn enrichment_workflow(&self) -> &Self::EnrichmentWf;
    fn rss_sync_workflow(&self) -> &Self::RssSyncWf;
    fn tag_service(&self) -> &Self::TagSvc;
    fn email_service(&self) -> &Self::EmailSvc;
    fn author_monitor_workflow(&self) -> &Self::AuthorMonitorWf;
    fn import_service(&self) -> &Self::ImportSvc;
    fn matching_service(&self) -> &Self::MatchingSvc;
    fn manual_import_scan(&self) -> &Self::ManualImportScan;
    fn readarr_import_workflow(&self) -> &Self::ReadarrImportWf;

    // --- Infrastructure accessors ---
    fn http_client(&self) -> &HttpClient;
    fn data_dir(&self) -> &std::path::Path;
    fn startup_time(&self) -> chrono::DateTime<chrono::Utc>;
    fn provider_health(&self) -> &Self::ProviderHealth;
    fn live_metadata_config(&self) -> &Self::LiveConfig;
    fn rss_sync(&self) -> &Self::RssSync;
    fn system(&self) -> &Self::System;
    fn cover_proxy_cache(&self) -> &Self::CoverCache;
}
