use livrarr_domain::services::{
    AppConfigService, AuthorLinkService, AuthorMonitorWorkflow, AuthorService, AuthorViewService,
    BookmarkService, ChapterService, CoverService, DiscoveryService,
    DownloadClientCredentialService, DownloadClientSettingsService, EmailService,
    EnrichmentWorkflow, FileService, GrabService, HistoryService, IdentityConflictService,
    IdentityResolver, ImportIoService, ImportService, ImportWorkflow, IndexerCredentialService,
    IndexerSettingsService, ListService, ManualImportService, MatchingService, NotificationService,
    ProviderStatsService, QueueService, ReadarrImportWorkflow, ReleaseService,
    RemotePathMappingService, RootFolderService, RssSyncWorkflow, SeriesQueryService,
    SeriesService, TagService, WorkIdentityRepository, WorkService,
};
use livrarr_http::HttpClient;

use crate::accessors::{
    CoverProxyCacheAccessor, LiveMetadataConfigAccessor, LogSurfaceAccessor,
    ManualImportScanAccessor, RssSyncAccessor, SystemAccessor,
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

pub trait HasDiscoveryService: Clone + Send + Sync + 'static {
    type DiscoverySvc: DiscoveryService + Send + Sync + 'static;
    fn discovery_service(&self) -> &Self::DiscoverySvc;
}

pub trait HasWorkIdentityRepository: Clone + Send + Sync + 'static {
    type WorkIdentityRepo: WorkIdentityRepository + Send + Sync + 'static;
    fn work_identity_repo(&self) -> &Self::WorkIdentityRepo;
}

/// Identity-layer-rewrite (F2). IR v1 `livrarr-handlers` module names this
/// `HasIdentityRoadService { identity_road_service: &S where S:
/// IdentityRoadService }`; represented with the same associated-type
/// accessor shape every other `Has*` trait in this file uses (`trait_variant`
/// traits are not dyn-compatible — see insight 8).
pub trait HasIdentityRoadService: Clone + Send + Sync + 'static {
    type IdentityRoadSvc: livrarr_domain::identity_layer::IdentityRoadService
        + Send
        + Sync
        + 'static;
    fn identity_road_service(&self) -> &Self::IdentityRoadSvc;
}

/// Persistence half of the F2 composition, used only by interactive doors to
/// place their typed review-card draft in the same settlement transaction.
pub trait HasIdentityLayerRepository: Clone + Send + Sync + 'static {
    type IdentityLayerRepo: livrarr_domain::identity_layer::WorkIdentityRepository
        + Send
        + Sync
        + 'static;
    fn identity_layer_repository(&self) -> &Self::IdentityLayerRepo;
}

/// Narrow ManualImport boundary for first-class Edition evidence. The
/// concrete server adapter owns persistence lookup/creation and delegates the
/// evidence decision to the domain `EditionRepository`; handlers see neither
/// SQL nor a database implementation.
#[trait_variant::make(Send)]
pub trait EditionEvidenceCapability: Send + Sync {
    async fn apply_evidence(
        &self,
        user_id: livrarr_domain::UserId,
        work_id: livrarr_domain::WorkId,
        format: livrarr_domain::identity_layer::EditionFormat,
        language: Option<String>,
    ) -> Result<
        livrarr_domain::identity_layer::EditionEvidenceOutcome,
        livrarr_domain::identity_layer::EditionRepositoryError,
    >;
}

pub trait HasEditionRepository: Clone + Send + Sync + 'static {
    type EditionRepo: EditionEvidenceCapability + Send + Sync + 'static;
    fn edition_repository(&self) -> &Self::EditionRepo;
}

pub trait HasFileService: Clone + Send + Sync + 'static {
    type FileSvc: FileService + Send + Sync + 'static;
    fn file_service(&self) -> &Self::FileSvc;
}

pub trait HasChapterService: Clone + Send + Sync + 'static {
    type ChapterSvc: ChapterService + Send + Sync + 'static;
    fn chapter_service(&self) -> &Self::ChapterSvc;
}

