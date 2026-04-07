use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, parse_dt};
use crate::{
    CreateIndexerDbRequest, DbError, Indexer, IndexerDb, IndexerId, UpdateIndexerDbRequest,
};

fn parse_categories(s: &str) -> Result<Vec<i32>, DbError> {
    serde_json::from_str(s).map_err(|e| DbError::IncompatibleData {
        detail: format!("invalid JSON in indexers.categories: {e}"),
    })
}

fn row_to_indexer(row: sqlx::sqlite::SqliteRow) -> Result<Indexer, DbError> {
    let categories_str: String = row
        .try_get("categories")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let added_at_str: String = row
        .try_get("added_at")
        .map_err(|e| DbError::Io(Box::new(e)))?;

    Ok(Indexer {
        id: row
            .try_get::<i64, _>("id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        name: row.try_get("name").map_err(|e| DbError::Io(Box::new(e)))?,
        protocol: row
            .try_get("protocol")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        url: row.try_get("url").map_err(|e| DbError::Io(Box::new(e)))?,
        api_path: row
            .try_get("api_path")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        api_key: row
            .try_get("api_key")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        categories: parse_categories(&categories_str)?,
        priority: row
            .try_get::<i32, _>("priority")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        enable_automatic_search: row
            .try_get::<bool, _>("enable_automatic_search")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        enable_interactive_search: row
            .try_get::<bool, _>("enable_interactive_search")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        supports_book_search: row
            .try_get::<bool, _>("supports_book_search")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        enabled: row
            .try_get::<bool, _>("enabled")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        added_at: parse_dt(&added_at_str)?,
    })
}

impl IndexerDb for SqliteDb {
    async fn get_indexer(&self, id: IndexerId) -> Result<Indexer, DbError> {
        let row = sqlx::query("SELECT * FROM indexers WHERE id = ?")
            .bind(id)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;
        row_to_indexer(row)
    }

    async fn list_indexers(&self) -> Result<Vec<Indexer>, DbError> {
        let rows = sqlx::query("SELECT * FROM indexers ORDER BY priority, id")
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;
        rows.into_iter().map(row_to_indexer).collect()
    }

    async fn list_enabled_interactive_indexers(&self) -> Result<Vec<Indexer>, DbError> {
        let rows = sqlx::query(
            "SELECT * FROM indexers WHERE enabled = 1 AND enable_interactive_search = 1 \
             ORDER BY priority, id",
        )
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;
        rows.into_iter().map(row_to_indexer).collect()
    }

    async fn create_indexer(&self, req: CreateIndexerDbRequest) -> Result<Indexer, DbError> {
        let categories_json =
            serde_json::to_string(&req.categories).map_err(|e| DbError::Io(Box::new(e)))?;

        let id = sqlx::query(
            "INSERT INTO indexers \
             (name, protocol, url, api_path, api_key, categories, priority, \
              enable_automatic_search, enable_interactive_search, enabled) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&req.name)
        .bind(&req.protocol)
        .bind(&req.url)
        .bind(&req.api_path)
        .bind(&req.api_key)
        .bind(&categories_json)
        .bind(req.priority)
        .bind(req.enable_automatic_search)
        .bind(req.enable_interactive_search)
        .bind(req.enabled)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?
        .last_insert_rowid();

        self.get_indexer(id).await
    }

    async fn update_indexer(
        &self,
        id: IndexerId,
        req: UpdateIndexerDbRequest,
    ) -> Result<Indexer, DbError> {
        // Fetch current record, merge changes, single atomic UPDATE.
        let current = self.get_indexer(id).await?;

        let name = req.name.unwrap_or(current.name);
        let url = req.url.clone().unwrap_or_else(|| current.url.clone());
        let api_path = req
            .api_path
            .clone()
            .unwrap_or_else(|| current.api_path.clone());
        // API key semantics: None = keep existing, Some("") = clear, Some(value) = set new.
        let api_key_opt = match req.api_key.as_deref() {
            None => current.api_key.clone(), // not provided, keep existing
            Some("") => None,                // explicitly cleared
            Some(value) => Some(value.to_string()), // set new value
        };
        let categories = req.categories.unwrap_or(current.categories);
        let priority = req.priority.unwrap_or(current.priority);
        let enable_automatic_search = req
            .enable_automatic_search
            .unwrap_or(current.enable_automatic_search);
        let enable_interactive_search = req
            .enable_interactive_search
            .unwrap_or(current.enable_interactive_search);
        let enabled = req.enabled.unwrap_or(current.enabled);

        // Reset supports_book_search only if connection-affecting fields actually changed.
        let url_changed = req.url.as_ref().is_some_and(|v| *v != current.url);
        let path_changed = req
            .api_path
            .as_ref()
            .is_some_and(|v| *v != current.api_path);
        let key_changed = api_key_opt != current.api_key;
        let supports_book_search = if url_changed || path_changed || key_changed {
            false
        } else {
            current.supports_book_search
        };

        let categories_json =
            serde_json::to_string(&categories).map_err(|e| DbError::Io(Box::new(e)))?;

        sqlx::query(
            "UPDATE indexers SET name = ?, url = ?, api_path = ?, api_key = ?, \
             categories = ?, priority = ?, enable_automatic_search = ?, \
             enable_interactive_search = ?, supports_book_search = ?, enabled = ? \
             WHERE id = ?",
        )
        .bind(&name)
        .bind(&url)
        .bind(&api_path)
        .bind(&api_key_opt)
        .bind(&categories_json)
        .bind(priority)
        .bind(enable_automatic_search)
        .bind(enable_interactive_search)
        .bind(supports_book_search)
        .bind(enabled)
        .bind(id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        self.get_indexer(id).await
    }

    async fn delete_indexer(&self, id: IndexerId) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM indexers WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "indexer" });
        }
        Ok(())
    }

    async fn set_supports_book_search(&self, id: IndexerId, supports: bool) -> Result<(), DbError> {
        let result = sqlx::query("UPDATE indexers SET supports_book_search = ? WHERE id = ?")
            .bind(supports)
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "indexer" });
        }
        Ok(())
    }
}
