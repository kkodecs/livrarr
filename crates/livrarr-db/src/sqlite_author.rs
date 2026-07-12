use chrono::Utc;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, parse_dt};
use crate::sqlite_series::row_to_series;
use crate::{
    Author, AuthorDb, AuthorId, CreateAuthorDbRequest, DbError, Series, UpdateAuthorDbRequest,
    UserId,
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
        import_id: row
            .try_get("import_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        monitor_language: row
            .try_get("monitor_language")
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

impl SqliteDb {
    /// The language to stamp on a newly-monitored author with no prior choice:
    /// the dominant language among the author's library works, else "en"
    /// (REQ-003 Q-001, shared rule via `livrarr_domain::seed::dominant_language`).
    async fn monitored_default_language_for_author(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<String, DbError> {
        let langs: Vec<Option<String>> =
            sqlx::query_scalar("SELECT language FROM works WHERE user_id = ? AND author_id = ?")
                .bind(user_id)
                .bind(author_id)
                .fetch_all(self.pool())
                .await
                .map_err(map_db_err)?;
        Ok(
            livrarr_domain::seed::dominant_language(langs.iter().map(|l| l.as_deref()))
                .unwrap_or_else(|| livrarr_domain::seed::DEFAULT_SEED_LANGUAGE.to_string()),
        )
    }
}

impl AuthorDb for SqliteDb {
    async fn merge_authors(
        &self,
        user_id: UserId,
        survivor_id: AuthorId,
        loser_id: AuthorId,
    ) -> Result<livrarr_domain::services::AuthorMergeReport, DbError> {
        if survivor_id == loser_id {
            return Err(DbError::Constraint {
                message: "cannot merge an author into itself".to_string(),
            });
        }

        let mut tx = self.pool().begin().await.map_err(map_db_err)?;

        // 1. validate: both exist, scoped to this user.
        let survivor_row = sqlx::query("SELECT * FROM authors WHERE id = ? AND user_id = ?")
            .bind(survivor_id)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_db_err)?;
        let survivor = row_to_author(survivor_row)?;

        let loser_row = sqlx::query("SELECT * FROM authors WHERE id = ? AND user_id = ?")
            .bind(loser_id)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_db_err)?;
        let loser = row_to_author(loser_row)?;

        // 2. works: repoint author_id + the display author_name, bump
        // merge_generation so tag convergence re-syncs file tags to the
        // survivor spelling (normalized_author is UNTOUCHED — D-3).
        let works_moved = sqlx::query(
            "UPDATE works SET author_id = ?, author_name = ?, merge_generation = merge_generation + 1 \
             WHERE author_id = ? AND user_id = ?",
        )
        .bind(survivor_id)
        .bind(&survivor.name)
        .bind(loser_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(map_db_err)?
        .rows_affected();

        // 3. series: fold each loser row onto a same-gr_key survivor row,
        // else move it. Repoint-before-delete throughout — series has
        // ON DELETE CASCADE on author_id and works.series_id has
        // ON DELETE SET NULL, so an un-repointed row would silently wipe
        // itself or unlink works when the loser author is deleted in step 6.
        let loser_series_rows =
            sqlx::query("SELECT * FROM series WHERE user_id = ? AND author_id = ?")
                .bind(user_id)
                .bind(loser_id)
                .fetch_all(&mut *tx)
                .await
                .map_err(map_db_err)?
                .into_iter()
                .map(row_to_series)
                .collect::<Result<Vec<Series>, DbError>>()?;

        let mut series_moved = 0u64;
        let mut series_folded = 0u64;

        for loser_series in loser_series_rows {
            let survivor_match = sqlx::query(
                "SELECT * FROM series WHERE user_id = ? AND author_id = ? AND gr_key = ?",
            )
            .bind(user_id)
            .bind(survivor_id)
            .bind(&loser_series.gr_key)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_db_err)?
            .map(row_to_series)
            .transpose()?;

            match survivor_match {
                Some(survivor_series) => {
                    // FOLD: same gr_key already tracked under the survivor.
                    let merged_ebook = survivor_series.monitor_ebook || loser_series.monitor_ebook;
                    let merged_audiobook =
                        survivor_series.monitor_audiobook || loser_series.monitor_audiobook;
                    let mut merged_language = survivor_series
                        .monitor_language
                        .clone()
                        .or(loser_series.monitor_language.clone());
                    // monitored-series⇒language invariant, re-enforced here
                    // since this write path bypasses
                    // update_series_flags/upsert_series where it normally
                    // lives.
                    if (merged_ebook || merged_audiobook) && merged_language.is_none() {
                        merged_language = Some("en".to_string());
                    }
                    let flags_changed = merged_ebook != survivor_series.monitor_ebook
                        || merged_audiobook != survivor_series.monitor_audiobook;

                    sqlx::query(
                        "UPDATE series SET monitor_ebook = ?, monitor_audiobook = ?, monitor_language = ? \
                         WHERE id = ? AND user_id = ?",
                    )
                    .bind(merged_ebook)
                    .bind(merged_audiobook)
                    .bind(&merged_language)
                    .bind(survivor_series.id)
                    .bind(user_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(map_db_err)?;

                    // Roster: keep the survivor's own; adopt the loser's
                    // only when the survivor has none (otherwise the
                    // loser's cascades away with its series row below).
                    sqlx::query(
                        "UPDATE series_roster SET series_id = ? \
                         WHERE series_id = ? AND NOT EXISTS ( \
                             SELECT 1 FROM series_roster WHERE series_id = ? \
                         )",
                    )
                    .bind(survivor_series.id)
                    .bind(loser_series.id)
                    .bind(survivor_series.id)
                    .execute(&mut *tx)
                    .await
                    .map_err(map_db_err)?;

                    // Work-flag propagation mirrors the two EXISTING write
                    // paths: a flag change restamps every work under the
                    // survivor series (update_series_flags semantics); no
                    // change stamps only the just-repointed loser works
                    // (link_work_to_series semantics), leaving pre-existing
                    // survivor works untouched.
                    if flags_changed {
                        sqlx::query(
                            "UPDATE works SET series_id = ? WHERE series_id = ? AND user_id = ?",
                        )
                        .bind(survivor_series.id)
                        .bind(loser_series.id)
                        .bind(user_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(map_db_err)?;

                        sqlx::query(
                            "UPDATE works SET monitor_ebook = ?, monitor_audiobook = ? \
                             WHERE series_id = ? AND user_id = ?",
                        )
                        .bind(merged_ebook)
                        .bind(merged_audiobook)
                        .bind(survivor_series.id)
                        .bind(user_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(map_db_err)?;
                    } else {
                        sqlx::query(
                            "UPDATE works SET series_id = ?, monitor_ebook = ?, monitor_audiobook = ? \
                             WHERE series_id = ? AND user_id = ?",
                        )
                        .bind(survivor_series.id)
                        .bind(merged_ebook)
                        .bind(merged_audiobook)
                        .bind(loser_series.id)
                        .bind(user_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(map_db_err)?;
                    }

                    sqlx::query("DELETE FROM series WHERE id = ? AND user_id = ?")
                        .bind(loser_series.id)
                        .bind(user_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(map_db_err)?;

                    series_folded += 1;
                }
                None => {
                    // MOVE: no gr_key collision — the row travels intact,
                    // its own language and roster untouched.
                    sqlx::query("UPDATE series SET author_id = ? WHERE id = ? AND user_id = ?")
                        .bind(survivor_id)
                        .bind(loser_series.id)
                        .bind(user_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(map_db_err)?;

                    series_moved += 1;
                }
            }
        }

        // 4. caches: refetchable, drop the loser's.
        sqlx::query("DELETE FROM author_series_cache WHERE author_id = ?")
            .bind(loser_id)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;
        sqlx::query("DELETE FROM author_bibliography WHERE author_id = ?")
            .bind(loser_id)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;

        // 5. author fields: monotonic merge onto the survivor row.
        let monitored = survivor.monitored || loser.monitored;
        let monitor_new_items = survivor.monitor_new_items || loser.monitor_new_items;
        let monitor_since = survivor
            .monitor_since
            .into_iter()
            .chain(loser.monitor_since)
            .min();
        let mut monitor_language = survivor
            .monitor_language
            .clone()
            .or(loser.monitor_language.clone());
        if monitored && monitor_language.is_none() {
            // seed::dominant_language over the survivor's works, computable
            // here because step 2 already reassigned them in this
            // transaction; else the shared "en" default (insight 53).
            let langs: Vec<Option<String>> = sqlx::query_scalar(
                "SELECT language FROM works WHERE user_id = ? AND author_id = ?",
            )
            .bind(user_id)
            .bind(survivor_id)
            .fetch_all(&mut *tx)
            .await
            .map_err(map_db_err)?;
            monitor_language = Some(
                livrarr_domain::seed::dominant_language(langs.iter().map(|l| l.as_deref()))
                    .unwrap_or_else(|| livrarr_domain::seed::DEFAULT_SEED_LANGUAGE.to_string()),
            );
        }

        sqlx::query(
            "UPDATE authors SET \
             sort_name = COALESCE(sort_name, ?), \
             ol_key = COALESCE(ol_key, ?), \
             gr_key = COALESCE(gr_key, ?), \
             hc_key = COALESCE(hc_key, ?), \
             import_id = COALESCE(import_id, ?), \
             monitored = ?, monitor_new_items = ?, monitor_since = ?, monitor_language = ? \
             WHERE id = ? AND user_id = ?",
        )
        .bind(&loser.sort_name)
        .bind(&loser.ol_key)
        .bind(&loser.gr_key)
        .bind(&loser.hc_key)
        .bind(&loser.import_id)
        .bind(monitored)
        .bind(monitor_new_items)
        .bind(monitor_since.map(|dt| dt.to_rfc3339()))
        .bind(&monitor_language)
        .bind(survivor_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(map_db_err)?;

        // 6. delete the loser row — step 3 handled every loser series row
        // (fold or move), so no series row remains for this delete's
        // CASCADE to touch, and works.author_id was repointed in step 2.
        let deleted = sqlx::query("DELETE FROM authors WHERE id = ? AND user_id = ?")
            .bind(loser_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;
        if deleted.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "author" });
        }

        tx.commit().await.map_err(map_db_err)?;

        Ok(livrarr_domain::services::AuthorMergeReport {
            works_moved,
            series_moved,
            series_folded,
        })
    }

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
        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            match row_to_author(row) {
                Ok(a) => results.push(a),
                Err(e) => {
                    tracing::warn!("authors: skipping corrupt row: {e}");
                }
            }
        }
        Ok(results)
    }

    async fn create_author(&self, req: CreateAuthorDbRequest) -> Result<Author, DbError> {
        let now = Utc::now().to_rfc3339();
        let id = sqlx::query(
            "INSERT INTO authors (user_id, name, sort_name, ol_key, gr_key, hc_key, import_id, added_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(req.user_id)
        .bind(&req.name)
        .bind(&req.sort_name)
        .bind(&req.ol_key)
        .bind(&req.gr_key)
        .bind(&req.hc_key)
        .bind(&req.import_id)
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
        let sort_name = match req.sort_name {
            None => current.sort_name,
            Some(v) => v,
        };
        let ol_key = match req.ol_key {
            None => current.ol_key,
            Some(v) => v,
        };
        let gr_key = match req.gr_key {
            None => current.gr_key,
            Some(v) => v,
        };
        let monitored = req.monitored.unwrap_or(current.monitored);
        let monitor_new_items = req.monitor_new_items.unwrap_or(current.monitor_new_items);
        let monitor_since = req.monitor_since.or(current.monitor_since);
        // Resolve the chosen value first: an explicit set or explicit clear
        // (Some(_)) wins; an absent field (None) preserves the current value.
        let chosen = match req.monitor_language {
            Some(v) => v,
            None => current.monitor_language,
        };
        // Enable guard (REQ-002/REQ-003): "monitored ⇒ never NULL" is absolute.
        // Whenever the author ends up monitored with no language — unset OR
        // explicitly cleared — the smart default (dominant library language,
        // else "en") is persisted. This is the one place the invariant holds,
        // independent of which UI surface flipped the toggle.
        let monitor_language = if monitored && chosen.is_none() {
            Some(
                self.monitored_default_language_for_author(user_id, id)
                    .await?,
            )
        } else {
            chosen
        };

        sqlx::query(
            "UPDATE authors SET name = ?, sort_name = ?, ol_key = ?, gr_key = ?, \
             monitored = ?, monitor_new_items = ?, monitor_since = ?, monitor_language = ? \
             WHERE id = ? AND user_id = ?",
        )
        .bind(&name)
        .bind(&sort_name)
        .bind(&ol_key)
        .bind(&gr_key)
        .bind(monitored)
        .bind(monitor_new_items)
        .bind(monitor_since.map(|dt| dt.to_rfc3339()))
        .bind(&monitor_language)
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

    async fn list_monitored_authors(&self, user_id: UserId) -> Result<Vec<Author>, DbError> {
        let rows = sqlx::query(
            "SELECT * FROM authors WHERE user_id = ? AND monitored = 1 AND ol_key IS NOT NULL ORDER BY id",
        )
        .bind(user_id)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;
        rows.into_iter().map(row_to_author).collect()
    }
}
