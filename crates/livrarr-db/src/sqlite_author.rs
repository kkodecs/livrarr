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
        gr_key: row
            .try_get("gr_key")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        hc_key: row
            .try_get("hc_key")
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
            "INSERT INTO authors (user_id, name, sort_name, ol_key, gr_key, hc_key, added_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(req.user_id)
        .bind(&req.name)
        .bind(&req.sort_name)
        .bind(&req.ol_key)
        .bind(&req.gr_key)
        .bind(&req.hc_key)
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
        let current = self.get_author(user_id, id).await?;

        let name = req.name.unwrap_or(current.name);
        let sort_name = req.sort_name.or(current.sort_name);
        let ol_key = req.ol_key.or(current.ol_key);
        let monitored = req.monitored.unwrap_or(current.monitored);
        let monitor_new_items = req.monitor_new_items.unwrap_or(current.monitor_new_items);
        let monitor_since = req.monitor_since.or(current.monitor_since);

        sqlx::query(
            "UPDATE authors SET name = ?, sort_name = ?, ol_key = ?, \
             monitored = ?, monitor_new_items = ?, monitor_since = ? \
             WHERE id = ? AND user_id = ?",
        )
        .bind(&name)
        .bind(&sort_name)
        .bind(&ol_key)
        .bind(monitored)
        .bind(monitor_new_items)
        .bind(monitor_since.map(|dt| dt.to_rfc3339()))
        .bind(id)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

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
