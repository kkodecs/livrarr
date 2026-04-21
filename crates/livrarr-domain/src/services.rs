/// TEMP(pk-tdd): compile-only scaffold — removed by pk-implement
///
/// Service layer consolidation types and traits. All defined in livrarr-domain
/// per architecture spec. Implementations live in capability crates.
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use crate::settings::{
    CreateDownloadClientParams, CreateIndexerParams, UpdateDownloadClientParams, UpdateEmailParams,
    UpdateIndexerConfigParams, UpdateIndexerParams, UpdateMediaManagementParams,
    UpdateMetadataParams, UpdateProwlarrParams,
};
use crate::{
    Author, AuthorId, DbError, DownloadClient, DownloadClientId, EnrichmentStatus, Grab, GrabId,
    GrabStatus, HistoryEvent, HistoryFilter, Indexer, IndexerConfig, IndexerId, LibraryItem,
    LibraryItemId, MediaType, MetadataProvider, Notification, NotificationId, NotificationType,
    OutcomeClass, PlaybackProgress, ProvenanceSetter, QueueProgress, RemotePathMapping,
    RemotePathMappingId, RootFolder, RootFolderId, Series, UserId, Work, WorkId,
};
use serde::{Deserialize, Serialize};

// =============================================================================
// HTTP Fetcher types
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RateBucket {
    OpenLibrary,
    Hardcover,
    Audnexus,
    Goodreads,
    Indexer(String),
    None,
}

#[derive(Debug, Clone)]
pub enum UserAgentProfile {
    Browser,
    Server,
    Custom(String),
}

#[derive(Debug)]
pub struct FetchRequest {
    pub url: String,
    pub method: HttpMethod,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
    pub rate_bucket: RateBucket,
    pub max_body_bytes: usize,
    pub anti_bot_check: bool,
    pub user_agent: UserAgentProfile,
}

#[derive(Debug)]
pub struct FetchResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("connection error: {0}")]
    Connection(String),
    #[error("timeout after {0:?}")]
    Timeout(Duration),
    #[error("response body exceeds {max_bytes} byte limit")]
    BodyTooLarge { max_bytes: usize },
    #[error("anti-bot page detected")]
    AntiBotDetected,
    #[error("SSRF: {0}")]
    Ssrf(String),
    #[error("HTTP {status}: {classification}")]
    HttpError { status: u16, classification: String },
    #[error("rate limited")]
    RateLimited,
}

// =============================================================================
// LLM Caller types
// =============================================================================

