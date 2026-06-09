use serde::{Deserialize, Serialize};

use crate::{
    AuthorId, DbError, EnrichmentStatus, LibraryItem, MediaType, ProvenanceSetter, UserId, Work,
    WorkId,
};

/// Domain-owned source metadata from external systems (e.g., Readarr import).
/// Enters the enrichment pipeline as a provider input via MetadataProvider::Readarr.
/// Converted to NormalizedWorkDetail at the livrarr-metadata crate boundary.
#[derive(Debug, Clone, Default)]
pub struct SourceProviderData {
    pub description: Option<String>,
    pub isbn: Option<String>,
    pub asin: Option<String>,
    pub publisher: Option<String>,
    pub genres: Option<Vec<String>>,
    pub page_count: Option<i32>,
    pub rating: Option<f64>,
    pub rating_count: Option<i32>,
    pub cover_url: Option<String>,
    pub series_name: Option<String>,
    pub series_position: Option<String>,
}

#[derive(Debug, Default)]
pub struct AddWorkRequest {
    // Core identity
    pub title: String,
    pub author_name: String,
    pub year: Option<i32>,
    pub language: Option<String>,

    // Provider keys
    pub ol_key: Option<String>,
    pub gr_key: Option<String>,
    pub author_ol_key: Option<String>,
    pub cover_url: Option<String>,
    pub detail_url: Option<String>,

    // Series
    pub series_id: Option<i64>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,

    // Monitoring — defaults to both true if None
    pub monitor_ebook: Option<bool>,
    pub monitor_audiobook: Option<bool>,

    // Provenance
    pub provenance_setter: Option<ProvenanceSetter>,

    // Import context
    pub import_id: Option<String>,

    // Source provider data (e.g., from Readarr import)
    // Passed into enrichment pipeline as MetadataProvider::Readarr input.
    // Not written to the work directly — the merge engine arbitrates.
    pub source_provider_data: Option<SourceProviderData>,
}

#[derive(Debug)]
pub struct AddWorkResult {
    pub work: Work,
    /// true if a new work was created, false if dedup matched an existing work.
    pub created: bool,
    pub author_created: bool,
    pub author_id: Option<i64>,
    pub messages: Vec<String>,
    pub cover_mtime: Option<i64>,
    pub audiobook_cover_mtime: Option<i64>,
    /// Final enrichment status after synchronous enrichment attempt.
    pub enrichment_status: EnrichmentStatus,
}

/// Per-item result from tag sync. TagService returns these;
/// the caller updates DB tag_status per item.
#[derive(Debug)]
pub struct TagSyncItemResult {
    pub library_item_id: i64,
    pub succeeded: bool,
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct UpdateWorkRequest {
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub series_name: Option<Option<String>>,
    pub series_position: Option<Option<f64>>,
    pub monitor_ebook: Option<bool>,
    pub monitor_audiobook: Option<bool>,
}

#[derive(Debug)]
pub struct WorkDetailView {
    pub work: Work,
    pub library_items: Vec<LibraryItem>,
    pub cover_mtime: Option<i64>,
    pub audiobook_cover_mtime: Option<i64>,
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
    RecentlyDownloaded,
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

/// Outcome of a single user-triggered "retry all incomplete" sweep.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct RetrySummary {
    /// Incomplete works found and swept (Failed, Unenriched, or identity-Pending).
    pub total: usize,
    /// Works that reached a settled+enriched state after the pass.
    pub recovered: usize,
    /// Works still incomplete after the pass (left for a later retry).
    pub still_incomplete: usize,
}

// Dead: bulk refresh is implemented at the handler layer
// (`crates/livrarr-handlers/src/work.rs::refresh_all`) per insight 9g
// (handler-level spawning for long-running background work). This type
// was an earlier design that never wired up. Restore only if the spawn
// pattern is ever moved into the service layer.
// #[derive(Debug)]
// pub struct RefreshAllHandle {
//     pub total_works: usize,
// }

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isbn_13: Option<String>,
    /// Handle to the cached per-provider payloads fetched during discovery, so
    /// the add path can reuse them without re-querying (R-002/R-009; REQ-014/015).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<crate::identity::CandidateId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hc_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gr_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asin: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LookupResponse {
    pub results: Vec<LookupResult>,
    pub filtered_count: usize,
    pub raw_count: usize,
    pub raw_available: bool,
}

