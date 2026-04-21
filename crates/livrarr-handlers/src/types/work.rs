use livrarr_domain::{
    AuthorId, EnrichmentStatus, LibraryItemId, MediaType, NarrationType, Work, WorkId,
};
use serde::{Deserialize, Serialize};

use super::api_error::ApiError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupApiResponse {
    pub results: Vec<WorkSearchResult>,
    pub filtered_count: usize,
    pub raw_count: usize,
    pub raw_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSearchResult {
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

#[trait_variant::make(Send)]
pub trait WorkApi: Send + Sync {
    async fn lookup(
        &self,
        user_id: livrarr_domain::UserId,
        term: &str,
    ) -> Result<Vec<WorkSearchResult>, ApiError>;
    async fn add(
        &self,
        user_id: livrarr_domain::UserId,
        req: AddWorkRequest,
    ) -> Result<AddWorkResponse, ApiError>;
    async fn list(
        &self,
        user_id: livrarr_domain::UserId,
    ) -> Result<Vec<WorkDetailResponse>, ApiError>;
    async fn get(
        &self,
        user_id: livrarr_domain::UserId,
        id: WorkId,
    ) -> Result<WorkDetailResponse, ApiError>;
    async fn update(
        &self,
        user_id: livrarr_domain::UserId,
        id: WorkId,
        req: UpdateWorkRequest,
    ) -> Result<WorkDetailResponse, ApiError>;
    async fn upload_cover(
        &self,
        user_id: livrarr_domain::UserId,
        id: WorkId,
        image_data: &[u8],
        content_type: &str,
    ) -> Result<(), ApiError>;
    async fn delete(
        &self,
        user_id: livrarr_domain::UserId,
        id: WorkId,
        delete_files: bool,
    ) -> Result<DeleteWorkResponse, ApiError>;
    async fn refresh(
        &self,
        user_id: livrarr_domain::UserId,
        id: WorkId,
    ) -> Result<RefreshWorkResponse, ApiError>;
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddWorkRequest {
    pub ol_key: Option<String>,
    pub title: String,
    pub author_name: String,
    pub author_ol_key: Option<String>,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_url: Option<String>,
    #[serde(default)]
    pub defer_enrichment: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddWorkResponse {
    pub work: WorkDetailResponse,
    pub author_created: bool,
    pub messages: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshWorkResponse {
    pub work: WorkDetailResponse,
    pub messages: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWorkRequest {
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
    pub monitor_ebook: Option<bool>,
    pub monitor_audiobook: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkDetailResponse {
    pub id: WorkId,
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
    pub enriched_at: Option<String>,
    pub enrichment_source: Option<String>,
    pub cover_manual: bool,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub added_at: String,
    pub library_items: Vec<LibraryItemResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_source: Option<String>,
}

/// Convert a domain `Work` into a `WorkDetailResponse` (with empty `library_items`).
/// Use this shared mapper instead of duplicating the field-by-field conversion.
pub fn work_to_detail(w: &Work) -> WorkDetailResponse {
    WorkDetailResponse {
        id: w.id,
        title: w.title.clone(),
        sort_title: w.sort_title.clone(),
        subtitle: w.subtitle.clone(),
        original_title: w.original_title.clone(),
        author_name: w.author_name.clone(),
        author_id: w.author_id,
        description: w.description.clone(),
        year: w.year,
        series_id: w.series_id,
        series_name: w.series_name.clone(),
        series_position: w.series_position,
        genres: w.genres.clone(),
        language: w.language.clone(),
        page_count: w.page_count,
        duration_seconds: w.duration_seconds,
        publisher: w.publisher.clone(),
        publish_date: w.publish_date.clone(),
        ol_key: w.ol_key.clone(),
        hc_key: w.hc_key.clone(),
        gr_key: w.gr_key.clone(),
        isbn_13: w.isbn_13.clone(),
        asin: w.asin.clone(),
        narrator: w.narrator.clone(),
        narration_type: w.narration_type,
        abridged: w.abridged,
        rating: w.rating,
        rating_count: w.rating_count,
        enrichment_status: w.enrichment_status,
        enriched_at: w.enriched_at.map(|d| d.to_rfc3339()),
        enrichment_source: w.enrichment_source.clone(),
        cover_manual: w.cover_manual,
        monitor_ebook: w.monitor_ebook,
        monitor_audiobook: w.monitor_audiobook,
        added_at: w.added_at.to_rfc3339(),
        library_items: vec![],
        metadata_source: w.metadata_source.clone(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItemResponse {
    pub id: LibraryItemId,
    pub path: String,
    pub media_type: MediaType,
    pub file_size: i64,
    pub imported_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteWorkResponse {
    pub warnings: Vec<String>,
}

#[trait_variant::make(Send)]
pub trait LibraryFileApi: Send + Sync {
    async fn list(
        &self,
        user_id: livrarr_domain::UserId,
    ) -> Result<Vec<LibraryItemResponse>, ApiError>;
    async fn get(
        &self,
        user_id: livrarr_domain::UserId,
        id: LibraryItemId,
    ) -> Result<LibraryItemResponse, ApiError>;
    async fn delete(
        &self,
        user_id: livrarr_domain::UserId,
        id: LibraryItemId,
    ) -> Result<(), ApiError>;
}
