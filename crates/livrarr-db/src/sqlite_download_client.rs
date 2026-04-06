use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::map_db_err;
use crate::{
    CreateDownloadClientDbRequest, DbError, DownloadClient, DownloadClientDb, DownloadClientId,
    DownloadClientImplementation, UpdateDownloadClientDbRequest,
};

fn row_to_download_client(row: sqlx::sqlite::SqliteRow) -> Result<DownloadClient, DbError> {
    Ok(DownloadClient {
        id: row
            .try_get::<i64, _>("id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        name: row.try_get("name").map_err(|e| DbError::Io(Box::new(e)))?,
        implementation: parse_implementation(
            &row.try_get::<String, _>("implementation")
                .map_err(|e| DbError::Io(Box::new(e)))?,
        ),
        host: row.try_get("host").map_err(|e| DbError::Io(Box::new(e)))?,
        port: row
            .try_get::<i32, _>("port")
            .map_err(|e| DbError::Io(Box::new(e)))? as u16,
        use_ssl: row
            .try_get::<bool, _>("use_ssl")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        skip_ssl_validation: row
            .try_get::<bool, _>("skip_ssl_validation")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        url_base: row
            .try_get("url_base")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        username: row
            .try_get("username")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        password: row
            .try_get("password")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        category: row
            .try_get("category")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        enabled: row
            .try_get::<bool, _>("enabled")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        client_type: row
            .try_get("client_type")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        api_key: row
            .try_get("api_key")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        is_default_for_protocol: row
            .try_get::<bool, _>("is_default_for_protocol")
            .map_err(|e| DbError::Io(Box::new(e)))?,
    })
}

fn parse_implementation(s: &str) -> DownloadClientImplementation {
    match s {
        "sabnzbd" | "SABnzbd" => DownloadClientImplementation::SABnzbd,
        _ => DownloadClientImplementation::QBittorrent,
    }
}

impl DownloadClientDb for SqliteDb {
    async fn get_download_client(&self, id: DownloadClientId) -> Result<DownloadClient, DbError> {
        let row = sqlx::query("SELECT * FROM download_clients WHERE id = ?")
            .bind(id)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;
        row_to_download_client(row)
    }

    async fn list_download_clients(&self) -> Result<Vec<DownloadClient>, DbError> {
        let rows = sqlx::query("SELECT * FROM download_clients ORDER BY id")
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;
        rows.into_iter().map(row_to_download_client).collect()
    }

    async fn create_download_client(
        &self,
        req: CreateDownloadClientDbRequest,
    ) -> Result<DownloadClient, DbError> {
        // Derive client_type from implementation enum — single source of truth.
        let client_type = req.implementation.client_type();
        let impl_str = client_type; // DB stores the same lowercase string

        // Auto-promote: if no other enabled client of this type, set as default.
        let existing_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM download_clients WHERE client_type = ? AND enabled = 1",
        )
        .bind(client_type)
        .fetch_one(self.pool())
        .await
        .map_err(map_db_err)?;

        let is_default = req.enabled && existing_count == 0;

        // If auto-promoting, clear any existing defaults for this type first.
        if is_default {
            sqlx::query(
                "UPDATE download_clients SET is_default_for_protocol = false WHERE client_type = ?",
            )
            .bind(client_type)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        }

        let id = sqlx::query(
            "INSERT INTO download_clients \
             (name, implementation, host, port, use_ssl, skip_ssl_validation, \
              url_base, username, password, category, enabled, client_type, api_key, is_default_for_protocol) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&req.name)
        .bind(impl_str)
        .bind(&req.host)
        .bind(req.port as i32)
        .bind(req.use_ssl)
        .bind(req.skip_ssl_validation)
        .bind(&req.url_base)
        .bind(&req.username)
        .bind(&req.password)
        .bind(&req.category)
        .bind(req.enabled)
        .bind(client_type)
        .bind(&req.api_key)
        .bind(is_default)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?
        .last_insert_rowid();

        self.get_download_client(id).await
    }

    async fn update_download_client(
        &self,
        id: DownloadClientId,
        req: UpdateDownloadClientDbRequest,
    ) -> Result<DownloadClient, DbError> {
        // Verify exists.
        let existing = self.get_download_client(id).await?;

        if let Some(name) = &req.name {
            sqlx::query("UPDATE download_clients SET name = ? WHERE id = ?")
                .bind(name)
                .bind(id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(host) = &req.host {
            sqlx::query("UPDATE download_clients SET host = ? WHERE id = ?")
                .bind(host)
                .bind(id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(port) = req.port {
            sqlx::query("UPDATE download_clients SET port = ? WHERE id = ?")
                .bind(port as i32)
                .bind(id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(use_ssl) = req.use_ssl {
            sqlx::query("UPDATE download_clients SET use_ssl = ? WHERE id = ?")
                .bind(use_ssl)
                .bind(id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(skip) = req.skip_ssl_validation {
            sqlx::query("UPDATE download_clients SET skip_ssl_validation = ? WHERE id = ?")
                .bind(skip)
                .bind(id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(url_base) = &req.url_base {
            sqlx::query("UPDATE download_clients SET url_base = ? WHERE id = ?")
                .bind(url_base)
                .bind(id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(username) = &req.username {
            sqlx::query("UPDATE download_clients SET username = ? WHERE id = ?")
                .bind(username)
                .bind(id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(password) = &req.password {
            sqlx::query("UPDATE download_clients SET password = ? WHERE id = ?")
                .bind(password)
                .bind(id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(category) = &req.category {
            sqlx::query("UPDATE download_clients SET category = ? WHERE id = ?")
                .bind(category)
                .bind(id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(enabled) = req.enabled {
            sqlx::query("UPDATE download_clients SET enabled = ? WHERE id = ?")
                .bind(enabled)
                .bind(id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(api_key) = &req.api_key {
            sqlx::query("UPDATE download_clients SET api_key = ? WHERE id = ?")
                .bind(api_key)
                .bind(id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(is_default) = req.is_default_for_protocol {
            if is_default {
                // Clear other defaults for this client_type.
                sqlx::query("UPDATE download_clients SET is_default_for_protocol = false WHERE client_type = ? AND id != ?")
                    .bind(&existing.client_type)
                    .bind(id)
                    .execute(self.pool())
                    .await
                    .map_err(map_db_err)?;
            }
            sqlx::query("UPDATE download_clients SET is_default_for_protocol = ? WHERE id = ?")
                .bind(is_default)
                .bind(id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }

        self.get_download_client(id).await
    }

    async fn delete_download_client(&self, id: DownloadClientId) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM download_clients WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "download_client",
            });
        }
        Ok(())
    }

    async fn get_default_download_client(
        &self,
        client_type: &str,
    ) -> Result<Option<DownloadClient>, DbError> {
        // First: explicit default for this protocol.
        let row = sqlx::query(
            "SELECT * FROM download_clients WHERE client_type = ? AND enabled = 1 AND is_default_for_protocol = 1 LIMIT 1",
        )
        .bind(client_type)
        .fetch_optional(self.pool())
        .await
        .map_err(map_db_err)?;

        if let Some(r) = row {
            return Ok(Some(row_to_download_client(r)?));
        }

        // Fallback: if exactly one enabled client of this type, use it.
        let rows =
            sqlx::query("SELECT * FROM download_clients WHERE client_type = ? AND enabled = 1")
                .bind(client_type)
                .fetch_all(self.pool())
                .await
                .map_err(map_db_err)?;

        if rows.len() == 1 {
            return Ok(Some(row_to_download_client(
                rows.into_iter().next().unwrap(),
            )?));
        }

        Ok(None)
    }
}
