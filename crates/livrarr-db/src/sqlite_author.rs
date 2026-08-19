use chrono::Utc;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, parse_dt};
use crate::sqlite_series::row_to_series;
use crate::{
    Author, AuthorDb, AuthorId, AuthorLinkDb, CreateAuthorDbRequest, CreateAuthorGateRequest,
    DbError, Series, UpdateAuthorDbRequest, UserId,
};

pub(crate) fn row_to_author(row: sqlx::sqlite::SqliteRow) -> Result<Author, DbError> {
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

    /// The shared author create/adopt gate: one transaction that converges a
    /// creation race onto a single author row and leaves that winner with its
    /// initial name variant and a due author-link progress row.
    ///
    /// Every F1 add path (interactive add, standalone author add, manual
    /// import, list import, series monitor, Readarr import) enters here, so an
    /// author can never be committed in a state the sweep cannot see. The
    /// pre-F1 [`AuthorDb::create_author`] writer stays as it is for legacy and
    /// fixture use; [`AuthorLinkDb::ensure_enqueued`] is what repairs an author
    /// that predates this gate.
    ///
    /// The operation itself is [`AuthorLinkDb::create_or_adopt_author`] — the
    /// trait seam the generic add services reach it through.
    pub(crate) async fn create_or_adopt_author_gate(
        &self,
        request: CreateAuthorGateRequest,
    ) -> Result<(Author, bool), DbError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(map_db_err)?;
        let converged =
            crate::sqlite_author_link::create_or_adopt_author_tx(&mut tx, &request).await?;
        tx.commit().await.map_err(map_db_err)?;
        Ok(converged)
    }
}

