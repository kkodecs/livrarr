use crate::{DbError, Notification, NotificationId, NotificationType, UserId};

#[derive(Debug, thiserror::Error)]
pub enum NotificationServiceError {
    #[error("notification not found")]
    NotFound,
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

pub struct CreateNotificationRequest {
    pub user_id: UserId,
    pub notification_type: NotificationType,
    pub ref_key: Option<String>,
    pub message: String,
    pub data: serde_json::Value,
}

#[trait_variant::make(Send)]
pub trait NotificationService: Send + Sync {
    async fn list_paginated(
        &self,
        user_id: UserId,
        unread_only: bool,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<Notification>, i64), NotificationServiceError>;

    async fn mark_read(
        &self,
        user_id: UserId,
        id: NotificationId,
    ) -> Result<(), NotificationServiceError>;

    async fn dismiss(
        &self,
        user_id: UserId,
        id: NotificationId,
    ) -> Result<(), NotificationServiceError>;

    async fn dismiss_all(&self, user_id: UserId) -> Result<(), NotificationServiceError>;

    async fn create(
        &self,
        req: CreateNotificationRequest,
    ) -> Result<Notification, NotificationServiceError>;
}
