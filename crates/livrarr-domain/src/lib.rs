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
///
/// Satisfies: SEARCH-006, SEARCH-008
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum EnrichmentStatus {
    #[default]
    Pending,
    Partial,
    Enriched,
    Failed,
    /// v2.1 — terminal state after 3 retry failures.
    /// Satisfies: IMPL-JOBS-005
    Exhausted,
    /// Foreign-language work — enrichment intentionally skipped.
    Skipped,
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
}

impl DownloadClientImplementation {
    /// Canonical client_type string for DB storage and protocol routing.
    pub fn client_type(&self) -> &'static str {
        match self {
            Self::QBittorrent => "qbittorrent",
            Self::SABnzbd => "sabnzbd",
        }
    }

    /// Protocol name for API responses and routing.
    pub fn protocol(&self) -> &'static str {
        match self {
            Self::QBittorrent => "torrent",
            Self::SABnzbd => "usenet",
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
    /// v2.1 — persisted retry counter for enrichment retry queue.
    /// Satisfies: IMPL-JOBS-005
    #[serde(default)]
    pub enrichment_retry_count: i32,
    pub enriched_at: Option<DateTime<Utc>>,
    pub enrichment_source: Option<String>,
    pub cover_url: Option<String>,
    pub cover_manual: bool,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub added_at: DateTime<Utc>,
    /// Foreign language provider attribution (e.g., "BnF", "lubimyczytac.pl").
    /// Null for existing English/OL works.
    #[serde(default)]
    pub metadata_source: Option<String>,
    /// Detail page URL for foreign work enrichment (e.g., Goodreads book page).
    /// Server-side only — never exposed in API responses.
    #[serde(default)]
    pub detail_url: Option<String>,
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
    pub monitored: bool,
    pub monitor_new_items: bool,
    pub monitor_since: Option<DateTime<Utc>>,
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
    pub imported_at: DateTime<Utc>,
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

/// Classifies a file path into a MediaType based on extension.
pub fn classify_file(path: &std::path::Path) -> Option<MediaType> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    match ext.as_str() {
        "epub" | "mobi" | "azw3" | "pdf" => Some(MediaType::Ebook),
        "mp3" | "m4a" | "m4b" | "flac" | "ogg" | "wma" => Some(MediaType::Audiobook),
        _ => None,
    }
}
