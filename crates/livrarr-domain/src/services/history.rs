use crate::{DbError, HistoryEvent, HistoryFilter, UserId};

#[derive(Debug, thiserror::Error)]
pub enum HistoryServiceError {
    #[error("not found")]
    NotFound,
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

#[trait_variant::make(Send)]
pub trait HistoryService: Send + Sync {
    async fn list_paginated(
        &self,
        user_id: UserId,
        filter: HistoryFilter,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<HistoryEvent>, i64), HistoryServiceError>;
}
