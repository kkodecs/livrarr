pub mod keyed_mutex;
pub mod readarr;
pub mod services;
pub mod settings;
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
    /// TEMP(pk-tdd): compile-only scaffold — Conflicting metadata from providers.
    Conflict,
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
}

impl DownloadClientImplementation {
    /// Canonical client_type string for DB storage and protocol routing.
    pub fn client_type(&self) -> &'static str {
        match self {
            Self::QBittorrent => "qbittorrent",
            Self::SABnzbd => "sabnzbd",
        }
    }

    pub fn protocol(&self) -> crate::services::DownloadProtocol {
        match self {
            Self::QBittorrent => crate::services::DownloadProtocol::Torrent,
            Self::SABnzbd => crate::services::DownloadProtocol::Usenet,
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
    pub import_id: Option<String>,
    pub added_at: DateTime<Utc>,
    /// Foreign language provider attribution (e.g., "BnF", "lubimyczytac.pl").
    /// Null for existing English/OL works.
    #[serde(default)]
    pub metadata_source: Option<String>,
    /// Detail page URL for foreign work enrichment (e.g., Goodreads book page).
    /// Server-side only — never exposed in API responses.
    #[serde(default, skip_serializing)]
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

/// Normalize a language value to ISO 639-1 two-letter code.
/// Handles full English names (from Goodreads JSON-LD), three-letter codes,
/// and passes through already-correct two-letter codes.
pub fn normalize_language(lang: &str) -> String {
    let lower = lang.trim().to_lowercase();
    match lower.as_str() {
        "english" | "eng" => "en",
        "french" | "français" | "fra" | "fre" => "fr",
        "german" | "deutsch" | "deu" | "ger" => "de",
        "spanish" | "español" | "spa" => "es",
        "polish" | "polski" | "pol" => "pl",
        "dutch" | "nederlands" | "nld" | "dut" => "nl",
        "italian" | "italiano" | "ita" => "it",
        "portuguese" | "português" | "por" => "pt",
        "japanese" | "日本語" | "jpn" => "ja",
        "korean" | "한국어" | "kor" => "ko",
        "chinese" | "中文" | "zho" | "chi" => "zh",
        "russian" | "русский" | "rus" => "ru",
        "swedish" | "svenska" | "swe" => "sv",
        "norwegian" | "norsk" | "nor" => "no",
        "danish" | "dansk" | "dan" => "da",
        "finnish" | "suomi" | "fin" => "fi",
        "czech" | "čeština" | "ces" | "cze" => "cs",
        "turkish" | "türkçe" | "tur" => "tr",
        "arabic" | "العربية" | "ara" => "ar",
        "hindi" | "हिन्दी" | "hin" => "hi",
        "romanian" | "română" | "ron" | "rum" => "ro",
        "hungarian" | "magyar" | "hun" => "hu",
        other => return other.to_string(),
    }
    .to_string()
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

// ---------------------------------------------------------------------------
// SourceKind — centralized metadata source representation
// ---------------------------------------------------------------------------

/// Canonical metadata source. Replaces string-based `is_foreign_source()` checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    OpenLibrary,
    Hardcover,
    Goodreads,
    Audnexus,
    CasaDelLibro,
    SruDnb,
    LubimyCzytac,
    WebSearch,
    Readarr,
    /// Catch-all for unrecognized sources that are treated as foreign.
    Other,
}

impl SourceKind {
    /// Returns true if this source represents a foreign-language provider.
    pub fn is_foreign(&self) -> bool {
        !matches!(self, Self::OpenLibrary | Self::Readarr)
    }
}

impl std::fmt::Display for SourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::OpenLibrary => "OpenLibrary",
            Self::Hardcover => "Hardcover",
            Self::Goodreads => "Goodreads",
            Self::Audnexus => "Audnexus",
            Self::CasaDelLibro => "CasaDelLibro",
            Self::SruDnb => "SruDnb",
            Self::LubimyCzytac => "lubimyczytac.pl",
            Self::WebSearch => "WebSearch",
            Self::Readarr => "readarr",
            Self::Other => "other",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for SourceKind {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "openlibrary" => Self::OpenLibrary,
            "hardcover" => Self::Hardcover,
            "goodreads" => Self::Goodreads,
            "audnexus" => Self::Audnexus,
            "casadellibro" => Self::CasaDelLibro,
            "srudnb" => Self::SruDnb,
            "lubimyczytac.pl" | "lubimyczytac" => Self::LubimyCzytac,
            "websearch" | "web search" => Self::WebSearch,
            "readarr" => Self::Readarr,
            _ => Self::Other,
        })
    }
}

/// Check if a metadata_source string represents a foreign source.
/// Convenience function wrapping SourceKind parsing.
pub fn is_foreign_source(metadata_source: Option<&str>) -> bool {
    match metadata_source {
        None => false,
        Some(s) => s.parse::<SourceKind>().unwrap().is_foreign(),
    }
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
