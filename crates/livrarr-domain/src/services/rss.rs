use crate::DbError;

use super::release::ReleaseServiceError;

#[derive(Debug)]
pub struct RssSyncReport {
    pub feeds_checked: usize,
    pub releases_matched: usize,
    pub grabs_attempted: usize,
    pub grabs_succeeded: usize,
    pub warnings: Vec<String>,
}

impl RssSyncReport {
    pub fn empty() -> Self {
        Self {
            feeds_checked: 0,
            releases_matched: 0,
            grabs_attempted: 0,
            grabs_succeeded: 0,
            warnings: vec![],
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RssSyncError {
    #[error("feed fetch failed: {0}")]
    FeedFetch(String),
    #[error("release search failed: {0}")]
    Search(#[from] ReleaseServiceError),
    #[error("grab failed: {0}")]
    Grab(String),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

#[trait_variant::make(Send)]
pub trait RssSyncWorkflow: Send + Sync {
    async fn run_sync(&self) -> Result<RssSyncReport, RssSyncError>;
}
