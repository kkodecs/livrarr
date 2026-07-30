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

/// Structured `details` object for the error envelope (identity-edit r4 API
/// error contract): a stable machine-readable `code`, plus the same-user
/// collision owner when applicable.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorDetails {
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owning_work_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owning_work_title: Option<String>,
}

impl ErrorDetails {
    pub fn code(code: &'static str) -> Self {
        Self {
            code,
            owning_work_id: None,
            owning_work_title: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("not found")]
    NotFound,
    #[error("conflict: {reason}")]
    Conflict { reason: String },
    /// 409 with the structured `details` object (stable code + optional
    /// collision owner) — the identity-edit door contract.
    #[error("conflict: {message}")]
    ConflictDetailed {
        message: String,
        details: ErrorDetails,
    },
    /// 422 with a caller-facing message (typed classification failures).
    #[error("{0}")]
    Unprocessable(String),
    /// 503 with a stable retryable code and a `Retry-After` header.
    #[error("service unavailable: {code}")]
    ServiceUnavailableRetry {
        code: &'static str,
        retry_after_secs: u64,
    },
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

impl From<livrarr_domain::services::ServiceError> for ApiError {
    fn from(e: livrarr_domain::services::ServiceError) -> Self {
        use livrarr_domain::services::ServiceError;
        match e {
            ServiceError::NotFound => ApiError::NotFound,
            ServiceError::Db(db_err) => ApiError::Db(db_err),
            ServiceError::Internal(msg) => ApiError::Internal(msg),
        }
    }
}

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

impl From<livrarr_domain::AuthorLinkError> for ApiError {
    fn from(e: livrarr_domain::AuthorLinkError) -> Self {
        use livrarr_domain::AuthorLinkError;
        match e {
            AuthorLinkError::NotFound => ApiError::NotFound,
            // The route is real and someone else's: the caller's request is
            // answerable, just not as asked.
            AuthorLinkError::RouteOwnedByOtherAuthor(author_id) => ApiError::Conflict {
                reason: format!("author route is already held by author {author_id}"),
            },
            // A lost claim means the evidence this action was about has moved on.
            AuthorLinkError::ClaimLost => ApiError::Conflict {
                reason: "author link state changed while this request was in flight".to_string(),
            },
            AuthorLinkError::InvalidRoute(message) => ApiError::BadRequest(message),
            AuthorLinkError::Database(message) => ApiError::Internal(message),
            AuthorLinkError::Provider(error) => ApiError::BadGateway(format!("{error:?}")),
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
            SeriesServiceError::MissingGoodreadsRoute => {
                ApiError::Unprocessable("missing active Goodreads author route".into())
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
            WorkServiceError::Validation(msg) => ApiError::BadRequest(msg),
            WorkServiceError::Enrichment(msg) => ApiError::Internal(msg),
            WorkServiceError::Cover(msg) => ApiError::Internal(msg),
            WorkServiceError::Db(db_err) => ApiError::Db(db_err),
            WorkServiceError::MergeChoiceRequired(fields) => ApiError::Conflict {
                reason: format!(
                    "merge requires an explicit choice for: {}",
                    fields
                        .iter()
                        .map(|f| format!("{f:?}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            },
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
            ImportWorkflowError::ImportFailed(msg) => ApiError::Internal(msg),
            ImportWorkflowError::TagWriteFailed(msg) => ApiError::Internal(msg),
            ImportWorkflowError::PathCollision(path) => ApiError::Conflict {
                reason: format!("{path} is already claimed by a different work"),
            },
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

impl From<livrarr_domain::services::CrossFormatError> for ApiError {
    fn from(e: livrarr_domain::services::CrossFormatError) -> Self {
        use livrarr_domain::services::CrossFormatError;
        match e {
            // All three map to NotFound: the reader treats 404 as "no
            // cross-format for this item" (REQ-007/REQ-008 silent fallback);
            // stale/unreadable details go to the log.
            CrossFormatError::NotLinked => ApiError::NotFound,
            CrossFormatError::LinkStale => {
                tracing::warn!("cross-format link stale — treating as absent");
                ApiError::NotFound
            }
            CrossFormatError::KashUnreadable => {
                tracing::warn!("kash sidecar unreadable — treating link as absent");
                ApiError::NotFound
            }
            CrossFormatError::Db(msg) => ApiError::Internal(msg),
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

impl From<livrarr_domain::identity_edit::IdentityEditError> for ApiError {
    fn from(e: livrarr_domain::identity_edit::IdentityEditError) -> Self {
        use livrarr_domain::identity_edit::IdentityEditError;
        match e {
            IdentityEditError::InvalidValue(message) => ApiError::Unprocessable(message),
            IdentityEditError::StalePreview => ApiError::ConflictDetailed {
                message: "identity changed or the preview expired — preview again".into(),
                details: ErrorDetails::code("preview_required"),
            },
            IdentityEditError::Collision {
                owning_work_id,
                owning_work_title,
            } => ApiError::ConflictDetailed {
                message: format!(
                    "this identifier already belongs to \"{owning_work_title}\" — merge the works instead"
                ),
                details: ErrorDetails {
                    code: "anchor_collision",
                    owning_work_id: Some(owning_work_id),
                    owning_work_title: Some(owning_work_title),
                },
            },
            IdentityEditError::NotFound | IdentityEditError::EmptySlot => ApiError::NotFound,
            IdentityEditError::Capacity { retry_after_secs } => {
                ApiError::ServiceUnavailableRetry {
                    code: "preview_capacity",
                    retry_after_secs,
                }
            }
            IdentityEditError::Unavailable => ApiError::ServiceUnavailable,
            IdentityEditError::Db(msg) => ApiError::Internal(msg),
        }
    }
}

impl From<livrarr_domain::services::ConflictError> for ApiError {
    fn from(e: livrarr_domain::services::ConflictError) -> Self {
        use livrarr_domain::services::ConflictError;
        match e {
            ConflictError::NotFound => ApiError::NotFound,
            ConflictError::AlreadyResolved => ApiError::Conflict {
                reason: e.to_string(),
            },
            // A lost first-statement generation claim: the conflict was open
            // at the door read, but a different identity mutation won.
            ConflictError::StaleIdentity => ApiError::ConflictDetailed {
                message: "identity changed; reload identity conflicts".into(),
                details: ErrorDetails::code("identity_conflict_stale"),
            },
            ConflictError::InvalidPrimaryAnchor => ApiError::BadRequest(e.to_string()),
            ConflictError::Db(msg) => {
                tracing::error!("conflict db error: {msg}");
                ApiError::Internal("Something went wrong".to_string())
            }
            e => {
                tracing::error!("conflict error: {e}");
                ApiError::Internal("Something went wrong".to_string())
            }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<ErrorDetails>,
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode;

        let mut details = None;
        let mut retry_after: Option<u64> = None;
        let (status, error_tag, message, field_errors) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not_found", "not found".into(), None),
            ApiError::Conflict { reason } => (StatusCode::CONFLICT, "conflict", reason, None),
            ApiError::ConflictDetailed {
                message,
                details: d,
            } => {
                details = Some(d);
                (StatusCode::CONFLICT, "conflict", message, None)
            }
            ApiError::Unprocessable(message) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation",
                message,
                None,
            ),
            ApiError::ServiceUnavailableRetry {
                code,
                retry_after_secs,
            } => {
                details = Some(ErrorDetails::code(code));
                retry_after = Some(retry_after_secs);
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "service_unavailable",
                    "service temporarily at capacity — retry shortly".into(),
                    None,
                )
            }
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
            details,
        };

        let mut response = (status, axum::Json(body)).into_response();
        if let Some(secs) = retry_after {
            if let Ok(value) = axum::http::HeaderValue::from_str(&secs.to_string()) {
                response
                    .headers_mut()
                    .insert(axum::http::header::RETRY_AFTER, value);
            }
        }
        response
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
        DbError::ClaimLost => (StatusCode::CONFLICT, "conflict", msg, None),
        DbError::IdentityCollision { .. } => (StatusCode::CONFLICT, "conflict", msg, None),
        DbError::LastAdmin => (StatusCode::CONFLICT, "conflict", msg, None),
        DbError::DataCorruption { .. } => {
            tracing::error!("DB data corruption: {msg}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "data_corruption",
                "Internal data inconsistency detected — check server logs".into(),
                None,
            )
        }
        DbError::IncompatibleData { .. } => {
            tracing::error!("DB incompatible data: {msg}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "incompatible_data",
                "Database contains data from a newer version — upgrade Livrarr".into(),
                None,
            )
        }
        DbError::Io(ref source) => {
            tracing::error!("DB I/O error: {source}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "Something went wrong".into(),
                None,
            )
        }
    }
}
