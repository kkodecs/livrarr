use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::map_db_err;
use crate::{DbError, SeriesRoster, SeriesRosterDb, SeriesRosterEntry};

impl SeriesRosterDb for SqliteDb {
    async fn get_series_roster(&self, series_id: i64) -> Result<Option<SeriesRoster>, DbError> {
        let row = sqlx::query("SELECT entries, fetched_at FROM series_roster WHERE series_id = ?")
            .bind(series_id)
            .fetch_optional(self.pool())
            .await
            .map_err(map_db_err)?;

        match row {
            Some(row) => {
                let entries_json: String = row.get("entries");
                let entries: Vec<SeriesRosterEntry> = match serde_json::from_str(&entries_json) {
                    Ok(e) => e,
                    // Invalid JSON → cache miss (same recovery as author_series_cache).
                    Err(_) => return Ok(None),
                };
                let fetched_at: String = row.get("fetched_at");
                Ok(Some(SeriesRoster {
                    series_id,
                    entries,
                    fetched_at,
                }))
            }
            None => Ok(None),
        }
    }

    async fn save_series_roster(
        &self,
        series_id: i64,
        entries: &[SeriesRosterEntry],
    ) -> Result<SeriesRoster, DbError> {
        let entries_json = serde_json::to_string(entries).map_err(|e| DbError::Io(Box::new(e)))?;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO series_roster (series_id, entries, fetched_at) \
             VALUES (?, ?, ?) \
             ON CONFLICT(series_id) DO UPDATE SET entries = excluded.entries, fetched_at = excluded.fetched_at",
        )
        .bind(series_id)
        .bind(&entries_json)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        Ok(SeriesRoster {
            series_id,
            entries: entries.to_vec(),
            fetched_at: now,
        })
    }
}
