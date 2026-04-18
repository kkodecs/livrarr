/// TEMP(pk-tdd): compile-only scaffold — removed by pk-implement
///
/// Service layer consolidation types and traits. All defined in livrarr-domain
/// per architecture spec. Implementations live in capability crates.
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use crate::{
    Author, AuthorId, DbError, EnrichmentStatus, Grab, GrabId, GrabStatus, LibraryItem, MediaType,
    MetadataProvider, OutcomeClass, Series, UserId, Work, WorkId,
};
use chrono::{DateTime, Utc};
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
}

#[derive(Debug)]
pub struct AddWorkResult {
    pub work: Work,
    pub author_created: bool,
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
    pub already_in_library: bool,
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

pub struct FileStream {
    pub reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    pub size: u64,
    pub media_type: MediaType,
    pub filename: String,
}

#[derive(Debug, thiserror::Error)]
pub enum FileServiceError {
    #[error("library item not found")]
    NotFound,
    #[error("root folder not found")]
    RootFolderNotFound,
    #[error("scan not found or expired")]
    ScanExpired,
    #[error("scan belongs to another user")]
    ScanForbidden,
    #[error("tag write failed: {0}")]
    TagWrite(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

// =============================================================================
// List Service types
// =============================================================================

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
}

#[derive(Debug)]
pub struct ListPreviewResponse {
    pub rows: Vec<ListPreviewRow>,
    pub import_id: String,
}

#[derive(Debug, Clone)]
pub struct ListPreviewRow {
    pub title: String,
    pub author: Option<String>,
    pub isbn: Option<String>,
    pub matched_work_id: Option<WorkId>,
    pub match_status: ListMatchStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListMatchStatus {
    Matched,
    NotFound,
    AlreadyExists,
}

#[derive(Debug)]
pub struct ListConfirmResponse {
    pub added: usize,
    pub skipped: usize,
    pub failed: Vec<ListFailedRow>,
}

#[derive(Debug)]
pub struct ListFailedRow {
    pub title: String,
    pub error: String,
}

#[derive(Debug)]
pub struct ListImportSummary {
    pub import_id: String,
    pub source: ListSource,
    pub added_count: usize,
    pub skipped_count: usize,
    pub failed_count: usize,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum ListServiceError {
    #[error("import not found")]
    NotFound,
    #[error("parse error: {0}")]
    Parse(String),
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
    ) -> Result<Vec<BibliographyEntry>, AuthorServiceError>;
    async fn refresh_bibliography(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<BibliographyEntry>, AuthorServiceError>;
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
    async fn list(
        &self,
        user_id: UserId,
        work_id: Option<WorkId>,
    ) -> Result<Vec<LibraryItem>, FileServiceError>;
    async fn get(&self, user_id: UserId, item_id: i64) -> Result<LibraryItem, FileServiceError>;
    async fn delete(&self, user_id: UserId, item_id: i64) -> Result<(), FileServiceError>;
    async fn retag(&self, user_id: UserId, item_id: i64) -> Result<(), FileServiceError>;
    async fn read_file(
        &self,
        user_id: UserId,
        item_id: i64,
    ) -> Result<FileStream, FileServiceError>;
    async fn scan_root_folder(
        &self,
        user_id: UserId,
        root_folder_id: i64,
    ) -> Result<ScanResult, FileServiceError>;
    async fn get_scan(
        &self,
        user_id: UserId,
        scan_id: &str,
    ) -> Result<ScanResult, FileServiceError>;
}

#[trait_variant::make(Send)]
pub trait ListService: Send + Sync {
    async fn preview(
        &self,
        user_id: UserId,
        req: ListPreviewRequest,
    ) -> Result<ListPreviewResponse, ListServiceError>;
    async fn confirm(
        &self,
        user_id: UserId,
        import_id: &str,
    ) -> Result<ListConfirmResponse, ListServiceError>;
    async fn undo(&self, user_id: UserId, import_id: &str) -> Result<usize, ListServiceError>;
    async fn list_imports(
        &self,
        user_id: UserId,
    ) -> Result<Vec<ListImportSummary>, ListServiceError>;
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
    async fn run_monitor(&self, user_id: UserId) -> Result<MonitorReport, MonitorError>;
}

// =============================================================================
// Free functions — TEMP(pk-tdd): compile-only scaffolds
// =============================================================================

pub async fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let path = path.to_path_buf();
    let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let tmp_path = parent.join(format!(".livrarr-tmp-{}", std::process::id()));
        let result = (|| {
            let mut f = std::fs::File::create(&tmp_path)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
            std::fs::rename(&tmp_path, &path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp_path);
        }
        result
    })
    .await
    .expect("spawn_blocking panicked")
}

pub async fn atomic_copy(src: &Path, dst: &Path) -> std::io::Result<u64> {
    let src = src.to_path_buf();
    let dst = dst.to_path_buf();
    tokio::task::spawn_blocking(move || {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let parent = dst.parent().unwrap_or_else(|| Path::new("."));
        let tmp_path = parent.join(format!(".livrarr-tmp-{}", std::process::id()));
        let result = (|| {
            let mut src_file = std::fs::File::open(&src)?;
            let mut dst_file = std::fs::File::create(&tmp_path)?;
            let copied = std::io::copy(&mut src_file, &mut dst_file)?;
            dst_file.sync_all()?;
            std::fs::rename(&tmp_path, &dst)?;
            Ok(copied)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp_path);
        }
        result
    })
    .await
    .expect("spawn_blocking panicked")
}

pub async fn cwa_copy(src: &Path, dst: &Path) -> CwaResult {
    let src = src.to_path_buf();
    let dst = dst.to_path_buf();
    tokio::task::spawn_blocking(move || {
        if let Some(parent) = dst.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return CwaResult {
                    success: false,
                    warning: Some(format!("failed to create parent directory: {e}")),
                };
            }
        }
        match std::fs::hard_link(&src, &dst) {
            Ok(()) => CwaResult {
                success: true,
                warning: None,
            },
            Err(link_err) => {
                let result = (|| -> std::io::Result<()> {
                    let mut src_file = std::fs::File::open(&src)?;
                    let parent = dst.parent().unwrap_or_else(|| Path::new("."));
                    let tmp_path = parent.join(format!(".livrarr-cwa-tmp-{}", std::process::id()));
                    let copy_result = (|| {
                        let mut dst_file = std::fs::File::create(&tmp_path)?;
                        std::io::copy(&mut src_file, &mut dst_file)?;
                        dst_file.sync_all()?;
                        std::fs::rename(&tmp_path, &dst)?;
                        Ok::<(), std::io::Error>(())
                    })();
                    if copy_result.is_err() {
                        let _ = std::fs::remove_file(&tmp_path);
                    }
                    copy_result
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
