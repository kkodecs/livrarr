use crate::history_events::HistoryDraft;
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

    /// Record one history event. Infallible by signature: history is an
    /// observer, never an actor — the impl absorbs write failures with a
    /// logged warning, so callers cannot even propagate one.
    async fn record(&self, user_id: UserId, draft: HistoryDraft);
}
