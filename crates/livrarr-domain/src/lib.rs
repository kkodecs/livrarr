pub mod identity;
pub mod kash;
pub mod keyed_mutex;
pub mod normalization;
pub mod readarr;
pub mod services;
pub mod settings;
pub mod text_norm;
pub mod title_cleanup;
pub mod torznab;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ID Type Aliases
// ---------------------------------------------------------------------------

pub type UserId = i64;
pub type WorkId = i64;
pub type AuthorId = i64;
pub type LibraryItemId = i64;
pub type RootFolderId = i64;
pub type GrabId = i64;
pub type DownloadClientId = i64;
pub type RemotePathMappingId = i64;
// SessionTokenHash and ApiKeyHash were previously defined here as type aliases
// for String. They were unused in struct fields (which use plain String) and
// have been removed to avoid confusion.
pub type HistoryId = i64;
pub type NotificationId = i64;
pub type ExternalIdRowId = i64;
pub type IndexerId = i64;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Canonical MediaType — ebook or audiobook.
///
/// Satisfies: IMPORT-001, IMPORT-007
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaType {
    Ebook,
    Audiobook,
}

/// Canonical UserRole — admin or user.
///
/// Satisfies: AUTH-002
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    Admin,
    User,
}

/// Grab status state machine.
///
/// Satisfies: DLC-006, DLC-008, DLC-009, DLC-012, DLC-015, IMPORT-005, IMPORT-006, IMPORT-014, IMPORT-016
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GrabStatus {
    Sent,
    Confirmed,
    Importing,
    Imported,
    ImportFailed,
    Removed,
    Failed,
}

/// Enrichment status per work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EnrichmentStatus {
    /// Initial state. Enrichment has not completed.
    /// Also used as crash recovery signal — retry job picks up Unenriched
    /// works older than 5 minutes.
    #[default]
    Unenriched,
    /// Enrichment completed. DB metadata is authoritative.
    Enriched,
    /// Confirmed identity but no meaningful text metadata was found
    /// (REQ-014/019): "we know the book, found no info." Distinct from
    /// `Unenriched` (not yet attempted) and from any identity problem.
    Thin,
    /// Enrichment attempted, transient error. Background job retries.
    Failed,
    // NOTE: identity-track outcomes ({Conflict, IdentityPending, NeedsReview}) used to
    // live here too. They were redundant projections of `IdentityStatus` and were
    // dropped (migration 055); identity state now lives solely on `IdentityStatus`
    // (incl. `NotFound` for "the LLM rejected all payloads"). EnrichmentStatus is
    // enrichment-quality only.
}

/// Persisted identity-confidence badge — the identity track of the two-state
/// split (REQ-014). This is the flat, stored, user-facing status; it is
/// distinct from the rich resolution-time [`identity::IdentityState`]
/// (`Confirmed{..}`/`Pending{..}`) and is derived from a work's anchors
/// (D-013 backfill): a work anchor → `Confirmed`; an ISBN/ASIN bridge with no
/// work anchor → `Provisional`; none → `Pending`; an open identity conflict →
/// `Conflict`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IdentityStatus {
    /// No confident identity yet (fuzzy title+author, no resolving key).
    #[default]
    Pending,
    /// Resolved to a work anchor (OL/GR/HC work key).
    Confirmed,
    /// De-facto identity (REQ-016): an ISBN/ASIN bridge resolved but no work
    /// anchor. Enriches; upgrades to `Confirmed` when a work anchor appears.
    Provisional,
    /// An identity contradiction is open (a differing confirmed anchor).
    /// Terminal until the user resolves it.
    Conflict,
    /// A non-interactive path exhausted resolution; surfaced for the user.
    NeedsReview,
    /// External sources could not verify this work's identity — the LLM rejected
    /// every provider payload as not-this-book. Distinct from `Conflict` (an open
    /// anchor dispute) and `NeedsReview` (resolver exhaustion). Terminal until
    /// `reset_for_manual_refresh`. Enrichment SIGNALS this outcome via
    /// [`crate::services::EnrichmentResult::identity_not_found`]; the caller — not
    /// enrichment — writes the badge (the one-way identity←enrichment seam).
    NotFound,
}

/// Per-file tag sync status. Tracked on LibraryItem, not on Work.
/// Enrichment is about works; tag sync is about files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TagStatus {
    /// File tags match DB metadata at `tagged_at_generation`.
    Synced,
    /// File exists but tags not yet written (imported for unenriched work,
    /// or crash interrupted tag sync).
    #[default]
    Pending,
    /// Tag write attempted and failed. Retried only when work.merge_generation
    /// advances (metadata changes).
    Failed,
}

/// History event types. Append-only.
///
/// Satisfies: spec Section 7 (history table)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EventType {
    Grabbed,
    DownloadCompleted,
    DownloadFailed,
    Imported,
    ImportFailed,
    Enriched,
    EnrichmentFailed,
    TagWritten,
    TagWriteFailed,
    FileDeleted,
}

