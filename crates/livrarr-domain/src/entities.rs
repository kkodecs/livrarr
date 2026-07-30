//! Core domain entities — the primary nouns of the system (User, Session,
//! Work, Author, Series, LibraryItem, Grab, HistoryEvent, Notification, ...)
//! plus their id type aliases, lifecycle-status enums, and the canonical
//! `DbError` type.

use crate::enrichment_types::CoverTrust;
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

impl MediaType {
    /// The serde string form ("ebook" / "audiobook"), for event payloads and
    /// other places that need the canonical lowercase name.
    pub fn as_str(&self) -> &'static str {
        match self {
            MediaType::Ebook => "ebook",
            MediaType::Audiobook => "audiobook",
        }
    }
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
/// distinct from the rich resolution-time [`crate::identity::IdentityState`]
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
    Added,
    WorkDeleted,
    WorksMerged,
    IdentityResolved,
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
    /// RSS sync skipped one or more releases that declared no language against
    /// a work outside the user's default language (REQ-011/AC-022) — surfaces
    /// the silent, background-only skip so it never sits in unnoticed limbo.
    RssLanguageSkipped,
    /// RSS sync suppressed further auto-grab attempts for a work+media_type
    /// after too many terminal failures (importFailed/failed) within the
    /// 30-day window — prevents re-downloading a release that keeps failing
    /// to import. Fires once per (work, media_type). Satisfies: 114a.
    RssGrabSuppressed,
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

impl QueueStatus {
    /// Canonical lowercase string form — matches this enum's serde serialization
    /// and the frontend `QueueStatus` union type.
    pub const fn as_str(self) -> &'static str {
        match self {
            QueueStatus::Downloading => "downloading",
            QueueStatus::Queued => "queued",
            QueueStatus::Paused => "paused",
            QueueStatus::Completed => "completed",
            QueueStatus::Warning => "warning",
            QueueStatus::Error => "error",
        }
    }
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

    #[error("author-link claim lost")]
    ClaimLost,

    #[error("identity collision: {entity} \"{name}\" (id {id}) already holds this identity")]
    IdentityCollision {
        entity: &'static str,
        id: i64,
        name: String,
    },

    #[error("cannot remove the last remaining admin")]
    LastAdmin,

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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
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
    /// The user's language choice for monitor-created works (REQ-003).
    /// `None` = never configured; the seed builder applies the system default.
    pub monitor_language: Option<String>,
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
    /// The user's language choice for monitor-created works (REQ-003).
    /// `None` = never configured; the seed builder applies the system default.
    pub monitor_language: Option<String>,
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
