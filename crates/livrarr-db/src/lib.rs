#![allow(dead_code, unused_variables, async_fn_in_trait)]

pub use livrarr_domain::*;

pub mod mem;
pub mod pool;
pub mod sqlite;
mod sqlite_author;
mod sqlite_bibliography;
pub(crate) mod sqlite_common;
mod sqlite_config;
mod sqlite_download_client;
mod sqlite_grab;
mod sqlite_history;
mod sqlite_indexer;
mod sqlite_library_item;
mod sqlite_notification;
mod sqlite_remote_path_mapping;
mod sqlite_root_folder;
mod sqlite_session;
mod sqlite_user;
mod sqlite_work;

// =============================================================================
// CRATE: livrarr-db
// =============================================================================
// All SQL queries. Trait-based data access.
// Every user-scoped query takes explicit user_id -- no unscoped queries (AUTH-003).

// ---------------------------------------------------------------------------
// User DB
// ---------------------------------------------------------------------------

/// User data access.
///
/// Satisfies: AUTH-010, AUTH-011, AUTH-012, AUTH-013
#[async_trait::async_trait]
pub trait UserDb: Send + Sync {
    /// Get user by ID.
    async fn get_user(&self, id: UserId) -> Result<User, DbError>;

    /// Get user by username (case-insensitive).
    async fn get_user_by_username(&self, username: &str) -> Result<User, DbError>;

    /// Get user by API key hash.
    ///
    /// Satisfies: AUTH-007
    async fn get_user_by_api_key_hash(&self, hash: &str) -> Result<User, DbError>;

    /// List all users.
    async fn list_users(&self) -> Result<Vec<User>, DbError>;

    /// Create user. Returns created user with generated ID.
    async fn create_user(&self, req: CreateUserDbRequest) -> Result<User, DbError>;

    /// Update user fields. Null fields mean "keep existing."
    async fn update_user(&self, id: UserId, req: UpdateUserDbRequest) -> Result<User, DbError>;

    /// Delete user by ID. Cascades to all user-scoped data.
    ///
    /// Satisfies: AUTH-011
    async fn delete_user(&self, id: UserId) -> Result<(), DbError>;

    /// Count users with admin role (for last-admin check).
    async fn count_admins(&self) -> Result<i64, DbError>;

    /// Complete setup: update placeholder admin with real credentials.
    /// Atomic conditional: only succeeds if setup_pending = true.
    ///
    /// Satisfies: AUTH-010
    async fn complete_setup(&self, req: CompleteSetupDbRequest) -> Result<User, DbError>;

    /// Update API key hash for a user.
    async fn update_api_key_hash(&self, user_id: UserId, hash: &str) -> Result<(), DbError>;
}

pub struct CreateUserDbRequest {
    pub username: String,
    pub password_hash: String,
    pub role: UserRole,
    pub api_key_hash: String,
}

pub struct UpdateUserDbRequest {
    pub username: Option<String>,
    pub password_hash: Option<String>,
    pub role: Option<UserRole>,
}

pub struct CompleteSetupDbRequest {
    pub username: String,
    pub password_hash: String,
    pub api_key_hash: String,
}

// ---------------------------------------------------------------------------
// Session DB
// ---------------------------------------------------------------------------

/// Session data access.
///
/// Satisfies: AUTH-005, AUTH-006, AUTH-014
#[async_trait::async_trait]
pub trait SessionDb: Send + Sync {
    /// Get session by token hash. Returns None if not found or expired.
    async fn get_session(&self, token_hash: &str) -> Result<Option<Session>, DbError>;

    /// Create session.
    async fn create_session(&self, session: &Session) -> Result<(), DbError>;

    /// Delete session (logout).
    async fn delete_session(&self, token_hash: &str) -> Result<(), DbError>;

    /// Extend session expiry (for rolling persistent sessions).
    ///
    /// Satisfies: AUTH-005 (debounced rolling -- extend only when <24h remaining)
    async fn extend_session(
        &self,
        token_hash: &str,
        new_expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), DbError>;

    /// Delete all expired sessions.
    ///
    /// Satisfies: AUTH-014
    async fn delete_expired_sessions(&self) -> Result<u64, DbError>;
}

