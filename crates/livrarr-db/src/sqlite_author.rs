use chrono::Utc;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, parse_dt};
use crate::{
    Author, AuthorDb, AuthorId, CreateAuthorDbRequest, DbError, UpdateAuthorDbRequest, UserId,
};

fn row_to_author(row: sqlx::sqlite::SqliteRow) -> Result<Author, DbError> {
    let monitor_since_str: Option<String> = row
        .try_get("monitor_since")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let added_at_str: String = row
        .try_get("added_at")
        .map_err(|e| DbError::Io(Box::new(e)))?;

    Ok(Author {
        id: row
            .try_get::<i64, _>("id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        user_id: row
            .try_get::<i64, _>("user_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        name: row.try_get("name").map_err(|e| DbError::Io(Box::new(e)))?,
        sort_name: row
            .try_get("sort_name")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        ol_key: row
            .try_get("ol_key")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        monitored: row
            .try_get::<bool, _>("monitored")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        monitor_new_items: row
            .try_get::<bool, _>("monitor_new_items")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        monitor_since: monitor_since_str.map(|s| parse_dt(&s)).transpose()?,
        added_at: parse_dt(&added_at_str)?,
    })
}

impl AuthorDb for SqliteDb {
    async fn get_author(&self, user_id: UserId, id: AuthorId) -> Result<Author, DbError> {
        let row = sqlx::query("SELECT * FROM authors WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;
        row_to_author(row)
    }

    async fn list_authors(&self, user_id: UserId) -> Result<Vec<Author>, DbError> {
        let rows = sqlx::query("SELECT * FROM authors WHERE user_id = ? ORDER BY id")
            .bind(user_id)
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;
        rows.into_iter().map(row_to_author).collect()
    }

    async fn create_author(&self, req: CreateAuthorDbRequest) -> Result<Author, DbError> {
        let now = Utc::now().to_rfc3339();
        let id = sqlx::query(
            "INSERT INTO authors (user_id, name, sort_name, ol_key, added_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(req.user_id)
        .bind(&req.name)
        .bind(&req.sort_name)
        .bind(&req.ol_key)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?
        .last_insert_rowid();

        self.get_author(req.user_id, id).await
    }

    async fn update_author(
        &self,
        user_id: UserId,
        id: AuthorId,
        req: UpdateAuthorDbRequest,
    ) -> Result<Author, DbError> {
        self.get_author(user_id, id).await?;

        if let Some(name) = &req.name {
            sqlx::query("UPDATE authors SET name = ? WHERE id = ? AND user_id = ?")
                .bind(name)
                .bind(id)
                .bind(user_id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(sort_name) = &req.sort_name {
            sqlx::query("UPDATE authors SET sort_name = ? WHERE id = ? AND user_id = ?")
                .bind(sort_name)
                .bind(id)
                .bind(user_id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(ol_key) = &req.ol_key {
            sqlx::query("UPDATE authors SET ol_key = ? WHERE id = ? AND user_id = ?")
                .bind(ol_key)
                .bind(id)
                .bind(user_id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(monitored) = req.monitored {
            sqlx::query("UPDATE authors SET monitored = ? WHERE id = ? AND user_id = ?")
                .bind(monitored)
                .bind(id)
                .bind(user_id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(monitor_new_items) = req.monitor_new_items {
            sqlx::query("UPDATE authors SET monitor_new_items = ? WHERE id = ? AND user_id = ?")
                .bind(monitor_new_items)
                .bind(id)
                .bind(user_id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(monitor_since) = &req.monitor_since {
            sqlx::query("UPDATE authors SET monitor_since = ? WHERE id = ? AND user_id = ?")
                .bind(monitor_since.to_rfc3339())
                .bind(id)
                .bind(user_id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }

        self.get_author(user_id, id).await
    }

    async fn delete_author(&self, user_id: UserId, id: AuthorId) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM authors WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "author" });
        }
        Ok(())
    }

    async fn find_author_by_name(
        &self,
        user_id: UserId,
        normalized_name: &str,
    ) -> Result<Option<Author>, DbError> {
        let row = sqlx::query(
            "SELECT * FROM authors WHERE user_id = ? AND LOWER(TRIM(name)) = LOWER(TRIM(?))",
        )
        .bind(user_id)
        .bind(normalized_name)
        .fetch_optional(self.pool())
        .await
        .map_err(map_db_err)?;

        match row {
            Some(r) => Ok(Some(row_to_author(r)?)),
            None => Ok(None),
        }
    }

    async fn list_monitored_authors(&self) -> Result<Vec<Author>, DbError> {
        let rows = sqlx::query(
            "SELECT * FROM authors WHERE monitored = 1 AND ol_key IS NOT NULL ORDER BY id",
        )
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;
        rows.into_iter().map(row_to_author).collect()
    }
}