/// In-transaction body of [`AuthorDb::merge_authors`] — every statement of
/// the merge contract (works repoint, series fold/move, cache drops,
/// monotonic author fields, loser delete) on the caller's own
/// connection/transaction. Shared between the live merge endpoint and the
/// startup author-identity repair so both paths apply the identical policy
/// (the same factoring as `merge_user_identity_state` in pool.rs).
pub(crate) async fn merge_authors_tx(
    conn: &mut sqlx::SqliteConnection,
    user_id: UserId,
    survivor_id: AuthorId,
    loser_id: AuthorId,
) -> Result<livrarr_domain::services::AuthorMergeReport, DbError> {
    // 1. validate: both exist, scoped to this user.
    let survivor_row = sqlx::query("SELECT * FROM authors WHERE id = ? AND user_id = ?")
        .bind(survivor_id)
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(map_db_err)?;
    let survivor = row_to_author(survivor_row)?;

    let loser_row = sqlx::query("SELECT * FROM authors WHERE id = ? AND user_id = ?")
        .bind(loser_id)
        .bind(user_id)
        .fetch_one(&mut *conn)
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
    .execute(&mut *conn)
    .await
    .map_err(map_db_err)?
    .rows_affected();

    // 3. series: fold each loser row onto a same-gr_key survivor row,
    // else move it. Repoint-before-delete throughout — series has
    // ON DELETE CASCADE on author_id and works.series_id has
    // ON DELETE SET NULL, so an un-repointed row would silently wipe
    // itself or unlink works when the loser author is deleted in step 6.
    let loser_series_rows = sqlx::query("SELECT * FROM series WHERE user_id = ? AND author_id = ?")
        .bind(user_id)
        .bind(loser_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(map_db_err)?
        .into_iter()
        .map(row_to_series)
        .collect::<Result<Vec<Series>, DbError>>()?;

    let mut series_moved = 0u64;
    let mut series_folded = 0u64;

    for loser_series in loser_series_rows {
        let survivor_match =
            sqlx::query("SELECT * FROM series WHERE user_id = ? AND author_id = ? AND gr_key = ?")
                .bind(user_id)
                .bind(survivor_id)
                .bind(&loser_series.gr_key)
                .fetch_optional(&mut *conn)
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
                .execute(&mut *conn)
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
                .execute(&mut *conn)
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
                    .execute(&mut *conn)
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
                    .execute(&mut *conn)
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
                    .execute(&mut *conn)
                    .await
                    .map_err(map_db_err)?;
                }

                sqlx::query("DELETE FROM series WHERE id = ? AND user_id = ?")
                    .bind(loser_series.id)
                    .bind(user_id)
                    .execute(&mut *conn)
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
                    .execute(&mut *conn)
                    .await
                    .map_err(map_db_err)?;

                series_moved += 1;
            }
        }
    }

    // 4. caches: refetchable, drop the loser's.
    sqlx::query("DELETE FROM author_series_cache WHERE author_id = ?")
        .bind(loser_id)
        .execute(&mut *conn)
        .await
        .map_err(map_db_err)?;
    sqlx::query("DELETE FROM author_bibliography WHERE author_id = ?")
        .bind(loser_id)
        .execute(&mut *conn)
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
        let langs: Vec<Option<String>> =
            sqlx::query_scalar("SELECT language FROM works WHERE user_id = ? AND author_id = ?")
                .bind(user_id)
                .bind(survivor_id)
                .fetch_all(&mut *conn)
                .await
                .map_err(map_db_err)?;
        monitor_language = Some(
            livrarr_domain::seed::dominant_language(langs.iter().map(|l| l.as_deref()))
                .unwrap_or_else(|| livrarr_domain::seed::DEFAULT_SEED_LANGUAGE.to_string()),
        );
    }

    // The provider key columns are deliberately absent: they are frozen legacy
    // compatibility input after the cutover (FP-031), and the loser's linkage
    // moves as route rows in step 5b below, where a tombstone still wins.
    sqlx::query(
        "UPDATE authors SET \
         sort_name = COALESCE(sort_name, ?), \
         import_id = COALESCE(import_id, ?), \
         monitored = ?, monitor_new_items = ?, monitor_since = ?, monitor_language = ? \
         WHERE id = ? AND user_id = ?",
    )
    .bind(&loser.sort_name)
    .bind(&loser.import_id)
    .bind(monitored)
    .bind(monitor_new_items)
    .bind(monitor_since.map(|dt| dt.to_rfc3339()))
    .bind(&monitor_language)
    .bind(survivor_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await
    .map_err(map_db_err)?;

    // 5b. author-link state: routes, name variants, candidates, and progress
    // fold onto the survivor before the delete below cascades the loser's rows
    // away. Survivor tombstones are never resurrected.
    crate::sqlite_author_link::fold_author_link_state_tx(conn, user_id, survivor_id, loser_id)
        .await?;

    // 6. delete the loser row — step 3 handled every loser series row
    // (fold or move), so no series row remains for this delete's
    // CASCADE to touch, and works.author_id was repointed in step 2.
    let deleted = sqlx::query("DELETE FROM authors WHERE id = ? AND user_id = ?")
        .bind(loser_id)
        .bind(user_id)
        .execute(&mut *conn)
        .await
        .map_err(map_db_err)?;
    if deleted.rows_affected() == 0 {
        return Err(DbError::NotFound { entity: "author" });
    }

    Ok(livrarr_domain::services::AuthorMergeReport {
        works_moved,
        series_moved,
        series_folded,
    })
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

        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(map_db_err)?;
        let report = merge_authors_tx(&mut tx, user_id, survivor_id, loser_id).await?;
        tx.commit().await.map_err(map_db_err)?;
        Ok(report)
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

    async fn create_author(&self, req: CreateAuthorDbRequest) -> Result<(Author, bool), DbError> {
        let now = Utc::now().to_rfc3339();
        // Rust-computed stored key (never SQL — SQLite LOWER is ASCII-only);
        // a non-canonicalizable name stores NULL, exempt from the unique
        // index by SQLite NULL-distinct semantics (ST-010) — never "".
        let key = livrarr_domain::identity_matching::canonical_author_key(&req.name);
        let normalized_name = (!key.is_empty()).then_some(key);
        // The conflict target names the partial idx_authors_identity index
        // (the WHERE clause is required for a partial-index target); a
        // same-key concurrent insert lands on DO NOTHING and the winner is
        // re-selected below — mirrors `create_work`.
        let result = sqlx::query(
            "INSERT INTO authors (user_id, name, sort_name, ol_key, gr_key, hc_key, import_id, \
             added_at, normalized_name) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(user_id, normalized_name) WHERE normalized_name IS NOT NULL DO NOTHING",
        )
        .bind(req.user_id)
        .bind(&req.name)
        .bind(&req.sort_name)
        .bind(&req.ol_key)
        .bind(&req.gr_key)
        .bind(&req.hc_key)
        .bind(&req.import_id)
        .bind(&now)
        .bind(&normalized_name)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        if result.rows_affected() == 1 {
            let author = self
                .get_author(req.user_id, result.last_insert_rowid())
                .await?;
            Ok((author, true))
        } else {
            let row =
                sqlx::query("SELECT * FROM authors WHERE user_id = ? AND normalized_name = ?")
                    .bind(req.user_id)
                    .bind(&normalized_name)
                    .fetch_one(self.pool())
                    .await
                    .map_err(map_db_err)?;
            Ok((row_to_author(row)?, false))
        }
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

        // The stored identity key follows every name write in the same
        // statement (REQ-004); a non-canonicalizable resolved name stores
        // NULL (ST-010). The write is a single UPDATE, so a unique
        // violation leaves no partial state.
        let key = livrarr_domain::identity_matching::canonical_author_key(&name);
        let normalized_name = (!key.is_empty()).then_some(key);

        let update = sqlx::query(
            "UPDATE authors SET name = ?, normalized_name = ?, sort_name = ?, ol_key = ?, \
             gr_key = ?, monitored = ?, monitor_new_items = ?, monitor_since = ?, \
             monitor_language = ? \
             WHERE id = ? AND user_id = ?",
        )
        .bind(&name)
        .bind(&normalized_name)
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
        .await;

        match update {
            Ok(_) => {}
            Err(sqlx::Error::Database(ref db_err)) if db_err.is_unique_violation() => {
                // The recomputed key is already held by a different row —
                // surface WHICH row (REQ-004: the error names the collider;
                // the recovery for an intended merge is the merge endpoint).
                let collider_row = sqlx::query(
                    "SELECT * FROM authors WHERE user_id = ? AND normalized_name = ? AND id != ?",
                )
                .bind(user_id)
                .bind(&normalized_name)
                .bind(id)
                .fetch_one(self.pool())
                .await
                .map_err(map_db_err)?;
                let collider = row_to_author(collider_row)?;
                return Err(DbError::IdentityCollision {
                    entity: "author",
                    id: collider.id,
                    name: collider.name,
                });
            }
            Err(e) => return Err(map_db_err(e)),
        }

        self.get_author(user_id, id).await
    }

    async fn delete_author(&self, user_id: UserId, id: AuthorId) -> Result<(), DbError> {
        // Contributor credits are historical presentation data, not ownership
        // of an Author row. Remove only this author's scoped credits first so
        // the intentional RESTRICT FK cannot turn the author-delete door into
        // a 500. Contributor roles cascade from those rows; sibling credits
        // and the Work itself remain untouched, and the legacy/primary author
        // FKs degrade through their SET NULL rules. One transaction prevents
        // an ownership miss from stripping any credits.
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(map_db_err)?;
        sqlx::query("DELETE FROM work_contributors WHERE user_id = ? AND author_id = ?")
            .bind(user_id)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;

        let result = sqlx::query("DELETE FROM authors WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "author" });
        }
        tx.commit().await.map_err(map_db_err)?;
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

    /// Monitored authors that have at least one *active OpenLibrary* route,
    /// grouped one target per author. Plural OL routes widen a target's feed
    /// coverage; they never turn one author into several monitor entries.
    async fn list_author_monitor_targets(
        &self,
        user_id: UserId,
    ) -> Result<Vec<livrarr_domain::AuthorMonitorTarget>, DbError> {
        let authors = sqlx::query(
            "SELECT * FROM authors a \
              WHERE a.user_id = ? AND a.monitored = 1 \
                AND EXISTS (SELECT 1 FROM author_provider_routes r \
                             WHERE r.user_id = a.user_id AND r.author_id = a.id \
                               AND r.provider = 'open_library' AND r.state = 'active') \
              ORDER BY a.id",
        )
        .bind(user_id)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;

        let mut targets = Vec::with_capacity(authors.len());
        for row in authors {
            let author = row_to_author(row)?;
            let ol_routes = self
                .list_active_routes(
                    user_id,
                    author.id,
                    Some(livrarr_domain::AuthorProvider::OpenLibrary),
                )
                .await?;
            targets.push(livrarr_domain::AuthorMonitorTarget { author, ol_routes });
        }
        Ok(targets)
    }

    async fn rename_author_and_cascade(
        &self,
        request: crate::RenameAuthorDbRequest,
    ) -> Result<Author, DbError> {
        self.display_name_cascade(request, DisplayNameOrigin::UserChoice)
            .await
    }

    async fn converge_author_display_name(
        &self,
        request: crate::RenameAuthorDbRequest,
    ) -> Result<Author, DbError> {
        self.display_name_cascade(request, DisplayNameOrigin::AutomaticConvergence)
            .await
    }
}

/// Who chose the display name a cascade is about to commit.
///
/// The name change itself is identical either way. What differs is user
/// authority: only a user's own choice is recorded as one, because a stamp
/// written on the machine's behalf would outrank every provider name the
/// ranking might later prefer and freeze convergence on the first guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayNameOrigin {
    UserChoice,
    AutomaticConvergence,
}