// ---------------------------------------------------------------------------
// Work DB
// ---------------------------------------------------------------------------

/// Work data access. All queries scoped to user_id.
///
/// Satisfies: AUTH-003
#[async_trait::async_trait]
pub trait WorkDb: Send + Sync {
    /// Get work by ID for a specific user.
    async fn get_work(&self, user_id: UserId, id: WorkId) -> Result<Work, DbError>;

    /// List works for a user.
    async fn list_works(&self, user_id: UserId) -> Result<Vec<Work>, DbError>;

    /// Create work. Returns created work with generated ID.
    ///
    /// Satisfies: SEARCH-004
    async fn create_work(&self, req: CreateWorkDbRequest) -> Result<Work, DbError>;

    /// Update work (enrichment fields -- overwrites).
    async fn update_work_enrichment(
        &self,
        user_id: UserId,
        id: WorkId,
        req: UpdateWorkEnrichmentDbRequest,
    ) -> Result<Work, DbError>;

    /// Update user-editable fields only.
    ///
    /// Satisfies: SEARCH-013
    async fn update_work_user_fields(
        &self,
        user_id: UserId,
        id: WorkId,
        req: UpdateWorkUserFieldsDbRequest,
    ) -> Result<Work, DbError>;

    /// Set cover_manual flag.
    ///
    /// Satisfies: SEARCH-014
    async fn set_cover_manual(
        &self,
        user_id: UserId,
        id: WorkId,
        manual: bool,
    ) -> Result<(), DbError>;

    /// Delete work. Returns deleted work for file cleanup.
    async fn delete_work(&self, user_id: UserId, id: WorkId) -> Result<Work, DbError>;

    /// Check if user already has a work with given ol_key.
    ///
    /// Satisfies: SEARCH-004 (duplicate detection)
    async fn work_exists_by_ol_key(&self, user_id: UserId, ol_key: &str) -> Result<bool, DbError>;

    /// List all works for bulk re-enrichment.
    ///
    /// Satisfies: SEARCH-011
    async fn list_works_for_enrichment(&self, user_id: UserId) -> Result<Vec<Work>, DbError>;

    /// Get all works for a user by a specific author (for monitoring dedup).
    ///
    /// Satisfies: AUTHOR-002
    async fn list_works_by_author_ol_keys(
        &self,
        user_id: UserId,
        author_ol_key: &str,
    ) -> Result<Vec<String>, DbError>;

    /// Find works by normalized title + author match (for manual scan matching).
    ///
    /// Satisfies: IMPORT-017
    async fn find_by_normalized_match(
        &self,
        user_id: UserId,
        title: &str,
        author: &str,
    ) -> Result<Vec<Work>, DbError>;

    /// Reset all pending enrichments to failed (startup recovery — JOBS-003).
    async fn reset_pending_enrichments(&self) -> Result<u64, DbError>;
}

pub struct CreateWorkDbRequest {
    pub user_id: UserId,
    pub title: String,
    pub author_name: String,
    pub author_id: Option<AuthorId>,
    pub ol_key: Option<String>,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
}

pub struct UpdateWorkEnrichmentDbRequest {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub original_title: Option<String>,
    pub author_name: Option<String>,
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
    pub hardcover_id: Option<String>,
    pub isbn_13: Option<String>,
    pub asin: Option<String>,
    pub narrator: Option<Vec<String>>,
    pub narration_type: Option<NarrationType>,
    pub abridged: Option<bool>,
    pub rating: Option<f64>,
    pub rating_count: Option<i32>,
    pub enrichment_status: EnrichmentStatus,
    pub enrichment_source: Option<String>,
    pub cover_url: Option<String>,
}

pub struct UpdateWorkUserFieldsDbRequest {
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
}

// ---------------------------------------------------------------------------
// Author DB
// ---------------------------------------------------------------------------

/// Author data access.
///
/// Satisfies: AUTHOR-001, SEARCH-005
#[async_trait::async_trait]
pub trait AuthorDb: Send + Sync {
    /// Get author by ID for a user.
    async fn get_author(&self, user_id: UserId, id: AuthorId) -> Result<Author, DbError>;

