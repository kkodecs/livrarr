pub use livrarr_domain::*;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[cfg(test)]
pub mod api_secondary_impl;
pub mod auth_crypto;
pub mod auth_impl;
pub mod auth_service;
pub mod config;
pub mod handlers;
pub mod jobs;
pub mod middleware;
pub mod router;
pub mod state;

// Re-export DB types used in API trait signatures.
pub use livrarr_db::HistoryFilter;

/// Shared pagination query params — default page=1, page_size=50.
#[derive(Debug, serde::Deserialize)]
pub struct PaginationQuery {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

impl PaginationQuery {
    pub fn page(&self) -> u32 {
        self.page.unwrap_or(1).max(1)
    }
    pub fn page_size(&self) -> u32 {
        self.page_size.unwrap_or(50).clamp(1, 500)
    }
}

/// Generic paginated response wrapper.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedResponse<T: Serialize> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

// Forward-declare error types from other crates that ApiError wraps.
// These are defined here as local types so the server crate compiles
// without depending on every service crate. The real composition root
// will use the actual types.

/// Placeholder for download error (from livrarr-download).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct DownloadError(pub String);

/// Placeholder for import error (from livrarr-organize).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ImportError(pub String);

/// Placeholder for metadata error (from livrarr-metadata).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct MetadataError(pub String);

/// Placeholder for enrichment error (from livrarr-metadata).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct EnrichmentError(pub String);

/// Placeholder for tag write error (from livrarr-tagwrite).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct TagWriteError(pub String);

/// Placeholder for scan error (from livrarr-organize).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ScanError(pub String);

// =============================================================================
// CRATE: livrarr-server
// =============================================================================
// Axum HTTP server, routes, auth middleware, composition root.

// ---------------------------------------------------------------------------
// Auth Middleware
// ---------------------------------------------------------------------------

pub use auth_impl::{AuthMiddleware, TestRequest, TestRequestKind};

/// Authenticated user context -- extracted by middleware, available in handlers.
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub user: User,
    pub auth_type: AuthType,
    /// Token hash for session-based auth (needed for logout). None for API key auth.
    pub session_token_hash: Option<String>,
}

// Axum extractor: pull AuthContext from request extensions (set by auth middleware).
impl<S: Send + Sync> axum::extract::FromRequestParts<S> for AuthContext {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(
            parts
                .extensions
                .remove::<AuthContext>()
                .ok_or(ApiError::Unauthorized),
        )
    }
}

// ---------------------------------------------------------------------------
// Auth Service
// ---------------------------------------------------------------------------

/// Auth operations (login, logout, setup, user management).
#[trait_variant::make(Send)]
pub trait AuthService: Send + Sync {
    /// Login with username + password. Returns session token (plaintext, shown once).
    async fn login(&self, req: LoginRequest) -> Result<LoginResponse, AuthError>;

    /// Logout. Deletes session.
    async fn logout(&self, token_hash: &str) -> Result<(), AuthError>;

    /// Complete setup wizard.
    async fn complete_setup(&self, req: SetupRequest) -> Result<SetupResponse, AuthError>;

    /// Get current user info + auth mechanism.
    async fn get_current_user(&self, auth: &AuthContext) -> Result<AuthMeResponse, AuthError>;

    /// Update own profile (username/password).
    async fn update_profile(
        &self,
        user_id: UserId,
        req: UpdateProfileRequest,
    ) -> Result<UserResponse, AuthError>;

    /// Regenerate own API key. Returns new key (plaintext, shown once).
    async fn regenerate_api_key(&self, user_id: UserId) -> Result<ApiKeyResponse, AuthError>;

    /// Create user (admin only).
    async fn create_user(&self, req: AdminCreateUserRequest) -> Result<UserResponse, AuthError>;

    /// List users (admin only).
    async fn list_users(&self) -> Result<Vec<UserResponse>, AuthError>;

