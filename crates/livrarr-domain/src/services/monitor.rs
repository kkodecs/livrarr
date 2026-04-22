use crate::DbError;

use super::work::WorkServiceError;

#[derive(Debug)]
pub struct MonitorReport {
    pub authors_checked: usize,
    pub new_works_found: usize,
    pub works_added: usize,
    pub notifications_created: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum MonitorError {
    #[error("OpenLibrary lookup failed: {0}")]
    ProviderFailed(String),
    #[error("OpenLibrary rate limited")]
    RateLimited,
    #[error("monitor already running")]
    AlreadyRunning,
    #[error("work add failed: {0}")]
    WorkAdd(#[from] WorkServiceError),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

#[trait_variant::make(Send)]
pub trait AuthorMonitorWorkflow: Send + Sync {
    async fn run_monitor(
        &self,
        user_id: crate::UserId,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<MonitorReport, MonitorError>;
    fn trigger_monitor(&self);
}
