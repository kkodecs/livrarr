use chrono::Utc;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, parse_dt};
use crate::{
    CreateNotificationDbRequest, DbError, Notification, NotificationDb, NotificationId,
    NotificationType, UserId,
};

fn row_to_notification(row: sqlx::sqlite::SqliteRow) -> Result<Notification, DbError> {
    let type_str: String = row.try_get("type").map_err(|e| DbError::Io(Box::new(e)))?;
    let data_str: String = row.try_get("data").map_err(|e| DbError::Io(Box::new(e)))?;
    let created_at_str: String = row
        .try_get("created_at")
        .map_err(|e| DbError::Io(Box::new(e)))?;

    Ok(Notification {
        id: row
            .try_get::<i64, _>("id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        user_id: row
            .try_get::<i64, _>("user_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        notification_type: parse_notification_type(&type_str),
        ref_key: row
            .try_get("ref_key")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        message: row
            .try_get("message")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        data: serde_json::from_str(&data_str).map_err(|e| DbError::Io(Box::new(e)))?,
        read: row
            .try_get::<bool, _>("read")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        dismissed: row
            .try_get::<bool, _>("dismissed")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        created_at: parse_dt(&created_at_str)?,
    })
}

fn parse_notification_type(s: &str) -> NotificationType {
    match s {
        "workAutoAdded" => NotificationType::WorkAutoAdded,
        "metadataUpdated" => NotificationType::MetadataUpdated,
        "bulkEnrichmentComplete" => NotificationType::BulkEnrichmentComplete,
        "jobPanicked" => NotificationType::JobPanicked,
        "rateLimitHit" => NotificationType::RateLimitHit,
        _ => NotificationType::NewWorkDetected,
    }
}

fn notification_type_str(t: NotificationType) -> &'static str {
    match t {
        NotificationType::NewWorkDetected => "newWorkDetected",
        NotificationType::WorkAutoAdded => "workAutoAdded",
        NotificationType::MetadataUpdated => "metadataUpdated",
        NotificationType::BulkEnrichmentComplete => "bulkEnrichmentComplete",
        NotificationType::JobPanicked => "jobPanicked",
        NotificationType::RateLimitHit => "rateLimitHit",
    }
}

impl NotificationDb for SqliteDb {
    async fn list_notifications(
        &self,
        user_id: UserId,
        unread_only: bool,
    ) -> Result<Vec<Notification>, DbError> {
        let query = if unread_only {
            "SELECT * FROM notifications WHERE user_id = ? AND dismissed = 0 AND read = 0 ORDER BY id DESC"
        } else {
            "SELECT * FROM notifications WHERE user_id = ? AND dismissed = 0 ORDER BY id DESC"
        };
        let rows = sqlx::query(query)
            .bind(user_id)
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;
        rows.into_iter().map(row_to_notification).collect()
    }

    async fn create_notification(
        &self,
        req: CreateNotificationDbRequest,
    ) -> Result<Notification, DbError> {
        let now = Utc::now().to_rfc3339();
        let type_str = notification_type_str(req.notification_type);
        let data_str = serde_json::to_string(&req.data).map_err(|e| DbError::Io(Box::new(e)))?;

        // Dedup: check if notification already exists for (user_id, type, ref_key).
        let existing = sqlx::query(
            "SELECT id FROM notifications WHERE user_id = ? AND type = ? AND ref_key IS ?",
        )
        .bind(req.user_id)
        .bind(type_str)
        .bind(&req.ref_key)
        .fetch_optional(self.pool())
        .await
        .map_err(map_db_err)?;

        if let Some(row) = existing {
            let id: i64 = row.try_get("id").map_err(|e| DbError::Io(Box::new(e)))?;
            // Return existing without creating.
            let all = self.list_notifications(req.user_id, false).await?;
            if let Some(n) = all.into_iter().find(|n| n.id == id) {
                return Ok(n);
            }
            // If dismissed, still return it by fetching directly.
            let row = sqlx::query("SELECT * FROM notifications WHERE id = ?")
                .bind(id)
                .fetch_one(self.pool())
                .await
                .map_err(map_db_err)?;
            return row_to_notification(row);
        }

        let id = sqlx::query(
            "INSERT INTO notifications (user_id, type, ref_key, message, data, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(req.user_id)
        .bind(type_str)
        .bind(&req.ref_key)
        .bind(&req.message)
        .bind(&data_str)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?
        .last_insert_rowid();

        let row = sqlx::query("SELECT * FROM notifications WHERE id = ?")
            .bind(id)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;
        row_to_notification(row)
    }

    async fn mark_notification_read(
        &self,
        user_id: UserId,
        id: NotificationId,
    ) -> Result<(), DbError> {
        let result = sqlx::query("UPDATE notifications SET read = 1 WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "notification",
            });
        }
        Ok(())
    }

    async fn dismiss_notification(
        &self,
        user_id: UserId,
        id: NotificationId,
    ) -> Result<(), DbError> {
        let result =
            sqlx::query("UPDATE notifications SET dismissed = 1 WHERE id = ? AND user_id = ?")
                .bind(id)
                .bind(user_id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "notification",
            });
        }
        Ok(())
    }

    async fn dismiss_all_notifications(&self, user_id: UserId) -> Result<(), DbError> {
        sqlx::query("UPDATE notifications SET dismissed = 1 WHERE user_id = ? AND dismissed = 0")
            .bind(user_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }
}