pub trait HasBookmarkService: Clone + Send + Sync + 'static {
    type BookmarkSvc: BookmarkService + Send + Sync + 'static;
    fn bookmark_service(&self) -> &Self::BookmarkSvc;
}

pub trait HasCrossFormatService: Clone + Send + Sync + 'static {
    type CrossFormatSvc: livrarr_domain::services::CrossFormatService + Send + Sync + 'static;
    fn cross_format_service(&self) -> &Self::CrossFormatSvc;
}

pub trait HasAuthorService: Clone + Send + Sync + 'static {
    type AuthorSvc: AuthorService + Send + Sync + 'static;
    fn author_service(&self) -> &Self::AuthorSvc;
}

pub trait HasAuthorLinkService: Clone + Send + Sync + 'static {
    type AuthorLinkSvc: AuthorLinkService + Send + Sync + 'static;
    fn author_link_service(&self) -> &Self::AuthorLinkSvc;
}

pub trait HasAuthorViewService: Clone + Send + Sync + 'static {
    type AuthorViewSvc: AuthorViewService + Send + Sync + 'static;
    fn author_view_service(&self) -> &Self::AuthorViewSvc;
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

pub trait HasIdentityConflictService: Clone + Send + Sync + 'static {
    type IdentityConflictSvc: IdentityConflictService + Send + Sync + 'static;
    fn identity_conflict_service(&self) -> &Self::IdentityConflictSvc;
}

pub trait HasIdentityResolver: Clone + Send + Sync + 'static {
    type IdentityResolverSvc: IdentityResolver + Send + Sync + 'static;
    fn identity_resolver(&self) -> &Self::IdentityResolverSvc;
}

pub trait HasAppConfigService: Clone + Send + Sync + 'static {
    type AppConfigSvc: AppConfigService + Send + Sync + 'static;
    fn app_config_service(&self) -> &Self::AppConfigSvc;
}

pub trait HasDownloadClientSettingsService: Clone + Send + Sync + 'static {
    type DownloadClientSettingsSvc: DownloadClientSettingsService + Send + Sync + 'static;
    fn download_client_settings_service(&self) -> &Self::DownloadClientSettingsSvc;
}

pub trait HasDownloadClientCredentialService: Clone + Send + Sync + 'static {
    type DownloadClientCredentialSvc: DownloadClientCredentialService + Send + Sync + 'static;
    fn download_client_credential_service(&self) -> &Self::DownloadClientCredentialSvc;
}

pub trait HasIndexerSettingsService: Clone + Send + Sync + 'static {
    type IndexerSettingsSvc: IndexerSettingsService + Send + Sync + 'static;
    fn indexer_settings_service(&self) -> &Self::IndexerSettingsSvc;
}

pub trait HasIndexerCredentialService: Clone + Send + Sync + 'static {
    type IndexerCredentialSvc: IndexerCredentialService + Send + Sync + 'static;
    fn indexer_credential_service(&self) -> &Self::IndexerCredentialSvc;
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

/// The queue-routed `HttpFetcher` — outbound requests through this accessor
/// are paced and capped by the process-global outbound queue.
pub trait HasHttpFetcher: Clone + Send + Sync + 'static {
    type Fetcher: livrarr_domain::services::HttpFetcher + Send + Sync + 'static;
    fn http_fetcher(&self) -> &Self::Fetcher;
}

pub trait HasDataDir: Clone + Send + Sync + 'static {
    fn data_dir(&self) -> &std::path::Path;
}

pub trait HasStartupTime: Clone + Send + Sync + 'static {
    fn startup_time(&self) -> chrono::DateTime<chrono::Utc>;
}

/// Record-fed provider panel stats (REQ-002, replaces the ok/error-only
/// provider health view).
pub trait HasProviderStats: Clone + Send + Sync + 'static {
    type ProviderStatsSvc: ProviderStatsService + Send + Sync + 'static;
    fn provider_stats(&self) -> &Self::ProviderStatsSvc;
}