/// Notification types — in-app notification system.
///
/// Satisfies: AUTHOR-003, AUTHOR-004, SEARCH-007
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotificationType {
    /// Author monitoring detected a new work by a monitored author.
    NewWorkDetected,
    /// Author monitoring auto-added a work (monitor_new_items enabled).
    WorkAutoAdded,
    /// Async LLM metadata resolution completed.
    MetadataUpdated,
    /// Bulk re-enrichment job completed.
    BulkEnrichmentComplete,
    /// v2.1 — a background job panicked.
    /// Satisfies: IMPL-JOBS-001
    JobPanicked,
    /// v2.1 — author monitor received 429 from Open Library.
    /// Satisfies: IMPL-JOBS-004
    RateLimitHit,
    /// Download complete but file not found locally — likely needs remote path mapping.
    PathNotFound,
    /// RSS sync auto-grabbed a release.
    RssGrabbed,
    /// RSS sync grab failed (download client unreachable or rejected).
    RssGrabFailed,
}

/// Narration type for audiobook metadata.
///
/// Satisfies: SEARCH-006 (Audnexus enrichment)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NarrationType {
    Human,
    Ai,
    AiAuthorizedReplica,
    /// TEMP(pk-tdd): compile-only scaffold for metadata-overhaul merge engine tests.
    Abridged,
    /// TEMP(pk-tdd): compile-only scaffold for metadata-overhaul merge engine tests.
    Unabridged,
}

/// Auth mechanism used for the current request.
///
/// Satisfies: AUTH-008
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthType {
    Session,
    ApiKey,
    ExternalAuth,
}

/// Queue item status (translated from qBit states).
///
/// Satisfies: DLC-011
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueStatus {
    Downloading,
    Queued,
    Paused,
    Completed,
    Warning,
    Error,
}

/// Download client implementation type.
///
/// Satisfies: DLC-002, USE-DLC-001
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DownloadClientImplementation {
    #[serde(rename = "qBittorrent")]
    #[default]
    QBittorrent,
    #[serde(rename = "sabnzbd")]
    SABnzbd,
    #[serde(rename = "transmission")]
    Transmission,
}

impl DownloadClientImplementation {
    /// Canonical client_type string for DB storage and protocol routing.
    pub fn client_type(&self) -> &'static str {
        match self {
            Self::QBittorrent => "qbittorrent",
            Self::SABnzbd => "sabnzbd",
            Self::Transmission => "transmission",
        }
    }

    pub fn protocol(&self) -> crate::services::DownloadProtocol {
        match self {
            Self::QBittorrent => crate::services::DownloadProtocol::Torrent,
            Self::SABnzbd => crate::services::DownloadProtocol::Usenet,
            Self::Transmission => crate::services::DownloadProtocol::Torrent,
        }
    }
}

/// LLM chat message role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmRole {
    System,
    User,
    Assistant,
}

/// LLM provider presets.
///
/// Satisfies: CONFIG-004
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmProvider {
    Groq,
    Gemini,
    Openai,
    Custom,
}

/// Health check result type.
///
/// Satisfies: SYS-001
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthCheckType {
    Ok,
    Warning,
    Error,
}

// ---------------------------------------------------------------------------
// Canonical Error Types
// ---------------------------------------------------------------------------

