use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::Path;

/// Create and configure a SQLite connection pool.
///
/// Per-connection PRAGMAs per error-handling-policy.md:
/// - WAL journal mode, synchronous=NORMAL (tradeoff for SD card perf)
/// - busy_timeout=5s, foreign_keys=ON
/// - journal_size_limit=64MB, wal_autocheckpoint=1000 pages (~4MB)
pub async fn create_sqlite_pool(data_dir: &Path) -> Result<SqlitePool, sqlx::Error> {
    let db_path = data_dir.join("livrarr.db");

    // Use filename() instead of URL parsing to safely handle paths containing
    // special characters like '#' or '?' that would break URL parsing.
    let options = SqliteConnectOptions::new()
        .filename(&db_path)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(5))
        .pragma("foreign_keys", "ON")
        .pragma("synchronous", "NORMAL")
        .pragma("journal_size_limit", "67108864")
        .pragma("wal_autocheckpoint", "1000");

    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .min_connections(1)
        .connect_with(options)
        .await?;

    Ok(pool)
}

/// Run embedded migrations.
///
/// Satisfies: RUNTIME-SQLITE-003
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}

// ── Startup checks ──────────────────────────────────────────────────────────

/// Maximum schema_version this binary understands.
const MAX_SCHEMA_VERSION: i64 = 38;
/// Maximum data_version this binary understands.
const MAX_DATA_VERSION: i64 = 1;

/// Check that the database version is compatible with this binary.
/// Fatal if either version exceeds the binary's supported max.
pub async fn check_version_gate(pool: &SqlitePool) -> Result<(), String> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM _livrarr_meta WHERE key = 'schema_version'")
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("failed to read schema_version: {e}"))?;

    if let Some((val,)) = row {
        let ver: i64 = val
            .parse()
            .map_err(|_| format!("invalid schema_version: {val}"))?;
        if ver > MAX_SCHEMA_VERSION {
            return Err(format!(
                "database schema_version {ver} is newer than this binary supports (max {MAX_SCHEMA_VERSION}). Upgrade Livrarr."
            ));
        }
    }

    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM _livrarr_meta WHERE key = 'data_version'")
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("failed to read data_version: {e}"))?;

    if let Some((val,)) = row {
        let ver: i64 = val
            .parse()
            .map_err(|_| format!("invalid data_version: {val}"))?;
        if ver > MAX_DATA_VERSION {
            return Err(format!(
                "database data_version {ver} is newer than this binary supports (max {MAX_DATA_VERSION}). Upgrade Livrarr."
            ));
        }
    }

    Ok(())
}

/// Verify the data directory is writable (write+delete a tempfile).
pub fn check_data_dir_permissions(data_dir: &Path) -> Result<(), String> {
    use std::io::Write;
    let mut probe = tempfile::Builder::new()
        .prefix(".livrarr_probe_")
        .tempfile_in(data_dir)
        .map_err(|e| format!("cannot write to data directory {}: {e}", data_dir.display()))?;
    probe
        .write_all(b"ok")
        .map_err(|e| format!("cannot write to data directory {}: {e}", data_dir.display()))?;
    Ok(())
}

/// Check whether the given PID belongs to a different running livrarr process.
///
/// Returns `true` only when the PID is alive, distinct from our own, and
/// `/proc/PID/comm` contains "livrarr". This prevents false positives in
/// Docker containers where a fresh PID namespace can reuse the same PID
/// number for an unrelated process, and avoids self-match when a stale PID
/// file lists our own PID.
fn is_livrarr_process(pid: u32) -> bool {
    if pid == std::process::id() {
        return false;
    }
    let comm_path = format!("/proc/{pid}/comm");
    match std::fs::read_to_string(&comm_path) {
        Ok(comm) => comm.trim().contains("livrarr"),
        // Process doesn't exist or /proc not readable — not a live livrarr.
        Err(_) => false,
    }
}

