use livrarr_domain::{NotificationId, NotificationType, UserId};
use serde::{Deserialize, Serialize};

use super::api_error::ApiError;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationResponse {
    pub id: NotificationId,
    pub notification_type: NotificationType,
    pub ref_key: Option<String>,
    pub message: String,
    pub data: serde_json::Value,
    pub read: bool,
    pub created_at: String,
}

#[trait_variant::make(Send)]
pub trait NotificationApi: Send + Sync {
    async fn list(
        &self,
        user_id: UserId,
        unread_only: bool,
    ) -> Result<Vec<NotificationResponse>, ApiError>;
    async fn mark_read(&self, user_id: UserId, id: NotificationId) -> Result<(), ApiError>;
    async fn dismiss(&self, user_id: UserId, id: NotificationId) -> Result<(), ApiError>;
    async fn dismiss_all(&self, user_id: UserId) -> Result<(), ApiError>;
}
