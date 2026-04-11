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
        notification_type: parse_notification_type(&type_str)?,
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

fn parse_notification_type(s: &str) -> Result<NotificationType, DbError> {
    match s {
        "newWorkDetected" => Ok(NotificationType::NewWorkDetected),
        "workAutoAdded" => Ok(NotificationType::WorkAutoAdded),
        "metadataUpdated" => Ok(NotificationType::MetadataUpdated),
        "bulkEnrichmentComplete" => Ok(NotificationType::BulkEnrichmentComplete),
        "jobPanicked" => Ok(NotificationType::JobPanicked),
        "rateLimitHit" => Ok(NotificationType::RateLimitHit),
        _ => Err(DbError::IncompatibleData {
            detail: format!("unknown notification type: {s}"),
        }),
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

    async fn list_notifications_paginated(
        &self,
        user_id: UserId,
        unread_only: bool,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<Notification>, i64), DbError> {
        let where_clause = if unread_only {
            "WHERE user_id = ? AND dismissed = 0 AND read = 0"
        } else {
            "WHERE user_id = ? AND dismissed = 0"
        };

        let total: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM notifications {where_clause}"
        ))
        .bind(user_id)
        .fetch_one(self.pool())
        .await
        .map_err(map_db_err)?;

        let offset = (page.saturating_sub(1) * per_page) as i64;
        let query =
            format!("SELECT * FROM notifications {where_clause} ORDER BY id DESC LIMIT ? OFFSET ?");
        let rows = sqlx::query(&query)
            .bind(user_id)
            .bind(per_page as i64)
            .bind(offset)
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;

        let items = rows
            .into_iter()
            .map(row_to_notification)
            .collect::<Result<Vec<_>, _>>()?;
        Ok((items, total))
    }

    async fn create_notification(
        &self,
        req: CreateNotificationDbRequest,
    ) -> Result<Notification, DbError> {
        let now = Utc::now().to_rfc3339();
        let type_str = notification_type_str(req.notification_type);
        let data_str = serde_json::to_string(&req.data).map_err(|e| DbError::Io(Box::new(e)))?;

        // Dedup: use explicit `= ?` for non-NULL ref_key and `IS NULL` for NULL.
        // `ref_key IS ?` is wrong for non-NULL because SQLite's IS operator is
        // meant for NULL-safe comparison and behaves unexpectedly with bound params.
        let existing = if req.ref_key.is_some() {
            sqlx::query(
                "SELECT id FROM notifications WHERE user_id = ? AND type = ? AND ref_key = ?",
            )
            .bind(req.user_id)
            .bind(type_str)
            .bind(&req.ref_key)
            .fetch_optional(self.pool())
            .await
            .map_err(map_db_err)?
        } else {
            sqlx::query(
                "SELECT id FROM notifications WHERE user_id = ? AND type = ? AND ref_key IS NULL",
            )
            .bind(req.user_id)
            .bind(type_str)
            .fetch_optional(self.pool())
            .await
            .map_err(map_db_err)?
        };

        if let Some(row) = existing {
            let id: i64 = row.try_get("id").map_err(|e| DbError::Io(Box::new(e)))?;
            let row = sqlx::query("SELECT * FROM notifications WHERE id = ?")
                .bind(id)
                .fetch_one(self.pool())
                .await
                .map_err(map_db_err)?;
            return row_to_notification(row);
        }

        // For non-NULL ref_key, INSERT OR IGNORE leverages the unique index
        // (user_id, type, ref_key) to atomically prevent duplicates from a
        // concurrent INSERT that slipped past the SELECT above.
        // For NULL ref_key, SQLite unique indexes treat NULLs as distinct,
        // so duplicates are allowed — the SELECT dedup above is best-effort.
        let result = sqlx::query(
            "INSERT OR IGNORE INTO notifications (user_id, type, ref_key, message, data, created_at) \
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
        .map_err(map_db_err)?;

        // If INSERT OR IGNORE found a conflict (concurrent insert won the race),
        // fetch the existing row instead.
        let id = if result.rows_affected() == 0 {
            let row = if req.ref_key.is_some() {
                sqlx::query(
                    "SELECT id FROM notifications WHERE user_id = ? AND type = ? AND ref_key = ?",
                )
                .bind(req.user_id)
                .bind(type_str)
                .bind(&req.ref_key)
                .fetch_one(self.pool())
                .await
                .map_err(map_db_err)?
            } else {
                // NULL ref_key: shouldn't reach here since unique index doesn't
                // conflict on NULLs, but handle gracefully.
                sqlx::query(
                    "SELECT id FROM notifications WHERE user_id = ? AND type = ? AND ref_key IS NULL ORDER BY id DESC LIMIT 1",
                )
                .bind(req.user_id)
                .bind(type_str)
                .fetch_one(self.pool())
                .await
                .map_err(map_db_err)?
            };
            row.try_get::<i64, _>("id")
                .map_err(|e| DbError::Io(Box::new(e)))?
        } else {
            result.last_insert_rowid()
        };

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