/// Write a PID lock file. Returns Err if a live instance is detected.
///
/// Uses O_EXCL (create_new) in a loop to atomically create the lock file.
/// If the file exists and the owning PID is dead, removes and retries.
/// If the file exists and the owning PID is alive, rejects.
/// Handles concurrent removal (NotFound) gracefully.
pub fn acquire_pid_lock(data_dir: &Path) -> Result<(), String> {
    use std::io::Write;
    let lock_path = data_dir.join("livrarr.pid");

    // Up to 2 attempts: first try, then retry after stale removal.
    for attempt in 0..2 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut f) => {
                write!(f, "{}", std::process::id())
                    .map_err(|e| format!("failed to write PID lock: {e}"))?;
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // File exists — check if the owning process is still alive.
            }
            Err(e) => {
                return Err(format!("failed to create PID lock: {e}"));
            }
        }

        // Only check staleness on first attempt to avoid infinite loop.
        if attempt > 0 {
            return Err(
                "failed to acquire PID lock after stale removal (concurrent startup?)".to_string(),
            );
        }

        // Lock file exists — read and check if stale.
        match std::fs::read_to_string(&lock_path) {
            Ok(contents) => {
                if let Ok(pid) = contents.trim().parse::<u32>() {
                    if is_livrarr_process(pid) {
                        return Err(format!(
                            "another Livrarr instance (PID {pid}) is running. Remove {lock_path:?} if this is stale."
                        ));
                    }
                }
                // PID is dead, not livrarr, or unreadable — remove and retry.
                tracing::warn!("stale PID lock file detected, removing");
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Concurrent removal — loop back and retry create_new.
                continue;
            }
            Err(_) => {
                // Unreadable/corrupt — warn and attempt removal.
                tracing::warn!("PID lock file unreadable, attempting removal");
            }
        }

        // Remove stale lock. Handle NotFound from concurrent remove gracefully.
        match std::fs::remove_file(&lock_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(format!("failed to remove stale PID lock: {e}"));
            }
        }
    }

    Err("failed to acquire PID lock after retries".to_string())
}

/// Remove the PID lock file on shutdown.
pub fn release_pid_lock(data_dir: &Path) {
    let lock_path = data_dir.join("livrarr.pid");
    let _ = std::fs::remove_file(lock_path);
}

/// Create a pre-migration backup using VACUUM INTO.
/// Returns the backup path on success.
pub async fn create_backup(
    pool: &SqlitePool,
    data_dir: &Path,
) -> Result<std::path::PathBuf, String> {
    let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let backup_name = format!("livrarr.db.pre-migrate-{timestamp}");
    let backup_path = data_dir.join(&backup_name);

    let canonical_parent = backup_path
        .parent()
        .ok_or("backup path has no parent")?
        .canonicalize()
        .map_err(|e| format!("cannot resolve backup parent dir: {e}"))?;
    let canonical_data = data_dir
        .canonicalize()
        .map_err(|e| format!("cannot resolve data dir: {e}"))?;
    if !canonical_parent.starts_with(&canonical_data) {
        return Err("backup path escapes data directory".into());
    }

    let backup_str = backup_path.display().to_string();
    if backup_str.contains('\'') {
        return Err("backup path contains invalid characters".into());
    }

    sqlx::query(&format!("VACUUM INTO '{backup_str}'"))
        .execute(pool)
        .await
        .map_err(|e| format!("VACUUM INTO backup failed: {e}"))?;

    tracing::info!("pre-migration backup: {backup_name}");
    Ok(backup_path)
}

