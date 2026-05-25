use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, parse_dt};
use crate::{DbError, LibraryItemId, PlaybackProgress, PlaybackProgressDb, UserId};

fn row_to_progress(row: sqlx::sqlite::SqliteRow) -> Result<PlaybackProgress, DbError> {
    Ok(PlaybackProgress {
        id: row
            .try_get::<i64, _>("id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        user_id: row
            .try_get::<i64, _>("user_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        library_item_id: row
            .try_get::<i64, _>("library_item_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        position: row
            .try_get("position")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        progress_pct: row
            .try_get("progress_pct")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        updated_at: parse_dt(
            &row.try_get::<String, _>("updated_at")
                .map_err(|e| DbError::Io(Box::new(e)))?,
        )?,
        finished_at: row
            .try_get::<Option<String>, _>("finished_at")
            .ok()
            .flatten()
            .map(|s| parse_dt(&s))
            .transpose()?,
    })
}

impl PlaybackProgressDb for SqliteDb {
    async fn get_progress(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
    ) -> Result<Option<PlaybackProgress>, DbError> {
        let row = sqlx::query(
            "SELECT id, user_id, library_item_id, position, progress_pct, updated_at, finished_at
             FROM playback_progress
             WHERE user_id = ? AND library_item_id = ?",
        )
        .bind(user_id)
        .bind(library_item_id)
        .fetch_optional(self.pool())
        .await
        .map_err(map_db_err)?;

        match row {
            Some(r) => Ok(Some(row_to_progress(r)?)),
            None => Ok(None),
        }
    }

    async fn upsert_progress(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
        position: &str,
        progress_pct: f64,
    ) -> Result<(), DbError> {
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let initial_finished_at: Option<&str> = if progress_pct >= 0.98 {
            Some(&now)
        } else {
            None
        };
        sqlx::query(
            "INSERT INTO playback_progress (user_id, library_item_id, position, progress_pct, updated_at, finished_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(user_id, library_item_id)
             DO UPDATE SET position = excluded.position,
                           progress_pct = excluded.progress_pct,
                           updated_at = excluded.updated_at,
                           finished_at = CASE
                             WHEN excluded.progress_pct >= 0.98 AND playback_progress.finished_at IS NULL THEN excluded.updated_at
                             WHEN excluded.progress_pct < 0.95 AND playback_progress.finished_at IS NOT NULL THEN NULL
                             ELSE playback_progress.finished_at
                           END",
        )
        .bind(user_id)
        .bind(library_item_id)
        .bind(position)
        .bind(progress_pct)
        .bind(&now)
        .bind(initial_finished_at)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        Ok(())
    }

    async fn upsert_progress_no_lifecycle(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
        position: &str,
        progress_pct: f64,
    ) -> Result<(), DbError> {
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        sqlx::query(
            "INSERT INTO playback_progress (user_id, library_item_id, position, progress_pct, updated_at, finished_at)
             VALUES (?, ?, ?, ?, ?, NULL)
             ON CONFLICT(user_id, library_item_id)
             DO UPDATE SET position = excluded.position,
                           progress_pct = excluded.progress_pct,
                           updated_at = excluded.updated_at",
        )
        .bind(user_id)
        .bind(library_item_id)
        .bind(position)
        .bind(progress_pct)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        Ok(())
    }

    async fn get_progress_for_items(
        &self,
        user_id: UserId,
        library_item_ids: &[LibraryItemId],
    ) -> Result<Vec<PlaybackProgress>, DbError> {
        if library_item_ids.is_empty() {
            return Ok(vec![]);
        }
        let placeholders = library_item_ids
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT id, user_id, library_item_id, position, progress_pct, updated_at, finished_at
             FROM playback_progress
             WHERE user_id = ? AND library_item_id IN ({})",
            placeholders
        );
        let mut query = sqlx::query(&sql).bind(user_id);
        for id in library_item_ids {
            query = query.bind(*id);
        }
        let rows = query.fetch_all(self.pool()).await.map_err(map_db_err)?;
        rows.into_iter().map(row_to_progress).collect()
    }
}