    /// List authors for a user.
    async fn list_authors(&self, user_id: UserId) -> Result<Vec<Author>, DbError>;

    /// Create author.
    async fn create_author(&self, req: CreateAuthorDbRequest) -> Result<Author, DbError>;

    /// Update author (monitoring settings).
    async fn update_author(
        &self,
        user_id: UserId,
        id: AuthorId,
        req: UpdateAuthorDbRequest,
    ) -> Result<Author, DbError>;

    /// Delete author.
    async fn delete_author(&self, user_id: UserId, id: AuthorId) -> Result<(), DbError>;

    /// Find author by exact normalized name for a user (dedup).
    ///
    /// Satisfies: SEARCH-005, AUTHOR-001
    async fn find_author_by_name(
        &self,
        user_id: UserId,
        normalized_name: &str,
    ) -> Result<Option<Author>, DbError>;

    /// List monitored authors with ol_key (for author monitoring job).
    ///
    /// Satisfies: AUTHOR-002
    async fn list_monitored_authors(&self) -> Result<Vec<Author>, DbError>;
}

pub struct CreateAuthorDbRequest {
    pub user_id: UserId,
    pub name: String,
    pub sort_name: Option<String>,
    pub ol_key: Option<String>,
}

pub struct UpdateAuthorDbRequest {
    pub name: Option<String>,
    pub sort_name: Option<String>,
    pub ol_key: Option<String>,
    pub monitored: Option<bool>,
    pub monitor_new_items: Option<bool>,
    pub monitor_since: Option<chrono::DateTime<chrono::Utc>>,
}

// ---------------------------------------------------------------------------
// Library Item DB
// ---------------------------------------------------------------------------

/// Library item data access.
///
/// Satisfies: IMPORT-015
#[async_trait::async_trait]
pub trait LibraryItemDb: Send + Sync {
    /// Get library item by ID for a user.
    async fn get_library_item(
        &self,
        user_id: UserId,
        id: LibraryItemId,
    ) -> Result<LibraryItem, DbError>;

    /// List library items for a user.
    async fn list_library_items(&self, user_id: UserId) -> Result<Vec<LibraryItem>, DbError>;

    /// List library items for a specific work.
    async fn list_library_items_by_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<LibraryItem>, DbError>;

    /// Create library item. Enforces UNIQUE(user_id, root_folder_id, path).
    ///
    /// Satisfies: IMPORT-015
    /// Precondition: File has been copied to the target path.
    /// Postcondition: Record created. On path conflict for same work -> returns existing (idempotent).
    ///                On path conflict for different work -> returns Constraint error.
    async fn create_library_item(
        &self,
        req: CreateLibraryItemDbRequest,
    ) -> Result<LibraryItem, DbError>;

    /// Delete library item.
    async fn delete_library_item(
        &self,
        user_id: UserId,
        id: LibraryItemId,
    ) -> Result<LibraryItem, DbError>;

    /// Check if any library items exist for a root folder (for root folder delete guard).
    ///
    /// Satisfies: IMPORT-004
    async fn library_items_exist_for_root(
        &self,
        root_folder_id: RootFolderId,
    ) -> Result<bool, DbError>;

    /// List library items for a work in supported tag-write formats (for re-enrichment tag rewrite).
    ///
    /// Satisfies: TAG-007
    async fn list_taggable_items_by_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<LibraryItem>, DbError>;

    /// Update library item file_size (after tag writing changes file size).
    ///
    /// Satisfies: TAG-V21-004
    async fn update_library_item_size(
        &self,
        user_id: UserId,
        id: LibraryItemId,
        file_size: i64,
    ) -> Result<(), DbError>;
}

pub struct CreateLibraryItemDbRequest {
    pub user_id: UserId,
    pub work_id: WorkId,
    pub root_folder_id: RootFolderId,
    pub path: String,
    pub media_type: MediaType,
    pub file_size: i64,
}

// ---------------------------------------------------------------------------
// Root Folder DB
// ---------------------------------------------------------------------------

