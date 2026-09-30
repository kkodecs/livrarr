use chrono::Utc;
use sqlx::{QueryBuilder, Row, Sqlite};

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, parse_dt};
use crate::sqlite_library_item::row_to_library_item;
use crate::{
    AuthorId, CreateImportDbRequest, DbError, Import, ImportDb, LibraryItem, LibraryItemId, UserId,
};

fn row_to_import(row: sqlx::sqlite::SqliteRow) -> Result<Import, DbError> {
    let started_at_str: String = row
        .try_get("started_at")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let completed_at_str: Option<String> = row
        .try_get("completed_at")
        .map_err(|e| DbError::Io(Box::new(e)))?;

    Ok(Import {
        id: row.try_get("id").map_err(|e| DbError::Io(Box::new(e)))?,
        user_id: row
            .try_get::<i64, _>("user_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        source: row
            .try_get("source")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        status: row
            .try_get("status")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        started_at: parse_dt(&started_at_str)?,
        completed_at: completed_at_str.map(|s| parse_dt(&s)).transpose()?,
        authors_created: row
            .try_get::<i64, _>("authors_created")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        works_created: row
            .try_get::<i64, _>("works_created")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        files_imported: row
            .try_get::<i64, _>("files_imported")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        files_skipped: row
            .try_get::<i64, _>("files_skipped")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        source_url: row
            .try_get("source_url")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        target_root_folder_id: row
            .try_get("target_root_folder_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
    })
}

impl ImportDb for SqliteDb {
    async fn create_import(&self, req: CreateImportDbRequest) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO imports (id, user_id, source, status, started_at, source_url, target_root_folder_id) \
             VALUES (?, ?, ?, 'running', ?, ?, ?)",
        )
        .bind(&req.id)
        .bind(req.user_id)
        .bind(&req.source)
        .bind(&now)
        .bind(&req.source_url)
        .bind(req.target_root_folder_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(())
    }

    async fn get_import(&self, id: &str) -> Result<Option<Import>, DbError> {
        let row = sqlx::query("SELECT * FROM imports WHERE id = ?")
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(map_db_err)?;
        match row {
            Some(r) => Ok(Some(row_to_import(r)?)),
            None => Ok(None),
        }
    }

    async fn list_imports(&self, user_id: UserId) -> Result<Vec<Import>, DbError> {
        let rows = sqlx::query("SELECT * FROM imports WHERE user_id = ? ORDER BY started_at DESC")
            .bind(user_id)
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;
        rows.into_iter().map(row_to_import).collect()
    }

    async fn update_import_status(&self, id: &str, status: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE imports SET status = ? WHERE id = ?")
            .bind(status)
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn update_import_counts(
        &self,
        id: &str,
        authors: i64,
        works: i64,
        files: i64,
        skipped: i64,
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE imports SET authors_created = ?, works_created = ?, files_imported = ?, files_skipped = ? WHERE id = ?",
        )
        .bind(authors)
        .bind(works)
        .bind(files)
        .bind(skipped)
        .bind(id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(())
    }

    async fn set_import_completed(&self, id: &str) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE imports SET status = 'completed', completed_at = ? WHERE id = ?")
            .bind(&now)
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn list_library_items_by_import(
        &self,
        import_id: &str,
    ) -> Result<Vec<LibraryItem>, DbError> {
        let rows = sqlx::query("SELECT * FROM library_items WHERE import_id = ? ORDER BY id")
            .bind(import_id)
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;
        rows.into_iter().map(row_to_library_item).collect()
    }

    async fn delete_library_item_by_id(&self, id: LibraryItemId) -> Result<(), DbError> {
        sqlx::query("DELETE FROM library_items WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn list_orphan_work_ids_by_import(&self, import_id: &str) -> Result<Vec<i64>, DbError> {
        let rows = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM works WHERE import_id = ? AND id NOT IN \
             (SELECT DISTINCT work_id FROM library_items WHERE work_id IS NOT NULL)",
        )
        .bind(import_id)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(rows)
    }

    async fn delete_orphan_works_by_import(&self, import_id: &str) -> Result<i64, DbError> {
        let result = sqlx::query(
            "DELETE FROM works WHERE import_id = ? AND id NOT IN \
             (SELECT DISTINCT work_id FROM library_items WHERE work_id IS NOT NULL)",
        )
        .bind(import_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(result.rows_affected() as i64)
    }

    async fn delete_orphan_authors_by_import(&self, import_id: &str) -> Result<i64, DbError> {
        let sql =
            format!("DELETE FROM authors AS a WHERE a.import_id = ?1 AND {EMPTY_UNTOUCHED_AUTHOR}");
        let result = sqlx::query(&sql)
            .bind(import_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(result.rows_affected() as i64)
    }

    async fn list_authors_of_import_works(
        &self,
        import_id: &str,
        user_id: UserId,
    ) -> Result<Vec<AuthorId>, DbError> {
        let rows: Vec<AuthorId> = sqlx::query_scalar(
            "SELECT w.author_id FROM works w \
              WHERE w.import_id = ?1 AND w.user_id = ?2 AND w.author_id IS NOT NULL \
             UNION \
             SELECT w.primary_author_id FROM works w \
              WHERE w.import_id = ?1 AND w.user_id = ?2 AND w.primary_author_id IS NOT NULL \
             UNION \
             SELECT wc.author_id FROM work_contributors wc \
               JOIN works w ON w.user_id = wc.user_id AND w.id = wc.work_id \
              WHERE w.import_id = ?1 AND w.user_id = ?2",
        )
        .bind(import_id)
        .bind(user_id)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(rows)
    }

    async fn delete_empty_authors_added_since_import(
        &self,
        import_id: &str,
        user_id: UserId,
        author_ids: &[AuthorId],
    ) -> Result<i64, DbError> {
        let mut removed = 0i64;
        for batch in author_ids.chunks(AUTHOR_ID_BATCH) {
            let mut query =
                QueryBuilder::<Sqlite>::new("DELETE FROM authors AS a WHERE a.user_id = ");
            query.push_bind(user_id);
            query.push(
                " AND julianday(a.added_at) >= \
                   (SELECT julianday(i.started_at) FROM imports i WHERE i.id = ",
            );
            query.push_bind(import_id);
            query.push(" AND i.user_id = ");
            query.push_bind(user_id);
            query.push(") AND a.id IN (");
            let mut ids = query.separated(", ");
            for author_id in batch {
                ids.push_bind(*author_id);
            }
            ids.push_unseparated(") AND ");
            query.push(EMPTY_UNTOUCHED_AUTHOR);
            let result = query
                .build()
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
            removed += result.rows_affected() as i64;
        }
        Ok(removed)
    }
}

/// Author ids bound per clean-up statement; with its three other binds this
/// stays under SQLite's historical 999-variable default as well as the
/// bundled 32,766.
const AUTHOR_ID_BATCH: usize = 500;

/// Keep-checks for an import clean-up over author row `a`: the author has no
/// work, contributor or series row, no monitoring field set, and no
/// user-picked or user-removed route, user name variant or picked link
/// candidate.
const EMPTY_UNTOUCHED_AUTHOR: &str = "a.monitored=0 AND a.monitor_new_items=0 \
       AND a.monitor_since IS NULL AND a.monitor_language IS NULL \
       AND NOT EXISTS (SELECT 1 FROM works w \
                        WHERE w.user_id=a.user_id \
                          AND (w.author_id=a.id OR w.primary_author_id=a.id)) \
       AND NOT EXISTS (SELECT 1 FROM work_contributors wc \
                        WHERE wc.user_id=a.user_id AND wc.author_id=a.id) \
       AND NOT EXISTS (SELECT 1 FROM series s \
                        WHERE s.user_id=a.user_id AND s.author_id=a.id) \
       AND NOT EXISTS (SELECT 1 FROM author_provider_routes r \
                        WHERE r.user_id=a.user_id AND r.author_id=a.id \
                          AND (r.provenance='user_picked' \
                               OR r.removed_by_user_id IS NOT NULL)) \
       AND NOT EXISTS (SELECT 1 FROM author_name_variants n \
                        WHERE n.user_id=a.user_id AND n.author_id=a.id \
                          AND (n.source='user' OR n.user_selected_at IS NOT NULL)) \
       AND NOT EXISTS (SELECT 1 FROM author_link_candidates c \
                        WHERE c.user_id=a.user_id AND c.author_id=a.id \
                          AND c.status='picked')";

#[cfg(test)]
mod tests {
    use crate::{
        test_helpers::create_test_db, AuthorDb, AuthorId, CreateAuthorDbRequest,
        CreateUserDbRequest, DbError, ImportDb, ListImportDb, UpdateAuthorDbRequest, UserDb,
        UserRole,
    };

    /// SQLite's default bound-variable limit in the bundled build.
    const SQLITE_MAX_VARIABLE_NUMBER: usize = 32_766;

    #[tokio::test]
    async fn empty_author_cleanup_accepts_more_candidates_than_sqlite_binds() {
        let db = create_test_db().await;
        let user_id = db
            .create_user(CreateUserDbRequest {
                username: "bulk".to_string(),
                password_hash: "h".to_string(),
                role: UserRole::User,
                api_key_hash: "k_bulk".to_string(),
            })
            .await
            .expect("create user")
            .id;
        db.create_list_import_record(
            "bulk-import",
            user_id,
            "goodreads",
            &chrono::Utc::now().to_rfc3339(),
        )
        .await
        .expect("create list import");

        let mut real_ids: Vec<AuthorId> = Vec::new();
        for name in ["Bulk One", "Bulk Two", "Bulk Three", "Bulk Monitored"] {
            let (author, _) = db
                .create_author(CreateAuthorDbRequest {
                    user_id,
                    name: name.to_string(),
                    sort_name: None,
                    ol_key: None,
                    gr_key: None,
                    hc_key: None,
                    import_id: None,
                })
                .await
                .expect("create author");
            real_ids.push(author.id);
        }
        let protected = real_ids[3];
        db.update_author(
            user_id,
            protected,
            UpdateAuthorDbRequest {
                name: None,
                sort_name: None,
                ol_key: None,
                gr_key: None,
                monitored: None,
                monitor_new_items: Some(true),
                monitor_since: None,
                monitor_language: None,
            },
        )
        .await
        .expect("monitor author");

        // Ids with no author row pad the candidate list past the limit. The
        // eligible authors sit in the first batch, a middle batch and the
        // final partial batch; the monitored author sits late in the list.
        let mut candidates: Vec<AuthorId> = (1..=SQLITE_MAX_VARIABLE_NUMBER as AuthorId)
            .map(|i| 1_000_000 + i)
            .collect();
        candidates.insert(0, real_ids[0]);
        let middle = candidates.len() / 2;
        candidates.insert(middle, real_ids[1]);
        candidates.push(protected);
        candidates.push(real_ids[2]);
        let last = candidates.len() - 1;
        assert!(candidates.len() > SQLITE_MAX_VARIABLE_NUMBER);
        assert!(middle / super::AUTHOR_ID_BATCH > 0);
        assert!(!candidates.len().is_multiple_of(super::AUTHOR_ID_BATCH));
        assert_eq!(
            last / super::AUTHOR_ID_BATCH,
            candidates.len() / super::AUTHOR_ID_BATCH
        );

        let removed = db
            .delete_empty_authors_added_since_import("bulk-import", user_id, &candidates)
            .await
            .expect("clean-up over a large candidate list");

        assert_eq!(removed, 3);
        for id in &real_ids[..3] {
            assert!(matches!(
                db.get_author(user_id, *id).await,
                Err(DbError::NotFound { .. })
            ));
        }
        db.get_author(user_id, protected)
            .await
            .expect("a monitored author stays");
    }
}
