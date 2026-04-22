use chrono::Utc;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, parse_dt};
use crate::{
    ApplyEnrichmentMergeRequest, ApplyMergeOutcome, AuthorId, CreateWorkDbRequest, DbError,
    EnrichmentStatus, NarrationType, ProvenanceSetter, UpdateWorkEnrichmentDbRequest,
    UpdateWorkUserFieldsDbRequest, UserId, Work, WorkDb, WorkId,
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
        series_id: row
            .try_get::<Option<i64>, _>("series_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
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
        "conflict" => Ok(EnrichmentStatus::Conflict),
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
        // TEMP(pk-tdd): compile-only scaffold
        EnrichmentStatus::Conflict => "conflict",
    }
}

fn parse_narration_type(s: &str) -> Result<NarrationType, DbError> {
    match s {
        "human" => Ok(NarrationType::Human),
        "ai" => Ok(NarrationType::Ai),
        "ai_authorized_replica" => Ok(NarrationType::AiAuthorizedReplica),
        "abridged" => Ok(NarrationType::Abridged),
        "unabridged" => Ok(NarrationType::Unabridged),
        _ => Err(DbError::DataCorruption {
            table: "works",
            column: "narration_type",
            row_id: 0,
            detail: format!("unknown narration type: {s}"),
        }),
    }
}

fn narration_type_str(n: &NarrationType) -> &'static str {
    match n {
        NarrationType::Human => "human",
        NarrationType::Ai => "ai",
        NarrationType::AiAuthorizedReplica => "ai_authorized_replica",
        // TEMP(pk-tdd): compile-only scaffold variants
        NarrationType::Abridged => "abridged",
        NarrationType::Unabridged => "unabridged",
    }
}

fn normalize(s: &str) -> String {
    s.trim().to_lowercase()
}

