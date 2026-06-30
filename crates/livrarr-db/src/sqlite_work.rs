use chrono::{DateTime, Utc};
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{absolute_http_cover_url, map_db_err, parse_dt};
use crate::{
    ApplyEnrichmentMergeRequest, ApplyMergeOutcome, AuthorId, CreateWorkDbRequest, DbError,
    EnrichmentStatus, MediaType, NarrationType, ProvenanceSetter, UpdateWorkEnrichmentDbRequest,
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
        identity_status: row
            .try_get::<String, _>("identity_status")
            .ok()
            .map(|s| parse_identity_status(&s))
            .transpose()?
            .unwrap_or_default(),
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
        cover_source: row.try_get("cover_source").unwrap_or(None),
        cover_trust: row
            .try_get::<String, _>("cover_trust")
            .ok()
            .map(|s| match s.as_str() {
                "user" => livrarr_domain::CoverTrust::User,
                "validated" => livrarr_domain::CoverTrust::Validated,
                _ => livrarr_domain::CoverTrust::Unvalidated,
            })
            .unwrap_or_default(),
        cover_width: row.try_get("cover_width").unwrap_or(0),
        cover_height: row.try_get("cover_height").unwrap_or(0),
        audiobook_cover_url: row.try_get("audiobook_cover_url").unwrap_or(None),
        audiobook_cover_source: row.try_get("audiobook_cover_source").unwrap_or(None),
        audiobook_cover_trust: row
            .try_get::<String, _>("audiobook_cover_trust")
            .ok()
            .map(|s| match s.as_str() {
                "user" => livrarr_domain::CoverTrust::User,
                "validated" => livrarr_domain::CoverTrust::Validated,
                _ => livrarr_domain::CoverTrust::Unvalidated,
            })
            .unwrap_or_default(),
        audiobook_cover_width: row.try_get("audiobook_cover_width").unwrap_or(0),
        audiobook_cover_height: row.try_get("audiobook_cover_height").unwrap_or(0),
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
    })
}

fn parse_enrichment_status(s: &str) -> Result<EnrichmentStatus, DbError> {
    match s {
        "unenriched" => Ok(EnrichmentStatus::Unenriched),
        // Legacy values migrated to unenriched by migration 035
        "pending" | "partial" => Ok(EnrichmentStatus::Unenriched),
        "enriched" => Ok(EnrichmentStatus::Enriched),
        "thin" => Ok(EnrichmentStatus::Thin),
        "failed" => Ok(EnrichmentStatus::Failed),
        // Legacy exhausted/skipped mapped to failed — migration handles DB rows
        "exhausted" | "skipped" => Ok(EnrichmentStatus::Failed),
        // Legacy identity-track values dropped from EnrichmentStatus (migration 055
        // moved identity to the identity_status column). Tolerate any pre-migration
        // row by reading them as Unenriched.
        "conflict" | "identity_pending" | "needs_review" => Ok(EnrichmentStatus::Unenriched),
        _ => Err(DbError::IncompatibleData {
            detail: format!("unknown enrichment status: {s}"),
        }),
    }
}

fn enrichment_status_str(s: EnrichmentStatus) -> &'static str {
    match s {
        EnrichmentStatus::Unenriched => "unenriched",
        EnrichmentStatus::Enriched => "enriched",
        EnrichmentStatus::Thin => "thin",
        EnrichmentStatus::Failed => "failed",
    }
}

fn parse_identity_status(s: &str) -> Result<livrarr_domain::IdentityStatus, DbError> {
    use livrarr_domain::IdentityStatus;
    match s {
        "pending" => Ok(IdentityStatus::Pending),
        "confirmed" => Ok(IdentityStatus::Confirmed),
        "provisional" => Ok(IdentityStatus::Provisional),
        "conflict" => Ok(IdentityStatus::Conflict),
        "needs_review" => Ok(IdentityStatus::NeedsReview),
        "not_found" => Ok(IdentityStatus::NotFound),
        _ => Err(DbError::IncompatibleData {
            detail: format!("unknown identity status: {s}"),
        }),
    }
}

