use chrono::Utc;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, parse_dt};
use crate::{CreateGrabDbRequest, DbError, Grab, GrabDb, GrabId, GrabStatus, UserId};

fn row_to_grab(row: sqlx::sqlite::SqliteRow) -> Result<Grab, DbError> {
    let status_str: String = row
        .try_get("status")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let grabbed_at_str: String = row
        .try_get("grabbed_at")
        .map_err(|e| DbError::Io(Box::new(e)))?;

    Ok(Grab {
        id: row
            .try_get::<i64, _>("id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        user_id: row
            .try_get::<i64, _>("user_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        work_id: row
            .try_get::<i64, _>("work_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        download_client_id: row
            .try_get::<i64, _>("download_client_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        title: row.try_get("title").map_err(|e| DbError::Io(Box::new(e)))?,
        indexer: row
            .try_get("indexer")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        guid: row.try_get("guid").map_err(|e| DbError::Io(Box::new(e)))?,
        size: row.try_get("size").map_err(|e| DbError::Io(Box::new(e)))?,
        download_url: row
            .try_get("download_url")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        download_id: row
            .try_get("download_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        status: parse_grab_status(&status_str)?,
        import_error: row
            .try_get("import_error")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        media_type: row
            .try_get::<Option<String>, _>("media_type")
            .map_err(|e| DbError::Io(Box::new(e)))?
            .and_then(|s| match s.as_str() {
                "ebook" => Some(livrarr_domain::MediaType::Ebook),
                "audiobook" => Some(livrarr_domain::MediaType::Audiobook),
                _ => None,
            }),
        grabbed_at: parse_dt(&grabbed_at_str)?,
    })
}

fn parse_grab_status(s: &str) -> Result<GrabStatus, DbError> {
    match s {
        "sent" => Ok(GrabStatus::Sent),
        "confirmed" => Ok(GrabStatus::Confirmed),
        "importing" => Ok(GrabStatus::Importing),
        "imported" => Ok(GrabStatus::Imported),
        "importFailed" => Ok(GrabStatus::ImportFailed),
        "removed" => Ok(GrabStatus::Removed),
        "failed" => Ok(GrabStatus::Failed),
        _ => Err(DbError::IncompatibleData {
            detail: format!("unknown grab status: {s}"),
        }),
    }
}

fn grab_status_str(s: GrabStatus) -> &'static str {
    match s {
        GrabStatus::Sent => "sent",
        GrabStatus::Confirmed => "confirmed",
        GrabStatus::Importing => "importing",
        GrabStatus::Imported => "imported",
        GrabStatus::ImportFailed => "importFailed",
        GrabStatus::Removed => "removed",
        GrabStatus::Failed => "failed",
    }
}

impl GrabDb for SqliteDb {
    async fn get_grab(&self, user_id: UserId, id: GrabId) -> Result<Grab, DbError> {
        let row = sqlx::query("SELECT * FROM grabs WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;
        row_to_grab(row)
    }

    async fn list_grabs_paginated(
        &self,
        user_id: UserId,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<Grab>, i64), DbError> {
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM grabs WHERE user_id = ?")
            .bind(user_id)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;

        let offset = (page.saturating_sub(1) * per_page) as i64;
        let rows = sqlx::query(
            "SELECT * FROM grabs WHERE user_id = ? ORDER BY grabbed_at DESC LIMIT ? OFFSET ?",
        )
        .bind(user_id)
        .bind(per_page as i64)
        .bind(offset)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;

        let grabs = rows
            .into_iter()
            .map(row_to_grab)
            .collect::<Result<Vec<_>, _>>()?;
        Ok((grabs, total))
    }

    async fn list_active_grabs(&self) -> Result<Vec<Grab>, DbError> {
        let rows =
            sqlx::query("SELECT * FROM grabs WHERE status IN ('sent', 'confirmed') ORDER BY id")
                .fetch_all(self.pool())
                .await
                .map_err(map_db_err)?;
        rows.into_iter().map(row_to_grab).collect()
    }

    async fn upsert_grab(&self, req: CreateGrabDbRequest) -> Result<Grab, DbError> {
        let now = Utc::now().to_rfc3339();
        let status_str = grab_status_str(req.status);
        let media_type_str = req.media_type.map(|mt| match mt {
            livrarr_domain::MediaType::Ebook => "ebook",
            livrarr_domain::MediaType::Audiobook => "audiobook",
        });

        // Atomic upsert using INSERT...ON CONFLICT against the
        // UNIQUE(user_id, guid, indexer) constraint.
        //
        // The ON CONFLICT DO UPDATE only fires when the existing row has
        // status 'failed' or 'removed' (the WHERE clause on the DO UPDATE).
        // If the existing row has any other status, the WHERE fails and the
        // INSERT is silently ignored — we detect that via changes() == 0.
        let result = sqlx::query(
            "INSERT INTO grabs \
             (user_id, work_id, download_client_id, title, indexer, guid, size, \
              download_url, download_id, status, media_type, grabbed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
             ON CONFLICT(user_id, guid, indexer) DO UPDATE SET \
               work_id = ?2, download_client_id = ?3, title = ?4, size = ?7, \
               download_url = ?8, download_id = ?9, status = ?10, \
               import_error = NULL, media_type = ?11, grabbed_at = ?12 \
             WHERE grabs.status IN ('failed', 'removed')",
        )
        .bind(req.user_id) // ?1
        .bind(req.work_id) // ?2
        .bind(req.download_client_id) // ?3
        .bind(&req.title) // ?4
        .bind(&req.indexer) // ?5
        .bind(&req.guid) // ?6
        .bind(req.size) // ?7
        .bind(&req.download_url) // ?8
        .bind(&req.download_id) // ?9
        .bind(status_str) // ?10
        .bind(media_type_str) // ?11
        .bind(&now) // ?12
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            // Conflict with a non-replaceable status — report it.
            return Err(DbError::Constraint {
                message: "grab already exists for this guid/indexer with a non-replaceable status"
                    .to_string(),
            });
        }

        let id = result.last_insert_rowid();
        self.get_grab(req.user_id, id).await
    }

    async fn update_grab_status(
        &self,
        user_id: UserId,
        id: GrabId,
        status: GrabStatus,
        import_error: Option<&str>,
    ) -> Result<(), DbError> {
        let result = sqlx::query(
            "UPDATE grabs SET status = ?, import_error = ? WHERE id = ? AND user_id = ?",
        )
        .bind(grab_status_str(status))
        .bind(import_error)
        .bind(id)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "grab" });
        }
        Ok(())
    }

    async fn update_grab_download_id(
        &self,
        user_id: UserId,
        id: GrabId,
        download_id: &str,
    ) -> Result<(), DbError> {
        let result = sqlx::query("UPDATE grabs SET download_id = ? WHERE id = ? AND user_id = ?")
            .bind(download_id)
            .bind(id)
            .bind(user_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "grab" });
        }
        Ok(())
    }

    async fn get_grab_by_download_id(&self, download_id: &str) -> Result<Option<Grab>, DbError> {
        let row = sqlx::query("SELECT * FROM grabs WHERE download_id = ?")
            .bind(download_id)
            .fetch_optional(self.pool())
            .await
            .map_err(map_db_err)?;

        match row {
            Some(r) => Ok(Some(row_to_grab(r)?)),
            None => Ok(None),
        }
    }

    async fn reset_importing_grabs(&self) -> Result<u64, DbError> {
        let result =
            sqlx::query("UPDATE grabs SET status = 'confirmed' WHERE status = 'importing'")
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        Ok(result.rows_affected())
    }

    async fn set_grab_download_id(
        &self,
        user_id: UserId,
        id: GrabId,
        download_id: &str,
    ) -> Result<(), DbError> {
        let result = sqlx::query("UPDATE grabs SET download_id = ? WHERE id = ? AND user_id = ?")
            .bind(download_id)
            .bind(id)
            .bind(user_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "grab" });
        }
        Ok(())
    }

    async fn try_set_importing(&self, user_id: UserId, id: GrabId) -> Result<bool, DbError> {
        // Only transition from sent/confirmed — excluding 'importing' prevents
        // two workers from both acquiring the same grab concurrently.
        let result = sqlx::query(
            "UPDATE grabs SET status = 'importing', import_error = NULL \
             WHERE id = ? AND user_id = ? AND status IN ('sent', 'confirmed')",
        )
        .bind(id)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        Ok(result.rows_affected() > 0)
    }
}
