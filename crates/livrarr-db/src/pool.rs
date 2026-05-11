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
                    let proc_path = format!("/proc/{pid}");
                    if Path::new(&proc_path).exists() {
                        return Err(format!(
                            "another Livrarr instance (PID {pid}) is running. Remove {lock_path:?} if this is stale."
                        ));
                    }
                }
                // PID is dead or unreadable — remove and retry.
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
        let norm_title = livrarr_domain::normalize_for_matching(title);
        let norm_author = livrarr_domain::normalize_for_matching(author_name);
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
