pub use livrarr_domain::*;

// Re-export everything from livrarr-handlers. Specific `pub use` items shadow
// the glob from livrarr_domain when names overlap (AddWorkRequest, etc.).
pub use livrarr_handlers::deserialize_optional_secret;

pub use livrarr_handlers::ApiError;
pub use livrarr_handlers::AuthContext;
pub use livrarr_handlers::AuthError;
pub use livrarr_handlers::AuthService;
pub use livrarr_handlers::DownloadError;
pub use livrarr_handlers::EnrichmentError;
pub use livrarr_handlers::FieldError;
pub use livrarr_handlers::ImportError;
pub use livrarr_handlers::MetadataError;
pub use livrarr_handlers::ScanError;
pub use livrarr_handlers::TagWriteError;

pub use livrarr_handlers::AdminCreateUserRequest;
pub use livrarr_handlers::AdminUpdateUserRequest;
pub use livrarr_handlers::ApiKeyResponse;
pub use livrarr_handlers::AuthMeResponse;
pub use livrarr_handlers::LoginRequest;
pub use livrarr_handlers::LoginResponse;
pub use livrarr_handlers::SetupRequest;
pub use livrarr_handlers::SetupResponse;
pub use livrarr_handlers::SetupStatusResponse;
pub use livrarr_handlers::UpdateProfileRequest;
pub use livrarr_handlers::UserResponse;

pub use livrarr_handlers::PaginatedResponse;
pub use livrarr_handlers::PaginationQuery;

pub use livrarr_handlers::types::work::AddWorkRequest;
pub use livrarr_handlers::types::work::UpdateWorkRequest;
pub use livrarr_handlers::AddWorkResponse;
pub use livrarr_handlers::DeleteWorkResponse;
pub use livrarr_handlers::LibraryItemResponse;
pub use livrarr_handlers::LookupApiResponse;
pub use livrarr_handlers::RefreshWorkResponse;
pub use livrarr_handlers::WorkApi;
pub use livrarr_handlers::WorkDetailResponse;
pub use livrarr_handlers::WorkSearchResult;

pub use livrarr_handlers::AddAuthorApiRequest;
pub use livrarr_handlers::AuthorApi;
pub use livrarr_handlers::AuthorDetailResponse;
pub use livrarr_handlers::AuthorResponse;
pub use livrarr_handlers::AuthorSearchResult;
pub use livrarr_handlers::UpdateAuthorApiRequest;

pub use livrarr_handlers::GrAuthorCandidate;
pub use livrarr_handlers::MonitorSeriesRequest;
pub use livrarr_handlers::ResolveGrResponse;
pub use livrarr_handlers::SeriesDetailResponse;
pub use livrarr_handlers::SeriesListResponse;
pub use livrarr_handlers::SeriesResponse;
pub use livrarr_handlers::SeriesWithAuthorResponse;
pub use livrarr_handlers::UpdateSeriesRequest;

pub use livrarr_handlers::NotificationApi;
pub use livrarr_handlers::NotificationResponse;

pub use livrarr_handlers::CreateRootFolderRequest;
pub use livrarr_handlers::RootFolderApi;
pub use livrarr_handlers::RootFolderResponse;

pub use livrarr_handlers::CreateDownloadClientApiRequest;
pub use livrarr_handlers::DownloadClientApi;
pub use livrarr_handlers::DownloadClientResponse;
pub use livrarr_handlers::UpdateDownloadClientApiRequest;

pub use livrarr_handlers::CreateIndexerApiRequest;
pub use livrarr_handlers::IndexerResponse;
pub use livrarr_handlers::TestIndexerApiRequest;
pub use livrarr_handlers::TestIndexerApiResponse;
pub use livrarr_handlers::UpdateIndexerApiRequest;

pub use livrarr_handlers::CreateRemotePathMappingApiRequest;
pub use livrarr_handlers::RemotePathMappingApi;
pub use livrarr_handlers::RemotePathMappingResponse;
pub use livrarr_handlers::UpdateRemotePathMappingRequest;

