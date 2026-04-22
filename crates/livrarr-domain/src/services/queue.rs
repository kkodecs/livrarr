use crate::{
    DbError, DownloadClient, Grab, GrabId, GrabStatus, QueueProgress, QueueSummary, UserId,
};

#[derive(Debug, thiserror::Error)]
pub enum QueueServiceError {
    #[error("grab not found")]
    NotFound,
    #[error("not in importable state")]
    NotImportable,
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

#[trait_variant::make(Send)]
pub trait QueueService: Send + Sync {
    async fn list_grabs_paginated(
        &self,
        user_id: UserId,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<Grab>, i64), QueueServiceError>;

    async fn list_download_clients(&self) -> Result<Vec<DownloadClient>, QueueServiceError>;

    async fn try_set_importing(
        &self,
        user_id: UserId,
        grab_id: GrabId,
    ) -> Result<bool, QueueServiceError>;

    async fn update_grab_status(
        &self,
        user_id: UserId,
        grab_id: GrabId,
        status: GrabStatus,
        error: Option<&str>,
    ) -> Result<(), QueueServiceError>;

    async fn fetch_download_progress(
        &self,
        client: &DownloadClient,
        download_id: &str,
    ) -> Option<QueueProgress>;

    async fn summary(&self, user_id: UserId) -> Result<QueueSummary, QueueServiceError>;
}
