use serde::Deserialize;

pub use livrarr_domain::settings::{
    EmailConfig, MediaManagementConfig, MetadataConfig, NamingConfig, ProwlarrConfig,
};
pub use livrarr_domain::{
    ApplyMergeOutcome, Author, AuthorId, DbError, DownloadClient, DownloadClientId,
    DownloadClientImplementation, EnrichmentStatus, EventType, ExternalIdRowId, ExternalIdType,
    FieldProvenance, Grab, GrabId, GrabStatus, HistoryEvent, HistoryFilter, HistoryId, Import,
    Indexer, IndexerConfig, IndexerId, IndexerRssState, LibraryItem, LibraryItemId, LlmProvider,
    MediaType, MergeResolved, MetadataProvider, NarrationType, Notification, NotificationId,
    NotificationType, OutcomeClass, PlaybackProgress, ProvenanceSetter, RemotePathMapping,
    RemotePathMappingId, RootFolder, RootFolderId, Series, Session, User, UserId, UserRole, Work,
    WorkField, WorkId,
};

pub mod pool;
pub mod sqlite;
mod sqlite_author;
mod sqlite_bibliography;
pub(crate) mod sqlite_common;
mod sqlite_config;
mod sqlite_download_client;
mod sqlite_external_id;
mod sqlite_grab;
mod sqlite_history;
mod sqlite_import;
mod sqlite_indexer;
mod sqlite_library_item;
mod sqlite_list_import;
mod sqlite_notification;
mod sqlite_playback_progress;
mod sqlite_provenance;
mod sqlite_remote_path_mapping;
mod sqlite_retry_state;
mod sqlite_root_folder;
mod sqlite_series;
mod sqlite_series_cache;
mod sqlite_session;
mod sqlite_user;
mod sqlite_work;

#[cfg(test)]
mod cross_user_isolation_tests;

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
#[trait_variant::make(Send)]
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
#[trait_variant::make(Send)]
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

    /// Delete all sessions for a user (e.g. after password change).
    async fn delete_user_sessions(&self, user_id: UserId) -> Result<u64, DbError>;

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
#[trait_variant::make(Send)]
pub trait WorkDb: Send + Sync {
    /// Get work by ID for a specific user.
    async fn get_work(&self, user_id: UserId, id: WorkId) -> Result<Work, DbError>;

    /// List works for a user (unbounded — for internal use).
    async fn list_works(&self, user_id: UserId) -> Result<Vec<Work>, DbError>;

    /// List works for a specific author.
    async fn list_works_by_author(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<Work>, DbError>;

    /// List works for a user, paginated with server-side sort.
    async fn list_works_paginated(
        &self,
        user_id: UserId,
        page: u32,
        per_page: u32,
        sort_by: &str,
        sort_dir: &str,
    ) -> Result<(Vec<Work>, i64), DbError>;

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

    async fn list_work_provider_keys_by_author(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<(Option<String>, Option<String>)>, DbError>;

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

    /// List all works where monitor_ebook=1 OR monitor_audiobook=1, across all users.
    ///
    /// Satisfies: RSS-MATCH-001, RSS-FILTER-002
    async fn list_monitored_works_all_users(&self) -> Result<Vec<Work>, DbError>;

    /// Set enrichment_status = 'skipped' for a work (no user scope — called from add pipeline).
    async fn set_enrichment_status_skipped(&self, id: WorkId) -> Result<(), DbError>;

    /// TEMP(pk-tdd): compile-only scaffold — apply a merge result to the work record.
    async fn apply_enrichment_merge(
        &self,
        req: ApplyEnrichmentMergeRequest,
    ) -> Result<ApplyMergeOutcome, DbError>;

    /// TEMP(pk-tdd): compile-only scaffold — reset enrichment state for manual refresh.
    async fn reset_for_manual_refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<(), DbError>;

    /// TEMP(pk-tdd): compile-only scaffold — list works in Conflict status.
    async fn list_conflict_works(&self, user_id: UserId) -> Result<Vec<Work>, DbError>;

    /// TEMP(pk-tdd): compile-only scaffold — get current merge generation counter.
    async fn get_merge_generation(&self, user_id: UserId, work_id: WorkId) -> Result<i64, DbError>;

    /// Search works by title or author_name LIKE match, paginated.
    async fn search_works(
        &self,
        user_id: UserId,
        query: &str,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<Work>, i64), DbError>;
}

#[derive(Default)]
pub struct CreateWorkDbRequest {
    pub user_id: UserId,
    pub title: String,
    pub author_name: String,
    pub author_id: Option<AuthorId>,
    pub ol_key: Option<String>,
    pub gr_key: Option<String>,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    pub metadata_source: Option<String>,
    pub detail_url: Option<String>,
    pub language: Option<String>,
    pub import_id: Option<String>,
    pub series_id: Option<i64>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
}

#[derive(Debug, Clone, Default)]
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
    pub ol_key: Option<String>,
    pub gr_key: Option<String>,
    pub hc_key: Option<String>,
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

#[derive(Default)]
pub struct UpdateWorkUserFieldsDbRequest {
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
    pub monitor_ebook: Option<bool>,
    pub monitor_audiobook: Option<bool>,
}

// ---------------------------------------------------------------------------
// Author DB
// ---------------------------------------------------------------------------

/// Author data access.
///
/// Satisfies: AUTHOR-001, SEARCH-005
#[trait_variant::make(Send)]
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
    pub gr_key: Option<String>,
    pub hc_key: Option<String>,
    pub import_id: Option<String>,
}

pub struct UpdateAuthorDbRequest {
    pub name: Option<String>,
    pub sort_name: Option<String>,
    pub ol_key: Option<String>,
    pub gr_key: Option<String>,
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
#[trait_variant::make(Send)]
pub trait LibraryItemDb: Send + Sync {
    /// Get library item by ID for a user.
    async fn get_library_item(
        &self,
        user_id: UserId,
        id: LibraryItemId,
    ) -> Result<LibraryItem, DbError>;