pub use livrarr_handlers::ConfigApi;
pub use livrarr_handlers::EmailConfigResponse;
pub use livrarr_handlers::MediaManagementConfigResponse;
pub use livrarr_handlers::MetadataConfigResponse;
pub use livrarr_handlers::NamingConfigResponse;
pub use livrarr_handlers::ProwlarrConfigResponse;
pub use livrarr_handlers::ProwlarrImportRequest;
pub use livrarr_handlers::ProwlarrImportResponse;
pub use livrarr_handlers::SendEmailRequest;
pub use livrarr_handlers::TestProwlarrRequest;
pub use livrarr_handlers::UpdateEmailApiRequest;
pub use livrarr_handlers::UpdateMediaManagementApiRequest;
pub use livrarr_handlers::UpdateMetadataApiRequest;
pub use livrarr_handlers::UpdateProwlarrApiRequest;

pub use livrarr_handlers::HealthCheckResult;
pub use livrarr_handlers::SystemApi;
pub use livrarr_handlers::SystemStatus;

pub use livrarr_handlers::HistoryApi;
pub use livrarr_handlers::HistoryResponse;

pub use livrarr_domain::QueueProgress;
pub use livrarr_handlers::QueueItemResponse;
pub use livrarr_handlers::QueueListResponse;

pub use livrarr_handlers::types::release::ReleaseSearchResponse;
pub use livrarr_handlers::GrabApiRequest;
pub use livrarr_handlers::ReleaseResponse;
pub use livrarr_handlers::SearchWarning;

pub use livrarr_handlers::types::scan::ScanResult;
pub use livrarr_handlers::ScanErrorEntry;
pub use livrarr_handlers::ScanUnmatchedFile;

// LibraryFileApi trait — defined in livrarr-handlers
pub use livrarr_handlers::types::work::LibraryFileApi;

// ---------------------------------------------------------------------------
// From impls that require crates not available in livrarr-handlers
// ---------------------------------------------------------------------------

// DownloadError conversion: livrarr-download -> livrarr-handlers DownloadError.
// This requires a function since we can't impl From due to orphan rules.
pub fn download_error_to_api(e: livrarr_download::DownloadError) -> DownloadError {
    use livrarr_download::DownloadError as D;
    match e {
        D::NoClient | D::NoEnabledClient => DownloadError::NoClient,
        D::ConnectionFailed(s) | D::ProwlarrUnreachable(s) => DownloadError::ConnectionFailed(s),
        D::Rejected { reason } => DownloadError::Rejected(reason),
        D::InvalidUrl | D::InvalidMagnet { .. } | D::InvalidTorrentFile { .. } => {
            DownloadError::InvalidSource(e.to_string())
        }
        _ => DownloadError::Client(e.to_string()),
    }
}

impl From<crate::readarr_import_service::ReadarrImportError> for ApiError {
    fn from(e: crate::readarr_import_service::ReadarrImportError) -> Self {
        use crate::readarr_import_service::ReadarrImportError;
        match e {
            ReadarrImportError::NotFound(msg) => ApiError::BadRequest(msg),
            ReadarrImportError::Conflict(msg) => ApiError::Conflict { reason: msg },
            ReadarrImportError::Db(db_err) => ApiError::Db(db_err),
        }
    }
}

// ---------------------------------------------------------------------------
// Server-only modules
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod api_secondary_impl;
pub mod auth_crypto;
pub mod auth_service;
pub mod config;
pub mod handlers;
pub mod jobs;
pub mod rate_limit;
pub use livrarr_matching as matching;
pub mod email_service;
pub mod history_service;
pub mod import_io_service;
pub mod import_service;
pub mod manual_import_scan_service;
pub mod manual_import_service;
pub mod matching_service;
pub mod middleware;
pub mod notification_service;
pub mod queue_service;
pub mod readarr_client;
pub mod readarr_import_service;
pub mod readarr_import_workflow;
pub mod router;
pub mod services;
pub mod state;
pub mod tag_service;
