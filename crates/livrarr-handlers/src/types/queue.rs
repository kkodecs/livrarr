use livrarr_domain::{GrabId, GrabStatus, MediaType, QueueProgress, WorkId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItemResponse {
    pub id: GrabId,
    pub title: String,
    pub status: GrabStatus,
    pub size: Option<i64>,
    pub media_type: Option<MediaType>,
    pub indexer: String,
    pub download_client: String,
    /// The download client's own id for this grab (the torrent's info hash for qBittorrent),
    /// so an external tool can match a download to its grab.
    pub download_id: Option<String>,
    pub work_id: WorkId,
    pub protocol: String,
    pub error: Option<String>,
    pub grabbed_at: String,
    pub progress: Option<QueueProgress>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueListResponse {
    pub items: Vec<QueueItemResponse>,
    pub total: i64,
    pub page: u32,
    pub per_page: u32,
}