    /// List library items for a user (unbounded — for internal use).
    async fn list_library_items(&self, user_id: UserId) -> Result<Vec<LibraryItem>, DbError>;

    /// List library items for a user, paginated.
    async fn list_library_items_paginated(
        &self,
        user_id: UserId,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<LibraryItem>, i64), DbError>;

    /// List library items for a set of work IDs (batch enrichment for paginated work lists).
    async fn list_library_items_by_work_ids(
        &self,
        user_id: UserId,
        work_ids: &[WorkId],
    ) -> Result<Vec<LibraryItem>, DbError>;

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

    /// Check if user has a library item for this work with the given media type.
    ///
    /// Satisfies: RSS-FILTER-002
    async fn work_has_library_item(
        &self,
        user_id: UserId,
        work_id: WorkId,
        media_type: MediaType,
    ) -> Result<bool, DbError>;
}

pub struct CreateLibraryItemDbRequest {
    pub user_id: UserId,
    pub work_id: WorkId,
    pub root_folder_id: RootFolderId,
    pub path: String,
    pub media_type: MediaType,
    pub file_size: i64,
    pub import_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Root Folder DB
// ---------------------------------------------------------------------------

/// Root folder data access.
/// Shared infrastructure: admin-managed, visible to all users.
///
/// Satisfies: IMPORT-001, IMPORT-002, IMPORT-004, AUTH-004
#[trait_variant::make(Send)]
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
#[trait_variant::make(Send)]
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