impl SqliteDb {
    /// The one display-name cascade, shared by rename, stored-variant pick, and
    /// automatic convergence.
    ///
    /// It changes what the library *shows* — `authors.name`, `works.author_name`
    /// — and bumps `merge_generation` so tag convergence re-syncs file tags. It
    /// never touches `works.normalized_author`: matching identity is not a
    /// display concern, and rewriting it here would silently re-key the library.
    async fn display_name_cascade(
        &self,
        request: crate::RenameAuthorDbRequest,
        origin: DisplayNameOrigin,
    ) -> Result<Author, DbError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(map_db_err)?;

        let owner: Option<i64> = sqlx::query_scalar("SELECT user_id FROM authors WHERE id = ?")
            .bind(request.author_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_db_err)?;
        if owner != Some(request.user_id) {
            return Err(DbError::NotFound { entity: "author" });
        }

        // A variant id selects a stored spelling; id 0 means the caller supplied
        // the display string directly.
        let display_name = if request.variant_id == 0 {
            request.display_name.trim().to_string()
        } else {
            sqlx::query_scalar::<_, String>(
                "SELECT name FROM author_name_variants \
                  WHERE id = ? AND user_id = ? AND author_id = ?",
            )
            .bind(request.variant_id)
            .bind(request.user_id)
            .bind(request.author_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_db_err)?
            .ok_or(DbError::NotFound {
                entity: "author name variant",
            })?
        };
        if display_name.is_empty() {
            return Err(DbError::Constraint {
                message: "author display name must not be empty".to_string(),
            });
        }

