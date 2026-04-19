use livrarr_domain::DbError;
use serde::{Deserialize, Serialize};

use super::auth::AuthError;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("no download client configured")]
    NoClient,
    #[error("download client connection failed: {0}")]
    ConnectionFailed(String),
    #[error("download client rejected: {0}")]
    Rejected(String),
    #[error("invalid download source: {0}")]
    InvalidSource(String),
    #[error("download client error: {0}")]
    Client(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("source path not found: {path}")]
    SourceNotFound { path: String },
    #[error("no media files found in download")]
    NoMediaFiles,
    #[error("no root folder configured for this media type")]
    NoRootFolder,
    #[error("path conflict with existing work {existing_work_id}")]
    PathConflict { existing_work_id: i64 },
    #[error("disk full")]
    DiskFull,
    #[error("import failed: {0}")]
    Failed(String),
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct MetadataError(pub String);

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct EnrichmentError(pub String);

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct TagWriteError(pub String);

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ScanError(pub String);

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("not found")]
    NotFound,
    #[error("conflict: {reason}")]
    Conflict { reason: String },
    #[error("validation error")]
    Validation { errors: Vec<FieldError> },
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("bad gateway: {0}")]
    BadGateway(String),
    #[error("bad gateway")]
    StructuredBadGateway { body: serde_json::Value },
    #[error("service unavailable")]
    ServiceUnavailable,
    #[error("not implemented")]
    NotImplemented,
    #[error("payload too large (max {max_bytes} bytes)")]
    PayloadTooLarge { max_bytes: usize },
    #[error("internal error: {0}")]
    Internal(String),

    #[error("{0}")]
    Auth(#[from] AuthError),
    #[error("{0}")]
    Download(#[from] DownloadError),
    #[error("{0}")]
    Import(#[from] ImportError),
    #[error("{0}")]
    Metadata(#[from] MetadataError),
    #[error("{0}")]
    Enrichment(#[from] EnrichmentError),
    #[error("{0}")]
    TagWrite(#[from] TagWriteError),
    #[error("{0}")]
    Scan(#[from] ScanError),
    #[error("{0}")]
    Db(#[from] DbError),
}

// --- Service error -> ApiError mappings ---

impl From<livrarr_domain::services::AuthorServiceError> for ApiError {
    fn from(e: livrarr_domain::services::AuthorServiceError) -> Self {
        use livrarr_domain::services::AuthorServiceError;
        match e {
            AuthorServiceError::NotFound => ApiError::NotFound,
            AuthorServiceError::AlreadyExists => ApiError::Conflict {
                reason: "author already exists".into(),
            },
            AuthorServiceError::Validation { field, message } => ApiError::Validation {
                errors: vec![FieldError { field, message }],
            },
            AuthorServiceError::OlRateLimited => ApiError::ServiceUnavailable,
            AuthorServiceError::Provider(msg) => ApiError::BadGateway(msg),
            AuthorServiceError::Db(db_err) => ApiError::Db(db_err),
        }
    }
}

impl From<livrarr_domain::services::SeriesServiceError> for ApiError {
    fn from(e: livrarr_domain::services::SeriesServiceError) -> Self {
        use livrarr_domain::services::SeriesServiceError;
        match e {
            SeriesServiceError::NotFound => ApiError::NotFound,
            SeriesServiceError::Validation { field, message } => ApiError::Validation {
                errors: vec![FieldError { field, message }],
            },
            SeriesServiceError::GoodreadsUnavailable => {
                ApiError::BadGateway("Goodreads unavailable".into())
            }
            SeriesServiceError::Db(db_err) => ApiError::Db(db_err),
        }
    }
}

impl From<livrarr_domain::services::WorkServiceError> for ApiError {
    fn from(e: livrarr_domain::services::WorkServiceError) -> Self {
        use livrarr_domain::services::WorkServiceError;
        match e {
            WorkServiceError::NotFound => ApiError::NotFound,
            WorkServiceError::AlreadyExists => ApiError::Conflict {
                reason: "work already exists".into(),
            },
            WorkServiceError::EnrichmentConflict => ApiError::Conflict {
                reason: "enrichment conflict".into(),
            },
            WorkServiceError::CoverTooLarge => ApiError::BadRequest("cover too large".into()),
            WorkServiceError::Enrichment(msg) => ApiError::Internal(msg),
            WorkServiceError::Db(db_err) => ApiError::Db(db_err),
        }
    }
}

impl From<livrarr_domain::services::ReleaseServiceError> for ApiError {
    fn from(e: livrarr_domain::services::ReleaseServiceError) -> Self {
        use livrarr_domain::services::ReleaseServiceError;
        match e {
            ReleaseServiceError::NoClient { protocol } => {
                let label = if protocol == "usenet" {
                    "Usenet"
                } else {
                    "torrent"
                };
                ApiError::BadRequest(format!("No {label} download client configured"))
            }
            ReleaseServiceError::ClientProtocolMismatch { protocol } => ApiError::BadRequest(
                format!("Selected download client does not support {protocol} protocol"),
            ),
            ReleaseServiceError::ClientUnreachable(msg) => ApiError::BadGateway(msg),
            ReleaseServiceError::DownloadClientAuth => {
                ApiError::BadGateway("Download client auth failed".into())
            }
            ReleaseServiceError::Ssrf(msg) => {
                ApiError::BadRequest(format!("Invalid download URL: {msg}"))
            }
            ReleaseServiceError::AllIndexersFailed => {
                ApiError::BadGateway("All indexers failed".into())
            }
            ReleaseServiceError::Db(db_err) => ApiError::Db(db_err),
        }
    }
}

impl From<livrarr_domain::services::GrabServiceError> for ApiError {
    fn from(e: livrarr_domain::services::GrabServiceError) -> Self {
        use livrarr_domain::services::GrabServiceError;
        match e {
            GrabServiceError::NotFound => ApiError::NotFound,
            GrabServiceError::ClientUnreachable(msg) => ApiError::BadGateway(msg),
            GrabServiceError::Db(db_err) => ApiError::Db(db_err),
        }
    }
}

impl From<livrarr_domain::services::ImportWorkflowError> for ApiError {
    fn from(e: livrarr_domain::services::ImportWorkflowError) -> Self {
        use livrarr_domain::services::ImportWorkflowError;
        match e {
            ImportWorkflowError::GrabNotFound => ApiError::NotFound,
            ImportWorkflowError::SourceNotResolved(msg) => ApiError::BadGateway(msg),
            ImportWorkflowError::ClientUnreachable(msg) => ApiError::BadGateway(msg),
            ImportWorkflowError::NoRootFolder { media_type } => {
                ApiError::BadRequest(format!("no root folder configured for {media_type:?}"))
            }
            ImportWorkflowError::SourceInaccessible => {
                ApiError::BadGateway("source directory not found or inaccessible".into())
            }
            ImportWorkflowError::ScanExpired => ApiError::NotFound,
            ImportWorkflowError::ScanForbidden => ApiError::Forbidden,
            ImportWorkflowError::ImportFailed(msg) => ApiError::Internal(msg),
            ImportWorkflowError::TagWriteFailed(msg) => ApiError::Internal(msg),
            ImportWorkflowError::Db(db_err) => ApiError::Db(db_err),
        }
    }
}

impl From<livrarr_domain::services::ListServiceError> for ApiError {
    fn from(e: livrarr_domain::services::ListServiceError) -> Self {
        use livrarr_domain::services::ListServiceError;
        match e {
            ListServiceError::NotFound => ApiError::NotFound,
            ListServiceError::Parse(msg) => ApiError::BadRequest(msg),
            ListServiceError::Conflict(msg) => ApiError::Conflict { reason: msg },
            ListServiceError::Db(db_err) => ApiError::Db(db_err),
        }
    }
}

impl From<livrarr_domain::services::FileServiceError> for ApiError {
    fn from(e: livrarr_domain::services::FileServiceError) -> Self {
        use livrarr_domain::services::FileServiceError;
        match e {
            FileServiceError::NotFound => ApiError::NotFound,
            FileServiceError::RootFolderNotFound => ApiError::NotFound,
            FileServiceError::Forbidden => ApiError::Forbidden,
            FileServiceError::BadRequest(msg) => ApiError::BadRequest(msg),
            FileServiceError::Io(io_err) => ApiError::Internal(format!("I/O error: {io_err}")),
            FileServiceError::Db(db_err) => ApiError::Db(db_err),
        }
    }
}

impl From<livrarr_domain::services::NotificationServiceError> for ApiError {
    fn from(e: livrarr_domain::services::NotificationServiceError) -> Self {
        use livrarr_domain::services::NotificationServiceError;
        match e {
            NotificationServiceError::NotFound => ApiError::NotFound,
            NotificationServiceError::Db(db_err) => ApiError::Db(db_err),
        }
    }
}

impl From<livrarr_domain::services::QueueServiceError> for ApiError {
    fn from(e: livrarr_domain::services::QueueServiceError) -> Self {
        use livrarr_domain::services::QueueServiceError;
        match e {
            QueueServiceError::NotFound => ApiError::NotFound,
            QueueServiceError::NotImportable => ApiError::Conflict {
                reason: "grab is not in an importable state".into(),
            },
            QueueServiceError::Db(db_err) => ApiError::Db(db_err),
        }
    }
}

impl From<livrarr_domain::services::ImportIoServiceError> for ApiError {
    fn from(e: livrarr_domain::services::ImportIoServiceError) -> Self {
        use livrarr_domain::services::ImportIoServiceError;
        match e {
            ImportIoServiceError::NotFound => ApiError::NotFound,
            ImportIoServiceError::Db(db_err) => ApiError::Db(db_err),
        }
    }
}

impl From<livrarr_domain::services::HistoryServiceError> for ApiError {
    fn from(e: livrarr_domain::services::HistoryServiceError) -> Self {
        use livrarr_domain::services::HistoryServiceError;
        match e {
            HistoryServiceError::NotFound => ApiError::NotFound,
            HistoryServiceError::Db(db_err) => ApiError::Db(db_err),
        }
    }
}

impl From<livrarr_domain::services::ManualImportServiceError> for ApiError {
    fn from(e: livrarr_domain::services::ManualImportServiceError) -> Self {
        use livrarr_domain::services::ManualImportServiceError;
        match e {
            ManualImportServiceError::NotFound => ApiError::NotFound,
            ManualImportServiceError::Db(db_err) => ApiError::Db(db_err),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiErrorBody {
    status: u16,
    error: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    field_errors: Option<Vec<FieldError>>,
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode;

        let (status, error_tag, message, field_errors) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not_found", "not found".into(), None),
            ApiError::Conflict { reason } => (StatusCode::CONFLICT, "conflict", reason, None),
            ApiError::Validation { errors } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation",
                "Validation failed".into(),
                Some(errors),
            ),
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "unauthorized".into(),
                None,
            ),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "forbidden", "forbidden".into(), None),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "bad_request", msg, None),
            ApiError::BadGateway(msg) => (StatusCode::BAD_GATEWAY, "bad_gateway", msg, None),
            ApiError::StructuredBadGateway { body } => {
                return (StatusCode::BAD_GATEWAY, axum::Json(body)).into_response();
            }
            ApiError::PayloadTooLarge { max_bytes } => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                format!("request body exceeds maximum size ({max_bytes} bytes)"),
                None,
            ),
            ApiError::ServiceUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                "service unavailable".into(),
                None,
            ),
            ApiError::NotImplemented => (
                StatusCode::NOT_IMPLEMENTED,
                "not_implemented",
                "not implemented".into(),
                None,
            ),
            ApiError::Internal(ref e) => {
                tracing::error!("internal error: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "Something went wrong".into(),
                    None,
                )
            }
            ApiError::Auth(e) => auth_error_to_http(e),
            ApiError::Download(e) => {
                tracing::warn!("download error: {e}");
                (
                    StatusCode::BAD_GATEWAY,
                    "bad_gateway",
                    "Download client error — check server logs".into(),
                    None,
                )
            }
            ApiError::Import(e) => {
                tracing::error!("import error: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "Import failed — check server logs".into(),
                    None,
                )
            }
            ApiError::Metadata(e) => {
                tracing::warn!("metadata error: {e}");
                (
                    StatusCode::BAD_GATEWAY,
                    "bad_gateway",
                    "Metadata provider error — check server logs".into(),
                    None,
                )
            }
            ApiError::Enrichment(e) => {
                tracing::warn!("enrichment error: {e}");
                (
                    StatusCode::BAD_GATEWAY,
                    "bad_gateway",
                    "Enrichment error — check server logs".into(),
                    None,
                )
            }
            ApiError::TagWrite(e) => {
                tracing::error!("tag write error: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "Tag write failed — check server logs".into(),
                    None,
                )
            }
            ApiError::Scan(e) => {
                tracing::error!("scan error: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "Scan failed — check server logs".into(),
                    None,
                )
            }
            ApiError::Db(e) => db_error_to_http(e),
        };

        let body = ApiErrorBody {
            status: status.as_u16(),
            error: error_tag.to_string(),
            message,
            field_errors,
        };

        (status, axum::Json(body)).into_response()
    }
}

fn auth_error_to_http(
    e: AuthError,
) -> (
    axum::http::StatusCode,
    &'static str,
    String,
    Option<Vec<FieldError>>,
) {
    use axum::http::StatusCode;
    let msg = e.to_string();
    match e {
        AuthError::InvalidCredentials => (StatusCode::UNAUTHORIZED, "unauthorized", msg, None),
        AuthError::AccountLocked => (StatusCode::FORBIDDEN, "forbidden", msg, None),
        AuthError::SetupCompleted | AuthError::SetupRequired => {
            (StatusCode::CONFLICT, "conflict", msg, None)
        }
        AuthError::CannotDeleteSelf | AuthError::LastAdmin | AuthError::UsernameTaken => {
            (StatusCode::CONFLICT, "conflict", msg, None)
        }
        AuthError::UserNotFound => (StatusCode::NOT_FOUND, "not_found", msg, None),
        AuthError::InvalidUsername { .. } | AuthError::InvalidPassword { .. } => {
            (StatusCode::UNPROCESSABLE_ENTITY, "validation", msg, None)
        }
        AuthError::SessionExpired => (StatusCode::UNAUTHORIZED, "unauthorized", msg, None),
        AuthError::Forbidden => (StatusCode::FORBIDDEN, "forbidden", msg, None),
        AuthError::Db(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "Something went wrong".into(),
            None,
        ),
    }
}

fn db_error_to_http(
    e: DbError,
) -> (
    axum::http::StatusCode,
    &'static str,
    String,
    Option<Vec<FieldError>>,
) {
    use axum::http::StatusCode;
    let msg = e.to_string();
    match e {
        DbError::NotFound { .. } => (StatusCode::NOT_FOUND, "not_found", msg, None),
        DbError::Constraint { .. } => (StatusCode::CONFLICT, "conflict", msg, None),
        DbError::Conflict { .. } => (StatusCode::CONFLICT, "conflict", msg, None),
        DbError::DataCorruption { .. } => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "data_corruption",
            "Internal data inconsistency detected — check server logs".into(),
            None,
        ),
        DbError::IncompatibleData { .. } => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "incompatible_data",
            "Database contains data from a newer version — upgrade Livrarr".into(),
            None,
        ),
        DbError::Io(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "Something went wrong".into(),
            None,
        ),
    }
}
