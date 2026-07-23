use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{SqliteConnection, SqlitePool};
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

/// Preserve a user's own confirmed identity anchor and metadata-field lock
/// on `loser_id` before it is merged into `keeper_id` — shared by the
/// startup dedup backfill ([`backfill_normalized_identity`]) and the live
/// work-merge action so both apply the identical policy. Must run on the
/// caller's own connection/transaction, BEFORE the caller's own generic
/// anchor-merge and provenance-drop statements for the pair: this function
/// relocates the user's own contested anchor and copies the user's own
/// contested provenance lock onto the keeper directly (clearing whatever
/// on the keeper would otherwise block either one) — the caller's own
/// generic merge-then-drop statements are left to handle everything else
/// (non-conflicting anchors, non-user provenance) exactly as before.
pub(crate) async fn merge_user_identity_state(
    conn: &mut SqliteConnection,
    keeper_id: i64,
    loser_id: i64,
    user_id: i64,
) -> Result<(), sqlx::Error> {
    // Anchor types where the keeper holds a non-user confirmed anchor and
    // the loser holds a user-confirmed anchor of the SAME type — the
    // user's anchor must win. Computed once, before any mutation below,
    // because the delete/move pair that follows removes the very evidence
    // a live subquery would otherwise need to re-derive it.
    let contested_anchor_types: Vec<String> = sqlx::query_scalar(
        "SELECT k.anchor_type FROM work_identity_anchors k \
         JOIN work_identity_anchors l \
           ON l.anchor_type = k.anchor_type AND l.user_id = k.user_id \
         WHERE k.work_id = ? AND k.user_id = ? AND k.confidence = 'confirmed' AND k.setter != 'user' \
           AND l.work_id = ? AND l.confidence = 'confirmed' AND l.setter = 'user'",
    )
    .bind(keeper_id)
    .bind(user_id)
    .bind(loser_id)
    .fetch_all(&mut *conn)
    .await?;

    for anchor_type in &contested_anchor_types {
        // Clear the keeper's losing anchor for this type — left in place,
        // the partial-unique constraint on (work_id, anchor_type) WHERE
        // confirmed would reject the move below.
        sqlx::query(
            "DELETE FROM work_identity_anchors \
             WHERE work_id = ? AND user_id = ? AND anchor_type = ? \
               AND confidence = 'confirmed' AND setter != 'user'",
        )
        .bind(keeper_id)
        .bind(user_id)
        .bind(anchor_type)
        .execute(&mut *conn)
        .await?;

        // Move the loser's winning anchor onto the keeper via an in-place
        // work_id update — never insert-then-delete: the OTHER partial
        // unique index, on (user_id, anchor_type, anchor_value) WHERE
        // confirmed, does not distinguish by work_id, so inserting a
        // keeper copy while the loser's identical row still exists would
        // self-collide and be silently swallowed by the caller's later
        // `INSERT OR IGNORE` anchor-merge below. An UPDATE that leaves
        // (user_id, anchor_type, anchor_value) unchanged cannot collide
        // with itself.
        sqlx::query(
            "UPDATE work_identity_anchors SET work_id = ? \
             WHERE work_id = ? AND user_id = ? AND anchor_type = ? \
               AND confidence = 'confirmed' AND setter = 'user'",
        )
        .bind(keeper_id)
        .bind(loser_id)
        .bind(user_id)
        .bind(anchor_type)
        .execute(&mut *conn)
        .await?;
    }

    // A user's own metadata-field lock on the loser must survive onto the
    // keeper, overriding any non-user provenance the keeper holds for the
    // same field. The keeper's own user lock (if any) is left untouched —
    // the (work_id, field) primary key makes the insert below a silent
    // no-op for any field the keeper already user-locked.
    sqlx::query(
        "DELETE FROM work_metadata_provenance \
         WHERE work_id = ? AND user_id = ? AND setter != 'user' \
           AND field IN ( \
               SELECT field FROM work_metadata_provenance \
               WHERE work_id = ? AND user_id = ? AND setter = 'user' \
           )",
    )
    .bind(keeper_id)
    .bind(user_id)
    .bind(loser_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;

    sqlx::query(
        "INSERT OR IGNORE INTO work_metadata_provenance \
         (user_id, work_id, field, source, set_at, setter, cleared) \
         SELECT user_id, ?, field, source, set_at, setter, cleared \
         FROM work_metadata_provenance WHERE work_id = ? AND user_id = ? AND setter = 'user'",
    )
    .bind(keeper_id)
    .bind(loser_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Backfill `normalized_title` / `normalized_author`, merge duplicate work
/// rows into the oldest keeper across every table that references `works`,
/// and create the UNIQUE(user_id, normalized_title, normalized_author)
/// index — all inside ONE transaction (Unit D1).
///
/// Migration 038 added the two columns with `'__UNMIGRATED__'` defaults and
/// no index — duplicate rows may share that sentinel. This computes real
/// values via `identity_matching::identity_key`, resolves any resulting
/// duplicates per-table (repoint / merge / intentional-cascade — see the
/// comments on each statement below), then creates the index. Steps 1-3 and
/// the `_livrarr_meta` completion marker share one transaction, the marker
/// as the last write before commit (mirrors the idiom in
/// `sqlite_work_identity.rs`'s `raise_identity_conflict`): a mid-transaction
/// failure rolls back every data change together with the marker, so a
/// partial run can never be mistaken for a complete one on the next startup.
///
/// Idempotent via the `normalized_identity_backfill_complete` marker: once
/// stamped, this is a single read and an early return.
pub async fn backfill_normalized_identity(pool: &SqlitePool) -> Result<(), String> {
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta WHERE key = 'normalized_identity_backfill_complete'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("read normalized_identity_backfill_complete: {e}"))?;

    if marker.as_deref() == Some("1") {
        tracing::debug!("normalized identity backfill: already complete (marker present)");
        return Ok(());
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("begin normalized identity backfill transaction: {e}"))?;

    // Step 1: compute normalized values for each __UNMIGRATED__ row.
    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT id, title, author_name FROM works WHERE normalized_title = '__UNMIGRATED__'",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| format!("select unmigrated rows: {e}"))?;

    if !rows.is_empty() {
        tracing::info!(
            "normalized identity backfill: {} works to backfill",
            rows.len()
        );
    }

    for (id, title, author_name) in &rows {
        let (norm_title, norm_author) =
            livrarr_domain::identity_matching::identity_key(title, author_name);
        sqlx::query("UPDATE works SET normalized_title = ?, normalized_author = ? WHERE id = ?")
            .bind(&norm_title)
            .bind(&norm_author)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("update normalized for work {id}: {e}"))?;
    }

    // Step 2: resolve duplicates. For each (user_id, norm_title, norm_author)
    // group with cnt > 1, keep the lowest id; every table that references
    // `works` is resolved before the duplicate row is dropped.
    let dupes: Vec<(i64, String, String, String, i64)> = sqlx::query_as(
        "SELECT user_id, normalized_title, normalized_author, \
                GROUP_CONCAT(id) as ids, COUNT(*) as cnt \
         FROM works \
         GROUP BY user_id, normalized_title, normalized_author \
         HAVING cnt > 1",
    )
    .fetch_all(&mut *tx)
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
            // --- Reconcile: the keeper's own user-sovereign work fields.
            // Monitor toggles OR together; series/cover keep the keeper's
            // own non-null value and only adopt the loser's when the
            // keeper's is null. A keeper value that wins over a genuinely
            // differing loser value is logged — that loser value becomes
            // unrecoverable once its `works` row is deleted below. ---
            let keeper_fields: (
                bool,
                bool,
                Option<String>,
                Option<f64>,
                Option<String>,
                bool,
            ) = sqlx::query_as(
                "SELECT monitor_ebook, monitor_audiobook, series_name, series_position, \
                     cover_url, cover_manual FROM works WHERE id = ?",
            )
            .bind(keeper_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| format!("read keeper work fields for work {keeper_id}: {e}"))?;
            let loser_fields: (
                bool,
                bool,
                Option<String>,
                Option<f64>,
                Option<String>,
                bool,
            ) = sqlx::query_as(
                "SELECT monitor_ebook, monitor_audiobook, series_name, series_position, \
                     cover_url, cover_manual FROM works WHERE id = ?",
            )
            .bind(dup_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| format!("read loser work fields for work {dup_id}: {e}"))?;

            if let (Some(k), Some(l)) = (&keeper_fields.2, &loser_fields.2) {
                if k != l {
                    tracing::warn!(
                        keeper_id,
                        dup_id,
                        field = "series_name",
                        "dedup: kept keeper, discarded loser"
                    );
                }
            }
            if let (Some(k), Some(l)) = (keeper_fields.3, loser_fields.3) {
                if k != l {
                    tracing::warn!(
                        keeper_id,
                        dup_id,
                        field = "series_position",
                        "dedup: kept keeper, discarded loser"
                    );
                }
            }
            if let (Some(k), Some(l)) = (&keeper_fields.4, &loser_fields.4) {
                if k != l {
                    tracing::warn!(
                        keeper_id,
                        dup_id,
                        field = "cover_url",
                        "dedup: kept keeper, discarded loser"
                    );
                }
            }
            if keeper_fields.5 != loser_fields.5 {
                tracing::warn!(
                    keeper_id,
                    dup_id,
                    field = "cover_manual",
                    "dedup: kept keeper, discarded loser"
                );
            }

            sqlx::query(
                "UPDATE works SET \
                 monitor_ebook = monitor_ebook OR ?, \
                 monitor_audiobook = monitor_audiobook OR ?, \
                 series_name = COALESCE(series_name, ?), \
                 series_position = COALESCE(series_position, ?), \
                 cover_url = COALESCE(cover_url, ?), \
                 cover_manual = COALESCE(cover_manual, ?) \
                 WHERE id = ?",
            )
            .bind(loser_fields.0)
            .bind(loser_fields.1)
            .bind(&loser_fields.2)
            .bind(loser_fields.3)
            .bind(&loser_fields.4)
            .bind(loser_fields.5)
            .bind(keeper_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("reconcile user fields onto keeper for work {keeper_id}: {e}"))?;

            // --- Repoint: rows the user relies on or authored directly. ---
            sqlx::query("UPDATE library_items SET work_id = ? WHERE work_id = ? AND user_id = ?")
                .bind(keeper_id)
                .bind(dup_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("redirect library_items for work {dup_id}: {e}"))?;

            sqlx::query("UPDATE grabs SET work_id = ? WHERE work_id = ? AND user_id = ?")
                .bind(keeper_id)
                .bind(dup_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("redirect grabs for work {dup_id}: {e}"))?;

            sqlx::query("UPDATE history SET work_id = ? WHERE work_id = ? AND user_id = ?")
                .bind(keeper_id)
                .bind(dup_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("redirect history for work {dup_id}: {e}"))?;

            // Bookmarks are user-authored (reading-position markers /
            // highlights) — repoint, never cascade-drop.
            sqlx::query("UPDATE bookmarks SET work_id = ? WHERE work_id = ? AND user_id = ?")
                .bind(keeper_id)
                .bind(dup_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("redirect bookmarks for work {dup_id}: {e}"))?;

            // Conflict rows are an audit trail of past identity decisions —
            // preserve under the surviving work, same treatment as history.
            sqlx::query(
                "UPDATE work_identity_conflicts SET existing_work_id = ? \
                 WHERE existing_work_id = ? AND user_id = ?",
            )
            .bind(keeper_id)
            .bind(dup_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("redirect work_identity_conflicts for work {dup_id}: {e}"))?;

            // Import-intent crash-consistency rows must move with the merge
            // (#20) — work_id is ON DELETE CASCADE (migration 074), so an
            // unrepointed row would vanish silently when the loser `works`
            // row is deleted below, instead of surfacing under the
            // surviving work on the next startup recovery pass.
            sqlx::query("UPDATE import_intents SET work_id = ? WHERE work_id = ? AND user_id = ?")
                .bind(keeper_id)
                .bind(dup_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("redirect import_intents for work {dup_id}: {e}"))?;

            // --- Merge: real identity data. Collision-safe — never blind-repoint. ---
            // external_ids: UNIQUE(work_id, id_type, id_value) — keep the
            // keeper's existing value on collision, adopt anything new.
            sqlx::query(
                "INSERT INTO external_ids (work_id, id_type, id_value) \
                 SELECT ?, id_type, id_value FROM external_ids WHERE work_id = ? \
                 ON CONFLICT(work_id, id_type, id_value) DO NOTHING",
            )
            .bind(keeper_id)
            .bind(dup_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("merge external_ids for work {dup_id}: {e}"))?;
            sqlx::query("DELETE FROM external_ids WHERE work_id = ?")
                .bind(dup_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("clear residual external_ids for work {dup_id}: {e}"))?;

            // Preserve the user's own confirmed anchor / metadata-field
            // lock on the loser before the merge-then-drop statements below
            // run — shared with the live work-merge action so both paths
            // apply the identical policy.
            merge_user_identity_state(&mut tx, keeper_id, dup_id, *user_id)
                .await
                .map_err(|e| format!("preserve user identity state for work {dup_id}: {e}"))?;

            // work_identity_anchors: THREE overlapping unique constraints —
            // the (work_id, anchor_type, anchor_value) primary key, one
            // confirmed anchor per (work_id, anchor_type), and one confirmed
            // anchor per (user_id, anchor_type, anchor_value). A single named
            // ON CONFLICT target cannot guard all three at once, so this uses
            // OR IGNORE (the same sanctioned dedup idiom as
            // `sqlite_notification.rs`'s race guard): the keeper's own
            // confirmed anchors win by default, and anything new merges in.
            // A contested type (loser's user-set anchor beating a keeper
            // non-user one) was already fully relocated by
            // `merge_user_identity_state` above, so there's nothing left
            // here to insert or drop for it.
            sqlx::query(
                "INSERT OR IGNORE INTO work_identity_anchors \
                 (work_id, anchor_type, anchor_value, confidence, setter, set_at, \
                  superseded_by, user_id) \
                 SELECT ?, anchor_type, anchor_value, confidence, setter, set_at, \
                        superseded_by, user_id \
                 FROM work_identity_anchors WHERE work_id = ? AND user_id = ?",
            )
            .bind(keeper_id)
            .bind(dup_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("merge work_identity_anchors for work {dup_id}: {e}"))?;
            sqlx::query("DELETE FROM work_identity_anchors WHERE work_id = ? AND user_id = ?")
                .bind(dup_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| {
                    format!("clear residual work_identity_anchors for work {dup_id}: {e}")
                })?;

            // --- Intentional cascade: system-derived state scoped to the old
            // row id. Composite-keyed on work_id (provenance/retry_state by
            // field/provider, dead_ends by anchor_type, review_candidates is
            // one row per work) — blind repoint risks a PK collision with the
            // keeper's own row, and a fresh pass regenerates all five for the
            // keeper as needed, so the duplicate's rows are simply dropped. ---
            sqlx::query("DELETE FROM work_metadata_provenance WHERE work_id = ? AND user_id = ?")
                .bind(dup_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("delete provenance for work {dup_id}: {e}"))?;

            sqlx::query("DELETE FROM provider_retry_state WHERE work_id = ? AND user_id = ?")
                .bind(dup_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("delete retry_state for work {dup_id}: {e}"))?;

            sqlx::query("DELETE FROM work_field_dissents WHERE work_id = ? AND user_id = ?")
                .bind(dup_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("delete field dissents for work {dup_id}: {e}"))?;

            sqlx::query("DELETE FROM work_anchor_dead_ends WHERE work_id = ? AND user_id = ?")
                .bind(dup_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("delete anchor dead-ends for work {dup_id}: {e}"))?;

            sqlx::query(
                "DELETE FROM work_identity_review_candidates WHERE work_id = ? AND user_id = ?",
            )
            .bind(dup_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("delete review candidates for work {dup_id}: {e}"))?;

            sqlx::query("DELETE FROM works WHERE id = ?")
                .bind(dup_id)
                .execute(&mut *tx)
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
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("create idx_works_identity: {e}"))?;

    // Completion marker — the LAST write before commit (Unit D1). A
    // mid-transaction failure above rolls back every data change together
    // with this marker, so a partial run never reads as "complete" on the
    // next startup.
    sqlx::query(
        "INSERT INTO _livrarr_meta (key, value) \
         VALUES ('normalized_identity_backfill_complete', '1') \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("stamp normalized_identity_backfill_complete marker: {e}"))?;

    tx.commit()
        .await
        .map_err(|e| format!("commit normalized identity backfill transaction: {e}"))?;

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

/// Unit D1: `backfill_normalized_identity` must be one atomic, table-complete
/// transaction. Tests seed the REAL pre-migration-038 precondition — raw
/// `works` rows carrying the `'__UNMIGRATED__'` sentinel, via direct SQL
/// (not `create_work()`, whose ON CONFLICT target requires the very unique
/// index this function is responsible for creating).
#[cfg(test)]
mod backfill_normalized_identity_tests {
    use super::*;
    use crate::sqlite::SqliteDb;
    use crate::test_helpers::create_test_db;
    use crate::{CreateUserDbRequest, UserDb, UserRole};

    async fn seed_user(db: &SqliteDb, username: &str) -> i64 {
        db.create_user(CreateUserDbRequest {
            username: username.into(),
            password_hash: "hash".into(),
            role: UserRole::User,
            api_key_hash: format!("{username}-key"),
        })
        .await
        .unwrap()
        .id
    }

    /// Raw-insert a work carrying the pre-backfill `'__UNMIGRATED__'`
    /// sentinel in both normalized columns — the exact precondition this
    /// function operates on.
    async fn seed_unmigrated_work(
        pool: &SqlitePool,
        user_id: i64,
        title: &str,
        author: &str,
    ) -> i64 {
        let result = sqlx::query(
            "INSERT INTO works (user_id, title, author_name, normalized_title, \
             normalized_author, enrichment_status, added_at) \
             VALUES (?, ?, ?, '__UNMIGRATED__', '__UNMIGRATED__', 'unenriched', '2026-01-01')",
        )
        .bind(user_id)
        .bind(title)
        .bind(author)
        .execute(pool)
        .await
        .unwrap();
        result.last_insert_rowid()
    }

    async fn seed_root_folder(pool: &SqlitePool) -> i64 {
        let result = sqlx::query(
            "INSERT INTO root_folders (path, media_type) VALUES ('/data/ebooks', 'ebook')",
        )
        .execute(pool)
        .await
        .unwrap();
        result.last_insert_rowid()
    }

    async fn seed_download_client(pool: &SqlitePool) -> i64 {
        let result =
            sqlx::query("INSERT INTO download_clients (name, host) VALUES ('qbit', 'localhost')")
                .execute(pool)
                .await
                .unwrap();
        result.last_insert_rowid()
    }

    async fn seed_library_item(
        pool: &SqlitePool,
        user_id: i64,
        work_id: i64,
        root_folder_id: i64,
        path: &str,
    ) -> i64 {
        let result = sqlx::query(
            "INSERT INTO library_items \
             (user_id, work_id, root_folder_id, path, media_type, file_size, imported_at) \
             VALUES (?, ?, ?, ?, 'ebook', 1024, '2026-01-01T00:00:00Z')",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(root_folder_id)
        .bind(path)
        .execute(pool)
        .await
        .unwrap();
        result.last_insert_rowid()
    }

    async fn marker_value(pool: &SqlitePool) -> Option<String> {
        sqlx::query_scalar(
            "SELECT value FROM _livrarr_meta WHERE key = 'normalized_identity_backfill_complete'",
        )
        .fetch_optional(pool)
        .await
        .unwrap()
    }

    /// The test harness bootstraps its own same-column unique index so
    /// `create_work()`'s ON CONFLICT target always resolves; this function's
    /// whole job runs BEFORE any such index exists (the real pre-backfill
    /// state), so tests that need more than one `'__UNMIGRATED__'` row per
    /// user must drop it first.
    async fn drop_harness_unique_index(pool: &SqlitePool) {
        sqlx::query("DROP INDEX IF EXISTS idx_works_user_normalized")
            .execute(pool)
            .await
            .unwrap();
    }

    // ---- (a) Idempotent rerun ----------------------------------------------

    #[tokio::test]
    async fn idempotent_rerun_leaves_state_byte_identical_and_marker_stable() {
        let db = create_test_db().await;
        drop_harness_unique_index(db.pool()).await;

        let user_id = seed_user(&db, "idempotent-user").await;
        seed_unmigrated_work(db.pool(), user_id, "Dune", "Frank Herbert").await;
        seed_unmigrated_work(db.pool(), user_id, "Foundation", "Isaac Asimov").await;

        backfill_normalized_identity(db.pool()).await.unwrap();

        let after_first: Vec<(i64, String, String)> =
            sqlx::query_as("SELECT id, normalized_title, normalized_author FROM works ORDER BY id")
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(
            after_first.len(),
            2,
            "both works survive, no merge expected"
        );
        assert!(
            after_first
                .iter()
                .all(|(_, t, a)| t != "__UNMIGRATED__" && a != "__UNMIGRATED__"),
            "both rows must carry real computed values: {after_first:?}"
        );
        assert_eq!(
            marker_value(db.pool()).await.as_deref(),
            Some("1"),
            "the completion marker must be stamped after a successful run"
        );

        // Second run: marker present -> single read, early return, nothing touched.
        backfill_normalized_identity(db.pool()).await.unwrap();

        let after_second: Vec<(i64, String, String)> =
            sqlx::query_as("SELECT id, normalized_title, normalized_author FROM works ORDER BY id")
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(
            after_first, after_second,
            "a second backfill run must leave every row byte-identical"
        );
        assert_eq!(marker_value(db.pool()).await.as_deref(), Some("1"));
    }

    // ---- (b) Mid-transaction failpoint --------------------------------------

    #[tokio::test]
    async fn mid_transaction_failure_rolls_back_all_data_and_marker_together() {
        let db = create_test_db().await;
        drop_harness_unique_index(db.pool()).await;

        let user_id = seed_user(&db, "failpoint-user").await;

        // A simple singleton row — proves Step 1's recompute rolls back too.
        let singleton_id =
            seed_unmigrated_work(db.pool(), user_id, "Foundation", "Isaac Asimov").await;

        // A duplicate pair ("Hobbit" / "The Hobbit" fold to the same
        // identity_key — the leading article is dropped) with dependent rows
        // in three of the tables the merge must touch.
        let keeper_id = seed_unmigrated_work(db.pool(), user_id, "Hobbit", "J.R.R. Tolkien").await;
        let dup_id = seed_unmigrated_work(db.pool(), user_id, "The Hobbit", "J.R.R. Tolkien").await;

        let root_folder_id = seed_root_folder(db.pool()).await;
        seed_library_item(db.pool(), user_id, dup_id, root_folder_id, "dup.epub").await;

        let client_id = seed_download_client(db.pool()).await;
        sqlx::query(
            "INSERT INTO grabs (user_id, work_id, download_client_id, title, indexer, guid, \
             download_url, grabbed_at) \
             VALUES (?, ?, ?, 'Some Release', 'indexer1', 'guid-1', 'http://x/y', '2026-01-01')",
        )
        .bind(user_id)
        .bind(dup_id)
        .bind(client_id)
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO history (user_id, work_id, event_type, date) \
             VALUES (?, ?, 'added', '2026-01-01')",
        )
        .bind(user_id)
        .bind(dup_id)
        .execute(db.pool())
        .await
        .unwrap();

        // The failpoint: drop a table the merge must touch, forcing a real,
        // naturally triggered SQL error ("no such table") — not a test-only
        // hook — partway through the per-duplicate sequence.
        sqlx::query("DROP TABLE bookmarks")
            .execute(db.pool())
            .await
            .unwrap();

        let result = backfill_normalized_identity(db.pool()).await;
        assert!(
            result.is_err(),
            "a mid-transaction SQL error must surface as Err, not a silent partial success"
        );

        // Step 1's recompute must have rolled back too — the sentinel survives.
        let singleton_norm: (String, String) =
            sqlx::query_as("SELECT normalized_title, normalized_author FROM works WHERE id = ?")
                .bind(singleton_id)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(
            singleton_norm,
            ("__UNMIGRATED__".to_string(), "__UNMIGRATED__".to_string()),
            "Step 1's update for an unrelated row must roll back with the rest of the transaction"
        );

        // The duplicate pair must both still exist — dup was never deleted.
        let still_present: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE id IN (?, ?)")
                .bind(keeper_id)
                .bind(dup_id)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(
            still_present, 2,
            "the duplicate row must not be deleted on a rolled-back run"
        );

        // library_items/grabs/history must still point at dup_id — statements
        // that executed (uncommitted) earlier in the same per-duplicate
        // sequence must roll back too, not just the one that errored.
        let li_work_id: i64 =
            sqlx::query_scalar("SELECT work_id FROM library_items WHERE root_folder_id = ?")
                .bind(root_folder_id)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(li_work_id, dup_id, "library_items redirect must roll back");

        let grab_work_id: i64 =
            sqlx::query_scalar("SELECT work_id FROM grabs WHERE guid = 'guid-1'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(grab_work_id, dup_id, "grabs redirect must roll back");

        let history_work_id: i64 =
            sqlx::query_scalar("SELECT work_id FROM history WHERE event_type = 'added'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(history_work_id, dup_id, "history redirect must roll back");

        // The completion marker must never have been written.
        assert_eq!(
            marker_value(db.pool()).await,
            None,
            "the marker must roll back with the data — a partial run must never read as complete"
        );
    }

    // ---- (c) Colliding external-ids/anchors merge cleanly; every referencing table is handled ----

    #[tokio::test]
    async fn duplicate_merge_resolves_every_referencing_table_without_constraint_failure() {
        let db = create_test_db().await;
        drop_harness_unique_index(db.pool()).await;

        let user_id = seed_user(&db, "merge-user").await;

        let keeper_id = seed_unmigrated_work(db.pool(), user_id, "Hobbit", "J.R.R. Tolkien").await;
        let dup_id = seed_unmigrated_work(db.pool(), user_id, "The Hobbit", "J.R.R. Tolkien").await;

        // library_items + bookmarks (one library item each on keeper and dup).
        let root_folder_id = seed_root_folder(db.pool()).await;
        seed_library_item(db.pool(), user_id, keeper_id, root_folder_id, "keeper.epub").await;
        let dup_item_id =
            seed_library_item(db.pool(), user_id, dup_id, root_folder_id, "dup.epub").await;

        sqlx::query(
            "INSERT INTO bookmarks (user_id, work_id, library_item_id, media_type, position, \
             sort_key, name) VALUES (?, ?, ?, 'ebook', 'epubcfi(/6/2)', 1.0, 'My highlight')",
        )
        .bind(user_id)
        .bind(dup_id)
        .bind(dup_item_id)
        .execute(db.pool())
        .await
        .unwrap();

        // grabs + history on dup.
        let client_id = seed_download_client(db.pool()).await;
        sqlx::query(
            "INSERT INTO grabs (user_id, work_id, download_client_id, title, indexer, guid, \
             download_url, grabbed_at) \
             VALUES (?, ?, ?, 'Some Release', 'indexer1', 'guid-1', 'http://x/y', '2026-01-01')",
        )
        .bind(user_id)
        .bind(dup_id)
        .bind(client_id)
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO history (user_id, work_id, event_type, date) \
             VALUES (?, ?, 'added', '2026-01-01')",
        )
        .bind(user_id)
        .bind(dup_id)
        .execute(db.pool())
        .await
        .unwrap();

        // work_identity_conflicts on dup — an audit-trail row that must be
        // preserved (repointed), not cascade-dropped.
        sqlx::query(
            "INSERT INTO work_identity_conflicts \
             (user_id, existing_work_id, kind, incoming_payload_json, raised_at, raised_by, status) \
             VALUES (?, ?, 'ol_redirect_collision', '{}', '2026-01-01', 'refresh', 'open')",
        )
        .bind(user_id)
        .bind(dup_id)
        .execute(db.pool())
        .await
        .unwrap();

        // external_ids: one COLLIDING pair (same id_type+id_value on both
        // keeper and dup) and one non-colliding pair on dup only.
        sqlx::query(
            "INSERT INTO external_ids (work_id, id_type, id_value) \
             VALUES (?, 'isbn_13', '9780000000000')",
        )
        .bind(keeper_id)
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO external_ids (work_id, id_type, id_value) \
             VALUES (?, 'isbn_13', '9780000000000')",
        )
        .bind(dup_id)
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO external_ids (work_id, id_type, id_value) \
             VALUES (?, 'asin', 'B000UNIQUE1')",
        )
        .bind(dup_id)
        .execute(db.pool())
        .await
        .unwrap();

        // work_identity_anchors: a same-type-different-value CONFIRMED
        // collision (violates uniq_primary_confirmed_anchor, NOT the primary
        // key) plus a clean non-colliding pending anchor.
        sqlx::query(
            "INSERT INTO work_identity_anchors \
             (work_id, anchor_type, anchor_value, confidence, setter, set_at, user_id) \
             VALUES (?, 'gr_key', '111222', 'confirmed', 'user', '2026-01-01', ?)",
        )
        .bind(keeper_id)
        .bind(user_id)
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO work_identity_anchors \
             (work_id, anchor_type, anchor_value, confidence, setter, set_at, user_id) \
             VALUES (?, 'gr_key', '999888', 'confirmed', 'auto_search', '2026-01-01', ?)",
        )
        .bind(dup_id)
        .bind(user_id)
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO work_identity_anchors \
             (work_id, anchor_type, anchor_value, confidence, setter, set_at, user_id) \
             VALUES (?, 'asin', 'B0XYZ00001', 'pending', 'auto_search', '2026-01-01', ?)",
        )
        .bind(dup_id)
        .bind(user_id)
        .execute(db.pool())
        .await
        .unwrap();

        // Ephemeral/system-derived state on dup — expected to be dropped,
        // not repointed (composite PKs make blind repoint collision-prone).
        sqlx::query(
            "INSERT INTO work_metadata_provenance (user_id, work_id, field, set_at, setter) \
             VALUES (?, ?, 'title', '2026-01-01', 'enrichment')",
        )
        .bind(user_id)
        .bind(dup_id)
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO provider_retry_state (user_id, work_id, provider) \
             VALUES (?, ?, 'hardcover')",
        )
        .bind(user_id)
        .bind(dup_id)
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO work_field_dissents \
             (user_id, work_id, provider, field, offered_value, reason, merge_generation, recorded_at) \
             VALUES (?, ?, 'openlibrary', 'title', 'Bad Title', 'mismatch', 1, '2026-01-01')",
        )
        .bind(user_id)
        .bind(dup_id)
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO work_anchor_dead_ends \
             (work_id, anchor_type, attempt_count, last_attempt_at, user_id) \
             VALUES (?, 'gr_key', 3, '2026-01-01', ?)",
        )
        .bind(dup_id)
        .bind(user_id)
        .execute(db.pool())
        .await
        .unwrap();

        // review_candidates: one on keeper (must survive UNTOUCHED) and one
        // on dup (must be dropped — the primary key is bare work_id, so a
        // blind repoint would collide with keeper's own row).
        sqlx::query(
            "INSERT INTO work_identity_review_candidates \
             (work_id, user_id, candidates_json, recorded_at) \
             VALUES (?, ?, '[\"A\"]', '2026-01-01')",
        )
        .bind(keeper_id)
        .bind(user_id)
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO work_identity_review_candidates \
             (work_id, user_id, candidates_json, recorded_at) \
             VALUES (?, ?, '[\"B\"]', '2026-01-01')",
        )
        .bind(dup_id)
        .bind(user_id)
        .execute(db.pool())
        .await
        .unwrap();

        // The core assertion: no constraint failure despite the collisions above.
        backfill_normalized_identity(db.pool())
            .await
            .expect("colliding external_ids/anchors must merge without a constraint violation");

        // dup work row is gone.
        let dup_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE id = ?")
            .bind(dup_id)
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(dup_count, 0, "the duplicate work row must be deleted");

        // library_items: both rows survive, both now point at keeper.
        let li_work_ids: Vec<i64> =
            sqlx::query_scalar("SELECT work_id FROM library_items ORDER BY id")
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(li_work_ids, vec![keeper_id, keeper_id]);

        // bookmarks: the user's highlight survives, repointed — never dropped.
        let bookmark_work_id: i64 = sqlx::query_scalar("SELECT work_id FROM bookmarks")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(
            bookmark_work_id, keeper_id,
            "a user bookmark must survive the merge, repointed to the keeper"
        );

        // grabs / history: repointed, not dropped.
        let grab_work_id: i64 = sqlx::query_scalar("SELECT work_id FROM grabs")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(grab_work_id, keeper_id);
        let history_work_id: i64 = sqlx::query_scalar("SELECT work_id FROM history")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(history_work_id, keeper_id);

        // work_identity_conflicts: repointed (audit trail preserved).
        let conflict_work_id: i64 =
            sqlx::query_scalar("SELECT existing_work_id FROM work_identity_conflicts")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(conflict_work_id, keeper_id);

        // external_ids: exactly 2 rows survive under keeper — the collided
        // isbn_13 (keeper's own value kept, not duplicated) + the merged asin.
        let ext_ids: Vec<(i64, String, String)> =
            sqlx::query_as("SELECT work_id, id_type, id_value FROM external_ids ORDER BY id_type")
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(
            ext_ids,
            vec![
                (keeper_id, "asin".to_string(), "B000UNIQUE1".to_string()),
                (keeper_id, "isbn_13".to_string(), "9780000000000".to_string()),
            ],
            "external_ids must merge to exactly one isbn_13 row plus the new asin, all under keeper"
        );

        // work_identity_anchors: keeper's own confirmed gr_key survives
        // unchanged (dup's conflicting confirmed gr_key was dropped, not
        // forced in), and the non-colliding pending asin merged in.
        let anchors: Vec<(i64, String, String, String)> = sqlx::query_as(
            "SELECT work_id, anchor_type, anchor_value, confidence FROM work_identity_anchors \
             ORDER BY anchor_type",
        )
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(
            anchors,
            vec![
                (
                    keeper_id,
                    "asin".to_string(),
                    "B0XYZ00001".to_string(),
                    "pending".to_string()
                ),
                (
                    keeper_id,
                    "gr_key".to_string(),
                    "111222".to_string(),
                    "confirmed".to_string()
                ),
            ],
            "anchors must merge without constraint failure: keeper's confirmed gr_key wins, \
             dup's conflicting confirmed gr_key is dropped, the non-colliding asin merges in: {anchors:?}"
        );

        // Ephemeral system-derived state: dropped, not repointed.
        let provenance_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM work_metadata_provenance")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(provenance_count, 0);
        let retry_state_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM provider_retry_state")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(retry_state_count, 0);
        let dissent_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM work_field_dissents")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(dissent_count, 0);
        let dead_end_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM work_anchor_dead_ends")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(dead_end_count, 0);

        // review_candidates: exactly keeper's own pre-existing row survives,
        // untouched — dup's incompatible row was dropped, not force-repointed.
        let review_candidates: Vec<(i64, String)> =
            sqlx::query_as("SELECT work_id, candidates_json FROM work_identity_review_candidates")
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(review_candidates, vec![(keeper_id, "[\"A\"]".to_string())]);
    }

    // ---- (d) Reverse direction: the loser's user data must survive the merge ----

    #[tokio::test]
    async fn duplicate_merge_preserves_loser_user_edits_and_repoints_import_intents() {
        let db = create_test_db().await;
        drop_harness_unique_index(db.pool()).await;

        let user_id = seed_user(&db, "reverse-user").await;

        let keeper_id = seed_unmigrated_work(db.pool(), user_id, "Hobbit", "J.R.R. Tolkien").await;
        let loser_id =
            seed_unmigrated_work(db.pool(), user_id, "The Hobbit", "J.R.R. Tolkien").await;

        // The keeper starts with monitor_ebook off (the seed default is on)
        // and no series/cover — the loser (the higher-id row about to be
        // dropped) carries all of the user's real edits.
        sqlx::query("UPDATE works SET monitor_ebook = 0 WHERE id = ?")
            .bind(keeper_id)
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query(
            "UPDATE works SET monitor_ebook = 1, series_name = 'Middle-earth', \
             series_position = 1.0, cover_url = 'http://covers.example/hobbit.jpg' WHERE id = ?",
        )
        .bind(loser_id)
        .execute(db.pool())
        .await
        .unwrap();

        // A user-confirmed anchor on the loser conflicts with a
        // non-user-confirmed anchor of the same type on the keeper.
        sqlx::query(
            "INSERT INTO work_identity_anchors \
             (work_id, anchor_type, anchor_value, confidence, setter, set_at, user_id) \
             VALUES (?, 'gr_key', 'AUTO123', 'confirmed', 'auto_search', '2026-01-01', ?)",
        )
        .bind(keeper_id)
        .bind(user_id)
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO work_identity_anchors \
             (work_id, anchor_type, anchor_value, confidence, setter, set_at, user_id) \
             VALUES (?, 'gr_key', 'USER456', 'confirmed', 'user', '2026-01-01', ?)",
        )
        .bind(loser_id)
        .bind(user_id)
        .execute(db.pool())
        .await
        .unwrap();

        // A user-set provenance lock on the loser, for a field the keeper
        // has no provenance row for at all.
        sqlx::query(
            "INSERT INTO work_metadata_provenance (user_id, work_id, field, set_at, setter) \
             VALUES (?, ?, 'title', '2026-01-01', 'user')",
        )
        .bind(user_id)
        .bind(loser_id)
        .execute(db.pool())
        .await
        .unwrap();

        // An import-intent crash-consistency row on the loser must be
        // repointed, not cascade-deleted when the loser `works` row goes.
        let root_folder_id = seed_root_folder(db.pool()).await;
        sqlx::query(
            "INSERT INTO import_intents \
             (user_id, work_id, root_folder_id, media_type, target_relative, staging_path, \
              expected_size, state, created_at) \
             VALUES (?, ?, ?, 'ebook', 'Hobbit/Hobbit.epub', '/staging/hobbit.epub.tmp', \
                     12345, 'staging', '2026-01-01T00:00:00Z')",
        )
        .bind(user_id)
        .bind(loser_id)
        .bind(root_folder_id)
        .execute(db.pool())
        .await
        .unwrap();

        backfill_normalized_identity(db.pool())
            .await
            .expect("the loser's user data must merge onto the keeper without error");

        // monitor_ebook: OR'd — the loser's "on" wins over the keeper's "off".
        let monitor_ebook: bool =
            sqlx::query_scalar("SELECT monitor_ebook FROM works WHERE id = ?")
                .bind(keeper_id)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert!(
            monitor_ebook,
            "monitor_ebook must OR in the loser's true value"
        );

        // series_name/series_position/cover_url: keeper was null, so it
        // must adopt the loser's value instead of losing it.
        let (series_name, series_position, cover_url): (
            Option<String>,
            Option<f64>,
            Option<String>,
        ) = sqlx::query_as(
            "SELECT series_name, series_position, cover_url FROM works WHERE id = ?",
        )
        .bind(keeper_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(series_name.as_deref(), Some("Middle-earth"));
        assert_eq!(series_position, Some(1.0));
        assert_eq!(
            cover_url.as_deref(),
            Some("http://covers.example/hobbit.jpg")
        );

        // work_identity_anchors: the loser's USER anchor must survive — not
        // the keeper's own auto_search anchor.
        let anchors: Vec<(i64, String, String, String)> = sqlx::query_as(
            "SELECT work_id, anchor_value, confidence, setter FROM work_identity_anchors",
        )
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(
            anchors,
            vec![(
                keeper_id,
                "USER456".to_string(),
                "confirmed".to_string(),
                "user".to_string()
            )],
            "the loser's user-confirmed anchor must win over the keeper's auto_search one: {anchors:?}"
        );

        // work_metadata_provenance: the loser's user-set lock must survive
        // under the keeper, not be dropped with the rest of the loser's row.
        let provenance: Vec<(i64, String, String)> =
            sqlx::query_as("SELECT work_id, field, setter FROM work_metadata_provenance")
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(
            provenance,
            vec![(keeper_id, "title".to_string(), "user".to_string())],
            "the loser's user provenance lock must survive under the keeper: {provenance:?}"
        );

        // import_intents: repointed to the keeper, never cascade-deleted.
        let intents: Vec<(i64, i64)> =
            sqlx::query_as("SELECT work_id, user_id FROM import_intents")
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(
            intents,
            vec![(keeper_id, user_id)],
            "the import_intent row must be repointed to the keeper, not cascade-deleted: {intents:?}"
        );
    }
}
