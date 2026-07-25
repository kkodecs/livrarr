use livrarr_domain::services::{MergeFieldChoice, MergeableField};
use livrarr_domain::{
    identity::CandidateId, AuthorId, CoverTrust, EnrichmentStatus, IdentityStatus, LibraryItemId,
    MediaType, NarrationType, Work, WorkId,
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
    /// Reuse handle for the per-provider payloads cached during discovery, echoed
    /// back on add so enrichment reuses them network-free (REQ-014/015).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<CandidateId>,
    /// Federated work/edition anchors carried from discovery so the add path can
    /// trust the user's pick (build identity directly, no re-resolve).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isbn_13: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hc_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gr_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asin: Option<String>,
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
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_url: Option<String>,
    #[serde(default)]
    pub cover_manual: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isbn_13: Option<String>,
    /// Echoed from the selected search result so add() reuses the discovery
    /// payloads instead of re-querying (REQ-014/015).
    #[serde(default)]
    pub candidate_id: Option<CandidateId>,
    /// Federated work anchors echoed from the pick so the handler builds identity
    /// directly (trust the pick — no re-resolve).
    #[serde(default)]
    pub hc_key: Option<String>,
    #[serde(default)]
    pub gr_key: Option<String>,
    #[serde(default)]
    pub asin: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddWorkResponse {
    pub work: WorkDetailResponse,
    /// REQ-004: explicit created-vs-existing outcome — `false` = dedup/conflict
    /// returned an existing work.
    #[serde(default)]
    pub created: bool,
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
    #[serde(default, deserialize_with = "super::double_option::deserialize")]
    pub title: Option<Option<String>>,
    #[serde(default, deserialize_with = "super::double_option::deserialize")]
    pub author_name: Option<Option<String>>,
    #[serde(default, deserialize_with = "super::double_option::deserialize")]
    pub series_name: Option<Option<String>>,
    #[serde(default, deserialize_with = "super::double_option::deserialize")]
    pub series_position: Option<Option<f64>>,
    #[serde(default, deserialize_with = "super::double_option::deserialize")]
    pub monitor_ebook: Option<Option<bool>>,
    #[serde(default, deserialize_with = "super::double_option::deserialize")]
    pub monitor_audiobook: Option<Option<bool>>,
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
    pub identity_status: IdentityStatus,
    pub enriched_at: Option<String>,
    pub enrichment_source: Option<String>,
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
    pub added_at: String,
    pub library_items: Vec<LibraryItemResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_mtime: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audiobook_cover_mtime: Option<i64>,
    /// REQ-005: true exactly while an enrichment run is executing for this
    /// work (in-memory signal; false after restart by design).
    #[serde(default)]
    pub enriching: bool,
    /// True exactly when open identity conflicts park the work (identity-edit
    /// r4): re-matching and enrichment stay paused until the Review surface
    /// settles them. Computed on every shared work-detail mapping from the
    /// persisted Conflict badge, which the conflict transactions and
    /// `derive_badge_in_tx` maintain.
    #[serde(default)]
    pub parked_by_conflicts: bool,
}

/// Convert a domain `Work` into a `WorkDetailResponse` (with empty `library_items`).
/// Use this shared mapper instead of duplicating the field-by-field conversion.
pub fn work_to_detail(w: &Work) -> WorkDetailResponse {
    work_to_detail_with_cover_mtime(w, None, None)
}

/// Convert a domain `MergePreview` into its API DTO.
pub fn merge_preview_to_response(
    p: livrarr_domain::services::MergePreview,
) -> MergePreviewResponse {
    MergePreviewResponse {
        survivor_id: p.survivor_id,
        loser_id: p.loser_id,
        library_items_moving: p.library_items_moving,
        grabs_moving: p.grabs_moving,
        monitor_ebook_result: p.monitor_ebook_result,
        monitor_audiobook_result: p.monitor_audiobook_result,
        conflicts: p
            .conflicts
            .into_iter()
            .map(|c| MergeConflictDTO {
                field: c.field,
                survivor_value: c.survivor_value,
                loser_value: c.loser_value,
            })
            .collect(),
    }
}

pub fn work_to_detail_with_cover_mtime(
    w: &Work,
    cover_mtime: Option<i64>,
    audiobook_cover_mtime: Option<i64>,
) -> WorkDetailResponse {
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
        identity_status: w.identity_status,
        enriched_at: w.enriched_at.map(|d| d.to_rfc3339()),
        enrichment_source: w.enrichment_source.clone(),
        cover_manual: w.cover_manual,
        cover_source: w.cover_source.clone(),
        cover_trust: w.cover_trust,
        cover_width: w.cover_width,
        cover_height: w.cover_height,
        audiobook_cover_url: w.audiobook_cover_url.clone(),
        audiobook_cover_source: w.audiobook_cover_source.clone(),
        audiobook_cover_trust: w.audiobook_cover_trust,
        audiobook_cover_width: w.audiobook_cover_width,
        audiobook_cover_height: w.audiobook_cover_height,
        monitor_ebook: w.monitor_ebook,
        monitor_audiobook: w.monitor_audiobook,
        added_at: w.added_at.to_rfc3339(),
        library_items: vec![],
        cover_mtime,
        audiobook_cover_mtime,
        enriching: false,
        parked_by_conflicts: w.identity_status == IdentityStatus::Conflict,
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
    pub progress_pct: Option<f64>,
    pub duration_seconds: Option<f64>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteWorkResponse {
    pub warnings: Vec<String>,
}

// --- Merge duplicates (REQ-015) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeConflictDTO {
    pub field: MergeableField,
    pub survivor_value: String,
    pub loser_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergePreviewResponse {
    pub survivor_id: WorkId,
    pub loser_id: WorkId,
    pub library_items_moving: usize,
    pub grabs_moving: usize,
    pub monitor_ebook_result: bool,
    pub monitor_audiobook_result: bool,
    /// Fields where both works carry a differing value — the execute call
    /// refuses unless every entry here has a matching choice (AC-025).
    pub conflicts: Vec<MergeConflictDTO>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeChoiceEntryDTO {
    pub field: MergeableField,
    pub choice: MergeFieldChoice,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeWorksRequest {
    #[serde(default)]
    pub choices: Vec<MergeChoiceEntryDTO>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeWorksResponse {
    pub survivor: WorkDetailResponse,
    pub library_items_moved: usize,
    pub grabs_moved: usize,
    /// Warnings from the best-effort physical file reorganization step —
    /// never a sign of lost data (REQ-015 c guarantees zero deletions).
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
