use livrarr_domain::{DownloadClientId, WorkId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseResponse {
    pub title: String,
    pub indexer: String,
    pub size: i64,
    pub guid: String,
    pub download_url: String,
    pub seeders: Option<i32>,
    pub leechers: Option<i32>,
    pub publish_date: Option<String>,
    pub protocol: String,
    pub categories: Vec<i32>,
    pub format: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseSearchResponse {
    pub results: Vec<ReleaseResponse>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<SearchWarning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_age_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchWarning {
    pub indexer: String,
    pub error: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrabApiRequest {
    pub work_id: WorkId,
    pub download_url: String,
    pub title: String,
    pub indexer: String,
    pub guid: String,
    pub size: i64,
    pub download_client_id: Option<DownloadClientId>,
    pub protocol: Option<String>,
    #[serde(default)]
    pub categories: Vec<i32>,
}
