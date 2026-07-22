use chrono::Utc;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::map_db_err;
use crate::{DbError, ReadarrOrigin, ReadarrOriginDb};

fn row_to_readarr_origin(row: sqlx::sqlite::SqliteRow) -> Result<ReadarrOrigin, DbError> {
    Ok(ReadarrOrigin {
        id: row.try_get("id").map_err(|e| DbError::Io(Box::new(e)))?,
        origin: row
            .try_get("origin")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| DbError::Io(Box::new(e)))?,
    })
}

impl ReadarrOriginDb for SqliteDb {
    async fn list_readarr_origins(&self) -> Result<Vec<ReadarrOrigin>, DbError> {
        let rows = sqlx::query("SELECT * FROM readarr_origins ORDER BY id")
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;
        rows.into_iter().map(row_to_readarr_origin).collect()
    }

    async fn create_readarr_origin(&self, origin: &str) -> Result<ReadarrOrigin, DbError> {
        let now = Utc::now().to_rfc3339();
        let id = sqlx::query("INSERT INTO readarr_origins (origin, created_at) VALUES (?, ?)")
            .bind(origin)
            .bind(&now)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?
            .last_insert_rowid();

        let row = sqlx::query("SELECT * FROM readarr_origins WHERE id = ?")
            .bind(id)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;
        row_to_readarr_origin(row)
    }

    async fn delete_readarr_origin(&self, id: i64) -> Result<(), DbError> {
        sqlx::query("DELETE FROM readarr_origins WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn is_readarr_origin_approved(&self, origin: &str) -> Result<bool, DbError> {
        let row = sqlx::query("SELECT 1 FROM readarr_origins WHERE origin = ?")
            .bind(origin)
            .fetch_optional(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(row.is_some())
    }
}