/// Database operation errors — canonical in livrarr-domain.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("not found: {entity}")]
    NotFound { entity: &'static str },

    #[error("constraint violation: {message}")]
    Constraint { message: String },

    #[error("conflict: {message}")]
    Conflict { message: String },

    #[error("data corruption in {table}.{column} (row {row_id}): {detail}")]
    DataCorruption {
        table: &'static str,
        column: &'static str,
        row_id: i64,
        detail: String,
    },

    #[error("incompatible data version: {detail}")]
    IncompatibleData { detail: String },

    #[error("database I/O error: {0}")]
    Io(#[source] Box<dyn std::error::Error + Send + Sync>),
}

// ---------------------------------------------------------------------------
// Domain Entities
// ---------------------------------------------------------------------------

/// User entity.
///
/// Satisfies: AUTH-002, AUTH-011, AUTH-013
#[derive(Clone, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub role: UserRole,
    #[serde(skip_serializing)]
    pub api_key_hash: String,
    pub setup_pending: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for User {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("User")
            .field("id", &self.id)
            .field("username", &self.username)
            .field("password_hash", &"[REDACTED]")
            .field("role", &self.role)
            .field("api_key_hash", &"[REDACTED]")
            .field("setup_pending", &self.setup_pending)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

/// Session entity.
///
/// Satisfies: AUTH-005, AUTH-006
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    #[serde(skip_serializing)]
    pub token_hash: String,
    pub user_id: UserId,
    pub persistent: bool,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("token_hash", &"[REDACTED]")
            .field("user_id", &self.user_id)
            .field("persistent", &self.persistent)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Work entity — the primary domain object.
///
/// Satisfies: SEARCH-004, SEARCH-006, SEARCH-013
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Work {
    pub id: WorkId,
    pub user_id: UserId,
    pub title: String,
    pub sort_title: Option<String>,
    pub subtitle: Option<String>,
    pub original_title: Option<String>,
    pub author_name: String,
    pub author_id: Option<AuthorId>,
    pub description: Option<String>,
    pub year: Option<i32>,
    pub series_id: Option<i64>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
    pub genres: Option<Vec<String>>,
    pub language: Option<String>,
    pub page_count: Option<i32>,
    pub duration_seconds: Option<i32>,
    pub publisher: Option<String>,
    pub publish_date: Option<String>,
    pub ol_key: Option<String>,
    pub hc_key: Option<String>,
    pub gr_key: Option<String>,
    pub isbn_13: Option<String>,
    pub asin: Option<String>,
    pub narrator: Option<Vec<String>>,
    pub narration_type: Option<NarrationType>,
    pub abridged: bool,
    pub rating: Option<f64>,
    pub rating_count: Option<i32>,
    pub enrichment_status: EnrichmentStatus,
    /// Identity-confidence track of the two-state split (REQ-014). Backfilled
    /// anchor-derived (D-013); see [`IdentityStatus`].
    #[serde(default)]
    pub identity_status: IdentityStatus,
    /// v2.1 — persisted retry counter for enrichment retry queue.
    /// Satisfies: IMPL-JOBS-005
    #[serde(default)]
    pub enrichment_retry_count: i32,
    pub enriched_at: Option<DateTime<Utc>>,
    pub enrichment_source: Option<String>,
    pub cover_url: Option<String>,
    pub cover_manual: bool,
    pub cover_source: Option<String>,
    pub cover_trust: CoverTrust,
    pub cover_width: i32,
    pub cover_height: i32,
    pub audiobook_cover_url: Option<String>,
    pub audiobook_cover_source: Option<String>,
    pub audiobook_cover_trust: CoverTrust,
    pub audiobook_cover_width: i32,
    pub audiobook_cover_height: i32,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub import_id: Option<String>,
    pub added_at: DateTime<Utc>,
}

/// Author entity.
///
/// Satisfies: AUTHOR-001, SEARCH-005
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    pub id: AuthorId,
    pub user_id: UserId,
    pub name: String,
    pub sort_name: Option<String>,
    pub ol_key: Option<String>,
    pub gr_key: Option<String>,
    pub hc_key: Option<String>,
    pub import_id: Option<String>,
    pub monitored: bool,
    pub monitor_new_items: bool,
    pub monitor_since: Option<DateTime<Utc>>,
    pub added_at: DateTime<Utc>,
}

/// Series entity — tracks a monitored book series for an author.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Series {
    pub id: i64,
    pub user_id: UserId,
    pub author_id: AuthorId,
    pub name: String,
    pub gr_key: String,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub work_count: i32,
    pub added_at: DateTime<Utc>,
}

/// Library item — one record per imported file.
///
/// Satisfies: IMPORT-015
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryItem {
    pub id: LibraryItemId,
    pub user_id: UserId,
    pub work_id: WorkId,
    pub root_folder_id: RootFolderId,
    pub path: String,
    pub media_type: MediaType,
    pub file_size: i64,
    pub import_id: Option<String>,
    pub imported_at: DateTime<Utc>,
    pub tag_status: TagStatus,
    pub tagged_at_generation: i64,
    pub duration_seconds: Option<f64>,
    pub chapter_scan_status: Option<String>,
}

/// Playback progress — reading/listening position for a library item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackProgress {
    pub id: i64,
    pub user_id: UserId,
    pub library_item_id: LibraryItemId,
    /// CFI string (EPUB), page number (PDF), or seconds as float (audio).
    pub position: String,
    /// 0.0 to 1.0.
    pub progress_pct: f64,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudiobookChapter {
    pub id: i64,
    pub library_item_id: LibraryItemId,
    pub chapter_index: i32,
    pub title: String,
    pub start_time_secs: f64,
    pub end_time_secs: f64,
}

/// A `.kash`-established 1:1 binding between one audiobook and one ebook
/// LibraryItem. Each item is in at most one link (UNIQUE both sides).
/// `kash_path` is never persisted — derived from the audio item's path.
#[derive(Debug, Clone, PartialEq)]
pub struct KashLink {
    pub id: i64,
    pub audio_item_id: LibraryItemId,
    pub ebook_item_id: LibraryItemId,
    /// Audio identity/drift reference (REQ-014): container duration at link time.
    pub container_duration_secs: f64,
    /// Ebook identity/drift reference (REQ-008).
    pub epub_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewKashLink {
    pub audio_item_id: LibraryItemId,
    pub ebook_item_id: LibraryItemId,
    pub container_duration_secs: f64,
    pub epub_hash: String,
}

/// Per-(user, link) cross-format resume state: the monotonic furthest mark
/// (audio-timestamp space) plus per-format decline-suppression thresholds.
#[derive(Debug, Clone, PartialEq)]
pub struct CrossFormatState {
    pub user_id: UserId,
    pub kash_link_id: i64,
    pub furthest_ts: f64,
    pub ebook_declined_at_ts: Option<f64>,
    pub audio_declined_at_ts: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: i64,
    pub user_id: UserId,
    pub work_id: WorkId,
    pub library_item_id: LibraryItemId,
    pub media_type: MediaType,
    pub position: String,
    pub sort_key: f64,
    pub name: String,
    pub chapter_title: Option<String>,
    pub paired_bookmark_id: Option<i64>,
    pub created_at: DateTime<Utc>,
}

/// Root folder.
///
/// Satisfies: IMPORT-001, IMPORT-002, IMPORT-003
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootFolder {
    pub id: RootFolderId,
    pub path: String,
    pub media_type: MediaType,
}

/// Download client configuration.
///
/// Satisfies: DLC-001, DLC-002, USE-DLC-001, USE-DLC-004
#[derive(Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DownloadClient {
    pub id: DownloadClientId,
    pub name: String,
    pub implementation: DownloadClientImplementation,
    pub host: String,
    pub port: u16,
    pub use_ssl: bool,
    pub skip_ssl_validation: bool,
    pub url_base: Option<String>,
    pub username: Option<String>,
    #[serde(skip_serializing)]
    pub password: Option<String>,
    pub category: String,
    pub download_dir: Option<String>,
    pub enabled: bool,
    #[serde(skip_serializing)]
    pub api_key: Option<String>,
    pub is_default_for_protocol: bool,
}