/// Backfill `normalized_title` / `normalized_author` and create the
/// UNIQUE(user_id, normalized_title, normalized_author) index.
///
/// Migration 038 added the columns with `'__UNMIGRATED__'` defaults and no
/// index — duplicates may exist that would violate UNIQUE. This function
/// computes real normalized values, merges duplicate work rows into the
/// oldest keeper, then creates the index.
///
/// Idempotent: skips quickly if no `__UNMIGRATED__` rows remain.
pub async fn backfill_normalized_identity(pool: &SqlitePool) -> Result<(), String> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE normalized_title = '__UNMIGRATED__'")
            .fetch_one(pool)
            .await
            .map_err(|e| format!("count unmigrated works: {e}"))?;

    if count == 0 {
        // Still ensure the index exists in case a prior run partially completed.
        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_works_identity \
             ON works(user_id, normalized_title, normalized_author)",
        )
        .execute(pool)
        .await
        .map_err(|e| format!("create idx_works_identity: {e}"))?;
        tracing::info!("normalized identity backfill: already complete");
        return Ok(());
    }

    tracing::info!("normalized identity backfill: {count} works to backfill");

    // Step 1: compute normalized values for each __UNMIGRATED__ row.
    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT id, title, author_name FROM works WHERE normalized_title = '__UNMIGRATED__'",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("select unmigrated rows: {e}"))?;

    for (id, title, author_name) in &rows {
        let (norm_title, norm_author) =
            livrarr_domain::identity_matching::identity_key(title, author_name);
        sqlx::query("UPDATE works SET normalized_title = ?, normalized_author = ? WHERE id = ?")
            .bind(&norm_title)
            .bind(&norm_author)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| format!("update normalized for work {id}: {e}"))?;
    }

    // Step 2: resolve duplicates. For each (user_id, norm_title, norm_author)
    // group with cnt > 1, keep the lowest id; redirect dependent rows; drop the rest.
    let dupes: Vec<(i64, String, String, String, i64)> = sqlx::query_as(
        "SELECT user_id, normalized_title, normalized_author, \
                GROUP_CONCAT(id) as ids, COUNT(*) as cnt \
         FROM works \
         GROUP BY user_id, normalized_title, normalized_author \
         HAVING cnt > 1",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("scan duplicates: {e}"))?;

    if !dupes.is_empty() {
        tracing::warn!(
            "normalized identity backfill: {} duplicate work groups detected",
            dupes.len()
        );
    }

    let mut merged_count = 0i64;
    for (user_id, _nt, _na, ids_csv, _cnt) in &dupes {
        let mut ids: Vec<i64> = ids_csv
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        ids.sort_unstable();
        let keeper_id = ids[0];
        for &dup_id in &ids[1..] {
            sqlx::query("UPDATE library_items SET work_id = ? WHERE work_id = ? AND user_id = ?")
                .bind(keeper_id)
                .bind(dup_id)
                .bind(user_id)
                .execute(pool)
                .await
                .map_err(|e| format!("redirect library_items for work {dup_id}: {e}"))?;

            sqlx::query("UPDATE grabs SET work_id = ? WHERE work_id = ? AND user_id = ?")
                .bind(keeper_id)
                .bind(dup_id)
                .bind(user_id)
                .execute(pool)
                .await
                .map_err(|e| format!("redirect grabs for work {dup_id}: {e}"))?;

            sqlx::query("UPDATE history SET work_id = ? WHERE work_id = ? AND user_id = ?")
                .bind(keeper_id)
                .bind(dup_id)
                .bind(user_id)
                .execute(pool)
                .await
                .map_err(|e| format!("redirect history for work {dup_id}: {e}"))?;

            sqlx::query("UPDATE external_ids SET work_id = ? WHERE work_id = ?")
                .bind(keeper_id)
                .bind(dup_id)
                .execute(pool)
                .await
                .map_err(|e| format!("redirect external_ids for work {dup_id}: {e}"))?;

            sqlx::query("DELETE FROM work_metadata_provenance WHERE work_id = ? AND user_id = ?")
                .bind(dup_id)
                .bind(user_id)
                .execute(pool)
                .await
                .map_err(|e| format!("delete provenance for work {dup_id}: {e}"))?;

            sqlx::query("DELETE FROM provider_retry_state WHERE work_id = ? AND user_id = ?")
                .bind(dup_id)
                .bind(user_id)
                .execute(pool)
                .await
                .map_err(|e| format!("delete retry_state for work {dup_id}: {e}"))?;

            sqlx::query("DELETE FROM works WHERE id = ?")
                .bind(dup_id)
                .execute(pool)
                .await
                .map_err(|e| format!("delete duplicate work {dup_id}: {e}"))?;

            merged_count += 1;
        }
        tracing::info!(
            "normalized identity backfill: merged {} duplicates into work {keeper_id}",
            ids.len() - 1
        );
    }

    // Step 3: create UNIQUE index (now that conflicts are resolved).
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_works_identity \
         ON works(user_id, normalized_title, normalized_author)",
    )
    .execute(pool)
    .await
    .map_err(|e| format!("create idx_works_identity: {e}"))?;

    tracing::info!(
        "normalized identity backfill complete: {} works, {merged_count} duplicates resolved",
        rows.len()
    );
    Ok(())
}