        let canonical = livrarr_domain::identity_matching::canonical_author_key(&display_name);
        if origin == DisplayNameOrigin::UserChoice {
            record_user_display_choice_tx(
                &mut tx,
                &request,
                &display_name,
                &canonical,
                &Utc::now().to_rfc3339(),
            )
            .await?;
        }

        let normalized_name = (!canonical.is_empty()).then_some(canonical);
        sqlx::query(
            "UPDATE authors SET name = ?, normalized_name = ? WHERE id = ? AND user_id = ?",
        )
        .bind(&display_name)
        .bind(&normalized_name)
        .bind(request.author_id)
        .bind(request.user_id)
        .execute(&mut *tx)
        .await
        .map_err(map_db_err)?;

        sqlx::query(
            "UPDATE works SET author_name = ?, merge_generation = merge_generation + 1 \
              WHERE author_id = ? AND user_id = ?",
        )
        .bind(&display_name)
        .bind(request.author_id)
        .bind(request.user_id)
        .execute(&mut *tx)
        .await
        .map_err(map_db_err)?;

        let row = sqlx::query("SELECT * FROM authors WHERE id = ? AND user_id = ?")
            .bind(request.author_id)
            .bind(request.user_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_db_err)?;
        let author = row_to_author(row)?;
        tx.commit().await.map_err(map_db_err)?;
        Ok(author)
    }
}