impl DownloadClient {
    /// Canonical client_type string derived from implementation — single source of truth.
    pub fn client_type(&self) -> &'static str {
        self.implementation.client_type()
    }
}

impl std::fmt::Debug for DownloadClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DownloadClient")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("implementation", &self.implementation)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("use_ssl", &self.use_ssl)
            .field("skip_ssl_validation", &self.skip_ssl_validation)
            .field("url_base", &self.url_base)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("category", &self.category)
            .field("enabled", &self.enabled)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("is_default_for_protocol", &self.is_default_for_protocol)
            .finish()
    }
}

/// Grab record — tracks a torrent download.
///
/// Satisfies: DLC-006, DLC-009
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Grab {
    pub id: GrabId,
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
    pub import_error: Option<String>,
    pub media_type: Option<MediaType>,
    /// Raw remote path from download client (pre-path-mapping).
    pub content_path: Option<String>,
    pub grabbed_at: DateTime<Utc>,
    pub import_retry_count: i32,
    pub import_failed_at: Option<DateTime<Utc>>,
}

/// Remote path mapping.
///
/// Satisfies: DLC-013
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RemotePathMapping {
    pub id: RemotePathMappingId,
    pub host: String,
    pub remote_path: String,
    pub local_path: String,
}

/// History event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEvent {
    pub id: HistoryId,
    pub user_id: UserId,
    pub work_id: Option<WorkId>,
    pub event_type: EventType,
    pub data: serde_json::Value,
    pub date: DateTime<Utc>,
}

pub struct HistoryFilter {
    pub event_type: Option<EventType>,
    pub work_id: Option<WorkId>,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
}

/// Notification.
///
/// Satisfies: AUTHOR-003, AUTHOR-005
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: NotificationId,
    pub user_id: UserId,
    pub notification_type: NotificationType,
    pub ref_key: Option<String>,
    pub message: String,
    pub data: serde_json::Value,
    pub read: bool,
    pub dismissed: bool,
    pub created_at: DateTime<Utc>,
}

/// External ID row (additional ISBNs, ASINs, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalId {
    pub id: ExternalIdRowId,
    pub work_id: WorkId,
    pub id_type: String,
    pub id_value: String,
}

/// Torznab/Newznab indexer configuration.
///
/// Satisfies: IDX-001, IDX-002, IDX-004, IDX-005, IDX-006, IDX-007
#[derive(Clone, Serialize, Deserialize)]
pub struct Indexer {
    pub id: IndexerId,
    pub name: String,
    pub protocol: String,
    pub url: String,
    pub api_path: String,
    #[serde(skip_serializing)]
    pub api_key: Option<String>,
    pub categories: Vec<i32>,
    pub priority: i32,
    pub enable_automatic_search: bool,
    pub enable_interactive_search: bool,
    pub supports_book_search: bool,
    pub enable_rss: bool,
    pub enabled: bool,
    pub added_at: DateTime<Utc>,
}

impl std::fmt::Debug for Indexer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Indexer")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("protocol", &self.protocol)
            .field("url", &self.url)
            .field("api_path", &self.api_path)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("categories", &self.categories)
            .field("priority", &self.priority)
            .field("enable_automatic_search", &self.enable_automatic_search)
            .field("enable_interactive_search", &self.enable_interactive_search)
            .field("supports_book_search", &self.supports_book_search)
            .field("enable_rss", &self.enable_rss)
            .field("enabled", &self.enabled)
            .field("added_at", &self.added_at)
            .finish()
    }
}

/// Per-indexer RSS sync state for gap detection.
///
/// Satisfies: RSS-GAP-001
#[derive(Debug, Clone)]
pub struct IndexerRssState {
    pub indexer_id: IndexerId,
    pub last_publish_date: Option<String>,
    pub last_guid: Option<String>,
}

/// Indexer config singleton (RSS sync settings).
///
/// Satisfies: RSS-CONFIG-001
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexerConfig {
    pub rss_sync_interval_minutes: i32,
    pub rss_match_threshold: f64,
}

