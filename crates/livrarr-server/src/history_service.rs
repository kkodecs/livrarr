use livrarr_db::HistoryDb;
use livrarr_domain::services::{HistoryService, HistoryServiceError};
use livrarr_domain::{HistoryEvent, HistoryFilter, UserId};

pub struct HistoryServiceImpl<D> {
    db: D,
}

impl<D> HistoryServiceImpl<D> {
    pub fn new(db: D) -> Self {
        Self { db }
    }
}

fn map_db_err(e: livrarr_domain::DbError) -> HistoryServiceError {
    match e {
        livrarr_domain::DbError::NotFound { .. } => HistoryServiceError::NotFound,
        other => HistoryServiceError::Db(other),
    }
}

impl<D> HistoryService for HistoryServiceImpl<D>
where
    D: HistoryDb + Send + Sync + 'static,
{
    async fn list_paginated(
        &self,
        user_id: UserId,
        filter: HistoryFilter,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<HistoryEvent>, i64), HistoryServiceError> {
        self.db
            .list_history_paginated(user_id, filter, page, page_size)
            .await
            .map_err(map_db_err)
    }
}
