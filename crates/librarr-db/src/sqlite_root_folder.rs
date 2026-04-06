use async_trait::async_trait;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::map_db_err;
use crate::{DbError, MediaType, RootFolder, RootFolderDb, RootFolderId};

fn row_to_root_folder(row: sqlx::sqlite::SqliteRow) -> Result<RootFolder, DbError> {
    Ok(RootFolder {
        id: row
            .try_get::<i64, _>("id")
            .map_err(|e| DbError::Io(e.to_string()))?,
        path: row
            .try_get("path")
            .map_err(|e| DbError::Io(e.to_string()))?,
        media_type: parse_media_type(
            &row.try_get::<String, _>("media_type")
                .map_err(|e| DbError::Io(e.to_string()))?,
        ),
    })
}

fn parse_media_type(s: &str) -> MediaType {
    match s {
        "audiobook" => MediaType::Audiobook,
        _ => MediaType::Ebook,
    }
}

fn media_type_str(mt: MediaType) -> &'static str {
    match mt {
        MediaType::Ebook => "ebook",
        MediaType::Audiobook => "audiobook",
    }
}

#[async_trait]
impl RootFolderDb for SqliteDb {
    async fn get_root_folder(&self, id: RootFolderId) -> Result<RootFolder, DbError> {
        let row = sqlx::query("SELECT * FROM root_folders WHERE id = ?")
            .bind(id)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;
        row_to_root_folder(row)
    }

    async fn list_root_folders(&self) -> Result<Vec<RootFolder>, DbError> {
        let rows = sqlx::query("SELECT * FROM root_folders ORDER BY id")
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;
        rows.into_iter().map(row_to_root_folder).collect()
    }

    async fn create_root_folder(
        &self,
        path: &str,
        media_type: MediaType,
    ) -> Result<RootFolder, DbError> {
        let mt = media_type_str(media_type);
        let id = sqlx::query("INSERT INTO root_folders (path, media_type) VALUES (?, ?)")
            .bind(path)
            .bind(mt)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?
            .last_insert_rowid();

        self.get_root_folder(id).await
    }

    async fn delete_root_folder(&self, id: RootFolderId) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM root_folders WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound);
        }
        Ok(())
    }

    async fn get_root_folder_by_media_type(
        &self,
        media_type: MediaType,
    ) -> Result<Option<RootFolder>, DbError> {
        let mt = media_type_str(media_type);
        let row = sqlx::query("SELECT * FROM root_folders WHERE media_type = ?")
            .bind(mt)
            .fetch_optional(self.pool())
            .await
            .map_err(map_db_err)?;

        match row {
            Some(r) => Ok(Some(row_to_root_folder(r)?)),
            None => Ok(None),
        }
    }
}
