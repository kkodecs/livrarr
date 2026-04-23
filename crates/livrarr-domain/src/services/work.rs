use serde::{Deserialize, Serialize};

use crate::{
    AuthorId, DbError, EnrichmentStatus, LibraryItem, MediaType, ProvenanceSetter, UserId, Work,
    WorkId,
};

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
    pub cover_mtime: Option<i64>,
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
    async fn download_cover_from_url(
        &self,
        user_id: i64,
        work_id: i64,
        cover_url: &str,
    ) -> Result<(), WorkServiceError>;
    fn try_start_bulk_refresh(&self, user_id: i64) -> bool;
    fn finish_bulk_refresh(&self, user_id: i64);
}