fn to_str<T: serde::Serialize>(v: T) -> String {
    serde_json::to_value(v)
        .expect("enum serialization is infallible")
        .as_str()
        .expect("enum serializes to string")
        .to_string()
}

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
        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            match row_to_work(row) {
                Ok(w) => results.push(w),
                Err(e) => {
                    tracing::warn!("works: skipping corrupt row: {e}");
                }
            }
        }
        Ok(results)
    }

    async fn list_works_by_author(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<Work>, DbError> {
        let rows =
            sqlx::query("SELECT * FROM works WHERE user_id = ? AND author_id = ? ORDER BY id")
                .bind(user_id)
                .bind(author_id)
                .fetch_all(self.pool())
                .await
                .map_err(map_db_err)?;
        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            match row_to_work(row) {
                Ok(w) => results.push(w),
                Err(e) => {
                    tracing::warn!("works: skipping corrupt row in list_by_author: {e}");
                }
            }
        }
        Ok(results)
    }

    async fn list_works_paginated(
        &self,
        user_id: UserId,
        page: u32,
        per_page: u32,
        sort_by: &str,
        sort_dir: &str,
    ) -> Result<(Vec<Work>, i64), DbError> {
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id = ?")
            .bind(user_id)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;

        let order_col = match sort_by {
            "title" => "title",
            "date_added" => "added_at",
            "year" => "year",
            "author" => "author_name",
            _ => "id",
        };
        let dir = if sort_dir == "asc" { "ASC" } else { "DESC" };
        let sql = format!(
            "SELECT * FROM works WHERE user_id = ? ORDER BY {order_col} {dir} LIMIT ? OFFSET ?"
        );

        let offset = (page.saturating_sub(1) * per_page) as i64;
        let rows = sqlx::query(&sql)
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
             cover_url, enrichment_status, added_at, metadata_source, detail_url, language, \
             import_id, series_id, series_name, series_position, monitor_ebook, monitor_audiobook) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
        .bind(req.series_id)
        .bind(&req.series_name)
        .bind(req.series_position)
        .bind(req.monitor_ebook)
        .bind(req.monitor_audiobook)
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

        // [I-10]: bump merge_generation when a user edit touches an enrichable
        // field, so concurrent enrichment dispatches detect the change via CAS.
        // monitor_ebook / monitor_audiobook are NOT enrichable per the IR's
        // WorkField enum, so flipping them does not bump.
        let enrichable_changed = req.title.is_some()
            || req.author_name.is_some()
            || req.series_name.is_some()
            || req.series_position.is_some();

        let title = req.title.unwrap_or(current.title);
        let author_name = req.author_name.unwrap_or(current.author_name);
        let series_name = match req.series_name {
            None => current.series_name,
            Some(v) => v,
        };
        let series_position = match req.series_position {
            None => current.series_position,
            Some(v) => v,
        };
        let monitor_ebook = req.monitor_ebook.unwrap_or(current.monitor_ebook);
        let monitor_audiobook = req.monitor_audiobook.unwrap_or(current.monitor_audiobook);

        let sql = if enrichable_changed {
            "UPDATE works SET title = ?, author_name = ?, series_name = ?, series_position = ?, \
             monitor_ebook = ?, monitor_audiobook = ?, \
             merge_generation = merge_generation + 1 \
             WHERE id = ? AND user_id = ?"
        } else {
            "UPDATE works SET title = ?, author_name = ?, series_name = ?, series_position = ?, \
             monitor_ebook = ?, monitor_audiobook = ? \
             WHERE id = ? AND user_id = ?"
        };

        sqlx::query(sql)
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

    async fn list_work_provider_keys_by_author(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<(Option<String>, Option<String>)>, DbError> {
        let rows =
            sqlx::query("SELECT ol_key, gr_key FROM works WHERE user_id = ? AND author_id = ?")
                .bind(user_id)
                .bind(author_id)
                .fetch_all(self.pool())
                .await
                .map_err(map_db_err)?;

        rows.into_iter()
            .map(|r| {
                let ol: Option<String> =
                    r.try_get("ol_key").map_err(|e| DbError::Io(Box::new(e)))?;
                let gr: Option<String> =
                    r.try_get("gr_key").map_err(|e| DbError::Io(Box::new(e)))?;
                Ok((ol, gr))
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

    async fn set_enrichment_status_skipped(&self, id: WorkId) -> Result<(), DbError> {
        sqlx::query("UPDATE works SET enrichment_status = 'skipped' WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn apply_enrichment_merge(
        &self,
        req: ApplyEnrichmentMergeRequest,
    ) -> Result<ApplyMergeOutcome, DbError> {
        let mut tx = self.pool().begin().await.map_err(map_db_err)?;

        // CAS check: read current merge_generation.
        let current_gen: i64 =
            sqlx::query_scalar("SELECT merge_generation FROM works WHERE id = ? AND user_id = ?")
                .bind(req.work_id)
                .bind(req.user_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(map_db_err)?;

        if current_gen != req.expected_merge_generation {
            return Ok(ApplyMergeOutcome::Superseded);
        }

        // Apply work update.
        let status_str = enrichment_status_str(req.new_enrichment_status);

        if let Some(work_update) = req.work_update {
            let u = work_update.into_inner();
            let now = Utc::now().to_rfc3339();
            let genres_json = u
                .genres
                .as_ref()
                .map(|g| serde_json::to_string(g).map_err(|e| DbError::Io(Box::new(e))))
                .transpose()?;
            let narrator_json = u
                .narrator
                .as_ref()
                .map(|n| serde_json::to_string(n).map_err(|e| DbError::Io(Box::new(e))))
                .transpose()?;
            let narration_type_val = u.narration_type.as_ref().map(narration_type_str);

            // Straight assignment: None → NULL (no COALESCE).
            sqlx::query(
                "UPDATE works SET \
                 title = ?, subtitle = ?, original_title = ?, author_name = ?, \
                 description = ?, year = ?, series_name = ?, series_position = ?, \
                 genres = ?, language = ?, page_count = ?, duration_seconds = ?, \
                 publisher = ?, publish_date = ?, hc_key = ?, gr_key = ?, ol_key = ?, \
                 isbn_13 = ?, asin = ?, narrator = ?, narration_type = ?, \
                 abridged = ?, rating = ?, rating_count = ?, cover_url = ?, \
                 enrichment_source = ?, enrichment_status = ?, enriched_at = ?, \
                 merge_generation = merge_generation + 1 \
                 WHERE id = ? AND user_id = ?",
            )
            .bind(u.title.as_deref())
            .bind(u.subtitle.as_deref())
            .bind(u.original_title.as_deref())
            .bind(u.author_name.as_deref())
            .bind(u.description.as_deref())
            .bind(u.year)
            .bind(u.series_name.as_deref())
            .bind(u.series_position)
            .bind(genres_json.as_deref())
            .bind(u.language.as_deref())
            .bind(u.page_count)
            .bind(u.duration_seconds)
            .bind(u.publisher.as_deref())
            .bind(u.publish_date.as_deref())
            .bind(u.hc_key.as_deref())
            .bind(u.gr_key.as_deref())
            .bind(u.ol_key.as_deref())
            .bind(u.isbn_13.as_deref())
            .bind(u.asin.as_deref())
            .bind(narrator_json.as_deref())
            .bind(narration_type_val)
            .bind(u.abridged)
            .bind(u.rating)
            .bind(u.rating_count)
            .bind(u.cover_url.as_deref())
            .bind(u.enrichment_source.as_deref())
            .bind(status_str)
            .bind(&now)
            .bind(req.work_id)
            .bind(req.user_id)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;
        } else {
            // Status-only path (e.g. Conflict).
            sqlx::query(
                "UPDATE works SET enrichment_status = ?, \
                 merge_generation = merge_generation + 1 \
                 WHERE id = ? AND user_id = ?",
            )
            .bind(status_str)
            .bind(req.work_id)
            .bind(req.user_id)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;
        }

        // Write provenance upserts.
        if !req.provenance_upserts.is_empty() {
            let prov_now = Utc::now().to_rfc3339();
            for prov in &req.provenance_upserts {
                // Validate invariant inline.
                match prov.setter {
                    ProvenanceSetter::Provider => {
                        if prov.source.is_none() {
                            return Err(DbError::Constraint {
                                message: "provider setter requires a non-null source".to_string(),
                            });
                        }
                        if prov.cleared {
                            return Err(DbError::Constraint {
                                message: "provider setter cannot have cleared=true".to_string(),
                            });
                        }
                    }
                    ProvenanceSetter::User
                    | ProvenanceSetter::System
                    | ProvenanceSetter::AutoAdded
                    | ProvenanceSetter::Imported => {
                        if prov.source.is_some() {
                            return Err(DbError::Constraint {
                                message: "user/system/auto_added setter must not have a source"
                                    .to_string(),
                            });
                        }
                    }
                }

                let field_str = to_str(prov.field);
                let setter_str = to_str(prov.setter);
                let source_str = prov.source.map(to_str);

                sqlx::query(
                    "INSERT INTO work_metadata_provenance \
                     (user_id, work_id, field, source, set_at, setter, cleared) \
                     VALUES (?, ?, ?, ?, ?, ?, ?) \
                     ON CONFLICT(work_id, field) DO UPDATE SET \
                     user_id = excluded.user_id, source = excluded.source, \
                     set_at = excluded.set_at, setter = excluded.setter, \
                     cleared = excluded.cleared",
                )
                .bind(prov.user_id)
                .bind(prov.work_id)
                .bind(&field_str)
                .bind(&source_str)
                .bind(&prov_now)
                .bind(&setter_str)
                .bind(prov.cleared as i64)
                .execute(&mut *tx)
                .await
                .map_err(map_db_err)?;
            }
        }

        // Write provenance deletes.
        for field in &req.provenance_deletes {
            let field_str = to_str(*field);
            sqlx::query("DELETE FROM work_metadata_provenance WHERE work_id = ? AND field = ?")
                .bind(req.work_id)
                .bind(&field_str)
                .execute(&mut *tx)
                .await
                .map_err(map_db_err)?;
        }

        // Write external ID upserts.
        for eid in &req.external_id_updates {
            let id_type_str = to_str(eid.id_type);
            sqlx::query(
                "INSERT INTO external_ids (work_id, id_type, id_value) \
                 VALUES (?, ?, ?) \
                 ON CONFLICT(work_id, id_type, id_value) DO NOTHING",
            )
            .bind(eid.work_id)
            .bind(&id_type_str)
            .bind(&eid.id_value)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;
        }

        tx.commit().await.map_err(map_db_err)?;
        Ok(ApplyMergeOutcome::Applied)
    }

    async fn reset_for_manual_refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<(), DbError> {
        let result = sqlx::query(
            "UPDATE works SET enrichment_status = 'pending', enriched_at = NULL, \
             merge_generation = merge_generation + 1 \
             WHERE id = ? AND user_id = ?",
        )
        .bind(work_id)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "work" });
        }

        // Delete retry state rows (preserves provenance).
        sqlx::query("DELETE FROM provider_retry_state WHERE work_id = ? AND user_id = ?")
            .bind(work_id)
            .bind(user_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;

        Ok(())
    }

    async fn list_conflict_works(&self, user_id: UserId) -> Result<Vec<Work>, DbError> {
        let rows = sqlx::query(
            "SELECT * FROM works WHERE user_id = ? AND enrichment_status = 'conflict' ORDER BY id",
        )
        .bind(user_id)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;

        rows.into_iter().map(row_to_work).collect()
    }

    async fn get_merge_generation(&self, user_id: UserId, work_id: WorkId) -> Result<i64, DbError> {
        let gen: i64 =
            sqlx::query_scalar("SELECT merge_generation FROM works WHERE id = ? AND user_id = ?")
                .bind(work_id)
                .bind(user_id)
                .fetch_one(self.pool())
                .await
                .map_err(map_db_err)?;

        Ok(gen)
    }

    async fn search_works(
        &self,
        user_id: UserId,
        query: &str,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<Work>, i64), DbError> {
        let escaped = query.replace('%', "\\%").replace('_', "\\_");
        let pattern = format!("%{escaped}%");
        let offset = ((page.max(1) - 1) * per_page) as i64;
        let limit = per_page as i64;

        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM works WHERE user_id = ? AND (title LIKE ? ESCAPE '\\' OR author_name LIKE ? ESCAPE '\\')",
        )
        .bind(user_id)
        .bind(&pattern)
        .bind(&pattern)
        .fetch_one(self.pool())
        .await
        .map_err(map_db_err)?;

        let rows = sqlx::query(
            "SELECT * FROM works WHERE user_id = ? AND (title LIKE ? ESCAPE '\\' OR author_name LIKE ? ESCAPE '\\') ORDER BY title ASC LIMIT ? OFFSET ?",
        )
        .bind(user_id)
        .bind(&pattern)
        .bind(&pattern)
        .bind(limit)
        .bind(offset)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;

        let works: Result<Vec<Work>, DbError> = rows.into_iter().map(row_to_work).collect();
        Ok((works?, total))
    }
}

impl crate::EnrichmentRetryDb for SqliteDb {
    async fn list_works_for_retry(&self) -> Result<Vec<Work>, crate::DbError> {
        let rows = sqlx::query(
            "SELECT * FROM works WHERE enrichment_status IN ('pending', 'failed', 'partial') \
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
