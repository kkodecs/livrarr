use livrarr_domain::{AuthorId, WorkId};
use serde::{Deserialize, Serialize};

use super::work::WorkDetailResponse;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesResponse {
    pub id: Option<i64>,
    pub name: String,
    pub gr_key: String,
    pub book_count: i32,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub works_in_library: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesListResponse {
    pub series: Vec<SeriesResponse>,
    pub fetched_at: Option<String>,
    pub raw_available: bool,
    pub filtered_count: usize,
    pub raw_count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesWithAuthorResponse {
    pub id: i64,
    pub name: String,
    pub gr_key: String,
    pub book_count: i32,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub works_in_library: i64,
    pub author_id: AuthorId,
    pub author_name: String,
    pub first_work_id: Option<WorkId>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesDetailResponse {
    pub id: i64,
    pub name: String,
    pub gr_key: String,
    pub book_count: i32,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub author_id: AuthorId,
    pub author_name: String,
    pub works: Vec<WorkDetailResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorSeriesRequest {
    pub gr_key: String,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromoteSeriesRequest {
    /// Picker choice on retry; None on first attempt (exact-match road).
    pub gr_key: Option<String>,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromoteSeriesResponse {
    /// "monitoring" | "needsAuthorResolution" | "needsPicker"
    pub status: String,
    pub author_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<SeriesResponse>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<SeriesResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesBooksResponse {
    /// false for stubs — no gr_key, no roster source; rows are linked works only.
    pub roster_available: bool,
    pub rows: Vec<SeriesBookRowResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesBookRowResponse {
    pub position: Option<f64>,
    pub in_library: bool,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    /// Present when `in_library` — the work with its library items, for the
    /// standard presence indication.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work: Option<crate::types::work::WorkDetailResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSeriesRequest {
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrAuthorCandidate {
    pub gr_key: String,
    pub name: String,
    pub profile_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveGrResponse {
    pub candidates: Vec<GrAuthorCandidate>,
    #[serde(default)]
    pub auto_linked: bool,
}