    /// Persist the raw remote content path from the download client.
    async fn set_grab_content_path(
        &self,
        user_id: UserId,
        id: GrabId,
        content_path: &str,
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

    /// Check if user has an active grab (sent/confirmed/importing) for this work+media type.
    ///
    /// Satisfies: RSS-FILTER-002
    async fn active_grab_exists(
        &self,
        user_id: UserId,
        work_id: WorkId,
        media_type: MediaType,
    ) -> Result<bool, DbError>;

    /// List importFailed grabs eligible for retry (backoff expired, under max retries).
    async fn list_retriable_grabs(&self, max_retries: i32) -> Result<Vec<Grab>, DbError>;

    /// Increment retry count and set import_failed_at timestamp on a grab.
    async fn increment_import_retry(&self, user_id: UserId, id: GrabId) -> Result<(), DbError>;
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
#[trait_variant::make(Send)]
pub trait DownloadClientDb: Send + Sync {
    async fn get_download_client(&self, id: DownloadClientId) -> Result<DownloadClient, DbError>;

    /// Get download client with credentials (password and api_key populated).
    /// Use for outbound connections (test, grab, import poll). Default get_download_client
    /// is equivalent but callers making outbound calls should use this variant to signal intent.
    async fn get_download_client_with_credentials(
        &self,
        id: DownloadClientId,
    ) -> Result<DownloadClient, DbError>;
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
    /// Tri-state: None = keep existing, Some(None) = clear, Some(Some(v)) = set.
    pub password: Option<Option<String>>,
    pub category: Option<String>,
    pub enabled: Option<bool>,
    /// Tri-state: None = keep existing, Some(None) = clear, Some(Some(v)) = set.
    pub api_key: Option<Option<String>>,
    pub is_default_for_protocol: Option<bool>,
}

// ---------------------------------------------------------------------------
// Remote Path Mapping DB
// ---------------------------------------------------------------------------

/// Remote path mapping data access.
/// Shared infrastructure: admin-managed, visible to all users.
///
/// Satisfies: DLC-013, AUTH-004
#[trait_variant::make(Send)]
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
#[trait_variant::make(Send)]
pub trait HistoryDb: Send + Sync {
    /// List history events for a user, with optional filters (unbounded).
    async fn list_history(
        &self,
        user_id: UserId,
        filter: HistoryFilter,
    ) -> Result<Vec<HistoryEvent>, DbError>;

    /// List history events, paginated.
    async fn list_history_paginated(
        &self,
        user_id: UserId,
        filter: HistoryFilter,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<HistoryEvent>, i64), DbError>;

    /// Record a history event.
    async fn create_history_event(&self, req: CreateHistoryEventDbRequest) -> Result<(), DbError>;
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
#[trait_variant::make(Send)]
pub trait NotificationDb: Send + Sync {
    /// List notifications for a user. Optional filter for unread only (unbounded).
    async fn list_notifications(
        &self,
        user_id: UserId,
        unread_only: bool,
    ) -> Result<Vec<Notification>, DbError>;

    /// List notifications, paginated.
    async fn list_notifications_paginated(
        &self,
        user_id: UserId,
        unread_only: bool,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<Notification>, i64), DbError>;

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
/// Satisfies: CONFIG-001, CONFIG-002, CONFIG-003, CONFIG-004, CONFIG-005, AUTH-004, RSS-CONFIG-001
#[trait_variant::make(Send)]
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

    /// Get email config.
    async fn get_email_config(&self) -> Result<EmailConfig, DbError>;

    /// Update email config.
    async fn update_email_config(
        &self,
        req: UpdateEmailConfigRequest,
    ) -> Result<EmailConfig, DbError>;

    /// Get indexer config singleton (RSS sync settings).
    ///
    /// Satisfies: RSS-CONFIG-001
    async fn get_indexer_config(&self) -> Result<IndexerConfig, DbError>;

    /// Update indexer config singleton.
    ///
    /// Satisfies: RSS-CONFIG-001
    async fn update_indexer_config(
        &self,
        req: UpdateIndexerConfigRequest,
    ) -> Result<IndexerConfig, DbError>;
}

// NamingConfig, MediaManagementConfig, ProwlarrConfig, MetadataConfig
// re-exported from livrarr_domain::settings above.

pub struct UpdateMediaManagementConfigRequest {
    pub cwa_ingest_path: Option<String>,
    pub preferred_ebook_formats: Vec<String>,
    pub preferred_audiobook_formats: Vec<String>,
}

pub struct UpdateProwlarrConfigRequest {
    pub url: Option<String>,
    /// Tri-state: None = keep existing, Some(None) = clear, Some(Some(v)) = set.
    pub api_key: Option<Option<String>>,
    pub enabled: Option<bool>,
}

// EmailConfig re-exported from livrarr_domain::settings above.

pub struct UpdateEmailConfigRequest {
    pub enabled: Option<bool>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i32>,
    pub encryption: Option<String>,
    pub username: Option<String>,
    /// Tri-state: None = keep existing, Some(None) = clear, Some(Some(v)) = set.
    pub password: Option<Option<String>>,
    pub from_address: Option<String>,
    pub recipient_email: Option<String>,
    pub send_on_import: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateIndexerConfigRequest {
    pub rss_sync_interval_minutes: Option<i32>,
    pub rss_match_threshold: Option<f64>,
}

pub struct UpdateMetadataConfigRequest {
    pub hardcover_enabled: Option<bool>,
    /// Tri-state: None = keep existing, Some(None) = clear, Some(Some(v)) = set.
    pub hardcover_api_token: Option<Option<String>>,
    pub llm_enabled: Option<bool>,
    pub llm_provider: Option<LlmProvider>,
    pub llm_endpoint: Option<String>,
    /// Tri-state: None = keep existing, Some(None) = clear, Some(Some(v)) = set.
    pub llm_api_key: Option<Option<String>>,
    pub llm_model: Option<String>,
    pub audnexus_url: Option<String>,
    pub languages: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// From impls: domain params -> DB request types
// ---------------------------------------------------------------------------

use livrarr_domain::settings::{
    CreateDownloadClientParams, CreateIndexerParams, UpdateDownloadClientParams, UpdateEmailParams,
    UpdateIndexerConfigParams, UpdateIndexerParams, UpdateMediaManagementParams,
    UpdateMetadataParams, UpdateProwlarrParams,
};

impl From<UpdateMediaManagementParams> for UpdateMediaManagementConfigRequest {
    fn from(p: UpdateMediaManagementParams) -> Self {
        Self {
            cwa_ingest_path: p.cwa_ingest_path,
            preferred_ebook_formats: p.preferred_ebook_formats,
            preferred_audiobook_formats: p.preferred_audiobook_formats,
        }
    }
}

impl From<UpdateMetadataParams> for UpdateMetadataConfigRequest {
    fn from(p: UpdateMetadataParams) -> Self {
        Self {
            hardcover_enabled: p.hardcover_enabled,
            hardcover_api_token: p.hardcover_api_token,
            llm_enabled: p.llm_enabled,
            llm_provider: p.llm_provider,
            llm_endpoint: p.llm_endpoint,
            llm_api_key: p.llm_api_key,
            llm_model: p.llm_model,
            audnexus_url: p.audnexus_url,
            languages: p.languages,
        }
    }
}

impl From<UpdateProwlarrParams> for UpdateProwlarrConfigRequest {
    fn from(p: UpdateProwlarrParams) -> Self {
        Self {
            url: p.url,
            api_key: p.api_key,
            enabled: p.enabled,
        }
    }
}

impl From<UpdateEmailParams> for UpdateEmailConfigRequest {
    fn from(p: UpdateEmailParams) -> Self {
        Self {
            enabled: p.enabled,
            smtp_host: p.smtp_host,
            smtp_port: p.smtp_port,
            encryption: p.encryption,
            username: p.username,
            password: p.password,
            from_address: p.from_address,
            recipient_email: p.recipient_email,
            send_on_import: p.send_on_import,
        }
    }
}

impl From<UpdateIndexerConfigParams> for UpdateIndexerConfigRequest {
    fn from(p: UpdateIndexerConfigParams) -> Self {
        Self {
            rss_sync_interval_minutes: p.rss_sync_interval_minutes,
            rss_match_threshold: p.rss_match_threshold,
        }
    }
}

impl From<CreateDownloadClientParams> for CreateDownloadClientDbRequest {
    fn from(p: CreateDownloadClientParams) -> Self {
        Self {
            name: p.name,
            implementation: p.implementation,
            host: p.host,
            port: p.port,
            use_ssl: p.use_ssl,
            skip_ssl_validation: p.skip_ssl_validation,
            url_base: p.url_base,
            username: p.username,
            password: p.password,
            category: p.category,
            enabled: p.enabled,
            api_key: p.api_key,
        }
    }
}

impl From<UpdateDownloadClientParams> for UpdateDownloadClientDbRequest {
    fn from(p: UpdateDownloadClientParams) -> Self {
        Self {
            name: p.name,
            host: p.host,
            port: p.port,
            use_ssl: p.use_ssl,
            skip_ssl_validation: p.skip_ssl_validation,
            url_base: p.url_base,
            username: p.username,
            password: p.password,
            category: p.category,
            enabled: p.enabled,
            api_key: p.api_key,
            is_default_for_protocol: p.is_default_for_protocol,
        }
    }
}

impl From<CreateIndexerParams> for CreateIndexerDbRequest {
    fn from(p: CreateIndexerParams) -> Self {
        Self {
            name: p.name,
            protocol: p.protocol,
            url: p.url,
            api_path: p.api_path,
            api_key: p.api_key,
            categories: p.categories,
            priority: p.priority,
            enable_automatic_search: p.enable_automatic_search,
            enable_interactive_search: p.enable_interactive_search,
            enable_rss: p.enable_rss,
            enabled: p.enabled,
        }
    }
}

impl From<UpdateIndexerParams> for UpdateIndexerDbRequest {
    fn from(p: UpdateIndexerParams) -> Self {
        Self {
            name: p.name,
            url: p.url,
            api_path: p.api_path,
            api_key: p.api_key,
            categories: p.categories,
            priority: p.priority,
            enable_automatic_search: p.enable_automatic_search,
            enable_interactive_search: p.enable_interactive_search,
            enable_rss: p.enable_rss,
            enabled: p.enabled,
        }
    }
}

// ---------------------------------------------------------------------------
// v2.1 — Enrichment Retry DB
// ---------------------------------------------------------------------------

/// Enrichment retry operations. Extends v2 WorkDb contract.
///
/// Satisfies: IMPL-JOBS-005
#[trait_variant::make(Send)]
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
/// Satisfies: IDX-001, IDX-002, IDX-004, IDX-009, IDX-010, RSS-FETCH-002, RSS-GAP-001
#[trait_variant::make(Send)]
pub trait IndexerDb: Send + Sync {
    async fn get_indexer(&self, id: IndexerId) -> Result<Indexer, DbError>;

    /// Get indexer with credentials (api_key populated).
    /// Use for outbound connections (test, search). Default get_indexer is equivalent
    /// but callers that make outbound calls should use this variant to signal intent.
    async fn get_indexer_with_credentials(&self, id: IndexerId) -> Result<Indexer, DbError>;
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

    /// List indexers with enabled=1 AND enable_rss=1.
    ///
    /// Satisfies: RSS-FETCH-002
    async fn list_enabled_rss_indexers(&self) -> Result<Vec<Indexer>, DbError>;

    /// Get RSS state for an indexer. Returns None if no state row exists (first sync).
    ///
    /// Satisfies: RSS-GAP-001, RSS-JOB-001
    async fn get_rss_state(
        &self,
        indexer_id: IndexerId,
    ) -> Result<Option<IndexerRssState>, DbError>;

    /// Insert or update RSS state for an indexer.
    ///
    /// Satisfies: RSS-GAP-001
    async fn upsert_rss_state(
        &self,
        indexer_id: IndexerId,
        last_publish_date: Option<&str>,
        last_guid: &str,
    ) -> Result<(), DbError>;
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
    pub enable_rss: bool,
    pub enabled: bool,
}

pub struct UpdateIndexerDbRequest {
    pub name: Option<String>,
    pub url: Option<String>,
    pub api_path: Option<String>,
    /// Tri-state: None = keep existing, Some(None) = clear, Some(Some(v)) = set.
    pub api_key: Option<Option<String>>,
    pub categories: Option<Vec<i32>>,
    pub priority: Option<i32>,
    pub enable_automatic_search: Option<bool>,
    pub enable_interactive_search: Option<bool>,
    pub enable_rss: Option<bool>,
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
    pub raw_entries: Option<Vec<BibliographyEntry>>,
    pub fetched_at: String,
}

#[trait_variant::make(Send)]
pub trait AuthorBibliographyDb: Send + Sync {
    async fn get_bibliography(&self, author_id: i64)
        -> Result<Option<AuthorBibliography>, DbError>;
    async fn save_bibliography(
        &self,
        author_id: i64,
        entries: &[BibliographyEntry],
        raw_entries: Option<&[BibliographyEntry]>,
    ) -> Result<AuthorBibliography, DbError>;

    async fn delete_bibliography(&self, author_id: i64) -> Result<(), DbError>;
}

// ---------------------------------------------------------------------------
// Series DB
// ---------------------------------------------------------------------------

/// Cached series list entry for an author (from GR scraping).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesCacheEntry {
    pub name: String,
    pub gr_key: String,
    pub book_count: i32,
}

/// Cached series list for an author.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorSeriesCache {
    pub author_id: i64,
    pub entries: Vec<SeriesCacheEntry>,
    pub raw_entries: Option<Vec<SeriesCacheEntry>>,
    pub fetched_at: String,
}

pub struct LinkWorkToSeriesRequest {
    pub work_id: WorkId,
    pub series_id: i64,
    pub series_work_count: i32,
    pub series_name: String,
    pub series_position: Option<f64>,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
}

pub struct CreateSeriesDbRequest {
    pub user_id: UserId,
    pub author_id: AuthorId,
    pub name: String,
    pub gr_key: String,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub work_count: i32,
}

#[trait_variant::make(Send)]
pub trait SeriesDb: Send + Sync {
    /// Get a series by ID, scoped to user.
    async fn get_series(&self, user_id: UserId, id: i64) -> Result<Option<Series>, DbError>;

    /// List all series for a user.
    async fn list_all_series(&self, user_id: UserId) -> Result<Vec<Series>, DbError>;

    /// List all series for an author.
    async fn list_series_for_author(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<Series>, DbError>;

    /// Create or update a series (upsert on user_id + author_id + gr_key).
    async fn upsert_series(&self, req: CreateSeriesDbRequest) -> Result<Series, DbError>;

    /// Update monitoring flags on a series and propagate to linked works.
    async fn update_series_flags(
        &self,
        user_id: UserId,
        id: i64,
        monitor_ebook: bool,
        monitor_audiobook: bool,
    ) -> Result<Series, DbError>;

    /// Update work_count for a series.
    async fn update_series_work_count(
        &self,
        user_id: UserId,
        id: i64,
        work_count: i32,
    ) -> Result<(), DbError>;

    /// Link a work to a series (with assignment guard: only if current series_id is NULL
    /// or new series has smaller work_count). Validates work ownership.
    async fn link_work_to_series(
        &self,
        user_id: UserId,
        req: LinkWorkToSeriesRequest,
    ) -> Result<(), DbError>;

    /// List monitored series (either flag true) for a list of author IDs, scoped to user.
    async fn list_monitored_series_for_authors(
        &self,
        user_id: UserId,
        author_ids: &[AuthorId],
    ) -> Result<Vec<Series>, DbError>;
}

#[trait_variant::make(Send)]
pub trait SeriesCacheDb: Send + Sync {
    async fn get_series_cache(&self, author_id: i64) -> Result<Option<AuthorSeriesCache>, DbError>;

    async fn save_series_cache(
        &self,
        author_id: i64,
        entries: &[SeriesCacheEntry],
        raw_entries: Option<&[SeriesCacheEntry]>,
    ) -> Result<AuthorSeriesCache, DbError>;

    async fn delete_series_cache(&self, author_id: i64) -> Result<(), DbError>;
}

// ---------------------------------------------------------------------------
// Import DB (Readarr Library Import)
// ---------------------------------------------------------------------------

/// Import tracking data access.
#[trait_variant::make(Send)]
pub trait ImportDb: Send + Sync {
    /// Create an import record.
    async fn create_import(&self, req: CreateImportDbRequest) -> Result<(), DbError>;

    /// Get an import by ID.
    async fn get_import(&self, id: &str) -> Result<Option<Import>, DbError>;

    /// List imports for a user (most recent first).
    async fn list_imports(&self, user_id: UserId) -> Result<Vec<Import>, DbError>;

    /// Update import status.
    async fn update_import_status(&self, id: &str, status: &str) -> Result<(), DbError>;

    /// Update import counters.
    async fn update_import_counts(
        &self,
        id: &str,
        authors: i64,
        works: i64,
        files: i64,
        skipped: i64,
    ) -> Result<(), DbError>;

    /// Mark import as completed (set status + completed_at timestamp).
    async fn set_import_completed(&self, id: &str) -> Result<(), DbError>;

    /// List library items by import_id (for undo).
    async fn list_library_items_by_import(
        &self,
        import_id: &str,
    ) -> Result<Vec<LibraryItem>, DbError>;

    /// Delete a library item by ID (no user scope — for undo).
    async fn delete_library_item_by_id(&self, id: LibraryItemId) -> Result<(), DbError>;

    /// Delete works by import_id that have zero library items.
    async fn delete_orphan_works_by_import(&self, import_id: &str) -> Result<i64, DbError>;

    /// Delete authors by import_id that have zero works.
    async fn delete_orphan_authors_by_import(&self, import_id: &str) -> Result<i64, DbError>;
}

/// Playback progress data access.
#[trait_variant::make(Send)]
pub trait PlaybackProgressDb: Send + Sync {
    /// Get playback progress for a user + library item.
    async fn get_progress(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
    ) -> Result<Option<PlaybackProgress>, DbError>;

    /// Insert or update playback progress.
    async fn upsert_progress(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
        position: &str,
        progress_pct: f64,
    ) -> Result<(), DbError>;
}

pub struct CreateImportDbRequest {
    pub id: String,
    pub user_id: UserId,
    pub source: String,
    pub source_url: Option<String>,
    pub target_root_folder_id: Option<i64>,
}

// ---------------------------------------------------------------------------
// List Import DB (CSV preview + confirm flow)
// ---------------------------------------------------------------------------

/// A single row from the list_import_previews table, returned by fetch operations.
pub struct ListImportPreviewRow {
    pub title: String,
    pub author: String,
    pub isbn_13: Option<String>,
    pub isbn_10: Option<String>,
    pub year: Option<i32>,
}

/// A row from the imports table for the list-import listing endpoint.
pub struct ListImportSummaryRow {
    pub id: String,
    pub source: String,
    pub status: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub works_created: i64,
}

/// Data needed to validate ownership of a running import record.
pub struct ListImportRecord {
    pub user_id: i64,
    pub status: String,
}

/// Database operations for the CSV list-import flow (Goodreads / Hardcover previews).
#[trait_variant::make(Send)]
pub trait ListImportDb: Send + Sync {
    /// Insert a single row into the list_import_previews table.
    #[allow(clippy::too_many_arguments)]
    async fn insert_list_import_preview_row(
        &self,
        preview_id: &str,
        user_id: UserId,
        row_index: i64,
        title: &str,
        author: &str,
        isbn_13: Option<&str>,
        isbn_10: Option<&str>,
        year: Option<i32>,
        source_status: Option<&str>,
        source_rating: Option<f32>,
        preview_status: &str,
        source: &str,
        created_at: &str,
    ) -> Result<(), DbError>;

    /// Count preview rows for the given (preview_id, user_id) pair.
    async fn count_list_import_previews(
        &self,
        preview_id: &str,
        user_id: UserId,
    ) -> Result<i64, DbError>;

    /// Get the source field from any preview row for the given (preview_id, user_id).
    async fn get_list_import_source(
        &self,
        preview_id: &str,
        user_id: UserId,
    ) -> Result<String, DbError>;

    /// Create a new list-import record with status = 'running'.
    async fn create_list_import_record(
        &self,
        id: &str,
        user_id: UserId,
        source: &str,
        started_at: &str,
    ) -> Result<(), DbError>;

    /// Get ownership + status for a list-import record.
    async fn get_list_import_record(&self, id: &str) -> Result<Option<ListImportRecord>, DbError>;

    /// Get a single preview row by (preview_id, user_id, row_index).
    async fn get_list_import_preview_row(
        &self,
        preview_id: &str,
        user_id: UserId,
        row_index: i64,
    ) -> Result<Option<ListImportPreviewRow>, DbError>;

    /// Tag the most-recently-created work for a user with the given import_id.
    async fn tag_last_work_with_import(
        &self,
        import_id: &str,
        user_id: UserId,
    ) -> Result<(), DbError>;

    /// Increment works_created counter on a list-import record.
    async fn increment_list_import_works_created(
        &self,
        import_id: &str,
        delta: i64,
    ) -> Result<(), DbError>;

    /// Complete a list-import (status = 'completed', completed_at = now).
    /// Scoped to user_id + status = 'running'. Returns rows_affected.
    async fn complete_list_import(
        &self,
        import_id: &str,
        user_id: UserId,
        completed_at: &str,
    ) -> Result<u64, DbError>;

    /// Validate + get status of a list-import for a specific user.
    async fn get_list_import_status_for_user(
        &self,
        import_id: &str,
        user_id: UserId,
    ) -> Result<Option<String>, DbError>;

    /// Delete works tagged with the given import_id for the given user.
    async fn delete_works_by_list_import(
        &self,
        import_id: &str,
        user_id: UserId,
    ) -> Result<i64, DbError>;

    /// Mark a list-import as 'undone'.
    async fn mark_list_import_undone(&self, import_id: &str) -> Result<(), DbError>;

    /// List list-imports (goodreads/hardcover sources) for a user, newest first, limit 50.
    async fn list_list_imports(
        &self,
        user_id: UserId,
    ) -> Result<Vec<ListImportSummaryRow>, DbError>;

    /// Check if a work with the given ISBN-13 exists for a user (via works table or external_ids).
    async fn work_exists_by_isbn_13(&self, user_id: UserId, isbn: &str) -> Result<bool, DbError>;

    /// Check if a work with the given ISBN-10 exists for a user (via external_ids).
    async fn work_exists_by_isbn_10(&self, user_id: UserId, isbn: &str) -> Result<bool, DbError>;

    /// Delete list_import_previews rows older than the given cutoff (RFC3339 string).
    async fn delete_stale_list_import_previews(&self, cutoff: &str) -> Result<u64, DbError>;

    /// Tag a specific work with an import_id. Explicit, race-free (unlike tag_last_work).
    async fn tag_work_with_import(
        &self,
        user_id: UserId,
        work_id: WorkId,
        import_id: &str,
    ) -> Result<(), DbError>;

    /// List work IDs tagged with the given import_id for a user.
    async fn list_works_by_import(
        &self,
        import_id: &str,
        user_id: UserId,
    ) -> Result<Vec<WorkId>, DbError>;
}

// ---------------------------------------------------------------------------
// Test Helpers
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "test-helpers"))]
pub mod test_helpers {
    use super::sqlite::SqliteDb;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    /// Create a test database backed by SQLite `:memory:`.
    ///
    /// Single connection (`:memory:` is per-connection), migrated, FK-on,
    /// busy_timeout matching production config.
    pub async fn create_test_db() -> SqliteDb {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .pragma("foreign_keys", "ON")
            .pragma("busy_timeout", "5000");

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();

        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        SqliteDb::new(pool)
    }
}

// ---------------------------------------------------------------------------
// TEMP(pk-tdd): compile-only scaffolding for metadata-overhaul behavioral tests
// ---------------------------------------------------------------------------

/// Re-export for external test crates that depend on `feature = "test-helpers"`.
#[cfg(any(test, feature = "test-helpers"))]
pub use test_helpers::create_test_db;

/// TEMP(pk-tdd): A typed external identifier for a work (DB layer).
/// Named ExternalId to match the behavioral test type expectations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalId {
    pub id: ExternalIdRowId,
    pub user_id: UserId,
    pub work_id: WorkId,
    pub id_type: ExternalIdType,
    pub id_value: String,
}

/// TEMP(pk-tdd): Request to upsert a typed external identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpsertExternalIdRequest {
    pub work_id: WorkId,
    pub id_type: ExternalIdType,
    pub id_value: String,
}

/// TEMP(pk-tdd): Request to set field provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetFieldProvenanceRequest {
    pub user_id: UserId,
    pub work_id: WorkId,
    pub field: WorkField,
    pub source: Option<MetadataProvider>,
    pub setter: ProvenanceSetter,
    pub cleared: bool,
}

