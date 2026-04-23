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
