use chrono::Utc;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, parse_dt};
use crate::{
    CreateWorkDbRequest, DbError, EnrichmentStatus, NarrationType, UpdateWorkEnrichmentDbRequest,
    UpdateWorkUserFieldsDbRequest, Work, WorkDb, WorkId,
};

fn row_to_work(row: sqlx::sqlite::SqliteRow) -> Result<Work, DbError> {
    let genres_str: Option<String> = row
        .try_get("genres")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let narrator_str: Option<String> = row
        .try_get("narrator")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let narration_type_str: Option<String> = row
        .try_get("narration_type")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let enrichment_status_str: String = row
        .try_get("enrichment_status")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let enriched_at_str: Option<String> = row
        .try_get("enriched_at")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let added_at_str: String = row
        .try_get("added_at")
        .map_err(|e| DbError::Io(Box::new(e)))?;

    Ok(Work {
        id: row
            .try_get::<i64, _>("id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        user_id: row
            .try_get::<i64, _>("user_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        title: row.try_get("title").map_err(|e| DbError::Io(Box::new(e)))?,
        sort_title: row
            .try_get("sort_title")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        subtitle: row
            .try_get("subtitle")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        original_title: row
            .try_get("original_title")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        author_name: row
            .try_get("author_name")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        author_id: row
            .try_get::<Option<i64>, _>("author_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        description: row
            .try_get("description")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        year: row.try_get("year").map_err(|e| DbError::Io(Box::new(e)))?,
        series_name: row
            .try_get("series_name")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        series_position: row
            .try_get("series_position")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        genres: genres_str
            .map(|s| {
                serde_json::from_str(&s).map_err(|e| DbError::IncompatibleData {
                    detail: format!("invalid JSON in works.genres: {e}"),
                })
            })
            .transpose()?,
        language: row
            .try_get("language")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        page_count: row
            .try_get("page_count")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        duration_seconds: row
            .try_get("duration_seconds")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        publisher: row
            .try_get("publisher")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        publish_date: row
            .try_get("publish_date")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        ol_key: row
            .try_get("ol_key")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        hc_key: row
            .try_get("hc_key")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        gr_key: row
            .try_get("gr_key")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        isbn_13: row
            .try_get("isbn_13")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        asin: row.try_get("asin").map_err(|e| DbError::Io(Box::new(e)))?,
        narrator: narrator_str
            .map(|s| {
                serde_json::from_str(&s).map_err(|e| DbError::IncompatibleData {
                    detail: format!("invalid JSON in works.narrator: {e}"),
                })
            })
            .transpose()?,
        narration_type: narration_type_str
            .map(|s| parse_narration_type(&s))
            .transpose()?,
        abridged: row
            .try_get::<bool, _>("abridged")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        rating: row
            .try_get("rating")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        rating_count: row
            .try_get("rating_count")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        enrichment_status: parse_enrichment_status(&enrichment_status_str)?,
        enrichment_retry_count: row
            .try_get::<i32, _>("enrichment_retry_count")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        enriched_at: enriched_at_str.map(|s| parse_dt(&s)).transpose()?,
        enrichment_source: row
            .try_get("enrichment_source")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        cover_url: row
            .try_get("cover_url")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        cover_manual: row
            .try_get::<bool, _>("cover_manual")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        monitor_ebook: row
            .try_get::<bool, _>("monitor_ebook")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        monitor_audiobook: row
            .try_get::<bool, _>("monitor_audiobook")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        import_id: row
            .try_get("import_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        added_at: parse_dt(&added_at_str)?,
        metadata_source: row
            .try_get("metadata_source")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        detail_url: row
            .try_get("detail_url")
            .map_err(|e| DbError::Io(Box::new(e)))?,
    })
}

fn parse_enrichment_status(s: &str) -> Result<EnrichmentStatus, DbError> {
    match s {
        "pending" => Ok(EnrichmentStatus::Pending),
        "partial" => Ok(EnrichmentStatus::Partial),
        "enriched" => Ok(EnrichmentStatus::Enriched),
        "failed" => Ok(EnrichmentStatus::Failed),
        "exhausted" => Ok(EnrichmentStatus::Exhausted),
        "skipped" => Ok(EnrichmentStatus::Skipped),
        _ => Err(DbError::IncompatibleData {
            detail: format!("unknown enrichment status: {s}"),
        }),
    }
}

fn enrichment_status_str(s: EnrichmentStatus) -> &'static str {
    match s {
        EnrichmentStatus::Pending => "pending",
        EnrichmentStatus::Partial => "partial",
        EnrichmentStatus::Enriched => "enriched",
        EnrichmentStatus::Failed => "failed",
        EnrichmentStatus::Exhausted => "exhausted",
        EnrichmentStatus::Skipped => "skipped",
    }
}

fn parse_narration_type(s: &str) -> Result<NarrationType, DbError> {
    match s {
        "human" => Ok(NarrationType::Human),
        "ai" => Ok(NarrationType::Ai),
        "ai_authorized_replica" => Ok(NarrationType::AiAuthorizedReplica),
        _ => Err(DbError::IncompatibleData {
            detail: format!("unknown narration type: {s}"),
        }),
    }
}

fn narration_type_str(n: &NarrationType) -> &'static str {
    match n {
        NarrationType::Human => "human",
        NarrationType::Ai => "ai",
        NarrationType::AiAuthorizedReplica => "ai_authorized_replica",
    }
}

fn normalize(s: &str) -> String {
    s.trim().to_lowercase()
}

type UserId = i64;

impl WorkDb for SqliteDb {
    async fn get_work(&self, user_id: UserId, id: WorkId) -> Result<Work, DbError> {
        let row = sqlx::query("SELECT * FROM works WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;
        row_to_work(row)
    }

    async fn list_works(&self, user_id: UserId) -> Result<Vec<Work>, DbError> {
        let rows = sqlx::query("SELECT * FROM works WHERE user_id = ? ORDER BY id")
            .bind(user_id)
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;
        rows.into_iter().map(row_to_work).collect()
    }

    async fn list_works_paginated(
        &self,
        user_id: UserId,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<Work>, i64), DbError> {
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id = ?")
            .bind(user_id)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;

        let offset = (page.saturating_sub(1) * per_page) as i64;
        let rows =
            sqlx::query("SELECT * FROM works WHERE user_id = ? ORDER BY id LIMIT ? OFFSET ?")
                .bind(user_id)
                .bind(per_page as i64)
                .bind(offset)
                .fetch_all(self.pool())
                .await
                .map_err(map_db_err)?;

        let works = rows
            .into_iter()
            .map(row_to_work)
            .collect::<Result<Vec<_>, _>>()?;
        Ok((works, total))
    }

    async fn create_work(&self, req: CreateWorkDbRequest) -> Result<Work, DbError> {
        let now = Utc::now().to_rfc3339();
        let id = sqlx::query(
            "INSERT INTO works (user_id, title, author_name, author_id, ol_key, gr_key, year, \
             cover_url, enrichment_status, added_at, metadata_source, detail_url, language, import_id) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?, ?, ?, ?)",
        )
        .bind(req.user_id)
        .bind(&req.title)
        .bind(&req.author_name)
        .bind(req.author_id)
        .bind(&req.ol_key)
        .bind(&req.gr_key)
        .bind(req.year)
        .bind(&req.cover_url)
        .bind(&now)
        .bind(&req.metadata_source)
        .bind(&req.detail_url)
        .bind(&req.language)
        .bind(&req.import_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?
        .last_insert_rowid();

        self.get_work(req.user_id, id).await
    }

    async fn update_work_enrichment(
        &self,
        user_id: UserId,
        id: WorkId,
        req: UpdateWorkEnrichmentDbRequest,
    ) -> Result<Work, DbError> {
        // Verify exists.
        self.get_work(user_id, id).await?;

        let now = Utc::now().to_rfc3339();
        let genres_json = req
            .genres
            .as_ref()
            .map(|g| serde_json::to_string(g).map_err(|e| DbError::Io(Box::new(e))))
            .transpose()?;
        let narrator_json = req
            .narrator
            .as_ref()
            .map(|n| serde_json::to_string(n).map_err(|e| DbError::Io(Box::new(e))))
            .transpose()?;
        let narration_type_val = req.narration_type.as_ref().map(narration_type_str);

        sqlx::query(
            "UPDATE works SET \
             title = COALESCE(?, title), \
             subtitle = COALESCE(?, subtitle), \
             original_title = COALESCE(?, original_title), \
             author_name = COALESCE(?, author_name), \
             description = COALESCE(?, description), \
             year = COALESCE(?, year), \
             series_name = COALESCE(?, series_name), \
             series_position = COALESCE(?, series_position), \
             genres = COALESCE(?, genres), \
             language = COALESCE(?, language), \
             page_count = COALESCE(?, page_count), \
             duration_seconds = COALESCE(?, duration_seconds), \
             publisher = COALESCE(?, publisher), \
             publish_date = COALESCE(?, publish_date), \
             hc_key = COALESCE(?, hc_key), \
             isbn_13 = COALESCE(?, isbn_13), \
             asin = COALESCE(?, asin), \
             narrator = COALESCE(?, narrator), \
             narration_type = COALESCE(?, narration_type), \
             abridged = COALESCE(?, abridged), \
             rating = COALESCE(?, rating), \
             rating_count = COALESCE(?, rating_count), \
             enrichment_status = ?, \
             enrichment_source = COALESCE(?, enrichment_source), \
             cover_url = COALESCE(?, cover_url), \
             enriched_at = ? \
             WHERE id = ? AND user_id = ?",
        )
        .bind(req.title.as_deref())
        .bind(req.subtitle.as_deref())
        .bind(req.original_title.as_deref())
        .bind(req.author_name.as_deref())
        .bind(req.description.as_deref())
        .bind(req.year)
        .bind(req.series_name.as_deref())
        .bind(req.series_position)
        .bind(genres_json.as_deref())
        .bind(req.language.as_deref())
        .bind(req.page_count)
        .bind(req.duration_seconds)
        .bind(req.publisher.as_deref())
        .bind(req.publish_date.as_deref())
        .bind(req.hc_key.as_deref())
        .bind(req.isbn_13.as_deref())
        .bind(req.asin.as_deref())
        .bind(narrator_json.as_deref())
        .bind(narration_type_val)
        .bind(req.abridged)
        .bind(req.rating)
        .bind(req.rating_count)
        .bind(enrichment_status_str(req.enrichment_status))
        .bind(req.enrichment_source.as_deref())
        .bind(req.cover_url.as_deref())
        .bind(&now)
        .bind(id)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        self.get_work(user_id, id).await
    }

    async fn update_work_user_fields(
        &self,
        user_id: UserId,
        id: WorkId,
        req: UpdateWorkUserFieldsDbRequest,
    ) -> Result<Work, DbError> {
        let current = self.get_work(user_id, id).await?;

        let title = req.title.unwrap_or(current.title);
        let author_name = req.author_name.unwrap_or(current.author_name);
        let series_name = req.series_name.or(current.series_name);
        let series_position = req.series_position.or(current.series_position);
        let monitor_ebook = req.monitor_ebook.unwrap_or(current.monitor_ebook);
        let monitor_audiobook = req.monitor_audiobook.unwrap_or(current.monitor_audiobook);

        sqlx::query(
            "UPDATE works SET title = ?, author_name = ?, series_name = ?, series_position = ?, \
             monitor_ebook = ?, monitor_audiobook = ? \
             WHERE id = ? AND user_id = ?",
        )
        .bind(&title)
        .bind(&author_name)
        .bind(&series_name)
        .bind(series_position)
        .bind(monitor_ebook)
        .bind(monitor_audiobook)
        .bind(id)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        self.get_work(user_id, id).await
    }

    async fn set_cover_manual(
        &self,
        user_id: UserId,
        id: WorkId,
        manual: bool,
    ) -> Result<(), DbError> {
        let result = sqlx::query("UPDATE works SET cover_manual = ? WHERE id = ? AND user_id = ?")
            .bind(manual)
            .bind(id)
            .bind(user_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "work" });
        }
        Ok(())
    }

    async fn delete_work(&self, user_id: UserId, id: WorkId) -> Result<Work, DbError> {
        let work = self.get_work(user_id, id).await?;
        sqlx::query("DELETE FROM works WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(work)
    }

    async fn work_exists_by_ol_key(&self, user_id: UserId, ol_key: &str) -> Result<bool, DbError> {
        let row = sqlx::query("SELECT COUNT(*) as cnt FROM works WHERE user_id = ? AND ol_key = ?")
            .bind(user_id)
            .bind(ol_key)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;
        let cnt: i64 = row.try_get("cnt").map_err(|e| DbError::Io(Box::new(e)))?;
        Ok(cnt > 0)
    }

    async fn list_works_for_enrichment(&self, user_id: UserId) -> Result<Vec<Work>, DbError> {
        let rows = sqlx::query(
            "SELECT * FROM works WHERE user_id = ? AND enrichment_status IN ('pending', 'partial', 'failed') ORDER BY id",
        )
        .bind(user_id)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;
        rows.into_iter().map(row_to_work).collect()
    }

    async fn list_works_by_author_ol_keys(
        &self,
        user_id: UserId,
        author_ol_key: &str,
    ) -> Result<Vec<String>, DbError> {
        let rows = sqlx::query(
            "SELECT w.ol_key FROM works w \
             JOIN authors a ON w.author_id = a.id \
             WHERE w.user_id = ? AND a.ol_key = ? AND w.ol_key IS NOT NULL",
        )
        .bind(user_id)
        .bind(author_ol_key)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;

        rows.into_iter()
            .map(|r| {
                r.try_get::<String, _>("ol_key")
                    .map_err(|e| DbError::Io(Box::new(e)))
            })
            .collect()
    }

    async fn find_by_normalized_match(
        &self,
        user_id: UserId,
        title: &str,
        author: &str,
    ) -> Result<Vec<Work>, DbError> {
        let norm_title = normalize(title);
        let norm_author = normalize(author);
        let rows = sqlx::query(
            "SELECT * FROM works WHERE user_id = ? AND LOWER(TRIM(title)) = ? AND LOWER(TRIM(author_name)) = ?",
        )
        .bind(user_id)
        .bind(&norm_title)
        .bind(&norm_author)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;
        rows.into_iter().map(row_to_work).collect()
    }

    async fn reset_pending_enrichments(&self) -> Result<u64, crate::DbError> {
        let result = sqlx::query(
            "UPDATE works SET enrichment_status = 'failed' WHERE enrichment_status = 'pending'",
        )
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(result.rows_affected())
    }

    async fn list_monitored_works_all_users(&self) -> Result<Vec<Work>, DbError> {
        let rows = sqlx::query(
            "SELECT * FROM works WHERE monitor_ebook = 1 OR monitor_audiobook = 1 \
             ORDER BY id",
        )
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;
        rows.into_iter().map(row_to_work).collect()
    }
}

impl crate::EnrichmentRetryDb for SqliteDb {
    async fn list_works_for_retry(&self) -> Result<Vec<Work>, crate::DbError> {
        let rows = sqlx::query(
            "SELECT * FROM works WHERE enrichment_status IN ('failed', 'partial') \
             AND enrichment_retry_count < 3 ORDER BY id",
        )
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;
        rows.into_iter().map(row_to_work).collect()
    }

    async fn reset_enrichment_for_refresh(
        &self,
        user_id: UserId,
        work_id: crate::WorkId,
    ) -> Result<(), crate::DbError> {
        let result = sqlx::query(
            "UPDATE works SET enrichment_status = 'pending', enrichment_retry_count = 0 \
             WHERE id = ? AND user_id = ?",
        )
        .bind(work_id)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        if result.rows_affected() == 0 {
            return Err(crate::DbError::NotFound { entity: "work" });
        }
        Ok(())
    }

    async fn increment_retry_count(
        &self,
        user_id: UserId,
        work_id: crate::WorkId,
    ) -> Result<(), crate::DbError> {
        sqlx::query(
            "UPDATE works SET enrichment_retry_count = enrichment_retry_count + 1 \
             WHERE id = ? AND user_id = ?",
        )
        .bind(work_id)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        // Transition to exhausted if count >= 3 and status is failed.
        sqlx::query(
            "UPDATE works SET enrichment_status = 'exhausted' \
             WHERE id = ? AND user_id = ? AND enrichment_retry_count >= 3 \
             AND enrichment_status = 'failed'",
        )
        .bind(work_id)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        Ok(())
    }
}