/// Import record — tracks a Readarr library import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Import {
    pub id: String,
    pub user_id: UserId,
    pub source: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub authors_created: i64,
    pub works_created: i64,
    pub files_imported: i64,
    pub files_skipped: i64,
    pub source_url: Option<String>,
    pub target_root_folder_id: Option<i64>,
}

/// Sanitizes a path component for filesystem use.
///
/// Satisfies: IMPORT-011
pub fn sanitize_path_component(input: &str, fallback: &str) -> String {
    const MAX_BYTES: usize = 255;
    const ELLIPSIS: &str = "...";

    fn sanitize_inner(s: &str) -> String {
        const ILLEGAL: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

        // Strip control characters, replace illegal chars with underscore
        let sanitized: String = s
            .chars()
            .filter(|c| !c.is_control())
            .map(|c| if ILLEGAL.contains(&c) { '_' } else { c })
            .collect();

        // Trim trailing dots and spaces
        sanitized.trim_end_matches(['.', ' ']).to_string()
    }

    let trimmed = sanitize_inner(input);

    // "." / ".." or empty after sanitization -> sanitize fallback too
    let result = if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        let fb = sanitize_inner(fallback);
        if fb.is_empty() || fb == "." || fb == ".." {
            // Ultimate fallback if even the fallback is invalid
            return "_".to_string();
        }
        fb
    } else {
        trimmed
    };

    // Truncate to MAX_BYTES if needed
    if result.len() > MAX_BYTES {
        let max_content = MAX_BYTES - ELLIPSIS.len();
        // Find the last valid UTF-8 char boundary at or before max_content
        let mut end = max_content;
        while !result.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}{}", &result[..end], ELLIPSIS)
    } else {
        result
    }
}

/// Derives sort name from display name using a surname-as-last-word heuristic.
///
/// Note: Assumes the last whitespace-delimited word is the surname. This is
/// incorrect for some naming conventions (e.g., East Asian, Iberian, compound
/// surnames like "van der Berg"), but matches the Readarr/Servarr convention.
///
/// "Frank Herbert" -> "Herbert, Frank"
/// "J.R.R. Tolkien" -> "Tolkien, J.R.R."
/// Single-word name -> returned as-is.
pub fn derive_sort_name(display_name: &str) -> String {
    let trimmed = display_name.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Use rsplit_once to split at the last whitespace boundary.
    // This avoids collecting into an intermediate Vec.
    match trimmed.rsplit_once(char::is_whitespace) {
        Some((given, surname)) => format!("{}, {}", surname.trim(), given.trim()),
        None => trimmed.to_string(),
    }
}

/// Normalizes a string for scan matching. Applies the same character rules
/// as `sanitize_path_component` but replaces illegal chars with spaces
/// (for matching) instead of underscores (for filesystem). Also replaces
/// dots and underscores with spaces so that Livrarr-imported filenames
/// (which use underscores for illegal chars) match back to their DB titles.
///
/// Satisfies: SCAN-002, SCAN-003
pub fn normalize_for_matching(s: &str) -> String {
    const ILLEGAL: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let normalized: String = s
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| {
            if ILLEGAL.contains(&c) || c == '.' || c == '_' {
                ' '
            } else {
                c
            }
        })
        .collect();
    // Collapse multiple spaces and trim
    let mut result = String::with_capacity(normalized.len());
    let mut prev_space = true; // trim leading
    for c in normalized.chars() {
        if c == ' ' {
            if !prev_space {
                result.push(' ');
            }
            prev_space = true;
        } else {
            result.push(c);
            prev_space = false;
        }
    }
    // Trim trailing space
    if result.ends_with(' ') {
        result.pop();
    }
    result.to_lowercase()
}

/// Marker prefix for stub series gr_keys (series rows created from work
/// metadata rather than Goodreads). A real GR series key is numeric, so the
/// prefix cannot collide. Stub keys are internal — API responses mask them.
pub const SERIES_STUB_KEY_PREFIX: &str = "stub:";

/// Sentinel `work_count` for stub series: any GR-backed series (real,
/// smaller roster) beats a stub under the "fewest books wins" assignment
/// guard; a stub never steals a work. Masked to 0 at the API boundary.
pub const SERIES_STUB_WORK_COUNT: i32 = i32::MAX;

pub fn is_series_stub_key(gr_key: &str) -> bool {
    gr_key.starts_with(SERIES_STUB_KEY_PREFIX)
}

/// Splits a positional suffix off a series name: `"The Wheel of Time, Book 3"`
/// → `("The Wheel of Time", Some(3.0))`. Recognized suffix forms after the
/// last comma: `Book N`, `#N`, `Vol N`, `Vol. N`, `Volume N` (N may be
/// fractional, e.g. `3.5`). A name with no recognized suffix is returned
/// trimmed, with `None`.
pub fn split_series_suffix(name: &str) -> (String, Option<f64>) {
    let trimmed = name.trim();
    if let Some((prefix, suffix)) = trimmed.rsplit_once(',') {
        let prefix = prefix.trim();
        let suffix = suffix.trim();
        if !prefix.is_empty() {
            let number_part = suffix
                .strip_prefix('#')
                .or_else(|| {
                    [
                        "Book", "book", "Volume", "volume", "Vol.", "vol.", "Vol", "vol",
                    ]
                    .iter()
                    .find_map(|kw| suffix.strip_prefix(kw))
                })
                .map(str::trim);
            if let Some(n) = number_part.and_then(|p| p.parse::<f64>().ok()) {
                if n.is_finite() {
                    return (prefix.to_string(), Some(n));
                }
            }
        }
    }
    (trimmed.to_string(), None)
}