    /// Get user by ID (admin only).
    async fn get_user(&self, id: UserId) -> Result<UserResponse, AuthError>;

    /// Update user (admin only).
    async fn update_user(
        &self,
        id: UserId,
        req: AdminUpdateUserRequest,
    ) -> Result<UserResponse, AuthError>;

    /// Delete user (admin only).
    async fn delete_user(
        &self,
        requesting_user_id: UserId,
        target_user_id: UserId,
    ) -> Result<(), AuthError>;

    /// Regenerate API key for another user (admin only).
    async fn regenerate_user_api_key(&self, user_id: UserId) -> Result<ApiKeyResponse, AuthError>;
}

// ---------------------------------------------------------------------------
// Auth Request/Response Types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    pub remember_me: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupResponse {
    pub api_key: String,
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatusResponse {
    pub setup_required: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProfileRequest {
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyResponse {
    pub api_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminCreateUserRequest {
    pub username: String,
    pub password: String,
    pub role: UserRole,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminUpdateUserRequest {
    pub username: Option<String>,
    pub password: Option<String>,
    pub role: Option<UserRole>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserResponse {
    pub id: UserId,
    pub username: String,
    pub role: UserRole,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthMeResponse {
    pub user: UserResponse,
    pub auth_type: AuthType,
}

// ---------------------------------------------------------------------------
// AuthError
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("account locked")]
    AccountLocked,
    #[error("setup already completed")]
    SetupCompleted,
    #[error("setup required")]
    SetupRequired,
    #[error("cannot delete self")]
    CannotDeleteSelf,
    #[error("cannot remove last admin")]
    LastAdmin,
    #[error("user not found")]
    UserNotFound,
    #[error("username already taken")]
    UsernameTaken,
    #[error("invalid username: {reason}")]
    InvalidUsername { reason: String },
    #[error("invalid password: {reason}")]
    InvalidPassword { reason: String },
    #[error("session expired")]
    SessionExpired,
    #[error("forbidden")]
    Forbidden,
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

// ---------------------------------------------------------------------------
// Work API
// ---------------------------------------------------------------------------

/// Work search result for user display (from metadata crate, re-defined here for API).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSearchResult {
    pub ol_key: Option<String>,
    pub title: String,
    pub author_name: String,
    pub author_ol_key: Option<String>,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_position: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<String>,
}

/// Work management operations.
#[trait_variant::make(Send)]
pub trait WorkApi: Send + Sync {
    /// Search OL for works.
    async fn lookup(&self, user_id: UserId, term: &str) -> Result<Vec<WorkSearchResult>, ApiError>;

    /// Add work to user's library. Triggers enrichment.
    async fn add(&self, user_id: UserId, req: AddWorkRequest) -> Result<AddWorkResponse, ApiError>;

    /// List user's works.
    async fn list(&self, user_id: UserId) -> Result<Vec<WorkDetailResponse>, ApiError>;

    /// Get work detail.
    async fn get(&self, user_id: UserId, id: WorkId) -> Result<WorkDetailResponse, ApiError>;

    /// Update user-editable fields.
    async fn update(
        &self,
        user_id: UserId,
        id: WorkId,
        req: UpdateWorkRequest,
    ) -> Result<WorkDetailResponse, ApiError>;

    /// Upload custom cover.
    async fn upload_cover(
        &self,
        user_id: UserId,
        id: WorkId,
        image_data: &[u8],
        content_type: &str,
    ) -> Result<(), ApiError>;

    /// Delete work.
    async fn delete(
        &self,
        user_id: UserId,
        id: WorkId,
        delete_files: bool,
    ) -> Result<DeleteWorkResponse, ApiError>;

    /// Re-enrich single work (synchronous).
    async fn refresh(&self, user_id: UserId, id: WorkId) -> Result<RefreshWorkResponse, ApiError>;
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddWorkRequest {
    pub ol_key: Option<String>,
    pub title: String,
    pub author_name: String,
    pub author_ol_key: Option<String>,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddWorkResponse {
    pub work: WorkDetailResponse,
    pub author_created: bool,
    pub messages: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshWorkResponse {
    pub work: WorkDetailResponse,
    pub messages: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWorkRequest {
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkDetailResponse {
    pub id: WorkId,
    pub title: String,
    pub sort_title: Option<String>,
    pub subtitle: Option<String>,
    pub original_title: Option<String>,
    pub author_name: String,
    pub author_id: Option<AuthorId>,
    pub description: Option<String>,
    pub year: Option<i32>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
    pub genres: Option<Vec<String>>,
    pub language: Option<String>,
    pub page_count: Option<i32>,
    pub duration_seconds: Option<i32>,
    pub publisher: Option<String>,
    pub publish_date: Option<String>,
    pub ol_key: Option<String>,
    pub hardcover_id: Option<String>,
    pub isbn_13: Option<String>,
    pub asin: Option<String>,
    pub narrator: Option<Vec<String>>,
    pub narration_type: Option<NarrationType>,
    pub abridged: bool,
    pub rating: Option<f64>,
    pub rating_count: Option<i32>,
    pub enrichment_status: EnrichmentStatus,
    pub enriched_at: Option<String>,
    pub enrichment_source: Option<String>,
    pub cover_manual: bool,
    pub monitored: bool,
    pub added_at: String,
    pub library_items: Vec<LibraryItemResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItemResponse {
    pub id: LibraryItemId,
    pub path: String,
    pub media_type: MediaType,
    pub file_size: i64,
    pub imported_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteWorkResponse {
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Author API
// ---------------------------------------------------------------------------

/// Author search result for user display.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorSearchResult {
    pub ol_key: String,
    pub name: String,
    pub sort_name: Option<String>,
}

/// Author management operations.
#[trait_variant::make(Send)]
pub trait AuthorApi: Send + Sync {
    /// Search OL for authors.
    async fn lookup(
        &self,
        user_id: UserId,
        term: &str,
    ) -> Result<Vec<AuthorSearchResult>, ApiError>;

    /// Add author (from OL result).
    async fn add(&self, user_id: UserId, req: AddAuthorRequest)
        -> Result<AuthorResponse, ApiError>;

    /// List user's authors.
    async fn list(&self, user_id: UserId) -> Result<Vec<AuthorResponse>, ApiError>;

    /// Get author detail with works.
    async fn get(&self, user_id: UserId, id: AuthorId) -> Result<AuthorDetailResponse, ApiError>;

    /// Update author (monitoring settings).
    async fn update(
        &self,
        user_id: UserId,
        id: AuthorId,
        req: UpdateAuthorApiRequest,
    ) -> Result<AuthorResponse, ApiError>;

    /// Delete author.
    async fn delete(&self, user_id: UserId, id: AuthorId) -> Result<(), ApiError>;
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddAuthorRequest {
    pub name: String,
    pub sort_name: Option<String>,
    pub ol_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAuthorApiRequest {
    pub monitored: Option<bool>,
    pub monitor_new_items: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorResponse {
    pub id: AuthorId,
    pub name: String,
    pub sort_name: Option<String>,
    pub ol_key: Option<String>,
    pub monitored: bool,
    pub monitor_new_items: bool,
    pub added_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorDetailResponse {
    pub author: AuthorResponse,
    pub works: Vec<WorkDetailResponse>,
}

// ---------------------------------------------------------------------------
// Notification API
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationResponse {
    pub id: NotificationId,
    pub notification_type: NotificationType,
    pub ref_key: Option<String>,
    pub message: String,
    pub data: serde_json::Value,
    pub read: bool,
    pub created_at: String,
}

/// Notification management.
#[trait_variant::make(Send)]
pub trait NotificationApi: Send + Sync {
    /// List notifications for a user.
    async fn list(
        &self,
        user_id: UserId,
        unread_only: bool,
    ) -> Result<Vec<NotificationResponse>, ApiError>;

    /// Mark notification as read.
    async fn mark_read(&self, user_id: UserId, id: NotificationId) -> Result<(), ApiError>;

    /// Dismiss notification.
    async fn dismiss(&self, user_id: UserId, id: NotificationId) -> Result<(), ApiError>;

    /// Dismiss all notifications for a user.
    async fn dismiss_all(&self, user_id: UserId) -> Result<(), ApiError>;
}

// ---------------------------------------------------------------------------
// Root Folder API
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootFolderResponse {
    pub id: RootFolderId,
    pub path: String,
    pub media_type: MediaType,
    pub free_space: Option<i64>,
    pub total_space: Option<i64>,
}

/// Root folder management (admin write, all read).
#[trait_variant::make(Send)]
pub trait RootFolderApi: Send + Sync {
    /// List root folders with free/total space.
    async fn list(&self) -> Result<Vec<RootFolderResponse>, ApiError>;

    /// Create root folder with validation.
    async fn create(
        &self,
        path: &str,
        media_type: MediaType,
    ) -> Result<RootFolderResponse, ApiError>;

    /// Get root folder by ID.
    async fn get(&self, id: RootFolderId) -> Result<RootFolderResponse, ApiError>;

    /// Delete root folder. 409 if library items exist.
    async fn delete(&self, id: RootFolderId) -> Result<(), ApiError>;
}

// ---------------------------------------------------------------------------
// Download Client API
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadClientResponse {
    pub id: DownloadClientId,
    pub name: String,
    pub implementation: DownloadClientImplementation,
    pub host: String,
    pub port: u16,
    pub use_ssl: bool,
    pub skip_ssl_validation: bool,
    pub url_base: Option<String>,
    pub username: Option<String>,
    // password intentionally omitted
    pub category: String,
    pub enabled: bool,
    pub client_type: String,
    pub api_key_set: bool,
    pub is_default_for_protocol: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDownloadClientApiRequest {
    pub name: String,
    pub implementation: DownloadClientImplementation,
    pub host: String,
    pub port: u16,
    pub use_ssl: bool,
    pub skip_ssl_validation: bool,
    pub url_base: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub category: String,
    pub enabled: bool,
    pub api_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDownloadClientApiRequest {
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub use_ssl: Option<bool>,
    pub skip_ssl_validation: Option<bool>,
    pub url_base: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub category: Option<String>,
    pub enabled: Option<bool>,
    pub api_key: Option<String>,
    pub is_default_for_protocol: Option<bool>,
}

/// Download client configuration management (admin write, all read).
#[trait_variant::make(Send)]
pub trait DownloadClientApi: Send + Sync {
    /// List download clients. Passwords omitted from response.
    async fn list(&self) -> Result<Vec<DownloadClientResponse>, ApiError>;

    /// Create download client with validation.
    async fn create(
        &self,
        req: CreateDownloadClientApiRequest,
    ) -> Result<DownloadClientResponse, ApiError>;

    /// Get download client. Password omitted.
    async fn get(&self, id: DownloadClientId) -> Result<DownloadClientResponse, ApiError>;

    /// Update download client.
    async fn update(
        &self,
        id: DownloadClientId,
        req: UpdateDownloadClientApiRequest,
    ) -> Result<DownloadClientResponse, ApiError>;

    /// Delete download client.
    async fn delete(&self, id: DownloadClientId) -> Result<(), ApiError>;

    /// Test connection without persisting config.
    async fn test(&self, req: CreateDownloadClientApiRequest) -> Result<(), ApiError>;
}

// ---------------------------------------------------------------------------
// Indexer API
// ---------------------------------------------------------------------------

fn default_api_path() -> String {
    "/api".to_string()
}

fn default_categories() -> Vec<i32> {
    vec![7020, 3030]
}

fn default_priority() -> i32 {
    25
}

fn default_true() -> bool {
    true
}

fn default_torrent_protocol() -> String {
    "torrent".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIndexerApiRequest {
    pub name: String,
    #[serde(default = "default_torrent_protocol")]
    pub protocol: String,
    pub url: String,
    #[serde(default = "default_api_path")]
    pub api_path: String,
    pub api_key: Option<String>,
    #[serde(default = "default_categories")]
    pub categories: Vec<i32>,
    #[serde(default = "default_priority")]
    pub priority: i32,
    #[serde(default = "default_true")]
    pub enable_automatic_search: bool,
    #[serde(default = "default_true")]
    pub enable_interactive_search: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateIndexerApiRequest {
    pub name: Option<String>,
    pub url: Option<String>,
    pub api_path: Option<String>,
    pub api_key: Option<String>,
    pub categories: Option<Vec<i32>>,
    pub priority: Option<i32>,
    pub enable_automatic_search: Option<bool>,
    pub enable_interactive_search: Option<bool>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexerResponse {
    pub id: IndexerId,
    pub name: String,
    pub protocol: String,
    pub url: String,
    pub api_path: String,
    pub api_key_set: bool,
    pub categories: Vec<i32>,
    pub priority: i32,
    pub enable_automatic_search: bool,
    pub enable_interactive_search: bool,
    pub supports_book_search: bool,
    pub enabled: bool,
    pub added_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestIndexerApiRequest {
    pub url: String,
    pub api_path: String,
    pub api_key: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestIndexerApiResponse {
    pub ok: bool,
    pub supports_book_search: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Remote Path Mapping API
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePathMappingResponse {
    pub id: RemotePathMappingId,
    pub host: String,
    pub remote_path: String,
    pub local_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRemotePathMappingRequest {
    pub host: Option<String>,
    pub remote_path: Option<String>,
    pub local_path: Option<String>,
}

/// Remote path mapping management (admin only).
#[trait_variant::make(Send)]
pub trait RemotePathMappingApi: Send + Sync {
    async fn list(&self) -> Result<Vec<RemotePathMappingResponse>, ApiError>;

    /// Create mapping. Both paths must end with `/` (422 otherwise).
    async fn create(
        &self,
        host: &str,
        remote_path: &str,
        local_path: &str,
    ) -> Result<RemotePathMappingResponse, ApiError>;

    async fn get(&self, id: RemotePathMappingId) -> Result<RemotePathMappingResponse, ApiError>;

    /// Update mapping.
    async fn update(
        &self,
        id: RemotePathMappingId,
        req: UpdateRemotePathMappingRequest,
    ) -> Result<RemotePathMappingResponse, ApiError>;

    async fn delete(&self, id: RemotePathMappingId) -> Result<(), ApiError>;
}

// ---------------------------------------------------------------------------
// Config API
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamingConfigResponse {
    pub author_folder_format: String,
    pub book_folder_format: String,
    pub rename_files: bool,
    pub replace_illegal_chars: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaManagementConfigResponse {
    pub cwa_ingest_path: Option<String>,
    pub preferred_ebook_formats: Vec<String>,
    pub preferred_audiobook_formats: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProwlarrConfigResponse {
    pub url: Option<String>,
    pub api_key_set: bool,
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataConfigResponse {
    pub hardcover_enabled: bool,
    pub hardcover_api_token_set: bool,
    pub llm_enabled: bool,
    pub llm_provider: Option<LlmProvider>,
    pub llm_endpoint: Option<String>,
    pub llm_api_key_set: bool,
    pub llm_model: Option<String>,
    pub audnexus_url: String,
    pub languages: Vec<String>,
    #[serde(skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub provider_status: std::collections::HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestProwlarrRequest {
    pub url: String,
    pub api_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProwlarrImportRequest {
    pub url: String,
    pub api_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProwlarrImportResponse {
    pub imported: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// Grab API Request
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrabApiRequest {
    pub work_id: WorkId,
    pub download_url: String,
    pub title: String,
    pub indexer: String,
    pub guid: String,
    pub size: i64,
    pub download_client_id: Option<DownloadClientId>,
    pub protocol: Option<String>,
    #[serde(default)]
    pub categories: Vec<i32>,
}

// ---------------------------------------------------------------------------
// Scan Result (stub)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub matched: i64,
    pub unmatched: Vec<ScanUnmatchedFile>,
    pub errors: Vec<ScanErrorEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanUnmatchedFile {
    pub path: String,
    pub media_type: MediaType,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanErrorEntry {
    pub path: String,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Root Folder API Request
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRootFolderRequest {
    pub path: String,
    pub media_type: MediaType,
}

// ---------------------------------------------------------------------------
// Remote Path Mapping API Request
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRemotePathMappingApiRequest {
    pub host: String,
    pub remote_path: String,
    pub local_path: String,
}

// ---------------------------------------------------------------------------
// Config API Requests (with serde for JSON deserialization)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMediaManagementApiRequest {
    pub cwa_ingest_path: Option<String>,
    #[serde(default)]
    pub preferred_ebook_formats: Vec<String>,
    #[serde(default)]
    pub preferred_audiobook_formats: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProwlarrApiRequest {
    pub url: Option<String>,
    pub api_key: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailConfigResponse {
    pub enabled: bool,
    pub smtp_host: String,
    pub smtp_port: i32,
    pub encryption: String,
    pub username: Option<String>,
    pub password_set: bool,
    pub from_address: Option<String>,
    pub recipient_email: Option<String>,
    pub send_on_import: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateEmailApiRequest {
    pub enabled: Option<bool>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i32>,
    pub encryption: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from_address: Option<String>,
    pub recipient_email: Option<String>,
    pub send_on_import: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendEmailRequest {
    pub library_item_id: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMetadataApiRequest {
    pub hardcover_enabled: Option<bool>,
    pub hardcover_api_token: Option<String>,
    pub llm_enabled: Option<bool>,
    pub llm_provider: Option<LlmProvider>,
    pub llm_endpoint: Option<String>,
    pub llm_api_key: Option<String>,
    pub llm_model: Option<String>,
    pub audnexus_url: Option<String>,
    pub languages: Option<Vec<String>>,
}

/// Configuration management (admin only).
#[trait_variant::make(Send)]
pub trait ConfigApi: Send + Sync {
    /// Get naming config (read-only).
    async fn get_naming(&self) -> Result<NamingConfigResponse, ApiError>;

    /// Get media management config.
    async fn get_media_management(&self) -> Result<MediaManagementConfigResponse, ApiError>;

    /// Update media management config.
    async fn update_media_management(
        &self,
        req: UpdateMediaManagementApiRequest,
    ) -> Result<MediaManagementConfigResponse, ApiError>;

    /// Get Prowlarr config.
    async fn get_prowlarr(&self) -> Result<ProwlarrConfigResponse, ApiError>;

    /// Update Prowlarr config.
    async fn update_prowlarr(
        &self,
        req: UpdateProwlarrApiRequest,
    ) -> Result<ProwlarrConfigResponse, ApiError>;

    /// Test Prowlarr connection without persisting.
    async fn test_prowlarr(&self, req: &TestProwlarrRequest) -> Result<(), ApiError>;

    /// Get metadata config. Secrets redacted in response.
    async fn get_metadata(&self) -> Result<MetadataConfigResponse, ApiError>;

    /// Update metadata config.
    async fn update_metadata(
        &self,
        req: UpdateMetadataApiRequest,
    ) -> Result<MetadataConfigResponse, ApiError>;

    /// Get email config. Password redacted in response.
    async fn get_email(&self) -> Result<EmailConfigResponse, ApiError>;

    /// Update email config.
    async fn update_email(
        &self,
        req: UpdateEmailApiRequest,
    ) -> Result<EmailConfigResponse, ApiError>;
}

// ---------------------------------------------------------------------------
// System API
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheckResult {
    pub source: String,
    pub check_type: HealthCheckType,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatus {
    pub version: String,
    pub os_info: String,
    pub data_directory: String,
    pub log_file: String,
    pub startup_time: DateTime<Utc>,
    pub log_level: String,
}

/// System and health endpoints.
#[trait_variant::make(Send)]
pub trait SystemApi: Send + Sync {
    /// Health check -- returns list of checks against all configured external dependencies.
    async fn health(&self) -> Result<Vec<HealthCheckResult>, ApiError>;

    /// System status -- version, OS, paths, startup time.
    async fn status(&self) -> Result<SystemStatus, ApiError>;
}

// ---------------------------------------------------------------------------
// Library File API
// ---------------------------------------------------------------------------

/// Library file (workfile) management.
#[trait_variant::make(Send)]
pub trait LibraryFileApi: Send + Sync {
    /// List user's library files.
    async fn list(&self, user_id: UserId) -> Result<Vec<LibraryItemResponse>, ApiError>;

    /// Get single library file.
    async fn get(
        &self,
        user_id: UserId,
        id: LibraryItemId,
    ) -> Result<LibraryItemResponse, ApiError>;

    /// Delete library file.
    async fn delete(&self, user_id: UserId, id: LibraryItemId) -> Result<(), ApiError>;
}

// ---------------------------------------------------------------------------
// History API
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryResponse {
    pub id: HistoryId,
    pub work_id: Option<WorkId>,
    pub event_type: EventType,
    pub data: serde_json::Value,
    pub date: String,
}

/// History event log.
#[trait_variant::make(Send)]
pub trait HistoryApi: Send + Sync {
    /// List history events with optional filters.
    async fn list(
        &self,
        user_id: UserId,
        target_user_id: Option<UserId>,
        filter: HistoryFilter,
    ) -> Result<Vec<HistoryResponse>, ApiError>;
}

// ---------------------------------------------------------------------------
// Queue API Responses
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItemResponse {
    pub id: GrabId,
    pub title: String,
    pub status: GrabStatus,
    pub size: Option<i64>,
    pub media_type: Option<MediaType>,
    pub indexer: String,
    pub download_client: String,
    pub work_id: WorkId,
    pub protocol: String,
    pub error: Option<String>,
    pub grabbed_at: String,
    /// Live progress from download client (only for active grabs).
    pub progress: Option<QueueProgress>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueProgress {
    pub percent: f64,
    pub eta: Option<i64>,
    pub download_status: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueListResponse {
    pub items: Vec<QueueItemResponse>,
    pub total: i64,
    pub page: u32,
    pub per_page: u32,
}

// ---------------------------------------------------------------------------
// Release API Responses
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseResponse {
    pub title: String,
    pub indexer: String,
    pub size: i64,
    pub guid: String,
    pub download_url: String,
    pub seeders: Option<i32>,
    pub leechers: Option<i32>,
    pub publish_date: Option<String>,
    pub protocol: String,
    pub categories: Vec<i32>,
}

/// Release search response — wraps results with optional warnings.
///
/// Satisfies: SEARCH-005, SEARCH-013
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseSearchResponse {
    pub results: Vec<ReleaseResponse>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<SearchWarning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_age_seconds: Option<u64>,
}

/// Warning for a failed indexer during search.
///
/// Satisfies: SEARCH-007, SEARCH-013
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchWarning {
    pub indexer: String,
    pub error: String,
}

// ---------------------------------------------------------------------------
// Unified API Error
// ---------------------------------------------------------------------------

/// Field-level validation error for 422 responses.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

/// API error type -- maps to HTTP status codes.
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

/// JSON error response body matching frontend's normalizeError expectations.
///
/// Format: { status, error, message, fieldErrors? }
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
            ApiError::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "Something went wrong".into(),
                None,
            ),
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
