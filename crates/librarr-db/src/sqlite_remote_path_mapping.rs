use async_trait::async_trait;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::map_db_err;
use crate::{DbError, RemotePathMapping, RemotePathMappingDb, RemotePathMappingId};

fn row_to_mapping(row: sqlx::sqlite::SqliteRow) -> Result<RemotePathMapping, DbError> {
    Ok(RemotePathMapping {
        id: row
            .try_get::<i64, _>("id")
            .map_err(|e| DbError::Io(e.to_string()))?,
        host: row
            .try_get("host")
            .map_err(|e| DbError::Io(e.to_string()))?,
        remote_path: row
            .try_get("remote_path")
            .map_err(|e| DbError::Io(e.to_string()))?,
        local_path: row
            .try_get("local_path")
            .map_err(|e| DbError::Io(e.to_string()))?,
    })
}

#[async_trait]
impl RemotePathMappingDb for SqliteDb {
    async fn get_remote_path_mapping(
        &self,
        id: RemotePathMappingId,
    ) -> Result<RemotePathMapping, DbError> {
        let row = sqlx::query("SELECT * FROM remote_path_mappings WHERE id = ?")
            .bind(id)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;
        row_to_mapping(row)
    }

    async fn list_remote_path_mappings(&self) -> Result<Vec<RemotePathMapping>, DbError> {
        let rows = sqlx::query("SELECT * FROM remote_path_mappings ORDER BY id")
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;
        rows.into_iter().map(row_to_mapping).collect()
    }

    async fn create_remote_path_mapping(
        &self,
        host: &str,
        remote_path: &str,
        local_path: &str,
    ) -> Result<RemotePathMapping, DbError> {
        let id = sqlx::query(
            "INSERT INTO remote_path_mappings (host, remote_path, local_path) VALUES (?, ?, ?)",
        )
        .bind(host)
        .bind(remote_path)
        .bind(local_path)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?
        .last_insert_rowid();

        self.get_remote_path_mapping(id).await
    }

    async fn update_remote_path_mapping(
        &self,
        id: RemotePathMappingId,
        host: &str,
        remote_path: &str,
        local_path: &str,
    ) -> Result<RemotePathMapping, DbError> {
        let result = sqlx::query(
            "UPDATE remote_path_mappings SET host = ?, remote_path = ?, local_path = ? WHERE id = ?",
        )
        .bind(host)
        .bind(remote_path)
        .bind(local_path)
        .bind(id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound);
        }

        self.get_remote_path_mapping(id).await
    }

    async fn delete_remote_path_mapping(&self, id: RemotePathMappingId) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM remote_path_mappings WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound);
        }
        Ok(())
    }
}