/// The compiled-in identity-key recipe generation (REQ-014). Bump this
/// constant whenever `identity_matching::identity_key`'s output changes in a
/// way that changes stored `works.normalized_title`/`normalized_author`
/// values — [`backfill_identity_key_recompute`] recomputes every work again
/// on the next startup. This is a single global marker (`_livrarr_meta`,
/// not a per-row column): a freshly-created work never needs the backfill
/// to touch it, because every creation/update path already computes its
/// normalized columns via `identity_key` directly (Part 1 of REQ-014).
const IDENTITY_KEY_GENERATION: i64 = 1;

/// Recompute `works.normalized_title`/`normalized_author` via
/// `identity_matching::identity_key` (REQ-014), replacing values written by
/// the retired `normalize_for_matching` recipe (which kept stopwords and
/// accents; `identity_key` drops leading articles and strips accents, so
/// subtitled/accented titles can now adopt at add time — ST-04).
///
/// Idempotent via the `identity_key_generation` marker seeded in
/// `_livrarr_meta` by migration 069: when the stored generation already
/// meets [`IDENTITY_KEY_GENERATION`], this is a single read and an early
/// return — no work rows are touched. This is the "schema_meta-style
/// marker" pattern already used for `schema_version`/`data_version`
/// (migration 010); recompute logic intentionally lives here in Rust, never
/// in SQL.
///
/// A row whose recomputed key would collide with another work's
/// `(user_id, normalized_title, normalized_author)` — a real possibility,
/// since the new recipe can fold two previously-distinct keys together
/// (e.g. "The Hobbit" and "Hobbit" now share one main title) — is left
/// unchanged and logged rather than failing the backfill: the UNIQUE index
/// stays authoritative, and a visible library duplicate the merge-two-works
/// action can resolve is the safe outcome (REQ-008), never a startup crash.
/// When any row is skipped this way the marker is deliberately NOT bumped,
/// so the next startup retries the full pass — safe, because recomputing an
/// already-correct row reproduces the same value.
pub async fn backfill_identity_key_recompute(pool: &SqlitePool) -> Result<(), String> {
    let current: Option<String> =
        sqlx::query_scalar("SELECT value FROM _livrarr_meta WHERE key = 'identity_key_generation'")
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("read identity_key_generation: {e}"))?;
    let current_gen: i64 = current.and_then(|v| v.parse().ok()).unwrap_or(0);
    if current_gen >= IDENTITY_KEY_GENERATION {
        tracing::debug!("identity-key recompute: already at generation {current_gen}");
        return Ok(());
    }

    let rows: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT id, title, author_name FROM works")
            .fetch_all(pool)
            .await
            .map_err(|e| format!("select works for identity-key recompute: {e}"))?;

    let mut recomputed = 0i64;
    let mut skipped = 0i64;
    for (id, title, author_name) in &rows {
        let (norm_title, norm_author) =
            livrarr_domain::identity_matching::identity_key(title, author_name);
        let result = sqlx::query(
            "UPDATE works SET normalized_title = ?, normalized_author = ? WHERE id = ?",
        )
        .bind(&norm_title)
        .bind(&norm_author)
        .bind(id)
        .execute(pool)
        .await;
        match result {
            Ok(_) => recomputed += 1,
            // ONLY a genuine unique-constraint violation is a collision
            // skip (the new recipe folded this row's key into another
            // work's — a visible library duplicate, resolvable via the
            // merge action). Every other error (readonly database, I/O
            // failure, corruption, lock) propagates: startup must fail
            // loudly rather than continue half-recomputed with only a
            // warning line (REQ-014).
            Err(e) if is_unique_violation(&e) => {
                tracing::warn!(
                    work_id = id,
                    "identity-key recompute: skipped work (new key collides \
                     with another work — visible as a library duplicate, \
                     resolvable via the merge action): {e}"
                );
                skipped += 1;
            }
            Err(e) => {
                return Err(format!("identity-key recompute for work {id}: {e}"));
            }
        }
    }

    if skipped == 0 {
        sqlx::query("UPDATE _livrarr_meta SET value = ? WHERE key = 'identity_key_generation'")
            .bind(IDENTITY_KEY_GENERATION.to_string())
            .execute(pool)
            .await
            .map_err(|e| format!("bump identity_key_generation: {e}"))?;
    }

    tracing::info!(
        "identity-key recompute: {recomputed} recomputed, {skipped} skipped (of {} works)",
        rows.len()
    );
    Ok(())
}

