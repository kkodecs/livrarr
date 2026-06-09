use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::map_db_err;
use crate::{CrossFormatState, CrossFormatStateDb, DbError, MediaType, UserId};

fn row_to_cross_format_state(row: sqlx::sqlite::SqliteRow) -> Result<CrossFormatState, DbError> {
    Ok(CrossFormatState {
        user_id: row
            .try_get::<i64, _>("user_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        kash_link_id: row
            .try_get::<i64, _>("kash_link_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        furthest_ts: row
            .try_get::<f64, _>("furthest_ts")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        ebook_declined_at_ts: row
            .try_get::<Option<f64>, _>("ebook_declined_at_ts")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        audio_declined_at_ts: row
            .try_get::<Option<f64>, _>("audio_declined_at_ts")
            .map_err(|e| DbError::Io(Box::new(e)))?,
    })
}

impl CrossFormatStateDb for SqliteDb {
    async fn get_or_default(
        &self,
        user_id: UserId,
        kash_link_id: i64,
    ) -> Result<CrossFormatState, DbError> {
        let row = sqlx::query(
            "SELECT user_id, kash_link_id, furthest_ts,
                    ebook_declined_at_ts, audio_declined_at_ts
             FROM cross_format_state
             WHERE user_id = ? AND kash_link_id = ?",
        )
        .bind(user_id)
        .bind(kash_link_id)
        .fetch_optional(self.pool())
        .await
        .map_err(map_db_err)?;

        match row {
            Some(r) => Ok(row_to_cross_format_state(r)?),
            // Return a zero-value default without inserting (read-only path).
            None => Ok(CrossFormatState {
                user_id,
                kash_link_id,
                furthest_ts: 0.0,
                ebook_declined_at_ts: None,
                audio_declined_at_ts: None,
            }),
        }
    }

    async fn set_decline(
        &self,
        user_id: UserId,
        kash_link_id: i64,
        format: MediaType,
        declined_at_ts: f64,
    ) -> Result<(), DbError> {
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

        // Two static SQL strings — one per format — so the column name is never
        // derived from runtime data (IR v2 rule: no column-name interpolation).
        // Only the named format's threshold changes; furthest_ts is untouched on
        // conflict (REQ-017). INSERT supplies furthest_ts = 0 so a fresh row gets
        // the correct default if this is the first state record for the pair.
        match format {
            MediaType::Ebook => {
                sqlx::query(
                    "INSERT INTO cross_format_state
                         (user_id, kash_link_id, furthest_ts, ebook_declined_at_ts, updated_at)
                     VALUES (?, ?, 0, ?, ?)
                     ON CONFLICT(user_id, kash_link_id)
                     DO UPDATE SET ebook_declined_at_ts = excluded.ebook_declined_at_ts,
                                   updated_at           = excluded.updated_at",
                )
                .bind(user_id)
                .bind(kash_link_id)
                .bind(declined_at_ts)
                .bind(&now)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
            }
            MediaType::Audiobook => {
                sqlx::query(
                    "INSERT INTO cross_format_state
                         (user_id, kash_link_id, furthest_ts, audio_declined_at_ts, updated_at)
                     VALUES (?, ?, 0, ?, ?)
                     ON CONFLICT(user_id, kash_link_id)
                     DO UPDATE SET audio_declined_at_ts = excluded.audio_declined_at_ts,
                                   updated_at           = excluded.updated_at",
                )
                .bind(user_id)
                .bind(kash_link_id)
                .bind(declined_at_ts)
                .bind(&now)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
            }
        }

        Ok(())
    }

    async fn sync_to(&self, user_id: UserId, kash_link_id: i64, ts: f64) -> Result<(), DbError> {
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

        // Explicit override: furthest_ts may DECREASE (REQ-018). Both decline
        // thresholds are cleared so prompts re-arm from the new baseline
        // (IR v2 sync-clears-declines decision).
        sqlx::query(
            "INSERT INTO cross_format_state
                 (user_id, kash_link_id, furthest_ts, updated_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(user_id, kash_link_id)
             DO UPDATE SET furthest_ts          = excluded.furthest_ts,
                           ebook_declined_at_ts = NULL,
                           audio_declined_at_ts = NULL,
                           updated_at           = excluded.updated_at",
        )
        .bind(user_id)
        .bind(kash_link_id)
        .bind(ts)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        Ok(())
    }
}
