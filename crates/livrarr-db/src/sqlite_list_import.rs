use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::map_db_err;
use crate::{
    DbError, ListImportDb, ListImportPreviewRow, ListImportRecord, ListImportSummaryRow, UserId,
};

impl ListImportDb for SqliteDb {
    async fn insert_list_import_preview_row(
        &self,
        preview_id: &str,
        user_id: UserId,
        row_index: i64,
        title: &str,
        author: &str,
        isbn_13: Option<&str>,
        isbn_10: Option<&str>,
        year: Option<i32>,
        source_status: Option<&str>,
        source_rating: Option<f32>,
        preview_status: &str,
        source: &str,
        created_at: &str,
    ) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO list_import_previews \
             (preview_id, user_id, row_index, title, author, isbn_13, isbn_10, year, \
              source_status, source_rating, preview_status, source, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(preview_id)
        .bind(user_id)
        .bind(row_index)
        .bind(title)
        .bind(author)
        .bind(isbn_13)
        .bind(isbn_10)
        .bind(year)
        .bind(source_status)
        .bind(source_rating)
        .bind(preview_status)
        .bind(source)
        .bind(created_at)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(())
    }

    async fn count_list_import_previews(
        &self,
        preview_id: &str,
        user_id: UserId,
    ) -> Result<i64, DbError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM list_import_previews WHERE preview_id = ? AND user_id = ?",
        )
        .bind(preview_id)
        .bind(user_id)
        .fetch_one(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(count)
    }

    async fn get_list_import_source(
        &self,
        preview_id: &str,
        user_id: UserId,
    ) -> Result<String, DbError> {
        let source: String = sqlx::query_scalar(
            "SELECT source FROM list_import_previews \
             WHERE preview_id = ? AND user_id = ? LIMIT 1",
        )
        .bind(preview_id)
        .bind(user_id)
        .fetch_one(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(source)
    }

    async fn create_list_import_record(
        &self,
        id: &str,
        user_id: UserId,
        source: &str,
        started_at: &str,
    ) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO imports (id, user_id, source, status, started_at) \
             VALUES (?, ?, ?, 'running', ?)",
        )
        .bind(id)
        .bind(user_id)
        .bind(source)
        .bind(started_at)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(())
    }

    async fn get_list_import_record(&self, id: &str) -> Result<Option<ListImportRecord>, DbError> {
        let row = sqlx::query("SELECT user_id, status FROM imports WHERE id = ?")
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(map_db_err)?;
        match row {
            Some(r) => Ok(Some(ListImportRecord {
                user_id: r.try_get("user_id").map_err(|e| DbError::Io(Box::new(e)))?,
                status: r.try_get("status").map_err(|e| DbError::Io(Box::new(e)))?,
            })),
            None => Ok(None),
        }
    }

    async fn get_list_import_preview_row(
        &self,
        preview_id: &str,
        user_id: UserId,
        row_index: i64,
    ) -> Result<Option<ListImportPreviewRow>, DbError> {
        let row = sqlx::query(
            "SELECT * FROM list_import_previews \
             WHERE preview_id = ? AND user_id = ? AND row_index = ?",
        )
        .bind(preview_id)
        .bind(user_id)
        .bind(row_index)
        .fetch_optional(self.pool())
        .await
        .map_err(map_db_err)?;
        match row {
            Some(r) => Ok(Some(ListImportPreviewRow {
                title: r.try_get("title").unwrap_or_default(),
                author: r.try_get("author").unwrap_or_default(),
                isbn_13: r.try_get("isbn_13").ok(),
                isbn_10: r.try_get("isbn_10").ok(),
                year: r.try_get("year").ok().flatten(),
            })),
            None => Ok(None),
        }
    }

    async fn tag_last_work_with_import(
        &self,
        import_id: &str,
        user_id: UserId,
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE works SET import_id = ? WHERE user_id = ? AND id = \
             (SELECT id FROM works WHERE user_id = ? ORDER BY id DESC LIMIT 1)",
        )
        .bind(import_id)
        .bind(user_id)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(())
    }

    async fn increment_list_import_works_created(
        &self,
        import_id: &str,
        delta: i64,
    ) -> Result<(), DbError> {
        sqlx::query("UPDATE imports SET works_created = works_created + ? WHERE id = ?")
            .bind(delta)
            .bind(import_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn complete_list_import(
        &self,
        import_id: &str,
        user_id: UserId,
        completed_at: &str,
    ) -> Result<u64, DbError> {
        let result = sqlx::query(
            "UPDATE imports SET status = 'completed', completed_at = ? \
             WHERE id = ? AND user_id = ? AND status = 'running'",
        )
        .bind(completed_at)
        .bind(import_id)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(result.rows_affected())
    }

    async fn get_list_import_status_for_user(
        &self,
        import_id: &str,
        user_id: UserId,
    ) -> Result<Option<String>, DbError> {
        let row = sqlx::query("SELECT status FROM imports WHERE id = ? AND user_id = ?")
            .bind(import_id)
            .bind(user_id)
            .fetch_optional(self.pool())
            .await
            .map_err(map_db_err)?;
        match row {
            Some(r) => {
                let status: String = r.try_get("status").map_err(|e| DbError::Io(Box::new(e)))?;
                Ok(Some(status))
            }
            None => Ok(None),
        }
    }

    async fn delete_works_by_list_import(
        &self,
        import_id: &str,
        user_id: UserId,
    ) -> Result<i64, DbError> {
        let result = sqlx::query("DELETE FROM works WHERE import_id = ? AND user_id = ?")
            .bind(import_id)
            .bind(user_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(result.rows_affected() as i64)
    }

    async fn mark_list_import_undone(&self, import_id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE imports SET status = 'undone' WHERE id = ?")
            .bind(import_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn list_list_imports(
        &self,
        user_id: UserId,
    ) -> Result<Vec<ListImportSummaryRow>, DbError> {
        let rows = sqlx::query(
            "SELECT id, source, status, started_at, completed_at, works_created \
             FROM imports WHERE user_id = ? AND source IN ('goodreads', 'hardcover') \
             ORDER BY started_at DESC LIMIT 50",
        )
        .bind(user_id)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;
        rows.into_iter()
            .map(|r| {
                Ok(ListImportSummaryRow {
                    id: r.try_get("id").map_err(|e| DbError::Io(Box::new(e)))?,
                    source: r.try_get("source").map_err(|e| DbError::Io(Box::new(e)))?,
                    status: r.try_get("status").map_err(|e| DbError::Io(Box::new(e)))?,
                    started_at: r
                        .try_get("started_at")
                        .map_err(|e| DbError::Io(Box::new(e)))?,
                    completed_at: r.try_get("completed_at").ok().flatten(),
                    works_created: r
                        .try_get::<i64, _>("works_created")
                        .map_err(|e| DbError::Io(Box::new(e)))?,
                })
            })
            .collect()
    }

    async fn work_exists_by_isbn_13(&self, user_id: UserId, isbn: &str) -> Result<bool, DbError> {
        // Check works table.
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id = ? AND isbn_13 = ?")
                .bind(user_id)
                .bind(isbn)
                .fetch_one(self.pool())
                .await
                .map_err(map_db_err)?;
        if count > 0 {
            return Ok(true);
        }

        // Also check external_ids table.
        let ext_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM external_ids ei \
             JOIN works w ON ei.work_id = w.id \
             WHERE w.user_id = ? AND ei.id_type = 'isbn_13' AND ei.id_value = ?",
        )
        .bind(user_id)
        .bind(isbn)
        .fetch_one(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(ext_count > 0)
    }

    async fn work_exists_by_isbn_10(&self, user_id: UserId, isbn: &str) -> Result<bool, DbError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM external_ids ei \
             JOIN works w ON ei.work_id = w.id \
             WHERE w.user_id = ? AND ei.id_type = 'isbn_10' AND ei.id_value = ?",
        )
        .bind(user_id)
        .bind(isbn)
        .fetch_one(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(count > 0)
    }

    async fn delete_stale_list_import_previews(&self, cutoff: &str) -> Result<u64, DbError> {
        let result = sqlx::query("DELETE FROM list_import_previews WHERE created_at < ?")
            .bind(cutoff)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(result.rows_affected())
    }
}