/// Root folder data access.
/// Shared infrastructure: admin-managed, visible to all users.
///
/// Satisfies: IMPORT-001, IMPORT-002, IMPORT-004, AUTH-004
#[async_trait::async_trait]
pub trait RootFolderDb: Send + Sync {
    async fn get_root_folder(&self, id: RootFolderId) -> Result<RootFolder, DbError>;
    async fn list_root_folders(&self) -> Result<Vec<RootFolder>, DbError>;
    async fn create_root_folder(
        &self,
        path: &str,
        media_type: MediaType,
    ) -> Result<RootFolder, DbError>;
    async fn delete_root_folder(&self, id: RootFolderId) -> Result<(), DbError>;

    /// Get root folder by media type (at most one per type).
    async fn get_root_folder_by_media_type(
        &self,
        media_type: MediaType,
    ) -> Result<Option<RootFolder>, DbError>;
}

// ---------------------------------------------------------------------------
// Grab DB
// ---------------------------------------------------------------------------

/// Grab data access.
///
/// Satisfies: DLC-006, DLC-009, DLC-012, DLC-015
#[async_trait::async_trait]
pub trait GrabDb: Send + Sync {
    async fn get_grab(&self, user_id: UserId, id: GrabId) -> Result<Grab, DbError>;

    /// List active grabs (sent/confirmed) for import polling.
    ///
    /// Satisfies: IMPORT-005
    async fn list_active_grabs(&self) -> Result<Vec<Grab>, DbError>;

    /// Create or replace grab. Enforces UNIQUE(user_id, guid, indexer).
    /// If existing grab is failed/removed, replaces it. If active, returns Constraint error.
    ///
    /// Satisfies: DLC-009
    async fn upsert_grab(&self, req: CreateGrabDbRequest) -> Result<Grab, DbError>;

    /// Update grab status.
    async fn update_grab_status(
        &self,
        user_id: UserId,
        id: GrabId,
        status: GrabStatus,
        import_error: Option<&str>,
    ) -> Result<(), DbError>;

    /// Update grab download_id (torrent hash set after confirmation).
    async fn update_grab_download_id(
        &self,
        user_id: UserId,
        id: GrabId,
        download_id: &str,
    ) -> Result<(), DbError>;

    /// Get grab by download_id (torrent hash) for poller matching.
    /// Note: cross-user lookup by design -- poller matches torrent hashes across all users.
    /// Scoping enforced by subsequent operations, not this query.
    async fn get_grab_by_download_id(&self, download_id: &str) -> Result<Option<Grab>, DbError>;

    /// Reset all importing grabs to confirmed (startup recovery — JOBS-003).
    async fn reset_importing_grabs(&self) -> Result<u64, DbError>;

    /// Backfill the download_id (torrent hash) on a grab record.
    async fn set_grab_download_id(
        &self,
        user_id: UserId,
        id: GrabId,
        download_id: &str,
    ) -> Result<(), DbError>;

    /// List grabs for a user, paginated, newest first.
    async fn list_grabs_paginated(
        &self,
        user_id: UserId,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<Grab>, i64), DbError>;

    /// Atomically transition grab to `importing` status.
    /// Only succeeds if current status is sent/confirmed/importing/importFailed.
    /// Returns true if transition happened, false if grab was in a non-importable state.
    ///
    /// Satisfies: IMPORT-V21-001 (atomic transition prevents concurrent imports)
    async fn try_set_importing(&self, user_id: UserId, id: GrabId) -> Result<bool, DbError>;
}

pub struct CreateGrabDbRequest {
    pub user_id: UserId,
    pub work_id: WorkId,
    pub download_client_id: DownloadClientId,
    pub title: String,
    pub indexer: String,
    pub guid: String,
    pub size: Option<i64>,
    pub download_url: String,
    pub download_id: Option<String>,
    pub status: GrabStatus,
    pub media_type: Option<MediaType>,
}

// ---------------------------------------------------------------------------
// Download Client DB
// ---------------------------------------------------------------------------