/// Normalize a language value to an ISO 639-1 two-letter code.
///
/// Delegates to [`crate::normalization::normalize_language`] — the single
/// normalization authority (REQ-005) — and falls back to the trimmed,
/// lower-cased input for a value that authority does not recognize, preserving
/// this function's historical pass-through contract for its enrichment callers.
/// (Unlike the previous local table, this now also strips region subtags from
/// recognized languages, e.g. `"en-US"` → `"en"`.)
pub fn normalize_language(lang: &str) -> String {
    crate::normalization::normalize_language(lang).unwrap_or_else(|| lang.trim().to_lowercase())
}

/// Normalize an optional language value.
pub fn normalize_language_opt(lang: Option<&str>) -> Option<String> {
    lang.filter(|s| !s.is_empty()).map(normalize_language)
}

/// Classifies a file path into a MediaType based on extension.
pub fn classify_file(path: &std::path::Path) -> Option<MediaType> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    match ext.as_str() {
        "epub" | "mobi" | "azw3" | "pdf" => Some(MediaType::Ebook),
        "mp3" | "m4a" | "m4b" | "flac" | "ogg" | "wma" => Some(MediaType::Audiobook),
        _ => None,
    }
}

pub fn decode_xml_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
}

/// Proxy an external cover URL through the internal cover proxy endpoint.
/// URLs already starting with '/' are returned as-is (already local).
pub fn proxy_cover_url(url: &str) -> String {
    if url.starts_with('/') {
        return url.to_string();
    }
    format!("/api/v1/coverproxy?url={}", urlencoding::encode(url))
}

/// Reverse `proxy_cover_url`: recover the canonical external URL from the
/// internal cover-proxy display form (`/api/v1/coverproxy?url=<encoded-url>`).
/// Values that are not in proxied form are returned unchanged.
///
/// The search results the UI renders carry covers in proxied form so `<img>`
/// tags can fetch them. When the user picks one of those covers, the persisted
/// value must be the real provider URL, not the proxied display string — a
/// proxied (leading-`/`) value is not a usable cover source.
pub fn unproxy_cover_url(url: &str) -> String {
    match url.strip_prefix("/api/v1/coverproxy?url=") {
        Some(encoded) => urlencoding::decode(encoded)
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| url.to_string()),
        None => url.to_string(),
    }
}

/// Strip all non-alphanumeric characters from an ISBN (hyphens, spaces, etc.).
pub fn normalize_isbn(isbn: &str) -> String {
    isbn.chars().filter(|c| c.is_alphanumeric()).collect()
}

// ---------------------------------------------------------------------------
// TEMP(pk-tdd): compile-only scaffolding for metadata-overhaul behavioral tests
// All types below are IR-aligned stubs. Remove TEMP tag when implemented.
// ---------------------------------------------------------------------------

/// Which metadata provider produced a given field value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataProvider {
    Hardcover,
    OpenLibrary,
    Goodreads,
    Audnexus,
    Llm,
    /// Source data from a Readarr import. Treated as another provider
    /// input in the merge engine — ranked above OL, below GR.
    Readarr,
    GoogleBooks,
    Audible,
}

impl MetadataProvider {
    /// Canonical snake_case provider key used in call records and retry state
    /// (the REQ-001 reporting vocabulary) — never a display name.
    pub fn record_key(self) -> &'static str {
        match self {
            Self::Hardcover => "hardcover",
            Self::OpenLibrary => "openlibrary",
            Self::Goodreads => "goodreads",
            Self::Audnexus => "audnexus",
            Self::Llm => "llm",
            Self::Readarr => "readarr",
            Self::GoogleBooks => "google_books",
            Self::Audible => "audible",
        }
    }
}

/// Trust level for a work's cover image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CoverTrust {
    #[default]
    Unvalidated,
    Validated,
    User,
}

impl CoverTrust {
    pub fn allows_replacement_by(self, incoming: CoverTrust) -> bool {
        match (self, incoming) {
            (CoverTrust::User, _) => false,
            (CoverTrust::Validated, CoverTrust::User) => true,
            (CoverTrust::Validated, CoverTrust::Validated) => true,
            (CoverTrust::Validated, CoverTrust::Unvalidated) => false,
            (CoverTrust::Unvalidated, _) => true,
        }
    }
}

/// Which cover slot: ebook or audiobook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoverMediaType {
    Ebook,
    Audiobook,
}

impl CoverMediaType {
    pub fn suffix(&self) -> &'static str {
        match self {
            CoverMediaType::Ebook => "",
            CoverMediaType::Audiobook => "_audio",
        }
    }
}