/// Truthful log surface for the status page (REQ-003).
pub trait HasLogSurface: Clone + Send + Sync + 'static {
    type LogSurface: LogSurfaceAccessor + Send + Sync + 'static;
    fn log_surface(&self) -> &Self::LogSurface;
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

pub trait HasCoverService: Clone + Send + Sync + 'static {
    type CoverSvc: CoverService + Send + Sync + 'static;
    fn cover_service(&self) -> &Self::CoverSvc;
}

pub trait HasPreaddCoverService: Clone + Send + Sync + 'static {
    type PreaddCoverSvc: livrarr_domain::services::PreaddCoverService + Send + Sync + 'static;
    fn preadd_cover_service(&self) -> &Self::PreaddCoverSvc;
}

pub trait HasHmacKey: Clone + Send + Sync + 'static {
    fn hmac_key(&self) -> &[u8];
}

pub trait HasTrustedOrigins: Clone + Send + Sync + 'static {
    type TrustedOrigins: crate::accessors::TrustedOriginsRebuilder + Send + Sync + 'static;
    fn trusted_origins(&self) -> &Self::TrustedOrigins;
}

// =============================================================================
// AppContext — supertrait union of all capability traits
// =============================================================================

pub trait AppContext:
    HasWorkService
    + HasFileService
    + HasChapterService
    + HasBookmarkService
    + HasCrossFormatService
    + HasAuthorService
    + HasAuthorLinkService
    + HasAuthorViewService
    + HasSeriesService
    + HasSeriesQueryService
    + HasGrabService
    + HasReleaseService
    + HasListService
    + HasIdentityConflictService
    + HasIdentityResolver
    + HasAppConfigService
    + HasDownloadClientSettingsService
    + HasDownloadClientCredentialService
    + HasIndexerSettingsService
    + HasIndexerCredentialService
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
    + HasProviderStats
    + HasLogSurface
    + HasLiveConfig
    + HasRssSync
    + HasSystem
    + HasCoverCache
    + HasCoverService
    + HasPreaddCoverService
    + HasHmacKey
    + HasTrustedOrigins
{
}

impl<T> AppContext for T where
    T: HasWorkService
        + HasFileService
        + HasChapterService
        + HasBookmarkService
        + HasCrossFormatService
        + HasAuthorService
        + HasAuthorLinkService
        + HasAuthorViewService
        + HasSeriesService
        + HasSeriesQueryService
        + HasGrabService
        + HasReleaseService
        + HasListService
        + HasIdentityConflictService
        + HasIdentityResolver
        + HasAppConfigService
        + HasDownloadClientSettingsService
        + HasDownloadClientCredentialService
        + HasIndexerSettingsService
        + HasIndexerCredentialService
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
        + HasProviderStats
        + HasLogSurface
        + HasLiveConfig
        + HasRssSync
        + HasSystem
        + HasCoverCache
        + HasCoverService
        + HasPreaddCoverService
        + HasHmacKey
        + HasTrustedOrigins
{
}

/// Rebuild the SSRF trusted-origins allowlist from the current set of
/// configured indexers and download clients. Called after any CRUD
/// mutation on either entity so the allowlist stays in sync.
pub async fn rebuild_trusted_origins<
    S: HasIndexerSettingsService + HasDownloadClientSettingsService + HasTrustedOrigins,
>(
    state: &S,
) {
    use crate::accessors::TrustedOriginsRebuilder;
    use livrarr_domain::services::{DownloadClientSettingsService, IndexerSettingsService};

    let mut urls = Vec::new();
    if let Ok(indexers) = state.indexer_settings_service().list_indexers().await {
        urls.extend(indexers.iter().map(|i| i.url.clone()));
    }
    if let Ok(clients) = state
        .download_client_settings_service()
        .list_download_clients()
        .await
    {
        for c in &clients {
            let scheme = if c.use_ssl { "https" } else { "http" };
            urls.push(format!("{}://{}:{}", scheme, c.host, c.port));
        }
    }
    state.trusted_origins().rebuild(&urls);
}
