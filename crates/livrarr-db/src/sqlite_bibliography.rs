use crate::sqlite::SqliteDb;
use crate::{AuthorBibliography, AuthorBibliographyDb, BibliographyEntry, DbError};
use sqlx::Row;

fn map_db_err(e: sqlx::Error) -> DbError {
    match e {
        sqlx::Error::RowNotFound => DbError::NotFound,
        _ => DbError::Io(e.to_string()),
    }
}

#[async_trait::async_trait]
impl AuthorBibliographyDb for SqliteDb {
    async fn get_bibliography(
        &self,
        author_id: i64,
    ) -> Result<Option<AuthorBibliography>, DbError> {
        let row =
            sqlx::query("SELECT entries, fetched_at FROM author_bibliography WHERE author_id = ?")
                .bind(author_id)
                .fetch_optional(self.pool())
                .await
                .map_err(map_db_err)?;

        match row {
            Some(row) => {
                let entries_json: String = row.get("entries");
                let entries: Vec<BibliographyEntry> =
                    serde_json::from_str(&entries_json).unwrap_or_default();
                let fetched_at: String = row.get("fetched_at");
                Ok(Some(AuthorBibliography {
                    author_id,
                    entries,
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
    ) -> Result<AuthorBibliography, DbError> {
        let entries_json =
            serde_json::to_string(entries).map_err(|e| DbError::Io(e.to_string()))?;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO author_bibliography (author_id, entries, fetched_at) \
             VALUES (?, ?, ?) \
             ON CONFLICT(author_id) DO UPDATE SET entries = excluded.entries, fetched_at = excluded.fetched_at",
        )
        .bind(author_id)
        .bind(&entries_json)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        Ok(AuthorBibliography {
            author_id,
            entries: entries.to_vec(),
            fetched_at: now,
        })
    }
}
