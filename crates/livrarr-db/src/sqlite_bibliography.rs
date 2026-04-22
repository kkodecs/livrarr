use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, map_db_err_with};
use crate::{AuthorBibliography, AuthorBibliographyDb, BibliographyEntry, DbError};
use sqlx::Row;

impl AuthorBibliographyDb for SqliteDb {
    async fn get_bibliography(
        &self,
        author_id: i64,
    ) -> Result<Option<AuthorBibliography>, DbError> {
        let row = sqlx::query(
            "SELECT entries, raw_entries, fetched_at FROM author_bibliography WHERE author_id = ?",
        )
        .bind(author_id)
        .fetch_optional(self.pool())
        .await
        .map_err(map_db_err_with("bibliography"))?;

        match row {
            Some(row) => {
                let entries_json: String = row.get("entries");
                let entries: Vec<BibliographyEntry> =
                    serde_json::from_str(&entries_json).map_err(|e| DbError::Io(Box::new(e)))?;
                let raw_entries: Option<Vec<BibliographyEntry>> = row
                    .try_get::<Option<String>, _>("raw_entries")
                    .ok()
                    .flatten()
                    .and_then(|s| serde_json::from_str(&s).ok());
                let fetched_at: String = row.get("fetched_at");
                Ok(Some(AuthorBibliography {
                    author_id,
                    entries,
                    raw_entries,
                    fetched_at,
                }))
            }
            None => Ok(None),
        }
    }

    async fn save_bibliography(
        &self,
        author_id: i64,
        entries: &[BibliographyEntry],
        raw_entries: Option<&[BibliographyEntry]>,
    ) -> Result<AuthorBibliography, DbError> {
        let entries_json = serde_json::to_string(entries).map_err(|e| DbError::Io(Box::new(e)))?;
        let raw_json = raw_entries
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| DbError::Io(Box::new(e)))?;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO author_bibliography (author_id, entries, raw_entries, fetched_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(author_id) DO UPDATE SET entries = excluded.entries, raw_entries = excluded.raw_entries, fetched_at = excluded.fetched_at",
        )
        .bind(author_id)
        .bind(&entries_json)
        .bind(&raw_json)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(map_db_err_with("bibliography"))?;

        Ok(AuthorBibliography {
            author_id,
            entries: entries.to_vec(),
            raw_entries: raw_entries.map(|r| r.to_vec()),
            fetched_at: now,
        })
    }

    async fn delete_bibliography(&self, author_id: i64) -> Result<(), DbError> {
        sqlx::query("DELETE FROM author_bibliography WHERE author_id = ?")
            .bind(author_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }
}