/// Record that the display name about to be committed is the user's own choice.
///
/// Exactly one variant carries the selection, so the stamp is cleared across the
/// author's variants first. A stored spelling the user picked is marked in place
/// — the provider observation that produced it stays intact with its source —
/// while a name the user typed exists only as the single `User` variant this
/// writes.
async fn record_user_display_choice_tx(
    tx: &mut sqlx::SqliteConnection,
    request: &crate::RenameAuthorDbRequest,
    display_name: &str,
    canonical: &str,
    now: &str,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE author_name_variants SET user_selected_at = NULL \
          WHERE user_id = ? AND author_id = ?",
    )
    .bind(request.user_id)
    .bind(request.author_id)
    .execute(&mut *tx)
    .await
    .map_err(map_db_err)?;

    if request.variant_id != 0 {
        sqlx::query(
            "UPDATE author_name_variants SET user_selected_at = ? \
              WHERE id = ? AND user_id = ? AND author_id = ?",
        )
        .bind(now)
        .bind(request.variant_id)
        .bind(request.user_id)
        .bind(request.author_id)
        .execute(&mut *tx)
        .await
        .map_err(map_db_err)?;
        return Ok(());
    }

    sqlx::query(
        "DELETE FROM author_name_variants \
          WHERE user_id = ? AND author_id = ? AND source = 'user'",
    )
    .bind(request.user_id)
    .bind(request.author_id)
    .execute(&mut *tx)
    .await
    .map_err(map_db_err)?;
    if canonical.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO author_name_variants \
             (user_id, author_id, name, canonical_name, source, user_selected_at, \
              observed_at) \
         VALUES (?, ?, ?, ?, 'user', ?, ?)",
    )
    .bind(request.user_id)
    .bind(request.author_id)
    .bind(display_name)
    .bind(canonical)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(map_db_err)?;
    Ok(())
}

/// The two cascade origins, which differ only in what they say about authority.
///
/// The user-chosen path is covered end to end by the author-link door and
/// database suites. Automatic convergence has no caller yet — the observer that
/// drives it is wired in a later unit — so its one distinguishing property is
/// pinned here rather than left to be discovered after it starts running.
#[cfg(test)]
mod display_name_origin_tests {
    use super::*;
    use crate::test_helpers::create_test_db;
    use crate::{
        AuthorNameVariantDb, CreateUserDbRequest, CreateWorkDbRequest, RenameAuthorDbRequest,
        UserDb, UserRole, WorkDbCreate,
    };
    use livrarr_domain::{normalize_for_matching, AuthorNameSource, ProviderAuthorNameObservation};

    /// One user, one author, one work, and one observed OpenLibrary name
    /// variant — the state both cascade origins act on.
    async fn seed(db: &SqliteDb) -> (i64, i64, i64, i64) {
        let user = db
            .create_user(CreateUserDbRequest {
                username: "cascade-origin".into(),
                password_hash: "hash".into(),
                role: UserRole::User,
                api_key_hash: "cascade-origin-key".into(),
            })
            .await
            .expect("user");
        let (author, _) = db
            .create_author(CreateAuthorDbRequest {
                user_id: user.id,
                name: "Stored Author".into(),
                sort_name: None,
                ol_key: None,
                gr_key: None,
                hc_key: None,
                import_id: None,
            })
            .await
            .expect("author");
        let (work, _) = db
            .create_work(CreateWorkDbRequest {
                user_id: user.id,
                title: "Cascade Work".into(),
                author_name: "Stored Author".into(),
                normalized_title: normalize_for_matching("Cascade Work"),
                normalized_author: normalize_for_matching("Stored Author"),
                author_id: Some(author.id),
                language: Some("en".into()),
                ..Default::default()
            })
            .await
            .expect("work");
        db.record_observed_names(
            user.id,
            work.id,
            &[ProviderAuthorNameObservation {
                source: AuthorNameSource::OpenLibrary,
                name: "Provider Author".into(),
            }],
        )
        .await
        .expect("observed name");
        let variant_id: i64 = sqlx::query_scalar(
            "SELECT id FROM author_name_variants WHERE user_id = ? AND name = ?",
        )
        .bind(user.id)
        .bind("Provider Author")
        .fetch_one(db.pool())
        .await
        .expect("variant id");
        (user.id, author.id, work.id, variant_id)
    }