/// True when a sqlx error is a database-level UNIQUE-constraint violation —
/// the one error class [`backfill_identity_key_recompute`] treats as a
/// collision skip. Same detection idiom as `sqlite_common::map_db_err`
/// (`DatabaseError::is_unique_violation`), but deliberately narrower: a
/// foreign-key violation or any non-constraint failure is NOT a skip.
fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db_err) if db_err.is_unique_violation())
}

/// Delete old backups, keeping the most recent `keep` versions.
pub fn cleanup_old_backups(data_dir: &Path, keep: usize) {
    let dir_entries = match std::fs::read_dir(data_dir) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!("failed to read data directory for backup cleanup: {e}");
            return;
        }
    };

    let mut backups: Vec<_> = dir_entries
        .filter_map(|entry| match entry {
            Ok(e) => Some(e),
            Err(e) => {
                tracing::warn!("error reading directory entry during backup cleanup: {e}");
                None
            }
        })
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("livrarr.db.pre-migrate-")
        })
        .collect();

    if backups.len() <= keep {
        return;
    }

    // Sort by name (timestamp-based, so lexicographic = chronological)
    backups.sort_by_key(|e| e.file_name());
    let to_delete = backups.len() - keep;
    for entry in backups.into_iter().take(to_delete) {
        if let Err(e) = std::fs::remove_file(entry.path()) {
            tracing::warn!("failed to delete old backup {:?}: {e}", entry.file_name());
        } else {
            tracing::info!("deleted old backup: {:?}", entry.file_name());
        }
    }
}

#[cfg(test)]
mod identity_key_recompute_tests {
    use super::*;
    use crate::sqlite::SqliteDb;
    use crate::test_helpers::create_test_db;
    use crate::{CreateUserDbRequest, CreateWorkDbRequest, UserDb, UserRole, WorkDbCreate};

    async fn seed_user(db: &SqliteDb) -> i64 {
        db.create_user(CreateUserDbRequest {
            username: "recompute-user".into(),
            password_hash: "hash".into(),
            role: UserRole::User,
            api_key_hash: "recompute-key".into(),
        })
        .await
        .unwrap()
        .id
    }

    /// Insert a work the way pre-Phase-5 code did: normalized_title/author
    /// computed via the OLD `normalize_for_matching` recipe (kept stopwords,
    /// no accent strip) — simulating a row that predates this migration.
    async fn seed_old_recipe_work(db: &SqliteDb, user_id: i64, title: &str, author: &str) -> i64 {
        let (work, _created) = db
            .create_work(CreateWorkDbRequest {
                user_id,
                title: title.to_string(),
                author_name: author.to_string(),
                normalized_title: livrarr_domain::normalize_for_matching(title),
                normalized_author: livrarr_domain::normalize_for_matching(author),
                ..Default::default()
            })
            .await
            .unwrap();
        work.id
    }

