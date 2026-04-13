use chrono::Utc;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, parse_dt};
use crate::{
    CreateImportDbRequest, DbError, Import, ImportDb, LibraryItem, LibraryItemId, MediaType, UserId,
};

fn row_to_import(row: sqlx::sqlite::SqliteRow) -> Result<Import, DbError> {
    let started_at_str: String = row
        .try_get("started_at")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let completed_at_str: Option<String> = row
        .try_get("completed_at")
        .map_err(|e| DbError::Io(Box::new(e)))?;

    Ok(Import {
        id: row.try_get("id").map_err(|e| DbError::Io(Box::new(e)))?,
        user_id: row
            .try_get::<i64, _>("user_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        source: row
            .try_get("source")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        status: row
            .try_get("status")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        started_at: parse_dt(&started_at_str)?,
        completed_at: completed_at_str.map(|s| parse_dt(&s)).transpose()?,
        authors_created: row
            .try_get::<i64, _>("authors_created")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        works_created: row
            .try_get::<i64, _>("works_created")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        files_imported: row
            .try_get::<i64, _>("files_imported")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        files_skipped: row
            .try_get::<i64, _>("files_skipped")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        source_url: row
            .try_get("source_url")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        target_root_folder_id: row
            .try_get("target_root_folder_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
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
        import_id: row
            .try_get("import_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        imported_at: parse_dt(&imported_at_str)?,
    })
}

impl ImportDb for SqliteDb {
    async fn create_import(&self, req: CreateImportDbRequest) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO imports (id, user_id, source, status, started_at, source_url, target_root_folder_id) \
             VALUES (?, ?, ?, 'running', ?, ?, ?)",
        )
        .bind(&req.id)
        .bind(req.user_id)
        .bind(&req.source)
        .bind(&now)
        .bind(&req.source_url)
        .bind(req.target_root_folder_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(())
    }

    async fn get_import(&self, id: &str) -> Result<Option<Import>, DbError> {
        let row = sqlx::query("SELECT * FROM imports WHERE id = ?")
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(map_db_err)?;
        match row {
            Some(r) => Ok(Some(row_to_import(r)?)),
            None => Ok(None),
        }
    }

    async fn list_imports(&self, user_id: UserId) -> Result<Vec<Import>, DbError> {
        let rows = sqlx::query("SELECT * FROM imports WHERE user_id = ? ORDER BY started_at DESC")
            .bind(user_id)
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;
        rows.into_iter().map(row_to_import).collect()
    }

    async fn update_import_status(&self, id: &str, status: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE imports SET status = ? WHERE id = ?")
            .bind(status)
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn update_import_counts(
        &self,
        id: &str,
        authors: i64,
        works: i64,
        files: i64,
        skipped: i64,
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE imports SET authors_created = ?, works_created = ?, files_imported = ?, files_skipped = ? WHERE id = ?",
        )
        .bind(authors)
        .bind(works)
        .bind(files)
        .bind(skipped)
        .bind(id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(())
    }

    async fn set_import_completed(&self, id: &str) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE imports SET status = 'completed', completed_at = ? WHERE id = ?")
            .bind(&now)
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn list_library_items_by_import(
        &self,
        import_id: &str,
    ) -> Result<Vec<LibraryItem>, DbError> {
        let rows = sqlx::query("SELECT * FROM library_items WHERE import_id = ? ORDER BY id")
            .bind(import_id)
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;
        rows.into_iter().map(row_to_library_item).collect()
    }

    async fn delete_library_item_by_id(&self, id: LibraryItemId) -> Result<(), DbError> {
        sqlx::query("DELETE FROM library_items WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn delete_orphan_works_by_import(&self, import_id: &str) -> Result<i64, DbError> {
        let result = sqlx::query(
            "DELETE FROM works WHERE import_id = ? AND id NOT IN \
             (SELECT DISTINCT work_id FROM library_items WHERE work_id IS NOT NULL)",
        )
        .bind(import_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(result.rows_affected() as i64)
    }

    async fn delete_orphan_authors_by_import(&self, import_id: &str) -> Result<i64, DbError> {
        let result = sqlx::query(
            "DELETE FROM authors WHERE import_id = ? AND id NOT IN \
             (SELECT DISTINCT author_id FROM works WHERE author_id IS NOT NULL)",
        )
        .bind(import_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(result.rows_affected() as i64)
    }
}