    #[tokio::test]
    async fn automatic_convergence_moves_the_display_name_and_claims_no_user_authority() {
        let db = create_test_db().await;
        let (user_id, author_id, work_id, variant_id) = seed(&db).await;
        let generation_before: i64 =
            sqlx::query_scalar("SELECT merge_generation FROM works WHERE id = ?")
                .bind(work_id)
                .fetch_one(db.pool())
                .await
                .expect("generation before");

        let converged = db
            .converge_author_display_name(RenameAuthorDbRequest {
                user_id,
                author_id,
                display_name: "Provider Author".into(),
                variant_id,
            })
            .await
            .expect("automatic convergence");

        assert_eq!(converged.name, "Provider Author");
        let work: (String, String, i64) = sqlx::query_as(
            "SELECT author_name, normalized_author, merge_generation FROM works WHERE id = ?",
        )
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .expect("work after convergence");
        assert_eq!(work.0, "Provider Author");
        assert_eq!(
            work.1,
            normalize_for_matching("Stored Author"),
            "convergence is a display change and must not re-key the library"
        );
        assert_eq!(work.2, generation_before + 1, "tags must re-sync");

        let user_variants: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM author_name_variants \
              WHERE user_id = ? AND author_id = ? AND source = 'user'",
        )
        .bind(user_id)
        .bind(author_id)
        .fetch_one(db.pool())
        .await
        .expect("user variant count");
        assert_eq!(
            user_variants, 0,
            "automatic convergence must not fabricate a User variant"
        );
        let selected: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM author_name_variants \
              WHERE user_id = ? AND author_id = ? AND user_selected_at IS NOT NULL",
        )
        .bind(user_id)
        .bind(author_id)
        .fetch_one(db.pool())
        .await
        .expect("selected count");
        assert_eq!(
            selected, 0,
            "automatic convergence must not stamp user authority on its own choice"
        );
    }

    #[tokio::test]
    async fn automatic_convergence_leaves_an_existing_user_selection_alone() {
        let db = create_test_db().await;
        let (user_id, author_id, _work_id, variant_id) = seed(&db).await;
        db.rename_author_and_cascade(RenameAuthorDbRequest {
            user_id,
            author_id,
            display_name: "Chosen By User".into(),
            variant_id: 0,
        })
        .await
        .expect("user rename");

        db.converge_author_display_name(RenameAuthorDbRequest {
            user_id,
            author_id,
            display_name: "Provider Author".into(),
            variant_id,
        })
        .await
        .expect("automatic convergence over a user choice");

        let still_selected: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM author_name_variants \
              WHERE user_id = ? AND author_id = ? AND user_selected_at IS NOT NULL",
        )
        .bind(user_id)
        .bind(author_id)
        .fetch_all(db.pool())
        .await
        .expect("selected variants");
        assert_eq!(
            still_selected,
            vec!["Chosen By User".to_string()],
            "the user's selection survives an automatic display-name change"
        );
    }

    #[tokio::test]
    async fn a_picked_stored_variant_records_the_user_as_its_chooser() {
        let db = create_test_db().await;
        let (user_id, author_id, _work_id, variant_id) = seed(&db).await;

        db.rename_author_and_cascade(RenameAuthorDbRequest {
            user_id,
            author_id,
            display_name: String::new(),
            variant_id,
        })
        .await
        .expect("stored-variant pick");

        let selected: Vec<(i64, String)> = sqlx::query_as(
            "SELECT id, source FROM author_name_variants \
              WHERE user_id = ? AND author_id = ? AND user_selected_at IS NOT NULL",
        )
        .bind(user_id)
        .bind(author_id)
        .fetch_all(db.pool())
        .await
        .expect("selected variants");
        assert_eq!(
            selected,
            vec![(variant_id, "open_library".to_string())],
            "the picked observation is marked in place, keeping its source"
        );
    }
}
