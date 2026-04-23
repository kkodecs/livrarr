use serde::{Deserialize, Serialize};

use crate::{DbError, Grab, UserId, WorkId};

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
    pub search_query: String,
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

#[trait_variant::make(Send)]
pub trait ReleaseService: Send + Sync {
    async fn search(
        &self,
        user_id: UserId,
        req: SearchReleasesRequest,
    ) -> Result<ReleaseSearchResponse, ReleaseServiceError>;
    async fn grab(&self, user_id: UserId, req: GrabRequest) -> Result<Grab, ReleaseServiceError>;
}