/// TEMP(pk-tdd): Provider retry state record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRetryState {
    pub user_id: UserId,
    pub work_id: WorkId,
    pub provider: MetadataProvider,
    pub attempts: u32,
    pub suppressed_passes: u32,
    pub last_outcome: Option<OutcomeClass>,
    pub next_attempt_at: Option<chrono::DateTime<chrono::Utc>>,
    pub first_suppressed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub normalized_payload_json: Option<String>,
}

/// TEMP(pk-tdd): Request to apply an enrichment merge result to a work.
pub struct ApplyEnrichmentMergeRequest {
    pub user_id: UserId,
    pub work_id: WorkId,
    pub expected_merge_generation: i64,
    pub work_update: Option<MergeResolved<UpdateWorkEnrichmentDbRequest>>,
    pub new_enrichment_status: EnrichmentStatus,
    pub provenance_upserts: Vec<SetFieldProvenanceRequest>,
    pub provenance_deletes: Vec<WorkField>,
    pub external_id_updates: Vec<UpsertExternalIdRequest>,
}

/// TEMP(pk-tdd): DB trait for field provenance.
/// Uses async_trait to match behavioral test impl style.
#[async_trait::async_trait]
pub trait ProvenanceDb: Send + Sync {
    async fn set_field_provenance(&self, req: SetFieldProvenanceRequest) -> Result<(), DbError>;