    #[tokio::test]
    async fn recompute_rewrites_old_recipe_rows_and_bumps_marker() {
        let db = create_test_db().await;
        let user_id = seed_user(&db).await;
        let work_id = seed_old_recipe_work(&db, user_id, "The Hobbit", "J.R.R. Tolkien").await;

        // Migration 069 seeds generation 0; the old recipe's value for "The
        // Hobbit" keeps the leading article ("the hobbit"), which the new
        // recipe drops ("hobbit") — a real, checkable difference.
        backfill_identity_key_recompute(db.pool()).await.unwrap();

        let (norm_title, norm_author): (String, String) =
            sqlx::query_as("SELECT normalized_title, normalized_author FROM works WHERE id = ?")
                .bind(work_id)
                .fetch_one(db.pool())
                .await
                .unwrap();
        let (expected_title, expected_author) =
            livrarr_domain::identity_matching::identity_key("The Hobbit", "J.R.R. Tolkien");
        assert_eq!(norm_title, expected_title);
        assert_eq!(norm_author, expected_author);
        assert_ne!(
            norm_title, "the hobbit",
            "must no longer be the old-recipe value"
        );

        let generation: String = sqlx::query_scalar(
            "SELECT value FROM _livrarr_meta WHERE key = 'identity_key_generation'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(generation, IDENTITY_KEY_GENERATION.to_string());
    }

    #[tokio::test]
    async fn recompute_is_idempotent_second_run_touches_nothing() {
        let db = create_test_db().await;
        let user_id = seed_user(&db).await;
        seed_old_recipe_work(&db, user_id, "Dune: Messiah", "Frank_Herbert").await;

        backfill_identity_key_recompute(db.pool()).await.unwrap();
        let after_first: Vec<(i64, String, String)> =
            sqlx::query_as("SELECT id, normalized_title, normalized_author FROM works ORDER BY id")
                .fetch_all(db.pool())
                .await
                .unwrap();

        // Second run: the marker already meets IDENTITY_KEY_GENERATION, so
        // this is a single read and an early return — no work rows touched.
        backfill_identity_key_recompute(db.pool()).await.unwrap();
        let after_second: Vec<(i64, String, String)> =
            sqlx::query_as("SELECT id, normalized_title, normalized_author FROM works ORDER BY id")
                .fetch_all(db.pool())
                .await
                .unwrap();

        assert_eq!(
            after_first, after_second,
            "a second backfill run must leave every row byte-identical"
        );
    }

    #[tokio::test]
    async fn recompute_skips_a_row_that_would_collide_and_leaves_marker_stale() {
        let db = create_test_db().await;
        let user_id = seed_user(&db).await;
        // Two distinct-under-the-old-recipe rows ("The Hobbit" keeps its
        // article, "Hobbit" has none) that the NEW recipe folds to the same
        // main title for the same author — a genuine new collision.
        seed_old_recipe_work(&db, user_id, "The Hobbit", "Tolkien").await;
        seed_old_recipe_work(&db, user_id, "Hobbit", "Tolkien").await;

        backfill_identity_key_recompute(db.pool()).await.unwrap();

        // At least one of the two rows must have been left at its old,
        // pre-recompute value (the UNIQUE index blocks both from adopting
        // the same new key) — the backfill must not have errored/panicked.
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT normalized_title, normalized_author FROM works ORDER BY id")
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(rows.len(), 2, "both rows survive — never a deletion");
        let distinct_titles: std::collections::HashSet<&str> =
            rows.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(
            distinct_titles.len(),
            2,
            "the two rows must stay distinct (one recomputed, one left at \
             its prior value) rather than colliding: {rows:?}"
        );

        // The marker must NOT have been bumped, since at least one row was
        // skipped — the next startup will retry the full pass.
        let generation: String = sqlx::query_scalar(
            "SELECT value FROM _livrarr_meta WHERE key = 'identity_key_generation'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(
            generation, "0",
            "marker stays stale when any row is skipped"
        );
    }

    #[tokio::test]
    async fn recompute_propagates_non_collision_errors_instead_of_skipping() {
        // A non-collision failure (here: the database is read-only, the
        // readonly/I-O class of error) must PROPAGATE as Err so startup
        // fails loudly — never be misclassified as a collision skip that
        // leaves rows silently stale behind a warning line.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recompute-ro.db");

        // Build and seed a real file-backed database.
        {
            let options = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .pragma("foreign_keys", "ON");
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(options)
                .await
                .unwrap();
            sqlx::migrate!("./migrations").run(&pool).await.unwrap();
            // Production creates this index in the step-9b startup hook
            // (backfill_normalized_identity), which always runs before the
            // identity-key recompute — mirror that state here so
            // create_work's ON CONFLICT target exists.
            sqlx::query(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_works_identity \
                 ON works(user_id, normalized_title, normalized_author)",
            )
            .execute(&pool)
            .await
            .unwrap();
            let db = SqliteDb::new(pool);
            let user_id = seed_user(&db).await;
            seed_old_recipe_work(&db, user_id, "The Hobbit", "Tolkien").await;
            db.pool().close().await;
        }

        // Reopen read-only: the marker read and works SELECT succeed, but
        // the per-row UPDATE fails with a readonly error — NOT a unique
        // violation — which must surface as Err.
        let ro_options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .read_only(true);
        let ro_pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(ro_options)
            .await
            .unwrap();
        let result = backfill_identity_key_recompute(&ro_pool).await;
        assert!(
            result.is_err(),
            "a readonly-database failure must propagate, not skip: {result:?}"
        );

        // And the row must still carry its OLD value — nothing was half
        // done behind the error.
        let norm_title: String = sqlx::query_scalar("SELECT normalized_title FROM works LIMIT 1")
            .fetch_one(&ro_pool)
            .await
            .unwrap();
        assert_eq!(
            norm_title,
            livrarr_domain::normalize_for_matching("The Hobbit")
        );
    }

    #[tokio::test]
    async fn unique_violation_classifier_separates_collision_from_other_errors() {
        // Feed the classifier REAL sqlx error objects captured from a live
        // database — a genuine UNIQUE violation vs a genuine non-unique
        // database error — rather than hand-constructed stand-ins.
        let db = create_test_db().await;
        let user_id = seed_user(&db).await;
        seed_old_recipe_work(&db, user_id, "Dune", "Frank Herbert").await;

        // Real unique violation: a second row with the same identity key
        // under the UNIQUE(user_id, normalized_title, normalized_author)
        // index.
        let unique_err = sqlx::query(
            "INSERT INTO works (user_id, title, author_name, normalized_title, \
             normalized_author, enrichment_status, added_at) \
             VALUES (?, 'Dune Again', 'Frank Herbert', ?, ?, 'unenriched', '2026-01-01')",
        )
        .bind(user_id)
        .bind(livrarr_domain::normalize_for_matching("Dune"))
        .bind(livrarr_domain::normalize_for_matching("Frank Herbert"))
        .execute(db.pool())
        .await
        .expect_err("duplicate identity key must violate the unique index");
        assert!(
            is_unique_violation(&unique_err),
            "a real unique-index violation must classify as a collision: {unique_err}"
        );

        // Real non-unique database error: NOT NULL violation (a constraint,
        // but not a UNIQUE constraint — must NOT classify as a collision).
        let not_null_err = sqlx::query(
            "INSERT INTO works (user_id, title, author_name, normalized_title, \
             normalized_author, enrichment_status, added_at) \
             VALUES (?, NULL, 'A', 'x', 'y', 'unenriched', '2026-01-01')",
        )
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect_err("NULL title must violate NOT NULL");
        assert!(
            !is_unique_violation(&not_null_err),
            "a non-unique database error must NOT classify as a collision: {not_null_err}"
        );

        // Real non-database error shape: statement against a missing table.
        let missing_table_err = sqlx::query("UPDATE no_such_table SET x = 1")
            .execute(db.pool())
            .await
            .expect_err("missing table must error");
        assert!(!is_unique_violation(&missing_table_err));
    }
}