/// Download client config data access.
/// Shared infrastructure: admin-managed, visible to all users.
///
/// Satisfies: DLC-001, DLC-003, DLC-005, AUTH-004, USE-DLC-001, USE-DLC-004
#[async_trait::async_trait]
pub trait DownloadClientDb: Send + Sync {
    async fn get_download_client(&self, id: DownloadClientId) -> Result<DownloadClient, DbError>;
    async fn list_download_clients(&self) -> Result<Vec<DownloadClient>, DbError>;
    async fn create_download_client(
        &self,
        req: CreateDownloadClientDbRequest,
    ) -> Result<DownloadClient, DbError>;
    async fn update_download_client(
        &self,
        id: DownloadClientId,
        req: UpdateDownloadClientDbRequest,
    ) -> Result<DownloadClient, DbError>;
    async fn delete_download_client(&self, id: DownloadClientId) -> Result<(), DbError>;

    /// Get the default download client for a given protocol (client_type).
    ///
    /// Satisfies: DLC-005, USE-DLC-004
    async fn get_default_download_client(
        &self,
        client_type: &str,
    ) -> Result<Option<DownloadClient>, DbError>;
}

pub struct CreateDownloadClientDbRequest {
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

#[derive(Default)]
pub struct UpdateDownloadClientDbRequest {
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

// ---------------------------------------------------------------------------
// Remote Path Mapping DB
// ---------------------------------------------------------------------------

/// Remote path mapping data access.
/// Shared infrastructure: admin-managed, visible to all users.
///
/// Satisfies: DLC-013, AUTH-004
#[async_trait::async_trait]
pub trait RemotePathMappingDb: Send + Sync {
    async fn get_remote_path_mapping(
        &self,
        id: RemotePathMappingId,
    ) -> Result<RemotePathMapping, DbError>;
    async fn list_remote_path_mappings(&self) -> Result<Vec<RemotePathMapping>, DbError>;
    async fn create_remote_path_mapping(
        &self,
        host: &str,
        remote_path: &str,
        local_path: &str,
    ) -> Result<RemotePathMapping, DbError>;
    async fn update_remote_path_mapping(
        &self,
        id: RemotePathMappingId,
        host: &str,
        remote_path: &str,
        local_path: &str,
    ) -> Result<RemotePathMapping, DbError>;
    async fn delete_remote_path_mapping(&self, id: RemotePathMappingId) -> Result<(), DbError>;
}

// ---------------------------------------------------------------------------
// History DB
// ---------------------------------------------------------------------------

/// History data access.
#[async_trait::async_trait]
pub trait HistoryDb: Send + Sync {
    /// List history events for a user, with optional filters.
    async fn list_history(
        &self,
        user_id: UserId,
        filter: HistoryFilter,
    ) -> Result<Vec<HistoryEvent>, DbError>;