    async fn set_field_provenance_batch(
        &self,
        reqs: Vec<SetFieldProvenanceRequest>,
    ) -> Result<(), DbError>;

    async fn get_field_provenance(
        &self,
        user_id: UserId,
        work_id: WorkId,
        field: WorkField,
    ) -> Result<Option<FieldProvenance>, DbError>;

    async fn list_work_provenance(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<FieldProvenance>, DbError>;

    async fn delete_field_provenance_batch(
        &self,
        user_id: UserId,
        work_id: WorkId,
        fields: Vec<WorkField>,
    ) -> Result<(), DbError>;

    async fn clear_work_provenance(&self, user_id: UserId, work_id: WorkId) -> Result<(), DbError>;
}

/// TEMP(pk-tdd): DB trait for provider retry state.
#[async_trait::async_trait]
pub trait ProviderRetryStateDb: Send + Sync {
    async fn get_retry_state(
        &self,
        user_id: UserId,
        work_id: WorkId,
        provider: MetadataProvider,
    ) -> Result<Option<ProviderRetryState>, DbError>;

    async fn list_retry_states(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<ProviderRetryState>, DbError>;

    async fn record_will_retry(
        &self,
        user_id: UserId,
        work_id: WorkId,
        provider: MetadataProvider,
        next_attempt_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<ProviderRetryState, DbError>;

    async fn record_suppressed(
        &self,
        user_id: UserId,
        work_id: WorkId,
        provider: MetadataProvider,
        until: chrono::DateTime<chrono::Utc>,
    ) -> Result<ProviderRetryState, DbError>;

    async fn record_terminal_outcome(
        &self,
        user_id: UserId,
        work_id: WorkId,
        provider: MetadataProvider,
        outcome: OutcomeClass,
        normalized_payload_json: Option<String>,
    ) -> Result<(), DbError>;

    async fn reset_all_retry_states(&self, user_id: UserId, work_id: WorkId)
        -> Result<(), DbError>;

    async fn list_works_due_for_retry(
        &self,
        user_id: UserId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<(WorkId, MetadataProvider)>, DbError>;

    async fn list_works_with_terminal_provider_rows(
        &self,
        user_id: UserId,
    ) -> Result<Vec<(WorkId, Vec<MetadataProvider>)>, DbError>;

    async fn reset_not_configured_outcomes(
        &self,
        provider: MetadataProvider,
    ) -> Result<u64, DbError>;
}

/// TEMP(pk-tdd): DB trait for typed external identifiers.
#[async_trait::async_trait]
pub trait ExternalIdDb: Send + Sync {
    async fn upsert_external_id(
        &self,
        user_id: UserId,
        req: UpsertExternalIdRequest,
    ) -> Result<(), DbError>;

    async fn upsert_external_ids_batch(
        &self,
        user_id: UserId,
        reqs: Vec<UpsertExternalIdRequest>,
    ) -> Result<(), DbError>;

    async fn list_external_ids(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<ExternalId>, DbError>;
}
