use livrarr_db::GrabDb;
use livrarr_domain::services::*;
use livrarr_domain::*;

pub struct GrabServiceImpl<D> {
    db: D,
}

impl<D> GrabServiceImpl<D> {
    pub fn new(db: D) -> Self {
        Self { db }
    }
}

impl<D> GrabService for GrabServiceImpl<D>
where
    D: GrabDb + Send + Sync,
{
    async fn list(
        &self,
        user_id: UserId,
        filter: GrabFilter,
    ) -> Result<Vec<QueueItem>, GrabServiceError> {
        let page = filter.page.unwrap_or(1);
        let per_page = filter.per_page.unwrap_or(50);

        let (grabs, _total) = self
            .db
            .list_grabs_paginated(user_id, page, per_page)
            .await
            .map_err(GrabServiceError::Db)?;

        let queue_items: Vec<QueueItem> = grabs
            .into_iter()
            .filter(|g| {
                if let Some(ref status) = filter.status {
                    g.status == *status
                } else {
                    true
                }
            })
            .map(|grab| {
                // Without a download client reference, progress is None.
                // Live progress polling requires download client integration.
                QueueItem {
                    grab,
                    progress: None,
                }
            })
            .collect();

        Ok(queue_items)
    }

    async fn get(&self, user_id: UserId, grab_id: GrabId) -> Result<QueueItem, GrabServiceError> {
        let grab = self
            .db
            .get_grab(user_id, grab_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => GrabServiceError::NotFound,
                other => GrabServiceError::Db(other),
            })?;

        // Without download client integration, progress is None.
        Ok(QueueItem {
            grab,
            progress: None,
        })
    }

    async fn remove(&self, user_id: UserId, grab_id: GrabId) -> Result<(), GrabServiceError> {
        // Verify grab exists for this user
        let _grab = self
            .db
            .get_grab(user_id, grab_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => GrabServiceError::NotFound,
                other => GrabServiceError::Db(other),
            })?;

        // Best-effort download client removal would go here.
        // DB state is authoritative; client state is best-effort.

        // Mark as removed in DB
        self.db
            .update_grab_status(user_id, grab_id, GrabStatus::Removed, None)
            .await
            .map_err(GrabServiceError::Db)?;

        Ok(())
    }
}