    /// Record a history event.
    async fn create_history_event(&self, req: CreateHistoryEventDbRequest) -> Result<(), DbError>;
}

pub struct HistoryFilter {
    pub event_type: Option<EventType>,
    pub work_id: Option<WorkId>,
    pub start_date: Option<chrono::DateTime<chrono::Utc>>,
    pub end_date: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct CreateHistoryEventDbRequest {
    pub user_id: UserId,
    pub work_id: Option<WorkId>,
    pub event_type: EventType,
    pub data: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Notification DB
// ---------------------------------------------------------------------------

/// Notification data access.
///
/// Satisfies: AUTHOR-003, AUTHOR-005
#[async_trait::async_trait]
pub trait NotificationDb: Send + Sync {
    /// List notifications for a user. Optional filter for unread only.
    async fn list_notifications(
        &self,
        user_id: UserId,
        unread_only: bool,
    ) -> Result<Vec<Notification>, DbError>;

    /// Create notification. Respects dedup: one per (user_id, type, ref_key) regardless
    /// of dismissed state. If any notification (active or dismissed) exists for that
    /// combination, returns Ok without creating. Dismissed means "don't tell me again."
    ///
    /// Satisfies: AUTHOR-003 (dedup)
    /// Postcondition: At most one notification per (user_id, type, ref_key) ever.
    async fn create_notification(
        &self,
        req: CreateNotificationDbRequest,
    ) -> Result<Notification, DbError>;

    /// Mark notification as read.
    async fn mark_notification_read(
        &self,
        user_id: UserId,
        id: NotificationId,
    ) -> Result<(), DbError>;

    /// Dismiss notification (sets dismissed=1). Permanent -- dedup blocks
    /// re-creation for this (user_id, type, ref_key) combination.
    ///
    /// Satisfies: AUTHOR-005
    async fn dismiss_notification(
        &self,
        user_id: UserId,
        id: NotificationId,
    ) -> Result<(), DbError>;

    /// Dismiss all notifications for a user. Permanent for each ref_key.
    async fn dismiss_all_notifications(&self, user_id: UserId) -> Result<(), DbError>;
}

pub struct CreateNotificationDbRequest {
    pub user_id: UserId,
    pub notification_type: NotificationType,
    pub ref_key: Option<String>,
    pub message: String,
    pub data: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Config DB
// ---------------------------------------------------------------------------

/// Configuration data access (DB singletons).
/// Shared infrastructure: admin-managed, visible to all users.
///
/// Satisfies: CONFIG-001, CONFIG-002, CONFIG-003, CONFIG-004, CONFIG-005, AUTH-004
#[async_trait::async_trait]
pub trait ConfigDb: Send + Sync {
    /// Get naming config (read-only singleton).
    async fn get_naming_config(&self) -> Result<NamingConfig, DbError>;

    /// Get media management config.
    async fn get_media_management_config(&self) -> Result<MediaManagementConfig, DbError>;

    /// Update media management config.
    async fn update_media_management_config(
        &self,
        req: UpdateMediaManagementConfigRequest,
    ) -> Result<MediaManagementConfig, DbError>;

    /// Get Prowlarr config.
    async fn get_prowlarr_config(&self) -> Result<ProwlarrConfig, DbError>;

    /// Update Prowlarr config.
    async fn update_prowlarr_config(
        &self,
        req: UpdateProwlarrConfigRequest,
    ) -> Result<ProwlarrConfig, DbError>;

    /// Get metadata config.
    async fn get_metadata_config(&self) -> Result<MetadataConfig, DbError>;

    /// Update metadata config.
    async fn update_metadata_config(
        &self,
        req: UpdateMetadataConfigRequest,
    ) -> Result<MetadataConfig, DbError>;
}

pub struct NamingConfig {
    pub author_folder_format: String,
    pub book_folder_format: String,
    pub rename_files: bool,
    pub replace_illegal_chars: bool,
}

pub struct MediaManagementConfig {
    pub cwa_ingest_path: Option<String>,
    pub preferred_ebook_formats: Vec<String>,
    pub preferred_audiobook_formats: Vec<String>,
}

#[derive(Default)]
pub struct ProwlarrConfig {
    pub url: Option<String>,
    pub api_key: Option<String>,
    pub enabled: bool,
}

pub struct MetadataConfig {
    pub hardcover_enabled: bool,
    pub hardcover_api_token: Option<String>,
    pub llm_enabled: bool,
    pub llm_provider: Option<LlmProvider>,
    pub llm_endpoint: Option<String>,
    pub llm_api_key: Option<String>,
    pub llm_model: Option<String>,
    pub audnexus_url: String,
    pub languages: Vec<String>,
}

pub struct UpdateMediaManagementConfigRequest {
    pub cwa_ingest_path: Option<String>,
    pub preferred_ebook_formats: Vec<String>,
    pub preferred_audiobook_formats: Vec<String>,
}

pub struct UpdateProwlarrConfigRequest {
    pub url: Option<String>,
    pub api_key: Option<String>,
    pub enabled: Option<bool>,
}

pub struct UpdateMetadataConfigRequest {
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

// ---------------------------------------------------------------------------
// v2.1 — Enrichment Retry DB
// ---------------------------------------------------------------------------

/// Enrichment retry operations. Extends v2 WorkDb contract.
///
/// Satisfies: IMPL-JOBS-005
#[async_trait::async_trait]
pub trait EnrichmentRetryDb: Send + Sync {
    /// List works eligible for retry: status in (failed, partial), retry_count < 3.
    async fn list_works_for_retry(&self) -> Result<Vec<Work>, DbError>;

    /// Reset enrichment for manual refresh: retry_count=0, status=pending.
    async fn reset_enrichment_for_refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<(), DbError>;

    /// Increment retry count. If count >= 3 and status is failed, transition to exhausted.
    async fn increment_retry_count(&self, user_id: UserId, work_id: WorkId) -> Result<(), DbError>;
}

// ---------------------------------------------------------------------------
// v2.1 — Indexer DB (Torznab)
// ---------------------------------------------------------------------------

/// Indexer data access. Not user-scoped — indexers are global.
///
/// Satisfies: IDX-001, IDX-002, IDX-004, IDX-009, IDX-010
#[async_trait::async_trait]
pub trait IndexerDb: Send + Sync {
    async fn get_indexer(&self, id: IndexerId) -> Result<Indexer, DbError>;
    async fn list_indexers(&self) -> Result<Vec<Indexer>, DbError>;
    async fn list_enabled_interactive_indexers(&self) -> Result<Vec<Indexer>, DbError>;
    async fn create_indexer(&self, req: CreateIndexerDbRequest) -> Result<Indexer, DbError>;
    async fn update_indexer(
        &self,
        id: IndexerId,
        req: UpdateIndexerDbRequest,
    ) -> Result<Indexer, DbError>;
    async fn delete_indexer(&self, id: IndexerId) -> Result<(), DbError>;
    async fn set_supports_book_search(&self, id: IndexerId, supports: bool) -> Result<(), DbError>;
}

pub struct CreateIndexerDbRequest {
    pub name: String,
    pub protocol: String,
    pub url: String,
    pub api_path: String,
    pub api_key: Option<String>,
    pub categories: Vec<i32>,
    pub priority: i32,
    pub enable_automatic_search: bool,
    pub enable_interactive_search: bool,
    pub enabled: bool,
}

pub struct UpdateIndexerDbRequest {
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

// ---------------------------------------------------------------------------
// Author Bibliography
// ---------------------------------------------------------------------------

/// Cached author bibliography entry (from OL/LLM cleanup).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibliographyEntry {
    pub ol_key: String,
    pub title: String,
    pub year: Option<i32>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
}

/// Cached bibliography for an author.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorBibliography {
    pub author_id: i64,
    pub entries: Vec<BibliographyEntry>,
    pub fetched_at: String,
}

#[async_trait::async_trait]
pub trait AuthorBibliographyDb: Send + Sync {
    async fn get_bibliography(&self, author_id: i64)
        -> Result<Option<AuthorBibliography>, DbError>;
    async fn save_bibliography(
        &self,
        author_id: i64,
        entries: &[BibliographyEntry],
    ) -> Result<AuthorBibliography, DbError>;
}

// ---------------------------------------------------------------------------
// v2.1 — SQLite Pool Creation
// ---------------------------------------------------------------------------

/// Create and configure a SQLite connection pool.
///
/// Satisfies: RUNTIME-SQLITE-001, RUNTIME-SQLITE-002
///
/// This is a placeholder signature for Phase 3 behavioral tests.
/// The real implementation uses sqlx::SqlitePool.
pub fn create_pool(
    data_dir: &std::path::Path,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), DbError>> + Send>> {
    let exists = data_dir.exists();
    let display = data_dir.display().to_string();
    Box::pin(async move {
        if !exists {
            return Err(DbError::Io(format!(
                "data directory does not exist: {display}"
            )));
        }
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Test Helpers
// ---------------------------------------------------------------------------

pub mod test_helpers {
    use super::mem::InMemoryDb;
    use super::*;

    pub type StubDb = InMemoryDb;

    pub async fn new_test_db() -> InMemoryDb {
        InMemoryDb::new()
    }

    pub async fn new_test_db_with_placeholder() -> InMemoryDb {
        InMemoryDb::with_placeholder_admin()
    }

    pub fn new_notification_test_db() -> InMemoryDb {
        InMemoryDb::new()
    }

    pub fn new_history_test_db() -> InMemoryDb {
        InMemoryDb::new()
    }

    pub fn new_config_test_db() -> InMemoryDb {
        InMemoryDb::new()
    }

    pub fn test_user_id() -> UserId {
        1
    }
}