/// Source of a cover candidate — wraps MetadataProvider for standard providers,
/// adds EPUB and ISBN-based sources that don't participate in enrichment infra.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoverCandidateSource {
    Provider(MetadataProvider),
    Epub,
    IsbnOl,
    IsbnAmazon,
}

impl std::fmt::Display for CoverCandidateSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provider(p) => write!(f, "{p:?}"),
            Self::Epub => write!(f, "epub"),
            Self::IsbnOl => write!(f, "isbn_ol"),
            Self::IsbnAmazon => write!(f, "isbn_amazon"),
        }
    }
}

/// A cover candidate with browser-safe proxied URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverCandidate {
    pub candidate_id: String,
    pub proxy_url: String,
    pub source: String,
    pub media_type: CoverMediaType,
    pub width: u32,
    pub height: u32,
    pub passes_quality_gate: bool,
}

/// Internal cover candidate with raw provider URL — never serialize to browser.
#[derive(Debug, Clone)]
pub struct InternalCoverCandidate {
    pub source: CoverCandidateSource,
    pub url: String,
    pub media_type: CoverMediaType,
    pub edition_title: Option<String>,
}

/// Request to select a cover from alternatives.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectCoverRequest {
    pub candidate_id: String,
    pub media_type: CoverMediaType,
}

/// Result of a cover resolution during enrichment merge.
#[derive(Debug, Clone)]
pub struct CoverResolution {
    pub url: String,
    pub source: String,
    pub trust: CoverTrust,
    pub media_type: CoverMediaType,
}

/// A named work field that can have per-provider provenance tracked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkField {
    Title,
    SortTitle,
    Subtitle,
    OriginalTitle,
    AuthorName,
    Description,
    Year,
    SeriesName,
    SeriesPosition,
    Genres,
    Language,
    PageCount,
    DurationSeconds,
    Publisher,
    PublishDate,
    OlKey,
    HcKey,
    GrKey,
    Isbn13,
    Asin,
    Narrator,
    NarrationType,
    Abridged,
    Rating,
    RatingCount,
    CoverUrl,
}

impl WorkField {
    /// TEMP(pk-tdd): compile-only scaffold — returns the normalization class for this field.
    pub fn normalization_class(self) -> NormalizationClass {
        match self {
            WorkField::Description => NormalizationClass::RichText,
            WorkField::Title
            | WorkField::SortTitle
            | WorkField::Subtitle
            | WorkField::OriginalTitle
            | WorkField::AuthorName
            | WorkField::SeriesName
            | WorkField::Publisher
            | WorkField::Narrator
            | WorkField::NarrationType => NormalizationClass::DisplayText,
            WorkField::Isbn13 | WorkField::Asin | WorkField::OlKey | WorkField::GrKey => {
                NormalizationClass::Identifier
            }
            _ => NormalizationClass::DisplayText,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceSetter {
    /// Field value was set by a metadata provider during enrichment.
    Provider,
    /// Field value was directly set or selected by the user (typing it,
    /// picking from search results, manually editing). Acts as the
    /// identity-lock anchor for LLM validation — providers returning data
    /// inconsistent with a User-set field have their payload rejected.
    User,
    /// Field value was set by the system in a contextless way (e.g.
    /// system-assigned defaults). Not a lock anchor.
    System,
    /// Field value originated from an automated add path (author-monitor
    /// auto-add or series auto-add) where the user did not per-work
    /// validate. Honest about provenance — NOT treated as a lock anchor
    /// for LLM identity verification. A user-confirm UX (future) can
    /// transition AutoAdded → User on confirm.
    AutoAdded,
    /// Field value originated from a bulk list import (CSV upload).
    Imported,
    /// Field value originated from an external system import (e.g., Readarr).
    /// Treated as provider-quality data, not user-sovereign.
    Import,
}

/// Provenance record for a single field value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldProvenance {
    pub user_id: UserId,
    pub work_id: WorkId,
    pub field: WorkField,
    pub source: Option<MetadataProvider>,
    pub set_at: DateTime<Utc>,
    pub setter: ProvenanceSetter,
    pub cleared: bool,
}

/// Priority of a metadata request, used for queue ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RequestPriority {
    Low,
    Normal,
    High,
    Interactive,
}

/// Normalization class for a field or work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NormalizationClass {
    /// Rich text fields (HTML, markdown).
    RichText,
    /// Plain display text fields.
    DisplayText,
    /// Structured identifier fields.
    Identifier,
    /// Work-level: English-language merge strategy.
    English,
    /// Work-level: foreign-language merge strategy.
    ForeignLanguage,
}

/// Outcome class returned by a provider for a single field or whole work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeClass {
    /// Provider returned a usable value.
    Success,
    /// Provider returned no match for this work.
    NotFound,
    /// Provider is not configured — retriable when config changes.
    NotConfigured,
    /// Provider returned data that will be retried.
    WillRetry,
    /// Provider returned an error that will not resolve on retry.
    PermanentFailure,
    /// Provider returned data that conflicts with existing confirmed data.
    Conflict,
    /// Provider was suppressed (circuit open, rate-limit window, etc.).
    Suppressed,
}

