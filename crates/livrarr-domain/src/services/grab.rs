use serde::{Deserialize, Serialize};

use crate::{DbError, Grab, GrabId, GrabStatus, UserId};

#[derive(Debug)]
pub struct GrabFilter {
    pub status: Option<GrabStatus>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug)]
pub struct QueueItem {
    pub grab: Grab,
    pub progress: Option<DownloadProgress>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadProgressStatus {
    Downloading,
    Paused,
    Queued,
    Stalled,
    Seeding,
    Extracting,
    Verifying,
    Unknown,
}

#[derive(Debug)]
pub struct DownloadProgress {
    pub percent: f64,
    pub speed_bytes_per_sec: Option<u64>,
    pub eta_seconds: Option<u64>,
    pub status: DownloadProgressStatus,
}

#[derive(Debug, thiserror::Error)]
pub enum GrabServiceError {
    #[error("grab not found")]
    NotFound,
    #[error("download client unreachable: {0}")]
    ClientUnreachable(String),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

#[trait_variant::make(Send)]
pub trait GrabService: Send + Sync {
    async fn list(
        &self,
        user_id: UserId,
        filter: GrabFilter,
    ) -> Result<Vec<QueueItem>, GrabServiceError>;
    async fn get(&self, user_id: UserId, grab_id: GrabId) -> Result<QueueItem, GrabServiceError>;
    async fn remove(&self, user_id: UserId, grab_id: GrabId) -> Result<(), GrabServiceError>;
}
