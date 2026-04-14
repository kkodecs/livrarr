use chrono::Utc;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, parse_dt};
use crate::{
    CreateGrabDbRequest, DbError, Grab, GrabDb, GrabId, GrabStatus, MediaType, UserId, WorkId,
};

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
        content_path: row
            .try_get("content_path")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        grabbed_at: parse_dt(&grabbed_at_str)?,
        import_retry_count: row
            .try_get::<i32, _>("import_retry_count")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        import_failed_at: row
            .try_get::<Option<String>, _>("import_failed_at")
            .map_err(|e| DbError::Io(Box::new(e)))?
            .map(|s| parse_dt(&s))
            .transpose()?,
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
        // Use RETURNING id so we get the correct row id regardless of whether
        // the INSERT or the ON CONFLICT DO UPDATE path fired. `last_insert_rowid()`
        // is unreliable on the upsert path.
        let row: Option<(i64,)> = sqlx::query_as(
            "INSERT INTO grabs \
             (user_id, work_id, download_client_id, title, indexer, guid, size, \
              download_url, download_id, status, media_type, grabbed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
             ON CONFLICT(user_id, guid, indexer) DO UPDATE SET \
               work_id = ?2, download_client_id = ?3, title = ?4, size = ?7, \
               download_url = ?8, download_id = ?9, status = ?10, \
               import_error = NULL, media_type = ?11, grabbed_at = ?12 \
             WHERE grabs.status IN ('failed', 'removed') \
             RETURNING id",
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
        .fetch_optional(self.pool())
        .await
        .map_err(map_db_err)?;

        let id = match row {
            Some((id,)) => id,
            None => {
                // Conflict with a non-replaceable status — report it.
                return Err(DbError::Constraint {
                    message:
                        "grab already exists for this guid/indexer with a non-replaceable status"
                            .to_string(),
                });
            }
        };

        self.get_grab(req.user_id, id).await
    }

    async fn update_grab_status(
        &self,
        user_id: UserId,
        id: GrabId,
        status: GrabStatus,
        import_error: Option<&str>,
    ) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        let failed_at = if status == GrabStatus::ImportFailed {
            Some(&now)
        } else {
            None
        };
        let result = sqlx::query(
            "UPDATE grabs SET status = ?, import_error = ?, \
             import_failed_at = COALESCE(?, import_failed_at) \
             WHERE id = ? AND user_id = ?",
        )
        .bind(grab_status_str(status))
        .bind(import_error)
        .bind(failed_at)
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

    async fn set_grab_content_path(
        &self,
        user_id: UserId,
        id: GrabId,
        content_path: &str,
    ) -> Result<(), DbError> {
        sqlx::query("UPDATE grabs SET content_path = ? WHERE id = ? AND user_id = ?")
            .bind(content_path)
            .bind(id)
            .bind(user_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn try_set_importing(&self, user_id: UserId, id: GrabId) -> Result<bool, DbError> {
        // Transition from sent/confirmed/importFailed. Excluding 'importing'
        // prevents two workers from both acquiring the same grab concurrently.
        // Including 'importFailed' enables retry of previously failed imports.
        let result = sqlx::query(
            "UPDATE grabs SET status = 'importing', import_error = NULL \
             WHERE id = ? AND user_id = ? AND status IN ('sent', 'confirmed', 'importFailed')",
        )
        .bind(id)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        Ok(result.rows_affected() > 0)
    }

    async fn active_grab_exists(
        &self,
        user_id: UserId,
        work_id: WorkId,
        media_type: MediaType,
    ) -> Result<bool, DbError> {
        let mt = match media_type {
            MediaType::Ebook => "ebook",
            MediaType::Audiobook => "audiobook",
        };
        let row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM grabs \
             WHERE user_id = ? AND work_id = ? AND media_type = ? \
             AND status IN ('sent', 'confirmed', 'importing')",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(mt)
        .fetch_one(self.pool())
        .await
        .map_err(map_db_err)?;
        let cnt: i64 = row.try_get("cnt").map_err(|e| DbError::Io(Box::new(e)))?;
        Ok(cnt > 0)
    }

    async fn list_retriable_grabs(&self, max_retries: i32) -> Result<Vec<Grab>, DbError> {
        // Return importFailed grabs under the retry limit whose backoff has expired.
        // Backoff schedule: 2^retry * 120 seconds (2min, 4min, 8min, 16min, 32min).
        let rows = sqlx::query(
            "SELECT * FROM grabs \
             WHERE status = 'importFailed' \
               AND import_retry_count < ? \
               AND (import_failed_at IS NULL \
                    OR unixepoch('now') - unixepoch(import_failed_at) > (1 << import_retry_count) * 120) \
             ORDER BY id",
        )
        .bind(max_retries)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;
        rows.into_iter().map(row_to_grab).collect()
    }

    async fn increment_import_retry(&self, user_id: UserId, id: GrabId) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE grabs SET import_retry_count = import_retry_count + 1, \
             import_failed_at = ? \
             WHERE id = ? AND user_id = ?",
        )
        .bind(&now)
        .bind(id)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(())
    }
}