/// Per-field / per-provider merge dissent (REQ-013/014): an excluded
/// contribution, recorded queryably. A dissent isolates at provider or field
/// granularity and never discards the work's merge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldDissent {
    pub work_id: WorkId,
    /// Canonical provider key (e.g. "google_books").
    pub provider: String,
    /// Work column name. Payload-level dissent records one row per affected
    /// field.
    pub field: String,
    pub offered_value: String,
    pub winning_value: Option<String>,
    pub reason: DissentReason,
    pub merge_generation: i64,
    pub recorded_at: DateTime<Utc>,
}

/// Why a contribution was excluded from the merge (REQ-013/014).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DissentReason {
    /// The provider appears to describe a different book.
    PayloadMismatch,
    /// Contradictory value for one field.
    FieldConflict,
    /// Known-incompatible payload language on a foreign work (REQ-013).
    LanguageIncompatible,
}

/// Anchor-only enrichment fetch vocabulary (REQ-006). The input type admits no
/// title or author, so no enrichment fetch can construct a text search
/// (AC-007). Text search exists only behind the lookup/identity surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AnchorQuery {
    Isbn13(String),
    GrKey(String),
    HcKey(String),
    OlKey(String),
    Asin(String),
}

/// Tracing-init outcome surfaced on the status page (REQ-003): the daily
/// rolling file actually written and any init failure — never swallowed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogSurfaceStatus {
    /// The daily rolling file the appender actually writes.
    pub active_path: std::path::PathBuf,
    /// Log-dir creation/write failure captured at startup (#102's vector).
    pub init_error: Option<String>,
}

impl OutcomeClass {
    pub fn is_phase2_terminal(&self) -> bool {
        matches!(
            self,
            OutcomeClass::Success
                | OutcomeClass::NotFound
                | OutcomeClass::PermanentFailure
                | OutcomeClass::Conflict
                | OutcomeClass::NotConfigured
        )
    }

    pub fn can_merge(&self) -> bool {
        matches!(
            self,
            OutcomeClass::Success | OutcomeClass::NotFound | OutcomeClass::PermanentFailure
        )
    }

    pub fn all_can_merge(outcomes: &[OutcomeClass]) -> bool {
        outcomes.iter().all(|o| o.can_merge())
    }
}

/// Reason a provider will be retried later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WillRetryReason {
    Timeout,
    RateLimit,
    ServerError,
    AntiBotBlock,
}

/// Reason a provider permanently failed for this work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermanentFailureReason {
    ProviderPanic,
    RetryBudgetExhausted,
    InvalidResponse,
    Unsupported,
    IdentityMismatch,
    SuppressionExhausted,
}

/// Result of applying an enrichment merge to the work record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplyMergeOutcome {
    Applied,
    NoChange,
    Deferred,
    Superseded,
}

/// A resolved value from a merge (newtype wrapper).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeResolved<T>(pub T);

impl<T> MergeResolved<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    pub fn into_inner(self) -> T {
        self.0
    }

    pub fn as_inner(&self) -> &T {
        &self.0
    }

    pub fn as_inner_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

/// Typed external identifier kind for a work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalIdType {
    Isbn10,
    Isbn13,
    Asin,
    OpenLibraryWork,
    OpenLibraryEdition,
    GoodreadsBook,
    HardcoverBook,
    GoogleBooksVolume,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueProgress {
    pub percent: f64,
    pub eta: Option<i64>,
    pub download_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSummary {
    pub total: i64,
    pub downloading: i64,
    pub importing: i64,
}

#[cfg(test)]
mod series_suffix_tests {
    use super::split_series_suffix;

    #[test]
    fn strips_book_n() {
        assert_eq!(
            split_series_suffix("The Wheel of Time, Book 3"),
            ("The Wheel of Time".to_string(), Some(3.0))
        );
    }

    #[test]
    fn strips_hash_n() {
        assert_eq!(
            split_series_suffix("Dresden Files, #12"),
            ("Dresden Files".to_string(), Some(12.0))
        );
    }

    #[test]
    fn strips_fractional_position() {
        assert_eq!(
            split_series_suffix("Saga, Book 3.5"),
            ("Saga".to_string(), Some(3.5))
        );
    }

    #[test]
    fn strips_volume_forms() {
        assert_eq!(
            split_series_suffix("Foo, Volume 2"),
            ("Foo".to_string(), Some(2.0))
        );
        assert_eq!(
            split_series_suffix("Foo, Vol. 4"),
            ("Foo".to_string(), Some(4.0))
        );
    }

    #[test]
    fn plain_name_untouched() {
        assert_eq!(
            split_series_suffix("The Green Bone Saga"),
            ("The Green Bone Saga".to_string(), None)
        );
    }

    #[test]
    fn comma_without_positional_suffix_untouched() {
        assert_eq!(
            split_series_suffix("Hello, World"),
            ("Hello, World".to_string(), None)
        );
        assert_eq!(
            split_series_suffix("The Series, 3"),
            ("The Series, 3".to_string(), None)
        );
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(
            split_series_suffix("  Uplift Saga  "),
            ("Uplift Saga".to_string(), None)
        );
    }
}
