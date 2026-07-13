//! Notification data access: `NotificationDb` trait + request type.

use crate::{DbError, Notification, NotificationId, NotificationType, UserId};

/// Notification data access.
///
/// Satisfies: AUTHOR-003, AUTHOR-005
#[trait_variant::make(Send)]
pub trait NotificationDb: Send + Sync {
    /// List notifications for a user. Optional filter for unread only (unbounded).
    async fn list_notifications(
        &self,
        user_id: UserId,
        unread_only: bool,
    ) -> Result<Vec<Notification>, DbError>;

    /// List notifications, paginated.
    async fn list_notifications_paginated(
        &self,
        user_id: UserId,
        unread_only: bool,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<Notification>, i64), DbError>;

    /// Create notification. Respects dedup: one per (user_id, type, ref_key) regardless
    /// of dismissed state. If any notification (active or dismissed) exists for that
    /// combination, returns Ok without creating. Dismissed means "don't tell me again."
    ///
    /// Satisfies: AUTHOR-003 (dedup)
    /// Postcondition: At most one notification per (user_id, type, ref_key) ever.
    async fn create_notification(
        &self,
        req: CreateNotificationDbRequest,
    ) -> Result<Notification, DbError>;

    /// Mark notification as read.
    async fn mark_notification_read(
        &self,
        user_id: UserId,
        id: NotificationId,
    ) -> Result<(), DbError>;

    /// Dismiss notification (sets dismissed=1). Permanent -- dedup blocks
    /// re-creation for this (user_id, type, ref_key) combination.
    ///
    /// Satisfies: AUTHOR-005
    async fn dismiss_notification(
        &self,
        user_id: UserId,
        id: NotificationId,
    ) -> Result<(), DbError>;

    /// Dismiss all notifications for a user. Permanent for each ref_key.
    async fn dismiss_all_notifications(&self, user_id: UserId) -> Result<(), DbError>;
}

pub struct CreateNotificationDbRequest {
    pub user_id: UserId,
    pub notification_type: NotificationType,
    pub ref_key: Option<String>,
    pub message: String,
    pub data: serde_json::Value,
}
