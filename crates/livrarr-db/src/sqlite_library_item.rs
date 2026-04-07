use chrono::Utc;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, parse_dt};
use crate::{
    CreateLibraryItemDbRequest, DbError, LibraryItem, LibraryItemDb, LibraryItemId, MediaType,
    RootFolderId, UserId, WorkId,
};

fn row_to_library_item(row: sqlx::sqlite::SqliteRow) -> Result<LibraryItem, DbError> {
    let media_type_str: String = row
        .try_get("media_type")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let imported_at_str: String = row
        .try_get("imported_at")
        .map_err(|e| DbError::Io(Box::new(e)))?;

    Ok(LibraryItem {
        id: row
            .try_get::<i64, _>("id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        user_id: row
            .try_get::<i64, _>("user_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        work_id: row
            .try_get::<i64, _>("work_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        root_folder_id: row
            .try_get::<i64, _>("root_folder_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        path: row.try_get("path").map_err(|e| DbError::Io(Box::new(e)))?,
        media_type: parse_media_type(&media_type_str)?,
        file_size: row
            .try_get::<i64, _>("file_size")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        imported_at: parse_dt(&imported_at_str)?,
    })
}

fn parse_media_type(s: &str) -> Result<MediaType, DbError> {
    match s {
        "ebook" => Ok(MediaType::Ebook),
        "audiobook" => Ok(MediaType::Audiobook),
        _ => Err(DbError::IncompatibleData {
            detail: format!("unknown media type: {s}"),
        }),
    }
}

fn media_type_str(mt: MediaType) -> &'static str {
    match mt {
        MediaType::Ebook => "ebook",
        MediaType::Audiobook => "audiobook",
    }
}

impl LibraryItemDb for SqliteDb {
    async fn get_library_item(
        &self,
        user_id: UserId,
        id: LibraryItemId,
    ) -> Result<LibraryItem, DbError> {
        let row = sqlx::query("SELECT * FROM library_items WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;
        row_to_library_item(row)
    }

    async fn list_library_items(&self, user_id: UserId) -> Result<Vec<LibraryItem>, DbError> {
        let rows = sqlx::query("SELECT * FROM library_items WHERE user_id = ? ORDER BY id")
            .bind(user_id)
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;
        rows.into_iter().map(row_to_library_item).collect()
    }

    async fn list_library_items_by_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<LibraryItem>, DbError> {
        let rows = sqlx::query(
            "SELECT * FROM library_items WHERE user_id = ? AND work_id = ? ORDER BY id",
        )
        .bind(user_id)
        .bind(work_id)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;
        rows.into_iter().map(row_to_library_item).collect()
    }

    async fn create_library_item(
        &self,
        req: CreateLibraryItemDbRequest,
    ) -> Result<LibraryItem, DbError> {
        let now = Utc::now().to_rfc3339();
        let mt = media_type_str(req.media_type);

        let id = sqlx::query(
            "INSERT INTO library_items (user_id, work_id, root_folder_id, path, media_type, file_size, imported_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(user_id, path) DO UPDATE SET \
               work_id = excluded.work_id, \
               root_folder_id = excluded.root_folder_id, \
               media_type = excluded.media_type, \
               file_size = excluded.file_size, \
               imported_at = excluded.imported_at",
        )
        .bind(req.user_id)
        .bind(req.work_id)
        .bind(req.root_folder_id)
        .bind(&req.path)
        .bind(mt)
        .bind(req.file_size)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?
        .last_insert_rowid();

        self.get_library_item(req.user_id, id).await
    }

    async fn delete_library_item(
        &self,
        user_id: UserId,
        id: LibraryItemId,
    ) -> Result<LibraryItem, DbError> {
        let item = self.get_library_item(user_id, id).await?;
        sqlx::query("DELETE FROM library_items WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(item)
    }

    async fn library_items_exist_for_root(
        &self,
        root_folder_id: RootFolderId,
    ) -> Result<bool, DbError> {
        let row = sqlx::query("SELECT COUNT(*) as cnt FROM library_items WHERE root_folder_id = ?")
            .bind(root_folder_id)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;
        let cnt: i64 = row.try_get("cnt").map_err(|e| DbError::Io(Box::new(e)))?;
        Ok(cnt > 0)
    }

    async fn list_taggable_items_by_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<LibraryItem>, DbError> {
        // All library items for a work are potentially taggable.
        self.list_library_items_by_work(user_id, work_id).await
    }

    async fn update_library_item_size(
        &self,
        user_id: UserId,
        id: LibraryItemId,
        file_size: i64,
    ) -> Result<(), DbError> {
        let result =
            sqlx::query("UPDATE library_items SET file_size = ? WHERE id = ? AND user_id = ?")
                .bind(file_size)
                .bind(id)
                .bind(user_id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "library_item",
            });
        }
        Ok(())
    }
}
