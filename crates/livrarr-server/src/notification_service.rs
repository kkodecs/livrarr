use livrarr_db::NotificationDb;
use livrarr_domain::services::{NotificationService, NotificationServiceError};
use livrarr_domain::{Notification, NotificationId, UserId};

pub struct NotificationServiceImpl<D> {
    db: D,
}

impl<D> NotificationServiceImpl<D> {
    pub fn new(db: D) -> Self {
        Self { db }
    }
}

fn map_db_err(e: livrarr_domain::DbError) -> NotificationServiceError {
    match e {
        livrarr_domain::DbError::NotFound { .. } => NotificationServiceError::NotFound,
        other => NotificationServiceError::Db(other),
    }
}

impl<D> NotificationService for NotificationServiceImpl<D>
where
    D: NotificationDb + Send + Sync + 'static,
{
    async fn list_paginated(
        &self,
        user_id: UserId,
        unread_only: bool,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<Notification>, i64), NotificationServiceError> {
        self.db
            .list_notifications_paginated(user_id, unread_only, page, page_size)
            .await
            .map_err(map_db_err)
    }

    async fn mark_read(
        &self,
        user_id: UserId,
        id: NotificationId,
    ) -> Result<(), NotificationServiceError> {
        self.db
            .mark_notification_read(user_id, id)
            .await
            .map_err(map_db_err)
    }

    async fn dismiss(
        &self,
        user_id: UserId,
        id: NotificationId,
    ) -> Result<(), NotificationServiceError> {
        self.db
            .dismiss_notification(user_id, id)
            .await
            .map_err(map_db_err)
    }

    async fn dismiss_all(&self, user_id: UserId) -> Result<(), NotificationServiceError> {
        self.db
            .dismiss_all_notifications(user_id)
            .await
            .map_err(map_db_err)
    }

    async fn create(
        &self,
        req: livrarr_domain::services::CreateNotificationRequest,
    ) -> Result<Notification, NotificationServiceError> {
        self.db
            .create_notification(livrarr_db::CreateNotificationDbRequest {
                user_id: req.user_id,
                notification_type: req.notification_type,
                ref_key: req.ref_key,
                message: req.message,
                data: req.data,
            })
            .await
            .map_err(map_db_err)
    }
}