/// One parsed manual-import file's best-guess query for the eager auto-match
/// pass (#97). `id` ties the match back to the originating file (results are
/// returned as `(id, LookupResult)` pairs); the eager matcher groups these by
/// `author` so one author-scoped provider query serves all of that author's
/// files, then matches each `title` — or the embedded `isbn`, which pins the
/// exact edition — against the author's returned corpus.
#[derive(Debug, Clone)]
pub struct EagerQuery {
    pub id: usize,
    pub title: String,
    pub author: String,
    pub language: Option<String>,
    pub isbn: Option<String>,
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
    #[error("validation error: {0}")]
    Validation(String),
    #[error("enrichment failed: {0}")]
    Enrichment(String),
    #[error("cover download failed: {0}")]
    Cover(String),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

#[trait_variant::make(Send)]
pub trait WorkService: Send + Sync {
    async fn add(
        &self,
        user_id: UserId,
        candidate: crate::identity::WorkCandidate,
    ) -> Result<AddWorkResult, WorkServiceError>;
    /// Resolve a raw identity harvest into a [`crate::identity::ResolvedIdentity`]
    /// through the shared multi-provider resolver — the single place identity is
    /// decided, so every creation path agrees on what a book is (P1). `tier`
    /// selects interactive (synchronous; a person is waiting) vs background (bulk).
    /// With no resolver configured or no usable anchor in the harvest, returns a
    /// `Pending` identity (anchors preserved) and no conflict — never a fabricated
    /// `Confirmed`.
    async fn resolve_identity(
        &self,
        user_id: UserId,
        harvest: crate::identity::RawHarvest,
        tier: crate::identity::LatencyTier,
    ) -> Result<crate::identity::ResolvedIdentity, WorkServiceError>;
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
    #[allow(clippy::too_many_arguments)]
    async fn list_paginated(
        &self,
        user_id: UserId,
        page: u32,
        page_size: u32,
        sort_by: WorkSortField,
        sort_dir: SortDirection,
        media_type: Option<MediaType>,
        language: Option<&str>,
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
    /// User-triggered bulk recovery (REQ-011 / PO §7): sweep every "incomplete"
    /// work for the user — Failed, Unenriched, or identity-Pending — and re-run
    /// each through the one road in a **single pass, no recurring loop**. A
    /// Pending work re-resolves identity first (the convergence the deleted
    /// background job used to do); the rest re-enrich via [`Self::refresh`].
    /// Replaces the removed `enrichment_retry_tick`.
    async fn retry_all_incomplete(&self, user_id: UserId)
        -> Result<RetrySummary, WorkServiceError>;
    // Dead: bulk refresh is implemented at the handler layer
    // (`crates/livrarr-handlers/src/work.rs::refresh_all`) per insight 9g.
    // Restore here only if the spawn pattern is ever moved into services.
    // async fn refresh_all(&self, user_id: UserId) -> Result<RefreshAllHandle, WorkServiceError>;
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
        user_id: UserId,
        req: LookupRequest,
        raw: bool,
    ) -> Result<LookupResponse, WorkServiceError>;
    /// Eager, bulk best-guess discovery for manual import (#97). Groups
    /// `queries` by author and issues one author-scoped query per provider
    /// (Google Books `inauthor:`, OpenLibrary `author:`) instead of one search
    /// per title — imports cluster heavily by author, so this collapses N
    /// title searches into ~one call per author per provider. Each query's
    /// title is then matched locally against the author's returned corpus.
    ///
    /// Suggestion-only: no resolver call, so the returned `LookupResult` carries
    /// `candidate_id: None`. Identity is locked later at create time by
    /// `add`'s resolve-at-pick. Queries with no confident corpus match are
    /// omitted from the result; each present entry pairs the query `id` with
    /// its best match.
    async fn eager_match_by_author(
        &self,
        user_id: UserId,
        queries: Vec<EagerQuery>,
    ) -> Result<Vec<(usize, LookupResult)>, WorkServiceError>;
    async fn search_works(
        &self,
        user_id: UserId,
        query: &str,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<Work>, i64), WorkServiceError>;
    fn try_start_bulk_refresh(&self, user_id: i64) -> bool;
    fn finish_bulk_refresh(&self, user_id: i64);
}
