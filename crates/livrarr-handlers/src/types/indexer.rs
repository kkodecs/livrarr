use livrarr_domain::IndexerId;
use serde::{Deserialize, Serialize};

fn default_api_path() -> String {
    "/api".to_string()
}

fn default_categories() -> Vec<i32> {
    vec![7020, 3030]
}

fn default_priority() -> i32 {
    25
}

fn default_true() -> bool {
    true
}

fn default_torrent_protocol() -> String {
    "torrent".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIndexerApiRequest {
    pub name: String,
    #[serde(default = "default_torrent_protocol")]
    pub protocol: String,
    pub url: String,
    #[serde(default = "default_api_path")]
    pub api_path: String,
    pub api_key: Option<String>,
    #[serde(default = "default_categories")]
    pub categories: Vec<i32>,
    #[serde(default = "default_priority")]
    pub priority: i32,
    #[serde(default = "default_true")]
    pub enable_automatic_search: bool,
    #[serde(default = "default_true")]
    pub enable_interactive_search: bool,
    #[serde(default = "default_true")]
    pub enable_rss: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateIndexerApiRequest {
    pub name: Option<String>,
    pub url: Option<String>,
    pub api_path: Option<String>,
    #[serde(default, deserialize_with = "crate::deserialize_optional_secret")]
    pub api_key: Option<Option<String>>,
    pub categories: Option<Vec<i32>>,
    pub priority: Option<i32>,
    pub enable_automatic_search: Option<bool>,
    pub enable_interactive_search: Option<bool>,
    pub enable_rss: Option<bool>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexerResponse {
    pub id: IndexerId,
    pub name: String,
    pub protocol: String,
    pub url: String,
    pub api_path: String,
    pub api_key_set: bool,
    pub categories: Vec<i32>,
    pub priority: i32,
    pub enable_automatic_search: bool,
    pub enable_interactive_search: bool,
    pub supports_book_search: bool,
    pub enable_rss: bool,
    pub enabled: bool,
    pub added_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestIndexerApiRequest {
    pub url: String,
    pub api_path: String,
    pub api_key: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestIndexerApiResponse {
    pub ok: bool,
    pub supports_book_search: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