fn identity_status_str(s: livrarr_domain::IdentityStatus) -> &'static str {
    use livrarr_domain::IdentityStatus;
    match s {
        IdentityStatus::Pending => "pending",
        IdentityStatus::Confirmed => "confirmed",
        IdentityStatus::Provisional => "provisional",
        IdentityStatus::Conflict => "conflict",
        IdentityStatus::NeedsReview => "needs_review",
        IdentityStatus::NotFound => "not_found",
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
        media_type: Option<MediaType>,
        language: Option<&str>,
    ) -> Result<(Vec<Work>, i64), DbError> {
        let media_clause = match media_type {
            Some(MediaType::Ebook) => " AND monitor_ebook = 1",
            Some(MediaType::Audiobook) => " AND monitor_audiobook = 1",
            None => "",
        };

        let lang_clause = if language.is_some() {
            " AND language = ?"
        } else {
            ""
        };

        let count_sql =
            format!("SELECT COUNT(*) FROM works WHERE user_id = ?{media_clause}{lang_clause}");
        let mut count_query = sqlx::query_scalar(&count_sql).bind(user_id);
        if let Some(lang) = language {
            count_query = count_query.bind(lang);
        }
        let total: i64 = count_query
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;

        let dir = if sort_dir == "asc" { "ASC" } else { "DESC" };
        let order_clause = match sort_by {
            "recently_downloaded" => format!(
                "COALESCE((SELECT MAX(imported_at) FROM library_items WHERE library_items.work_id = works.id), '1970-01-01') {dir}"
            ),
            other => {
                let col = match other {
                    "title" => "title",
                    "date_added" => "added_at",
                    "year" => "year",
                    "author" => "author_name",
                    _ => "id",
                };
                format!("{col} {dir}")
            }
        };
        let sql = format!(
            "SELECT * FROM works WHERE user_id = ?{media_clause}{lang_clause} ORDER BY {order_clause} LIMIT ? OFFSET ?"
        );

        let offset = (page.saturating_sub(1) * per_page) as i64;
        let mut query = sqlx::query(&sql).bind(user_id);
        if let Some(lang) = language {
            query = query.bind(lang);
        }
        let rows = query
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
        .bind(narrator_json.as_deref())
        .bind(narration_type_val)
        .bind(req.abridged)
        .bind(req.rating)
        .bind(req.rating_count)
        .bind(enrichment_status_str(req.enrichment_status))
        .bind(req.enrichment_source.as_deref())
        .bind(absolute_http_cover_url(req.cover_url.as_deref()))
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

    async fn set_identity_status(
        &self,
        user_id: UserId,
        id: WorkId,
        status: livrarr_domain::IdentityStatus,
    ) -> Result<(), DbError> {
        let result =
            sqlx::query("UPDATE works SET identity_status = ? WHERE id = ? AND user_id = ?")
                .bind(identity_status_str(status))
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

    #[allow(clippy::too_many_arguments)]
    async fn update_cover_metadata(
        &self,
        user_id: UserId,
        work_id: WorkId,
        cover_url: Option<&str>,
        cover_source: &str,
        cover_trust: livrarr_domain::CoverTrust,
        cover_width: i32,
        cover_height: i32,
    ) -> Result<(), DbError> {
        let trust_str = match cover_trust {
            livrarr_domain::CoverTrust::User => "user",
            livrarr_domain::CoverTrust::Validated => "validated",
            livrarr_domain::CoverTrust::Unvalidated => "unvalidated",
        };
        let cover_manual = cover_trust == livrarr_domain::CoverTrust::User;
        let result = sqlx::query(
            "UPDATE works SET cover_url = ?, cover_source = ?, cover_trust = ?, \
             cover_width = ?, cover_height = ?, cover_manual = ? \
             WHERE id = ? AND user_id = ?",
        )
        .bind(absolute_http_cover_url(cover_url))
        .bind(cover_source)
        .bind(trust_str)
        .bind(cover_width)
        .bind(cover_height)
        .bind(cover_manual)
        .bind(work_id)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "work" });
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_audiobook_cover_metadata(
        &self,
        user_id: UserId,
        work_id: WorkId,
        audiobook_cover_url: Option<&str>,
        audiobook_cover_source: &str,
        audiobook_cover_trust: livrarr_domain::CoverTrust,
        audiobook_cover_width: i32,
        audiobook_cover_height: i32,
    ) -> Result<(), DbError> {
        let trust_str = match audiobook_cover_trust {
            livrarr_domain::CoverTrust::User => "user",
            livrarr_domain::CoverTrust::Validated => "validated",
            livrarr_domain::CoverTrust::Unvalidated => "unvalidated",
        };
        let result = sqlx::query(
            "UPDATE works SET audiobook_cover_url = ?, audiobook_cover_source = ?, \
             audiobook_cover_trust = ?, audiobook_cover_width = ?, audiobook_cover_height = ? \
             WHERE id = ? AND user_id = ?",
        )
        .bind(absolute_http_cover_url(audiobook_cover_url))
        .bind(audiobook_cover_source)
        .bind(trust_str)
        .bind(audiobook_cover_width)
        .bind(audiobook_cover_height)
        .bind(work_id)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "work" });
        }
        Ok(())
    }

    async fn update_cover_dimensions(
        &self,
        user_id: UserId,
        work_id: WorkId,
        width: i32,
        height: i32,
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE works SET cover_width = ?, cover_height = ? WHERE id = ? AND user_id = ?",
        )
        .bind(width)
        .bind(height)
        .bind(work_id)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(())
    }

    async fn update_audiobook_cover_dimensions(
        &self,
        user_id: UserId,
        work_id: WorkId,
        width: i32,
        height: i32,
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE works SET audiobook_cover_width = ?, audiobook_cover_height = ? WHERE id = ? AND user_id = ?",
        )
        .bind(width)
        .bind(height)
        .bind(work_id)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
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
            "SELECT * FROM works WHERE user_id = ? AND normalized_title = ? AND normalized_author = ?",
        )
        .bind(user_id)
        .bind(&norm_title)
        .bind(&norm_author)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;
        rows.into_iter().map(row_to_work).collect()
    }

    async fn find_normalized_match_no_anchor_for_user(
        &self,
        user_id: UserId,
        raw_title: &str,
        raw_author: &str,
    ) -> Result<Option<Work>, DbError> {
        let norm_title = normalize(raw_title);
        let norm_author = normalize(raw_author);
        if norm_title.is_empty() || norm_author.is_empty() {
            return Ok(None);
        }
        let row = sqlx::query(
            "SELECT w.* FROM works w \
             WHERE w.user_id = ? \
               AND w.normalized_title = ? \
               AND w.normalized_author = ? \
               AND NOT EXISTS ( \
                   SELECT 1 FROM work_identity_anchors a \
                   WHERE a.work_id = w.id \
                     AND a.anchor_type = 'ol_work' \
                     AND a.confidence = 'confirmed' \
               ) \
             LIMIT 1",
        )
        .bind(user_id)
        .bind(&norm_title)
        .bind(&norm_author)
        .fetch_optional(self.pool())
        .await
        .map_err(map_db_err)?;
        row.map(row_to_work).transpose()
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

    async fn list_identity_pending_works(&self) -> Result<Vec<Work>, DbError> {
        let rows = sqlx::query("SELECT * FROM works WHERE identity_status = 'pending'")
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;
        rows.into_iter().map(row_to_work).collect()
    }

    async fn list_stale_unenriched_works(
        &self,
        older_than: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<Work>, DbError> {
        // REQ-015: only a SETTLED identity auto-enriches. A held identity (pending →
        // converges first; conflict/needs_review/not_found → terminal, user acts) must
        // not be picked up by the stale-unenriched retry — otherwise a pending work
        // enriches prematurely and a not_found work loops (both are enrichment
        // 'unenriched' now that identity lives on identity_status).
        let rows = sqlx::query(
            "SELECT * FROM works WHERE enrichment_status = 'unenriched' \
             AND identity_status IN ('confirmed', 'provisional') AND added_at < ?",
        )
        .bind(older_than.to_rfc3339())
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;
        rows.into_iter().map(row_to_work).collect()
    }

    async fn list_failed_works_without_retry_state(&self) -> Result<Vec<Work>, DbError> {
        let rows = sqlx::query(
            "SELECT w.* FROM works w \
             LEFT JOIN provider_retry_state prs \
                 ON w.id = prs.work_id AND w.user_id = prs.user_id \
             WHERE w.enrichment_status = 'failed' AND prs.work_id IS NULL",
        )
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;
        rows.into_iter().map(row_to_work).collect()
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
            let norm_title = u
                .title
                .as_deref()
                .map(livrarr_domain::normalize_for_matching);
            let norm_author = u
                .author_name
                .as_deref()
                .map(livrarr_domain::normalize_for_matching);

            // REQ-007: no anchor columns (hc_key/gr_key/ol_key/isbn_13/asin)
            // in this UPDATE — anchors move exclusively via the identity
            // track. REQ-009: None/empty cover_url and language never clobber
            // populated values (COALESCE + empty-filtered binds).
            sqlx::query(
                "UPDATE works SET \
                 title = COALESCE(?, title), subtitle = ?, original_title = ?, \
                 author_name = COALESCE(?, author_name), \
                 normalized_title = COALESCE(?, normalized_title), \
                 normalized_author = COALESCE(?, normalized_author), \
                 description = ?, year = ?, series_name = ?, series_position = ?, \
                 genres = ?, language = COALESCE(?, language), page_count = ?, \
                 duration_seconds = ?, publisher = ?, publish_date = ?, \
                 narrator = ?, narration_type = ?, \
                 abridged = ?, rating = ?, rating_count = ?, \
                 cover_url = COALESCE(?, cover_url), \
                 enrichment_source = ?, enrichment_status = ?, enriched_at = ?, \
                 merge_generation = merge_generation + 1 \
                 WHERE id = ? AND user_id = ?",
            )
            .bind(u.title.as_deref())
            .bind(u.subtitle.as_deref())
            .bind(u.original_title.as_deref())
            .bind(u.author_name.as_deref())
            .bind(norm_title.as_deref())
            .bind(norm_author.as_deref())
            .bind(u.description.as_deref())
            .bind(u.year)
            .bind(u.series_name.as_deref())
            .bind(u.series_position)
            .bind(genres_json.as_deref())
            .bind(u.language.as_deref().filter(|s| !s.is_empty()))
            .bind(u.page_count)
            .bind(u.duration_seconds)
            .bind(u.publisher.as_deref())
            .bind(u.publish_date.as_deref())
            .bind(narrator_json.as_deref())
            .bind(narration_type_val)
            .bind(u.abridged)
            .bind(u.rating)
            .bind(u.rating_count)
            .bind(absolute_http_cover_url(
                u.cover_url.as_deref().filter(|s| !s.is_empty()),
            ))
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
                    | ProvenanceSetter::Imported
                    | ProvenanceSetter::Import => {
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

        // REQ-007: the merge never writes external_ids — the identity track
        // owns all three anchor stores.

        tx.commit().await.map_err(map_db_err)?;
        Ok(ApplyMergeOutcome::Applied)
    }

    async fn reset_for_manual_refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<(), DbError> {
        // Recovering a terminal `not_found` identity (the LLM rejected all payloads):
        // a manual refresh re-derives identity from the work's anchors so it can
        // re-resolve + re-enrich. Other identity states are left untouched — an open
        // `conflict` is a data dispute a refresh must not silently clear, and
        // confirmed/provisional/pending already re-enrich on their own. Clearing
        // next_convergence_at re-derives the work as due-now for the background loop.
        let result = sqlx::query(
            "UPDATE works SET enrichment_status = 'pending', enriched_at = NULL, \
             merge_generation = merge_generation + 1, \
             next_convergence_at = NULL, \
             identity_status = CASE \
                 WHEN identity_status = 'not_found' AND (ol_key IS NOT NULL OR gr_key IS NOT NULL OR hc_key IS NOT NULL) THEN 'confirmed' \
                 WHEN identity_status = 'not_found' AND (isbn_13 IS NOT NULL OR asin IS NOT NULL) THEN 'provisional' \
                 WHEN identity_status = 'not_found' THEN 'pending' \
                 ELSE identity_status \
             END \
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

    async fn list_convergence_due(
        &self,
        user_id: UserId,
        now: DateTime<Utc>,
        threshold: u32,
        limit: i64,
    ) -> Result<Vec<WorkId>, DbError> {
        // Branch (1) identity-pending and branch (2) enrichment-incomplete select
        // independently; the chaseable-missing-anchor guard scopes ONLY branch (3)
        // ID-chasing, so a fully-anchored work with failed enrichment is still
        // selected via (2). A missing anchor is chaseable when it has neither an
        // outstanding pending guess nor a dead-end at the configured threshold; a
        // work is selected for ID-chasing if it has at least one such anchor.
        let now_str = now.to_rfc3339();
        let threshold = i64::from(threshold);
        let ids: Vec<WorkId> = sqlx::query_scalar(
            "SELECT w.id FROM works w
             WHERE w.user_id = ?1
               AND (
                     w.identity_status = 'pending'
                     OR (w.identity_status IN ('confirmed','provisional')
                         AND w.enrichment_status NOT IN ('enriched','thin'))
                     OR (w.identity_status IN ('confirmed','provisional') AND (
                           (w.ol_key IS NULL
                              AND NOT EXISTS (SELECT 1 FROM work_identity_anchors a WHERE a.work_id = w.id AND a.anchor_type = 'ol_work' AND a.confidence = 'pending')
                              AND NOT EXISTS (SELECT 1 FROM work_anchor_dead_ends d WHERE d.work_id = w.id AND d.anchor_type = 'ol_work' AND d.attempt_count >= ?3))
                        OR (w.gr_key IS NULL
                              AND NOT EXISTS (SELECT 1 FROM work_identity_anchors a WHERE a.work_id = w.id AND a.anchor_type = 'gr_work' AND a.confidence = 'pending')
                              AND NOT EXISTS (SELECT 1 FROM work_anchor_dead_ends d WHERE d.work_id = w.id AND d.anchor_type = 'gr_work' AND d.attempt_count >= ?3))
                        OR (w.hc_key IS NULL
                              AND NOT EXISTS (SELECT 1 FROM work_identity_anchors a WHERE a.work_id = w.id AND a.anchor_type = 'hc_work' AND a.confidence = 'pending')
                              AND NOT EXISTS (SELECT 1 FROM work_anchor_dead_ends d WHERE d.work_id = w.id AND d.anchor_type = 'hc_work' AND d.attempt_count >= ?3))
                        OR (w.isbn_13 IS NULL
                              AND NOT EXISTS (SELECT 1 FROM work_identity_anchors a WHERE a.work_id = w.id AND a.anchor_type = 'isbn_13' AND a.confidence = 'pending')
                              AND NOT EXISTS (SELECT 1 FROM work_anchor_dead_ends d WHERE d.work_id = w.id AND d.anchor_type = 'isbn_13' AND d.attempt_count >= ?3))
                        OR (w.asin IS NULL
                              AND NOT EXISTS (SELECT 1 FROM work_identity_anchors a WHERE a.work_id = w.id AND a.anchor_type = 'asin' AND a.confidence = 'pending')
                              AND NOT EXISTS (SELECT 1 FROM work_anchor_dead_ends d WHERE d.work_id = w.id AND d.anchor_type = 'asin' AND d.attempt_count >= ?3))
                     ))
                   )
               AND (w.next_convergence_at IS NULL OR w.next_convergence_at <= ?2)
             ORDER BY w.added_at ASC
             LIMIT ?4",
        )
        .bind(user_id)
        .bind(&now_str)
        .bind(threshold)
        .bind(limit)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(ids)
    }

    async fn set_next_convergence_at(
        &self,
        user_id: UserId,
        work_id: WorkId,
        at: Option<DateTime<Utc>>,
    ) -> Result<(), DbError> {
        let at_str = at.map(|dt| dt.to_rfc3339());
        sqlx::query("UPDATE works SET next_convergence_at = ?1 WHERE id = ?2 AND user_id = ?3")
            .bind(at_str)
            .bind(work_id)
            .bind(user_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn list_conflict_works(&self, user_id: UserId) -> Result<Vec<Work>, DbError> {
        // Works with an unresolved identity problem the user must act on: an open
        // anchor `conflict` or an unverifiable `not_found` (the LLM rejected all
        // payloads — formerly enrichment_status='conflict', now identity_status).
        let rows = sqlx::query(
            "SELECT * FROM works WHERE user_id = ? AND identity_status IN ('conflict', 'not_found') ORDER BY id",
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

    async fn set_work_series_id(
        &self,
        user_id: UserId,
        work_id: WorkId,
        series_id: Option<i64>,
    ) -> Result<(), DbError> {
        sqlx::query("UPDATE works SET series_id = ? WHERE id = ? AND user_id = ?")
            .bind(series_id)
            .bind(work_id)
            .bind(user_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn normalize_work_series_fields(
        &self,
        user_id: UserId,
        work_id: WorkId,
        series_name: &str,
        series_position: Option<f64>,
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE works SET series_name = ?, \
             series_position = COALESCE(series_position, ?) \
             WHERE id = ? AND user_id = ?",
        )
        .bind(series_name)
        .bind(series_position)
        .bind(work_id)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(())
    }

    async fn list_orphan_series_works_all_users(&self) -> Result<Vec<Work>, DbError> {
        let rows = sqlx::query(
            "SELECT * FROM works WHERE series_id IS NULL \
             AND series_name IS NOT NULL AND series_name != '' \
             AND author_id IS NOT NULL ORDER BY user_id, author_id, series_name",
        )
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;
        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            match row_to_work(row) {
                Ok(w) => results.push(w),
                Err(e) => {
                    tracing::warn!("works: skipping corrupt row in orphan-series list: {e}");
                }
            }
        }
        Ok(results)
    }
}

impl crate::WorkDbCreate for SqliteDb {
    async fn create_work(&self, req: CreateWorkDbRequest) -> Result<(Work, bool), DbError> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            "INSERT INTO works (user_id, title, author_name, normalized_title, normalized_author, \
             author_id, ol_key, gr_key, year, cover_url, enrichment_status, added_at, \
             language, import_id, series_id, series_name, series_position, \
             monitor_ebook, monitor_audiobook, isbn_13, asin, description, cover_manual) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'unenriched', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(user_id, normalized_title, normalized_author) DO NOTHING",
        )
        .bind(req.user_id)
        .bind(&req.title)
        .bind(&req.author_name)
        .bind(&req.normalized_title)
        .bind(&req.normalized_author)
        .bind(req.author_id)
        .bind(&req.ol_key)
        .bind(&req.gr_key)
        .bind(req.year)
        .bind(absolute_http_cover_url(req.cover_url.as_deref()))
        .bind(&now)
        .bind(req.language.as_deref())
        .bind(&req.import_id)
        .bind(req.series_id)
        .bind(&req.series_name)
        .bind(req.series_position)
        .bind(req.monitor_ebook)
        .bind(req.monitor_audiobook)
        .bind(&req.isbn_13)
        .bind(&req.asin)
        .bind(&req.description)
        .bind(req.cover_manual)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        if result.rows_affected() == 1 {
            // Newly inserted — last_insert_rowid is the new ID.
            let id = result.last_insert_rowid();
            let work = self.get_work(req.user_id, id).await?;
            Ok((work, true))
        } else {
            // Conflict — fetch the existing row.
            let row = sqlx::query(
                "SELECT * FROM works WHERE user_id = ? AND normalized_title = ? AND normalized_author = ?",
            )
            .bind(req.user_id)
            .bind(&req.normalized_title)
            .bind(&req.normalized_author)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;
            Ok((row_to_work(row)?, false))
        }
    }

    async fn create_work_with_anchor(
        &self,
        req: CreateWorkDbRequest,
        ol_key: &str,
        anchor_setter: livrarr_domain::identity::AnchorSetter,
    ) -> Result<(Work, bool), DbError> {
        use livrarr_domain::services::WorkIdentityRepository;

        let (work, created) = self.create_work(req).await?;
        if created {
            self.confirm_anchor(
                work.id,
                livrarr_domain::identity::AnchorType::new(
                    livrarr_domain::identity::AnchorType::OL_WORK,
                ),
                ol_key,
                anchor_setter,
            )
            .await
            .map_err(|e| DbError::Constraint {
                message: format!("anchor write failed: {e}"),
            })?;
            let work = self.get_work(work.user_id, work.id).await?;
            Ok((work, true))
        } else {
            Ok((work, false))
        }
    }
}

impl crate::EnrichmentRetryDb for SqliteDb {
    async fn list_works_for_retry(&self) -> Result<Vec<Work>, crate::DbError> {
        let rows = sqlx::query(
            "SELECT * FROM works WHERE enrichment_status = 'failed' \
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