#[derive(Debug, Clone)]
pub enum LlmValue {
    Text(String),
    Number(i64),
    TextList(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmField {
    Title,
    AuthorName,
    Description,
    SeriesName,
    Genres,
    Language,
    Publisher,
    Year,
    Isbn,
    SearchResults,
    BibliographyHtml,
    ProviderName,
    CandidateTitle,
    CandidateAuthor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmPurpose {
    IdentityValidation,
    SearchResultCleanup,
    BibliographyCleanup,
}

#[derive(Debug)]
pub struct LlmCallRequest {
    pub system_template: &'static str,
    pub user_template: &'static str,
    pub context: HashMap<LlmField, LlmValue>,
    pub allowed_fields: &'static [LlmField],
    pub timeout: Duration,
    pub purpose: LlmPurpose,
}

#[derive(Debug)]
pub struct LlmCallResponse {
    pub content: String,
    pub model_used: String,
    pub elapsed: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("LLM not configured")]
    NotConfigured,
    #[error("disallowed field in context: {field:?}")]
    DisallowedField { field: LlmField },
    #[error("provider error: {0}")]
    Provider(String),
    #[error("timeout")]
    Timeout,
    #[error("invalid response: {0}")]
    InvalidResponse(String),
}

// =============================================================================
// Enrichment types
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrichmentMode {
    Background,
    Manual,
    HardRefresh,
}

// =============================================================================
// Work Service types
// =============================================================================

#[derive(Debug)]
pub struct AddWorkRequest {
    pub title: String,
    pub author_name: String,
    pub author_ol_key: Option<String>,
    pub ol_key: Option<String>,
    pub gr_key: Option<String>,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    pub metadata_source: Option<String>,
    pub language: Option<String>,
    pub detail_url: Option<String>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
    pub defer_enrichment: bool,
    pub provenance_setter: Option<ProvenanceSetter>,
}

#[derive(Debug)]
pub struct AddWorkResult {
    pub work: Work,
    pub author_created: bool,
    pub author_id: Option<i64>,
    pub messages: Vec<String>,
}

#[derive(Debug)]
pub struct UpdateWorkRequest {
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
    pub monitor_ebook: Option<bool>,
    pub monitor_audiobook: Option<bool>,
}

#[derive(Debug)]
pub struct WorkDetailView {
    pub work: Work,
    pub library_items: Vec<LibraryItem>,
}

#[derive(Debug)]
pub struct PaginatedWorksView {
    pub works: Vec<WorkDetailView>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug)]
pub struct WorkFilter {
    /// Always AND'd with user_id at DB level — never bypasses tenant scoping.
    pub author_id: Option<AuthorId>,
    pub monitored: Option<bool>,
    pub enrichment_status: Option<EnrichmentStatus>,
    pub media_type: Option<MediaType>,
    pub sort_by: Option<WorkSortField>,
    pub sort_dir: Option<SortDirection>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkSortField {
    Title,
    DateAdded,
    Year,
    Author,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug)]
pub struct RefreshWorkResult {
    pub work: Work,
    pub messages: Vec<String>,
    pub taggable_items: Vec<LibraryItem>,
    pub merge_deferred: bool,
}

#[derive(Debug)]
pub struct RefreshAllHandle {
    pub total_works: usize,
}

#[derive(Debug)]
pub struct LookupRequest {
    pub term: String,
    pub lang_override: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LookupResult {
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

#[derive(Debug, Clone, Serialize)]
pub struct LookupResponse {
    pub results: Vec<LookupResult>,
    pub filtered_count: usize,
    pub raw_count: usize,
    pub raw_available: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkServiceError {
    #[error("work not found")]
    NotFound,
    #[error("work already exists")]
    AlreadyExists,
    #[error("enrichment conflict")]
    EnrichmentConflict,
    #[error("cover too large")]
    CoverTooLarge,
    #[error("enrichment failed: {0}")]
    Enrichment(String),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

// =============================================================================
// Author Service types
// =============================================================================

#[derive(Debug)]
pub struct AddAuthorRequest {
    pub name: String,
    pub sort_name: Option<String>,
    pub ol_key: Option<String>,
    pub monitored: bool,
}

#[derive(Debug)]
pub enum AddAuthorResult {
    Created(Author),
    Updated(Author),
}

impl AddAuthorResult {
    pub fn author(&self) -> &Author {
        match self {
            Self::Created(a) | Self::Updated(a) => a,
        }
    }

    pub fn is_created(&self) -> bool {
        matches!(self, Self::Created(_))
    }

    pub fn into_author(self) -> Author {
        match self {
            Self::Created(a) | Self::Updated(a) => a,
        }
    }
}

#[derive(Debug)]
pub struct UpdateAuthorRequest {
    pub name: Option<String>,
    pub sort_name: Option<String>,
    pub ol_key: Option<String>,
    pub gr_key: Option<String>,
    pub monitored: Option<bool>,
    pub monitor_new_items: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BibliographyEntry {
    pub title: String,
    pub year: Option<i32>,
    pub ol_key: Option<String>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
    pub already_in_library: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BibliographyResult {
    pub entries: Vec<BibliographyEntry>,
    pub filtered_count: usize,
    pub raw_count: usize,
    pub raw_available: bool,
    pub fetched_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorLookupResult {
    pub ol_key: String,
    pub name: String,
    pub sort_name: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthorServiceError {
    #[error("author not found")]
    NotFound,
    #[error("author already exists")]
    AlreadyExists,
    #[error("validation: {field}: {message}")]
    Validation { field: String, message: String },
    #[error("OpenLibrary rate limited")]
    OlRateLimited,
    #[error("provider error: {0}")]
    Provider(String),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

// =============================================================================
// Series Service types
// =============================================================================

#[derive(Debug, thiserror::Error)]
pub enum SeriesServiceError {
    #[error("series not found")]
    NotFound,
    #[error("validation: {field}: {message}")]
    Validation { field: String, message: String },
    #[error("Goodreads unavailable")]
    GoodreadsUnavailable,
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

// =============================================================================
// Release Service types
// =============================================================================

#[derive(Debug)]
pub struct SearchReleasesRequest {
    pub work_id: WorkId,
    pub refresh: bool,
    pub cache_only: bool,
}

#[derive(Debug)]
pub struct ReleaseSearchResponse {
    pub results: Vec<ReleaseResult>,
    pub warnings: Vec<String>,
    pub cache_age_seconds: Option<u64>,
}

#[derive(Debug)]
pub struct ReleaseResult {
    pub title: String,
    pub indexer: String,
    pub size: i64,
    pub guid: String,
    pub download_url: String,
    pub seeders: Option<i32>,
    pub leechers: Option<i32>,
    pub publish_date: Option<String>,
    pub protocol: DownloadProtocol,
    pub categories: Vec<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadProtocol {
    Torrent,
    Usenet,
}

impl std::fmt::Display for DownloadProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Torrent => write!(f, "torrent"),
            Self::Usenet => write!(f, "usenet"),
        }
    }
}

#[derive(Debug)]
pub struct GrabRequest {
    pub work_id: WorkId,
    pub download_url: String,
    pub title: String,
    pub indexer: String,
    pub guid: String,
    pub size: i64,
    pub protocol: DownloadProtocol,
    pub categories: Vec<i32>,
    pub download_client_id: Option<i64>,
    pub source: GrabSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrabSource {
    Manual,
    RssSync,
    AutoAdd,
}

#[derive(Debug, thiserror::Error)]
pub enum ReleaseServiceError {
    #[error("no download client configured for {protocol}")]
    NoClient { protocol: String },
    #[error("download client does not support {protocol}")]
    ClientProtocolMismatch { protocol: String },
    #[error("download client unreachable: {0}")]
    ClientUnreachable(String),
    #[error("download client auth failed")]
    DownloadClientAuth,
    #[error("SSRF: {0}")]
    Ssrf(String),
    #[error("all indexers failed")]
    AllIndexersFailed,
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

// =============================================================================
// Grab Service types
// =============================================================================

#[derive(Debug)]
pub struct GrabFilter {
    pub status: Option<GrabStatus>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug)]
pub struct QueueItem {
    pub grab: Grab,
    pub progress: Option<DownloadProgress>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadProgressStatus {
    Downloading,
    Paused,
    Queued,
    Stalled,
    Seeding,
    Extracting,
    Verifying,
    Unknown,
}

#[derive(Debug)]
pub struct DownloadProgress {
    pub percent: f64,
    pub speed_bytes_per_sec: Option<u64>,
    pub eta_seconds: Option<u64>,
    pub status: DownloadProgressStatus,
}

#[derive(Debug, thiserror::Error)]
pub enum GrabServiceError {
    #[error("grab not found")]
    NotFound,
    #[error("download client unreachable: {0}")]
    ClientUnreachable(String),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

// =============================================================================
// File Service types
// =============================================================================

#[derive(Debug)]
pub struct ScanResult {
    pub scan_id: String,
    pub files: Vec<ScannedFile>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub struct ScannedFile {
    pub relative_path: String,
    pub filename: String,
    pub media_type: MediaType,
    pub size: u64,
    pub matched_work_id: Option<WorkId>,
    pub has_existing_item: bool,
}

/// Prepared email payload — contains validated file data for the handler to send via SMTP.
/// The handler is responsible for fetching `EmailConfig` and calling `email::send_file`.
#[derive(Debug)]
pub struct EmailPayload {
    pub file_bytes: Vec<u8>,
    pub filename: String,
    pub extension: String,
}

#[derive(Debug, thiserror::Error)]
pub enum FileServiceError {
    #[error("library item not found")]
    NotFound,
    #[error("root folder not found")]
    RootFolderNotFound,
    #[error("path traversal denied")]
    Forbidden,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

// =============================================================================
// List Service types
// =============================================================================

/// Kept for backward compatibility with existing behavioral tests.
/// New code should use the redesigned preview(bytes) API.
#[derive(Debug)]
pub struct ListPreviewRequest {
    pub source: ListSource,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListSource {
    GoodreadsCsv,
    OpenLibrary,
    Hardcover,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListPreviewResponse {
    pub preview_id: String,
    pub source: String,
    pub total_rows: usize,
    pub rows: Vec<ListPreviewRow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListPreviewRow {
    pub row_index: usize,
    pub title: String,
    pub author: String,
    pub isbn_13: Option<String>,
    pub isbn_10: Option<String>,
    pub year: Option<i32>,
    pub source_status: Option<String>,
    pub source_rating: Option<f32>,
    pub preview_status: String,
}

/// Legacy match status for backward-compat behavioral tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListMatchStatus {
    Matched,
    NotFound,
    AlreadyExists,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListConfirmResponse {
    pub import_id: String,
    pub results: Vec<ListConfirmRowResult>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListConfirmRowResult {
    pub row_index: usize,
    pub status: String,
    pub message: Option<String>,
}

/// Legacy response shape for backward-compat behavioral tests.
#[derive(Debug)]
pub struct ListConfirmLegacyResponse {
    pub added: usize,
    pub skipped: usize,
    pub failed: Vec<ListFailedRow>,
}

#[derive(Debug)]
pub struct ListFailedRow {
    pub title: String,
    pub error: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListUndoResponse {
    pub works_removed: usize,
    pub works_skipped: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListImportSummary {
    pub id: String,
    pub source: String,
    pub status: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub works_created: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum ListServiceError {
    #[error("import not found")]
    NotFound,
    #[error("parse error: {0}")]
    Parse(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

#[derive(Debug)]
pub struct ScanConfirmation {
    pub relative_path: String,
    pub work_id: WorkId,
    pub media_type: MediaType,
}

// =============================================================================
// Import Workflow types
// =============================================================================

#[derive(Debug)]
pub struct ImportResult {
    pub grab_id: GrabId,
    pub final_status: GrabStatus,
    pub imported_files: Vec<ImportedFile>,
    pub failed_files: Vec<FailedFile>,
    pub skipped_files: Vec<SkippedFile>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub struct ImportedFile {
    pub source_name: String,
    pub target_relative_path: String,
    pub media_type: MediaType,
    pub file_size: u64,
    pub library_item_id: i64,
    pub tags_written: bool,
    pub cwa_copied: bool,
}

#[derive(Debug)]
pub struct FailedFile {
    pub source_name: String,
    pub error: String,
}

#[derive(Debug)]
pub struct SkippedFile {
    pub source_name: String,
    pub reason: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportWorkflowError {
    #[error("grab not found")]
    GrabNotFound,
    #[error("source path not resolved: {0}")]
    SourceNotResolved(String),
    #[error("download client unreachable: {0}")]
    ClientUnreachable(String),
    #[error("no root folder for media type: {media_type:?}")]
    NoRootFolder { media_type: MediaType },
    #[error("source directory not found or inaccessible")]
    SourceInaccessible,
    #[error("scan not found or expired")]
    ScanExpired,
    #[error("scan belongs to another user")]
    ScanForbidden,
    #[error("import failed: {0}")]
    ImportFailed(String),
    #[error("tag write failed: {0}")]
    TagWriteFailed(String),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

// =============================================================================
// Enrichment Workflow types
// =============================================================================

#[derive(Debug)]
pub struct EnrichmentResult {
    pub enrichment_status: EnrichmentStatus,
    pub enrichment_source: Option<String>,
    pub work: Work,
    pub merge_deferred: bool,
    pub provider_outcomes: HashMap<MetadataProvider, OutcomeClass>,
}

#[derive(Debug, thiserror::Error)]
pub enum EnrichmentWorkflowError {
    #[error("work not found")]
    WorkNotFound,
    #[error("merge superseded after CAS retries")]
    MergeSuperseded,
    #[error("merge error: {0}")]
    Merge(String),
    #[error("all providers exhausted for work {work_id}")]
    ProviderExhausted { work_id: WorkId },
    #[error("corrupt retry payload for {provider:?} on work {work_id}")]
    CorruptRetryPayload {
        work_id: WorkId,
        provider: MetadataProvider,
    },
    #[error("provider queue error: {0}")]
    Queue(String),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

// =============================================================================
// RSS Sync types
// =============================================================================

#[derive(Debug)]
pub struct RssSyncReport {
    pub feeds_checked: usize,
    pub releases_matched: usize,
    pub grabs_attempted: usize,
    pub grabs_succeeded: usize,
    pub warnings: Vec<String>,
}

impl RssSyncReport {
    pub fn empty() -> Self {
        Self {
            feeds_checked: 0,
            releases_matched: 0,
            grabs_attempted: 0,
            grabs_succeeded: 0,
            warnings: vec![],
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RssSyncError {
    #[error("feed fetch failed: {0}")]
    FeedFetch(String),
    #[error("release search failed: {0}")]
    Search(#[from] ReleaseServiceError),
    #[error("grab failed: {0}")]
    Grab(String),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

// =============================================================================
// Author Monitor types
// =============================================================================

#[derive(Debug)]
pub struct MonitorReport {
    pub authors_checked: usize,
    pub new_works_found: usize,
    pub works_added: usize,
    pub notifications_created: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum MonitorError {
    #[error("OpenLibrary lookup failed: {0}")]
    ProviderFailed(String),
    #[error("OpenLibrary rate limited")]
    RateLimited,
    #[error("work add failed: {0}")]
    WorkAdd(#[from] WorkServiceError),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

// =============================================================================
// CWA Copy result
// =============================================================================

#[derive(Debug)]
pub struct CwaResult {
    pub success: bool,
    pub warning: Option<String>,
}

// =============================================================================
// Trait definitions — TEMP(pk-tdd): compile-only scaffolds
// =============================================================================

#[trait_variant::make(Send)]
pub trait HttpFetcher: Send + Sync {
    async fn fetch(&self, req: FetchRequest) -> Result<FetchResponse, FetchError>;
    async fn fetch_ssrf_safe(&self, req: FetchRequest) -> Result<FetchResponse, FetchError>;
}

#[trait_variant::make(Send)]
pub trait LlmCaller: Send + Sync {
    async fn call(&self, req: LlmCallRequest) -> Result<LlmCallResponse, LlmError>;
}

#[trait_variant::make(Send)]
pub trait WorkService: Send + Sync {
    async fn add(
        &self,
        user_id: UserId,
        req: AddWorkRequest,
    ) -> Result<AddWorkResult, WorkServiceError>;
    async fn get(&self, user_id: UserId, work_id: WorkId) -> Result<Work, WorkServiceError>;
    async fn get_detail(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<WorkDetailView, WorkServiceError>;
    async fn list(
        &self,
        user_id: UserId,
        filter: WorkFilter,
    ) -> Result<Vec<Work>, WorkServiceError>;
    async fn list_paginated(
        &self,
        user_id: UserId,
        page: u32,
        page_size: u32,
        sort_by: WorkSortField,
        sort_dir: SortDirection,
    ) -> Result<PaginatedWorksView, WorkServiceError>;
    async fn update(
        &self,
        user_id: UserId,
        work_id: WorkId,
        req: UpdateWorkRequest,
    ) -> Result<Work, WorkServiceError>;
    async fn delete(&self, user_id: UserId, work_id: WorkId) -> Result<(), WorkServiceError>;
    async fn refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<RefreshWorkResult, WorkServiceError>;
    async fn refresh_all(&self, user_id: UserId) -> Result<RefreshAllHandle, WorkServiceError>;
    async fn upload_cover(
        &self,
        user_id: UserId,
        work_id: WorkId,
        bytes: &[u8],
    ) -> Result<(), WorkServiceError>;
    async fn download_cover(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<u8>, WorkServiceError>;
    async fn lookup(&self, req: LookupRequest) -> Result<Vec<LookupResult>, WorkServiceError>;
    async fn lookup_filtered(
        &self,
        req: LookupRequest,
        raw: bool,
    ) -> Result<LookupResponse, WorkServiceError>;
    /// Search works by title or author name (LIKE match). Used by OPDS search.
    async fn search_works(
        &self,
        user_id: UserId,
        query: &str,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<Work>, i64), WorkServiceError>;
    async fn download_cover_from_url(&self, user_id: i64, work_id: i64, cover_url: &str);
    fn try_start_bulk_refresh(&self, user_id: i64) -> bool;
    fn finish_bulk_refresh(&self, user_id: i64);
}

#[trait_variant::make(Send)]
pub trait AuthorService: Send + Sync {
    async fn add(
        &self,
        user_id: UserId,
        req: AddAuthorRequest,
    ) -> Result<AddAuthorResult, AuthorServiceError>;
    async fn get(&self, user_id: UserId, author_id: AuthorId)
        -> Result<Author, AuthorServiceError>;
    async fn list(&self, user_id: UserId) -> Result<Vec<Author>, AuthorServiceError>;
    async fn update(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        req: UpdateAuthorRequest,
    ) -> Result<Author, AuthorServiceError>;
    async fn delete(&self, user_id: UserId, author_id: AuthorId) -> Result<(), AuthorServiceError>;
    async fn lookup(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<AuthorLookupResult>, AuthorServiceError>;
    async fn search(&self, user_id: UserId, query: &str)
        -> Result<Vec<Author>, AuthorServiceError>;
    async fn bibliography(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        raw: bool,
    ) -> Result<BibliographyResult, AuthorServiceError>;
    async fn refresh_bibliography(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<BibliographyResult, AuthorServiceError>;
    fn spawn_bibliography_refresh(&self, author_id: i64, user_id: i64);
    async fn lookup_authors(
        &self,
        term: &str,
        limit: u32,
    ) -> Result<Vec<AuthorLookupResult>, AuthorServiceError>;
}

#[trait_variant::make(Send)]
pub trait SeriesService: Send + Sync {
    async fn list(&self, user_id: UserId) -> Result<Vec<Series>, SeriesServiceError>;
    async fn get(&self, user_id: UserId, series_id: i64) -> Result<Series, SeriesServiceError>;
    async fn refresh(&self, user_id: UserId, series_id: i64) -> Result<Series, SeriesServiceError>;
    async fn monitor(
        &self,
        user_id: UserId,
        series_id: i64,
        monitored: bool,
    ) -> Result<Series, SeriesServiceError>;
    async fn update(
        &self,
        user_id: UserId,
        series_id: i64,
        title: Option<String>,
    ) -> Result<Series, SeriesServiceError>;
}

// =============================================================================
// Series Query Service (cross-entity read views)
// =============================================================================

#[derive(Debug)]
pub struct SeriesListView {
    pub id: i64,
    pub name: String,
    pub gr_key: String,
    pub book_count: i32,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub works_in_library: i64,
    pub author_id: i64,
    pub author_name: String,
    pub first_work_id: Option<WorkId>,
}

#[derive(Debug)]
pub struct SeriesDetailView {
    pub id: i64,
    pub name: String,
    pub gr_key: String,
    pub book_count: i32,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub author_id: i64,
    pub author_name: String,
    pub works: Vec<SeriesWorkView>,
}

#[derive(Debug)]
pub struct SeriesWorkView {
    pub work: Work,
    pub library_items: Vec<LibraryItem>,
}

#[derive(Debug)]
pub struct UpdateSeriesView {
    pub id: i64,
    pub name: String,
    pub gr_key: String,
    pub book_count: i32,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub works_in_library: i64,
}

#[derive(Debug, Clone)]
pub struct GrAuthorCandidateView {
    pub gr_key: String,
    pub name: String,
    pub profile_url: String,
}

#[derive(Debug)]
pub struct AuthorSeriesListView {
    pub series: Vec<AuthorSeriesItemView>,
    pub fetched_at: Option<String>,
}

#[derive(Debug)]
pub struct AuthorSeriesItemView {
    pub id: Option<i64>,
    pub name: String,
    pub gr_key: String,
    pub book_count: i32,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub works_in_library: i64,
}

#[derive(Debug)]
pub struct MonitorSeriesServiceRequest {
    pub gr_key: String,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
}

#[derive(Debug)]
pub struct MonitorSeriesView {
    pub id: i64,
    pub name: String,
    pub gr_key: String,
    pub book_count: i32,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub works_in_library: i64,
}

#[derive(Debug)]
pub struct SeriesMonitorWorkerParams {
    pub user_id: UserId,
    pub author_id: AuthorId,
    pub series_id: i64,
    pub series_name: String,
    pub series_gr_key: String,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
}

#[trait_variant::make(Send)]
pub trait SeriesQueryService: Send + Sync {
    async fn list_enriched(
        &self,
        user_id: UserId,
    ) -> Result<Vec<SeriesListView>, SeriesServiceError>;
    async fn get_detail(
        &self,
        user_id: UserId,
        series_id: i64,
    ) -> Result<SeriesDetailView, SeriesServiceError>;
    async fn update_flags(
        &self,
        user_id: UserId,
        series_id: i64,
        monitor_ebook: bool,
        monitor_audiobook: bool,
    ) -> Result<UpdateSeriesView, SeriesServiceError>;
    async fn resolve_gr_candidates(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<GrAuthorCandidateView>, SeriesServiceError>;
    async fn list_author_series(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<AuthorSeriesListView, SeriesServiceError>;
    async fn refresh_author_series(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<AuthorSeriesListView, SeriesServiceError>;
    async fn monitor_series(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        req: MonitorSeriesServiceRequest,
    ) -> Result<MonitorSeriesView, SeriesServiceError>;
    async fn run_series_monitor_worker(
        &self,
        params: SeriesMonitorWorkerParams,
    ) -> Result<(), SeriesServiceError>;
}

#[trait_variant::make(Send)]
pub trait ReleaseService: Send + Sync {
    async fn search(
        &self,
        user_id: UserId,
        req: SearchReleasesRequest,
    ) -> Result<ReleaseSearchResponse, ReleaseServiceError>;
    async fn grab(&self, user_id: UserId, req: GrabRequest) -> Result<Grab, ReleaseServiceError>;
}

#[trait_variant::make(Send)]
pub trait GrabService: Send + Sync {
    async fn list(
        &self,
        user_id: UserId,
        filter: GrabFilter,
    ) -> Result<Vec<QueueItem>, GrabServiceError>;
    async fn get(&self, user_id: UserId, grab_id: GrabId) -> Result<QueueItem, GrabServiceError>;
    async fn remove(&self, user_id: UserId, grab_id: GrabId) -> Result<(), GrabServiceError>;
}

#[trait_variant::make(Send)]
pub trait FileService: Send + Sync {
    async fn list(&self, user_id: UserId) -> Result<Vec<LibraryItem>, FileServiceError>;

    // CRUD
    async fn list_paginated(
        &self,
        user_id: UserId,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<LibraryItem>, i64), FileServiceError>;
    async fn get(&self, user_id: UserId, item_id: i64) -> Result<LibraryItem, FileServiceError>;
    async fn delete(&self, user_id: UserId, item_id: i64) -> Result<(), FileServiceError>;

    // File access — returns validated, canonicalized path for ServeFile
    async fn resolve_path(
        &self,
        user_id: UserId,
        item_id: i64,
    ) -> Result<std::path::PathBuf, FileServiceError>;

    // Email preparation (validation + file read, not SMTP send)
    async fn prepare_email(
        &self,
        user_id: UserId,
        item_id: i64,
    ) -> Result<EmailPayload, FileServiceError>;

    // Progress tracking
    async fn get_progress(
        &self,
        user_id: UserId,
        item_id: i64,
    ) -> Result<Option<PlaybackProgress>, FileServiceError>;
    async fn update_progress(
        &self,
        user_id: UserId,
        item_id: i64,
        position: &str,
        progress_pct: f64,
    ) -> Result<(), FileServiceError>;
}

#[trait_variant::make(Send)]
pub trait ListService: Send + Sync {
    async fn preview(
        &self,
        user_id: UserId,
        bytes: Vec<u8>,
    ) -> Result<ListPreviewResponse, ListServiceError>;

    async fn confirm(
        &self,
        user_id: UserId,
        preview_id: &str,
        import_id: Option<&str>,
        row_indices: &[usize],
    ) -> Result<ListConfirmResponse, ListServiceError>;

    async fn complete(&self, user_id: UserId, import_id: &str) -> Result<(), ListServiceError>;

    async fn undo(
        &self,
        user_id: UserId,
        import_id: &str,
    ) -> Result<ListUndoResponse, ListServiceError>;

    async fn list_imports(
        &self,
        user_id: UserId,
    ) -> Result<Vec<ListImportSummary>, ListServiceError>;
}

/// Fire-and-forget bibliography fetch trigger for newly created authors.
/// Trait lives in domain; impl in livrarr-server (spawns background task).
#[trait_variant::make(Send)]
pub trait BibliographyTrigger: Send + Sync {
    fn trigger(&self, author_id: i64, user_id: UserId);
}

#[trait_variant::make(Send)]
pub trait ImportWorkflow: Send + Sync {
    async fn import_grab(
        &self,
        user_id: UserId,
        grab_id: GrabId,
    ) -> Result<ImportResult, ImportWorkflowError>;
    async fn retry_import(
        &self,
        user_id: UserId,
        grab_id: GrabId,
    ) -> Result<ImportResult, ImportWorkflowError>;
    async fn confirm_scan(
        &self,
        user_id: UserId,
        scan_id: &str,
        selections: Vec<ScanConfirmation>,
    ) -> Result<ImportResult, ImportWorkflowError>;
}

#[trait_variant::make(Send)]
pub trait EnrichmentWorkflow: Send + Sync {
    async fn enrich_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
        mode: EnrichmentMode,
    ) -> Result<EnrichmentResult, EnrichmentWorkflowError>;
    async fn reset_for_manual_refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<(), EnrichmentWorkflowError>;
}

#[trait_variant::make(Send)]
pub trait RssSyncWorkflow: Send + Sync {
    async fn run_sync(&self) -> Result<RssSyncReport, RssSyncError>;
}

#[trait_variant::make(Send)]
pub trait AuthorMonitorWorkflow: Send + Sync {
    async fn run_monitor(
        &self,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<MonitorReport, MonitorError>;
    fn trigger_monitor(&self);
}

// =============================================================================
// Notification service
// =============================================================================

#[derive(Debug, thiserror::Error)]
pub enum NotificationServiceError {
    #[error("notification not found")]
    NotFound,
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

pub struct CreateNotificationRequest {
    pub user_id: UserId,
    pub notification_type: NotificationType,
    pub ref_key: Option<String>,
    pub message: String,
    pub data: serde_json::Value,
}

#[trait_variant::make(Send)]
pub trait NotificationService: Send + Sync {
    async fn list_paginated(
        &self,
        user_id: UserId,
        unread_only: bool,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<Notification>, i64), NotificationServiceError>;

    async fn mark_read(
        &self,
        user_id: UserId,
        id: NotificationId,
    ) -> Result<(), NotificationServiceError>;

    async fn dismiss(
        &self,
        user_id: UserId,
        id: NotificationId,
    ) -> Result<(), NotificationServiceError>;

    async fn dismiss_all(&self, user_id: UserId) -> Result<(), NotificationServiceError>;

    async fn create(
        &self,
        req: CreateNotificationRequest,
    ) -> Result<Notification, NotificationServiceError>;
}

// =============================================================================
// History service — abstracts DB queries for history handlers
// =============================================================================

#[derive(Debug, thiserror::Error)]
pub enum HistoryServiceError {
    #[error("not found")]
    NotFound,
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

#[trait_variant::make(Send)]
pub trait HistoryService: Send + Sync {
    async fn list_paginated(
        &self,
        user_id: UserId,
        filter: HistoryFilter,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<HistoryEvent>, i64), HistoryServiceError>;
}

// =============================================================================
// Queue service
// =============================================================================

#[derive(Debug, thiserror::Error)]
pub enum QueueServiceError {
    #[error("grab not found")]
    NotFound,
    #[error("not in importable state")]
    NotImportable,
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

#[trait_variant::make(Send)]
pub trait QueueService: Send + Sync {
    async fn list_grabs_paginated(
        &self,
        user_id: UserId,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<Grab>, i64), QueueServiceError>;

    async fn list_download_clients(&self) -> Result<Vec<DownloadClient>, QueueServiceError>;

    async fn try_set_importing(
        &self,
        user_id: UserId,
        grab_id: GrabId,
    ) -> Result<bool, QueueServiceError>;

    async fn update_grab_status(
        &self,
        user_id: UserId,
        grab_id: GrabId,
        status: GrabStatus,
        error: Option<&str>,
    ) -> Result<(), QueueServiceError>;

    async fn fetch_download_progress(
        &self,
        client: &DownloadClient,
        download_id: &str,
    ) -> Option<QueueProgress>;
}

// =============================================================================
// Import IO service — wraps DB calls used by import pipeline handlers
// =============================================================================

pub struct CreateLibraryItemRequest {
    pub user_id: UserId,
    pub work_id: WorkId,
    pub root_folder_id: RootFolderId,
    pub path: String,
    pub media_type: MediaType,
    pub file_size: i64,
    pub import_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportIoServiceError {
    #[error("not found")]
    NotFound,
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

#[trait_variant::make(Send)]
pub trait ImportIoService: Send + Sync {
    async fn get_grab(
        &self,
        user_id: UserId,
        grab_id: GrabId,
    ) -> Result<Grab, ImportIoServiceError>;

    async fn get_download_client(
        &self,
        client_id: DownloadClientId,
    ) -> Result<DownloadClient, ImportIoServiceError>;

    async fn set_grab_content_path(
        &self,
        user_id: UserId,
        grab_id: GrabId,
        content_path: &str,
    ) -> Result<(), ImportIoServiceError>;

    async fn get_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Work, ImportIoServiceError>;

    async fn list_library_items_by_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<LibraryItem>, ImportIoServiceError>;

    async fn get_root_folder(
        &self,
        root_folder_id: RootFolderId,
    ) -> Result<RootFolder, ImportIoServiceError>;

    async fn list_root_folders(&self) -> Result<Vec<RootFolder>, ImportIoServiceError>;

    async fn list_remote_path_mappings(
        &self,
    ) -> Result<Vec<crate::RemotePathMapping>, ImportIoServiceError>;

    async fn update_library_item_size(
        &self,
        user_id: UserId,
        item_id: LibraryItemId,
        new_size: i64,
    ) -> Result<(), ImportIoServiceError>;

    async fn create_library_item(
        &self,
        req: CreateLibraryItemRequest,
    ) -> Result<LibraryItem, ImportIoServiceError>;
}

// =============================================================================
// Manual import session service — abstracts in-memory scan state + DB queries
// =============================================================================

#[derive(Debug, thiserror::Error)]
pub enum ManualImportServiceError {
    #[error("not found")]
    NotFound,
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

#[trait_variant::make(Send)]
pub trait ManualImportService: Send + Sync {
    async fn list_works(&self, user_id: UserId) -> Result<Vec<Work>, ManualImportServiceError>;

    async fn list_root_folders(&self) -> Result<Vec<RootFolder>, ManualImportServiceError>;

    async fn list_library_items_by_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<LibraryItem>, ManualImportServiceError>;

    async fn list_library_items_by_work_ids(
        &self,
        user_id: UserId,
        work_ids: &[WorkId],
    ) -> Result<Vec<LibraryItem>, ManualImportServiceError>;

    async fn get_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Work, ManualImportServiceError>;

    async fn delete_library_item(
        &self,
        user_id: UserId,
        item_id: LibraryItemId,
    ) -> Result<LibraryItem, ManualImportServiceError>;

    async fn create_library_item(
        &self,
        user_id: UserId,
        work_id: WorkId,
        root_folder_id: RootFolderId,
        path: String,
        media_type: MediaType,
        file_size: i64,
    ) -> Result<LibraryItem, ManualImportServiceError>;

    async fn create_history_event(
        &self,
        user_id: UserId,
        work_id: Option<WorkId>,
        event_type: crate::EventType,
        data: serde_json::Value,
    ) -> Result<(), ManualImportServiceError>;
}

// =============================================================================
// Settings Service
// =============================================================================

#[trait_variant::make(Send)]
pub trait SettingsService: Send + Sync {
    // --- Config ---
    async fn get_naming_config(&self) -> Result<crate::settings::NamingConfig, DbError>;
    async fn get_media_management_config(
        &self,
    ) -> Result<crate::settings::MediaManagementConfig, DbError>;
    async fn update_media_management_config(
        &self,
        params: UpdateMediaManagementParams,
    ) -> Result<crate::settings::MediaManagementConfig, DbError>;
    async fn get_metadata_config(&self) -> Result<crate::settings::MetadataConfig, DbError>;
    async fn update_metadata_config(
        &self,
        params: UpdateMetadataParams,
    ) -> Result<crate::settings::MetadataConfig, DbError>;
    async fn get_prowlarr_config(&self) -> Result<crate::settings::ProwlarrConfig, DbError>;
    async fn update_prowlarr_config(
        &self,
        params: UpdateProwlarrParams,
    ) -> Result<crate::settings::ProwlarrConfig, DbError>;
    async fn get_email_config(&self) -> Result<crate::settings::EmailConfig, DbError>;
    async fn update_email_config(
        &self,
        params: UpdateEmailParams,
    ) -> Result<crate::settings::EmailConfig, DbError>;
    async fn validate_metadata_languages(
        &self,
        languages: &[String],
        llm_enabled: Option<bool>,
        llm_endpoint: Option<&str>,
        llm_api_key: Option<&str>,
        llm_model: Option<&str>,
    ) -> Result<Vec<String>, String>;

    async fn get_indexer_config(&self) -> Result<IndexerConfig, DbError>;
    async fn update_indexer_config(
        &self,
        params: UpdateIndexerConfigParams,
    ) -> Result<IndexerConfig, DbError>;

    // --- Download clients ---
    async fn get_download_client(&self, id: DownloadClientId) -> Result<DownloadClient, DbError>;
    async fn get_download_client_with_credentials(
        &self,
        id: DownloadClientId,
    ) -> Result<DownloadClient, DbError>;
    async fn list_download_clients(&self) -> Result<Vec<DownloadClient>, DbError>;
    async fn create_download_client(
        &self,
        params: CreateDownloadClientParams,
    ) -> Result<DownloadClient, DbError>;
    async fn update_download_client(
        &self,
        id: DownloadClientId,
        params: UpdateDownloadClientParams,
    ) -> Result<DownloadClient, DbError>;
    async fn delete_download_client(&self, id: DownloadClientId) -> Result<(), DbError>;

    // --- Indexers ---
    async fn get_indexer(&self, id: IndexerId) -> Result<Indexer, DbError>;
    async fn get_indexer_with_credentials(&self, id: IndexerId) -> Result<Indexer, DbError>;
    async fn list_indexers(&self) -> Result<Vec<Indexer>, DbError>;
    async fn create_indexer(&self, params: CreateIndexerParams) -> Result<Indexer, DbError>;
    async fn update_indexer(
        &self,
        id: IndexerId,
        params: UpdateIndexerParams,
    ) -> Result<Indexer, DbError>;
    async fn delete_indexer(&self, id: IndexerId) -> Result<(), DbError>;
    async fn set_supports_book_search(&self, id: IndexerId, supports: bool) -> Result<(), DbError>;

    // --- Root folders ---
    async fn get_root_folder(&self, id: RootFolderId) -> Result<RootFolder, DbError>;
    async fn list_root_folders(&self) -> Result<Vec<RootFolder>, DbError>;
    async fn create_root_folder(
        &self,
        path: &str,
        media_type: MediaType,
    ) -> Result<RootFolder, DbError>;
    async fn delete_root_folder(&self, id: RootFolderId) -> Result<(), DbError>;

    // --- Remote path mappings ---
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

// =============================================================================
// Free functions — TEMP(pk-tdd): compile-only scaffolds
// =============================================================================

pub async fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let path = path.to_path_buf();
    let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let parent = path.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path has no parent directory",
            )
        })?;
        std::fs::create_dir_all(parent)?;
        let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
        tmp.write_all(&bytes)?;
        tmp.as_file().sync_all()?;
        tmp.persist(&path).map_err(|e| e.error)?;
        Ok(())
    })
    .await
    .expect("spawn_blocking panicked")
}

pub async fn atomic_copy(src: &Path, dst: &Path) -> std::io::Result<u64> {
    let src = src.to_path_buf();
    let dst = dst.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let parent = dst.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path has no parent directory",
            )
        })?;
        std::fs::create_dir_all(parent)?;
        let mut src_file = std::fs::File::open(&src)?;
        let tmp = tempfile::NamedTempFile::new_in(parent)?;
        let mut dst_file = tmp.as_file().try_clone()?;
        let copied = std::io::copy(&mut src_file, &mut dst_file)?;
        dst_file.sync_all()?;
        drop(dst_file);
        tmp.persist(&dst).map_err(|e| e.error)?;
        Ok(copied)
    })
    .await
    .expect("spawn_blocking panicked")
}

pub async fn cwa_copy(src: &Path, dst: &Path) -> CwaResult {
    let src = src.to_path_buf();
    let dst = dst.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let parent = match dst.parent() {
            Some(p) => p.to_path_buf(),
            None => {
                return CwaResult {
                    success: false,
                    warning: Some("destination path has no parent directory".into()),
                };
            }
        };
        if let Err(e) = std::fs::create_dir_all(&parent) {
            return CwaResult {
                success: false,
                warning: Some(format!("failed to create parent directory: {e}")),
            };
        }
        match std::fs::hard_link(&src, &dst) {
            Ok(()) => CwaResult {
                success: true,
                warning: None,
            },
            Err(link_err) => {
                let result = (|| -> std::io::Result<()> {
                    let mut src_file = std::fs::File::open(&src)?;
                    let tmp = tempfile::NamedTempFile::new_in(&parent)?;
                    let mut dst_file = tmp.as_file().try_clone()?;
                    std::io::copy(&mut src_file, &mut dst_file)?;
                    dst_file.sync_all()?;
                    drop(dst_file);
                    tmp.persist(&dst).map_err(|e| e.error)?;
                    Ok(())
                })();
                match result {
                    Ok(()) => CwaResult {
                        success: true,
                        warning: Some(format!("hardlink failed ({link_err}), fell back to copy")),
                    },
                    Err(copy_err) => CwaResult {
                        success: false,
                        warning: Some(format!(
                            "hardlink failed ({link_err}), copy also failed ({copy_err})"
                        )),
                    },
                }
            }
        }
    })
    .await
    .expect("spawn_blocking panicked")
}

// =============================================================================
// Phase 5: General service error
// =============================================================================

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{0}")]
    Db(DbError),
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Internal(String),
}

impl From<DbError> for ServiceError {
    fn from(e: DbError) -> Self {
        match e {
            DbError::NotFound { .. } => ServiceError::NotFound,
            other => ServiceError::Db(other),
        }
    }
}

// =============================================================================
// Phase 5: Import pipeline services
// =============================================================================

#[derive(Debug, Clone)]
pub struct ImportGrabResult {
    pub final_status: GrabStatus,
    pub imported_count: usize,
    pub failed_count: usize,
    pub skipped_count: usize,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct ImportSingleFileRequest {
    pub source: std::path::PathBuf,
    pub target_path: String,
    pub root_folder_path: String,
    pub root_folder_id: i64,
    pub media_type: MediaType,
    pub user_id: i64,
    pub work_id: i64,
    pub author_name: String,
    pub title: String,
}

#[derive(Debug)]
pub enum ImportFileResult {
    Ok,
    Warning(String),
    Failed(String),
}

#[trait_variant::make(Send)]
pub trait ImportService: Send + Sync {
    async fn import_grab(
        &self,
        user_id: i64,
        grab_id: i64,
    ) -> Result<ImportGrabResult, ServiceError>;

    async fn import_single_file(&self, req: ImportSingleFileRequest) -> ImportFileResult;

    #[allow(clippy::too_many_arguments)]
    fn build_target_path(
        &self,
        root_folder_path: &str,
        user_id: i64,
        author: &str,
        title: &str,
        media_type: MediaType,
        source: &std::path::Path,
        source_root: &std::path::Path,
    ) -> String;
}

#[trait_variant::make(Send)]
pub trait TagService: Send + Sync {
    async fn retag_library_items(&self, work: &Work, items: &[LibraryItem]) -> Vec<String>;
}

#[trait_variant::make(Send)]
pub trait CoverIoService: Send + Sync {
    async fn read_cover_bytes(&self, work_id: i64) -> Option<Vec<u8>>;
}

// =============================================================================
// Phase 5: Email service
// =============================================================================

#[derive(Debug, thiserror::Error)]
pub enum EmailServiceError {
    #[error("{0}")]
    Config(String),
    #[error("{0}")]
    Send(String),
}

#[trait_variant::make(Send)]
pub trait EmailService: Send + Sync {
    async fn send_test(&self) -> Result<(), EmailServiceError>;
    async fn send_file(
        &self,
        file_bytes: Vec<u8>,
        filename: &str,
        extension: &str,
    ) -> Result<(), EmailServiceError>;
}

// =============================================================================
// Phase 5: Matching service
// =============================================================================

#[derive(Debug, Clone)]
pub struct MatchCluster {
    pub author: Option<String>,
    pub title: Option<String>,
    pub series: Option<String>,
    pub series_position: Option<f64>,
    pub language: Option<String>,
}

#[derive(Debug)]
pub struct MatchInput {
    pub file_path: Option<std::path::PathBuf>,
    pub grouped_paths: Option<Vec<std::path::PathBuf>>,
    pub parse_string: Option<String>,
    pub media_type: Option<MediaType>,
    pub scan_root: Option<std::path::PathBuf>,
}

#[trait_variant::make(Send)]
pub trait MatchingService: Send + Sync {
    async fn extract_and_reconcile(&self, input: &MatchInput) -> Vec<MatchCluster>;
}

// =============================================================================
// Phase 5: Readarr import workflow
// =============================================================================

#[trait_variant::make(Send)]
pub trait ReadarrImportWorkflow: Send + Sync {
    async fn connect(
        &self,
        req: crate::readarr::ReadarrConnectRequest,
    ) -> Result<crate::readarr::ReadarrConnectResponse, ServiceError>;

    async fn preview(
        &self,
        user_id: i64,
        req: crate::readarr::ReadarrImportRequest,
    ) -> Result<crate::readarr::ReadarrPreviewResponse, ServiceError>;

    async fn start(
        &self,
        user_id: i64,
        req: crate::readarr::ReadarrImportRequest,
    ) -> Result<crate::readarr::ReadarrStartResponse, ServiceError>;

    async fn progress(&self) -> crate::readarr::ReadarrImportProgress;

    async fn history(
        &self,
        user_id: i64,
    ) -> Result<crate::readarr::ReadarrHistoryResponse, ServiceError>;

    async fn undo(
        &self,
        user_id: i64,
        import_id: String,
    ) -> Result<crate::readarr::ReadarrUndoResponse, ServiceError>;
}
