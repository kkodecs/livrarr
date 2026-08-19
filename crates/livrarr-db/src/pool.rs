use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{SqliteConnection, SqlitePool};
use std::collections::{BTreeMap, BTreeSet};
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

/// Start a transaction that intends to write.
///
/// SQLite's deferred default can read successfully and then reject the first
/// write immediately with `SQLITE_BUSY`, bypassing `busy_timeout`. Reserving
/// the write slot at BEGIN makes concurrent writers queue at the only safe
/// wait point instead.
pub(crate) async fn begin_write(
    pool: &SqlitePool,
) -> Result<sqlx::Transaction<'static, sqlx::Sqlite>, sqlx::Error> {
    pool.begin_with("BEGIN IMMEDIATE").await
}

/// Run embedded migrations.
///
/// Satisfies: RUNTIME-SQLITE-003
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}

// ── Startup checks ──────────────────────────────────────────────────────────

/// Maximum schema_version this binary understands.
///
/// Migration 083 writes this shared compatibility key, and the identity
/// cutover report requires the same version before activation. Keep the guard
/// on the legacy key so databases produced by genuinely newer binaries remain
/// rejected.
const MAX_SCHEMA_VERSION: i64 = 83;
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

/// Preserve a user's own confirmed identity anchors and metadata-field lock
/// on `loser_id` before it is merged into `keeper_id` — shared by the
/// startup dedup backfill ([`backfill_normalized_identity`]) and the live
/// work-merge action so both apply the identical policy. Must run on the
/// caller's own connection/transaction, BEFORE the caller's own generic
/// anchor-merge and provenance-drop statements for the pair.
///
/// Relocates EVERY user-confirmed anchor the loser holds onto the keeper, so
/// none is lost when the loser row is later deleted — split by what the
/// keeper holds for the same `anchor_type`:
/// - keeper holds a NON-user confirmed anchor of that type (contested): the
///   keeper's is cleared and the loser's user anchor moved in — the user's
///   own choice wins;
/// - keeper holds NO confirmed anchor of that type (non-contested): a plain
///   repoint of the loser's user anchor, with nothing on the keeper to
///   displace;
/// - keeper ALREADY holds a user-confirmed anchor of that type: left
///   untouched — the keeper is the surviving row and the one-confirmed-per
///   (work_id, anchor_type) index permits only one, so the keeper's own user
///   anchor stays.
///
/// Also copies the user's own contested provenance lock onto the keeper
/// (clearing whatever on the keeper would otherwise block it). The caller's
/// own generic merge-then-drop statements are left to handle everything else
/// (non-user anchors, non-user provenance) exactly as before.
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

    // Anchor types where the loser holds a user-confirmed anchor and the
    // keeper holds NO confirmed anchor of that type at all — non-contested:
    // the loser's user anchor moves onto the keeper by a plain repoint, with
    // nothing on the keeper to displace. Computed from the same pre-mutation
    // state as the contested set; the two are disjoint by construction (a
    // type is contested only when the keeper DOES hold a confirmed anchor of
    // it), so the order the two loops run in is immaterial.
    let noncontested_anchor_types: Vec<String> = sqlx::query_scalar(
        "SELECT l.anchor_type FROM work_identity_anchors l \
         WHERE l.work_id = ? AND l.user_id = ? AND l.confidence = 'confirmed' AND l.setter = 'user' \
           AND NOT EXISTS ( \
               SELECT 1 FROM work_identity_anchors k \
               WHERE k.work_id = ? AND k.user_id = l.user_id \
                 AND k.anchor_type = l.anchor_type AND k.confidence = 'confirmed' \
           )",
    )
    .bind(loser_id)
    .bind(user_id)
    .bind(keeper_id)
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

    for anchor_type in &noncontested_anchor_types {
        // Plain repoint — the keeper holds no confirmed anchor of this type,
        // so moving the loser's user anchor cannot collide on the
        // (work_id, anchor_type) WHERE-confirmed index. Like the contested
        // move above this is an in-place work_id UPDATE, never
        // insert-then-delete: it leaves (user_id, anchor_type, anchor_value)
        // unchanged, so it cannot self-collide on the (user_id, anchor_type,
        // anchor_value) WHERE-confirmed index either.
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

    let mut tx = crate::pool::begin_write(pool)
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

            // cover_manual follows whichever cover_url wins just above: the
            // keeper's own flag when its cover_url survives unchanged,
            // otherwise the loser's — never a plain COALESCE, which is a
            // no-op here since cover_manual is `INTEGER NOT NULL`.
            let final_cover_manual = if keeper_fields.4.is_some() {
                keeper_fields.5
            } else {
                loser_fields.5
            };

            sqlx::query(
                "UPDATE works SET \
                 monitor_ebook = monitor_ebook OR ?, \
                 monitor_audiobook = monitor_audiobook OR ?, \
                 series_name = COALESCE(series_name, ?), \
                 series_position = COALESCE(series_position, ?), \
                 cover_url = COALESCE(cover_url, ?), \
                 cover_manual = ? \
                 WHERE id = ?",
            )
            .bind(loser_fields.0)
            .bind(loser_fields.1)
            .bind(&loser_fields.2)
            .bind(loser_fields.3)
            .bind(&loser_fields.4)
            .bind(final_cover_manual)
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

/// Backfill `authors.normalized_name`, merge duplicate author rows per
/// (user_id, stored key) through the shared merge contract, and create the
/// partial UNIQUE index — all inside ONE transaction (issue #175, REQ-003).
///
/// Migration 077 added the column NULL-valued and unindexed — installs may
/// hold duplicate author rows. This computes each row's key via
/// `identity_matching::canonical_author_key` (a non-canonicalizable name
/// keeps NULL — the ST-010 exemption), resolves every duplicate group
/// through `merge_authors_tx` — the same policy as the live merge endpoint —
/// onto the D-5 keeper (most works → most external keys → oldest id), then
/// creates `idx_authors_identity`. The `_livrarr_meta` completion marker is
/// the last write before commit: a mid-transaction failure rolls back every
/// data change together with the marker, so a partial run can never be
/// mistaken for a complete one on the next startup.
///
/// Idempotent via the `author_identity_backfill_complete` marker: once
/// stamped, this is a single read and an early return.
pub async fn backfill_author_identity(pool: &SqlitePool) -> Result<(), String> {
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta WHERE key = 'author_identity_backfill_complete'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("read author_identity_backfill_complete: {e}"))?;

    if marker.as_deref() == Some("1") {
        tracing::debug!("author identity backfill: already complete (marker present)");
        return Ok(());
    }

    let mut tx = crate::pool::begin_write(pool)
        .await
        .map_err(|e| format!("begin author identity backfill transaction: {e}"))?;

    // Step 1: compute the stored key for every unkeyed row; an empty-recipe
    // name keeps NULL (ST-010) — "" is never stored.
    let rows: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, name FROM authors WHERE normalized_name IS NULL")
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| format!("select unkeyed authors: {e}"))?;

    let mut keyed = 0usize;
    for (id, name) in &rows {
        let key = livrarr_domain::identity_matching::canonical_author_key(name);
        if key.is_empty() {
            continue;
        }
        sqlx::query("UPDATE authors SET normalized_name = ? WHERE id = ?")
            .bind(&key)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("backfill normalized_name for author {id}: {e}"))?;
        keyed += 1;
    }

    // Step 2: merge duplicate groups per (user_id, key). Keeper per the
    // shipped D-5 policy: most works → most external keys → oldest id.
    let dupes: Vec<(i64, String)> = sqlx::query_as(
        "SELECT user_id, normalized_name FROM authors \
         WHERE normalized_name IS NOT NULL \
         GROUP BY user_id, normalized_name \
         HAVING COUNT(*) > 1",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| format!("scan duplicate authors: {e}"))?;

    if !dupes.is_empty() {
        tracing::warn!(
            "author identity backfill: {} duplicate author groups detected",
            dupes.len()
        );
    }

    let mut merged_count = 0i64;
    for (user_id, key) in &dupes {
        let ranked: Vec<i64> = sqlx::query_scalar(
            "SELECT a.id FROM authors a \
             WHERE a.user_id = ? AND a.normalized_name = ? \
             ORDER BY (SELECT COUNT(*) FROM works w \
                       WHERE w.author_id = a.id AND w.user_id = a.user_id) DESC, \
                      (a.ol_key IS NOT NULL) + (a.gr_key IS NOT NULL) + (a.hc_key IS NOT NULL) DESC, \
                      a.id ASC",
        )
        .bind(user_id)
        .bind(key)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| format!("rank duplicate author group for user {user_id}: {e}"))?;

        let keeper_id = ranked[0];
        for &loser_id in &ranked[1..] {
            crate::sqlite_author::merge_authors_tx(&mut tx, *user_id, keeper_id, loser_id)
                .await
                .map_err(|e| format!("merge author {loser_id} into {keeper_id}: {e}"))?;
            merged_count += 1;
        }
        tracing::info!(
            "author identity backfill: merged {} duplicates into author {keeper_id}",
            ranked.len() - 1
        );
    }

    // Step 3: create the partial UNIQUE index now that groups are resolved;
    // NULL-keyed rows stay outside it (ST-010).
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_authors_identity \
         ON authors(user_id, normalized_name) WHERE normalized_name IS NOT NULL",
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("create idx_authors_identity: {e}"))?;

    // Completion marker — the LAST write before commit; a failure above
    // rolls back every data change together with this marker.
    sqlx::query(
        "INSERT INTO _livrarr_meta (key, value) \
         VALUES ('author_identity_backfill_complete', '1') \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("stamp author_identity_backfill_complete marker: {e}"))?;

    tx.commit()
        .await
        .map_err(|e| format!("commit author identity backfill transaction: {e}"))?;

    tracing::info!(
        "author identity backfill complete: {keyed} authors keyed, {merged_count} duplicates resolved"
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

/// Compiled-in generation for the provider-title policy. This is a runtime
/// data heal, deliberately separate from immutable schema migrations 082-084.
const IDENTITY_TITLE_POLICY_GENERATION: i64 = 2;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdentityTitlePolicyHealReport {
    pub healed: usize,
    pub review_cards_minted: usize,
    pub blocked_cohorts: usize,
    pub article_cohorts: usize,
    pub article_folds: usize,
}

/// Heal pre-policy provider titles after F2 authority is active.
///
/// Every trailing parenthetical recognized by the shared parser moves out of
/// the immutable main title; series markers retain their identity volume and
/// edition qualifiers retain their parsed subtitle. The complete row rewrite,
/// generation bump, and settlement audit share one transaction. If the new key
/// collides, the whole cohort is left untouched and enters the existing
/// GroupIdentity review mechanics; the generation marker intentionally remains
/// behind so a later startup retries after review resolution.
pub async fn heal_identity_title_policy(
    pool: &SqlitePool,
) -> Result<IdentityTitlePolicyHealReport, String> {
    let mut tx = crate::pool::begin_write(pool)
        .await
        .map_err(|error| format!("begin identity title-policy heal: {error}"))?;
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta WHERE key = 'identity_title_policy_generation'",
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| format!("read identity_title_policy_generation: {error}"))?;
    if marker
        .and_then(|value| value.parse::<i64>().ok())
        .is_some_and(|generation| generation >= IDENTITY_TITLE_POLICY_GENERATION)
    {
        tx.rollback()
            .await
            .map_err(|error| format!("close completed identity title-policy heal: {error}"))?;
        return Ok(IdentityTitlePolicyHealReport::default());
    }

    type IdentityKey = (i64, String, String, String, i64, String);
    #[derive(sqlx::FromRow)]
    struct WorkRow {
        id: i64,
        user_id: i64,
        title: String,
        subtitle: Option<String>,
        normalized_identity_main: String,
        normalized_identity_subtitle: String,
        normalized_identity_volume: String,
        primary_author_id: i64,
        text_distinction: String,
        identity_generation: i64,
        enrichment_status: String,
        enriched_at: Option<String>,
        series_id: Option<i64>,
        import_id: Option<String>,
        next_convergence_at: Option<String>,
        cover_url: Option<String>,
        audiobook_cover_url: Option<String>,
    }
    struct HealWork {
        row: WorkRow,
        title: livrarr_domain::identity_layer::IdentityTitleTuple,
        normalized_display_main: String,
        fold_eligible: bool,
    }
    struct Proposal {
        id: i64,
        user_id: i64,
        generation: i64,
        key: IdentityKey,
        title: livrarr_domain::identity_layer::IdentityTitleTuple,
    }

    let raw_rows: Vec<WorkRow> = sqlx::query_as(
        "SELECT id, user_id, title, subtitle, normalized_identity_main, \
                normalized_identity_subtitle, normalized_identity_volume, \
                primary_author_id, text_distinction, identity_generation, \
                enrichment_status, enriched_at, series_id, import_id, \
                next_convergence_at, cover_url, audiobook_cover_url \
           FROM works ORDER BY user_id, id",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| format!("select works for identity title-policy heal: {error}"))?;

    let mut works = Vec::with_capacity(raw_rows.len());
    for row in raw_rows {
        let title = livrarr_domain::identity_layer::title_parts_from_provider(
            row.title.clone(),
            row.subtitle.clone(),
        )
        .map_err(|error| format!("parse identity title for work {}: {error}", row.id))?;
        let normalized_display_main =
            livrarr_domain::title_cleanup::collapse_whitespace(&title.main).to_lowercase();
        let fold_eligible = row.identity_generation == 1
            && row.enrichment_status == "pending"
            && row.enriched_at.is_none()
            && row.series_id.is_none()
            && row.import_id.is_none()
            && row.next_convergence_at.is_none()
            && row.cover_url.is_none()
            && row.audiobook_cover_url.is_none();
        works.push(HealWork {
            row,
            title,
            normalized_display_main,
            fold_eligible,
        });
    }

    // Semantic article cohorts are found before tuple rewrites so pairs whose
    // one-sided volume keeps them out of the unique-index collision set still
    // enter the same match semantics as the add road.
    let strict_lost = livrarr_domain::identity_layer::LostMatchGuardSet {
        one_sided_subtitle_recovery: true,
        shared_edition_id_confirmation: true,
        translation_same_text_signals: Default::default(),
    };
    let strict_wrong = livrarr_domain::identity_layer::WrongMergeGuardSet {
        main_title_guard: livrarr_domain::identity_layer::MainTitleGuard(true),
        volume_conflict_guard: true,
        author_disagreement_guard: true,
        work_key_contradiction_guard: true,
        audited_different_text_guard: true,
    };
    let mut adjacency = vec![BTreeSet::new(); works.len()];
    for left_index in 0..works.len() {
        for right_index in (left_index + 1)..works.len() {
            let left = &works[left_index];
            let right = &works[right_index];
            if left.row.user_id != right.row.user_id
                || left.row.primary_author_id != right.row.primary_author_id
                || left.row.text_distinction != "common"
                || right.row.text_distinction != "common"
                || left.title.normalized_subtitle != right.title.normalized_subtitle
            {
                continue;
            }
            let left_stripped = livrarr_domain::identity_matching::strip_leading_identity_article(
                &left.normalized_display_main,
            );
            let right_stripped = livrarr_domain::identity_matching::strip_leading_identity_article(
                &right.normalized_display_main,
            );
            let differs_only_by_article = left.normalized_display_main
                != right.normalized_display_main
                && left_stripped == right_stripped
                && (left_stripped != left.normalized_display_main
                    || right_stripped != right.normalized_display_main);
            if !differs_only_by_article {
                continue;
            }
            let verdicts = livrarr_domain::identity_layer::evaluate_match(
                livrarr_domain::identity_layer::WorkIdentityEvidence {
                    title: left.title.clone(),
                    primary_author_id: left.row.primary_author_id,
                    routes: Vec::new(),
                },
                livrarr_domain::identity_layer::WorkIdentityEvidence {
                    title: right.title.clone(),
                    primary_author_id: right.row.primary_author_id,
                    routes: Vec::new(),
                },
                strict_lost.clone(),
                strict_wrong.clone(),
            );
            let title_tolerated = matches!(
                verdicts.title,
                livrarr_domain::identity_matching::TitleVerdict::Same
                    | livrarr_domain::identity_matching::TitleVerdict::Grey {
                        cause: livrarr_domain::identity_matching::GreyCause::VolumeAsymmetry,
                        ..
                    }
            );
            if title_tolerated
                && matches!(
                    verdicts.author,
                    livrarr_domain::identity_matching::AuthorVerdict::Agree
                )
            {
                adjacency[left_index].insert(right_index);
                adjacency[right_index].insert(left_index);
            }
        }
    }

    let mut visited = BTreeSet::new();
    let mut article_components: Vec<Vec<usize>> = Vec::new();
    for start in 0..works.len() {
        if adjacency[start].is_empty() || !visited.insert(start) {
            continue;
        }
        let mut stack = vec![start];
        let mut component = Vec::new();
        while let Some(index) = stack.pop() {
            component.push(index);
            for neighbor in &adjacency[index] {
                if visited.insert(*neighbor) {
                    stack.push(*neighbor);
                }
            }
        }
        component.sort_by_key(|index| works[*index].row.id);
        article_components.push(component);
    }

    let mut report = IdentityTitlePolicyHealReport {
        article_cohorts: article_components.len(),
        ..Default::default()
    };
    let mut folded_ids = BTreeSet::new();
    let mut folded_winners: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    let mut review_cohorts: BTreeSet<Vec<i64>> = BTreeSet::new();
    for component in article_components {
        let winner_index = component[0];
        let winner = &works[winner_index];
        let mut safe = true;
        for loser_index in component.iter().skip(1) {
            let loser = &works[*loser_index];
            if !loser.fold_eligible
                || !crate::identity_layer::article_duplicate_is_safe_to_fold(
                    &mut tx,
                    loser.row.user_id,
                    loser.row.id,
                )
                .await?
            {
                safe = false;
                break;
            }
        }
        if safe {
            for loser_index in component.iter().skip(1) {
                let loser = &works[*loser_index];
                crate::identity_layer::absorb_article_duplicate(
                    &mut tx,
                    winner.row.user_id,
                    winner.row.id,
                    loser.row.id,
                )
                .await?;
                folded_ids.insert(loser.row.id);
                folded_winners
                    .entry(winner.row.id)
                    .or_default()
                    .push(loser.row.id);
                report.article_folds += 1;
            }
        } else {
            review_cohorts.insert(component.iter().map(|index| works[*index].row.id).collect());
        }
    }

    let mut current_ids: BTreeMap<IdentityKey, BTreeSet<i64>> = BTreeMap::new();
    let mut proposals = Vec::new();
    for work in works
        .iter()
        .filter(|work| !folded_ids.contains(&work.row.id))
    {
        current_ids
            .entry((
                work.row.user_id,
                work.row.normalized_identity_main.clone(),
                work.row.normalized_identity_subtitle.clone(),
                work.row.normalized_identity_volume.clone(),
                work.row.primary_author_id,
                work.row.text_distinction.clone(),
            ))
            .or_default()
            .insert(work.row.id);
        if work.title.main == livrarr_domain::title_cleanup::collapse_whitespace(&work.row.title)
            && work.title.subtitle == work.row.subtitle
            && work.title.normalized_main == work.row.normalized_identity_main
            && work.title.normalized_subtitle == work.row.normalized_identity_subtitle
            && work.title.normalized_volume == work.row.normalized_identity_volume
        {
            continue;
        }
        proposals.push(Proposal {
            id: work.row.id,
            user_id: work.row.user_id,
            generation: work.row.identity_generation,
            key: (
                work.row.user_id,
                work.title.normalized_main.clone(),
                work.title.normalized_subtitle.clone(),
                work.title.normalized_volume.clone(),
                work.row.primary_author_id,
                work.row.text_distinction.clone(),
            ),
            title: work.title.clone(),
        });
    }

    let mut proposals_by_key: BTreeMap<IdentityKey, Vec<usize>> = BTreeMap::new();
    for (index, proposal) in proposals.iter().enumerate() {
        proposals_by_key
            .entry(proposal.key.clone())
            .or_default()
            .push(index);
    }
    let mut blocked_keys = BTreeSet::new();
    for (key, indexes) in &proposals_by_key {
        let mut cohort = current_ids.get(key).cloned().unwrap_or_default();
        cohort.extend(indexes.iter().map(|index| proposals[*index].id));
        if cohort.len() > 1 {
            blocked_keys.insert(key.clone());
            review_cohorts.insert(cohort.into_iter().collect());
        }
    }

    report.blocked_cohorts = review_cohorts.len();
    for work_ids in &review_cohorts {
        let anchor_id = *work_ids.first().expect("review cohort is nonempty");
        let anchor = works
            .iter()
            .find(|work| work.row.id == anchor_id)
            .expect("review cohort anchor came from selected works");
        let anchor_will_bump = proposals
            .iter()
            .any(|proposal| proposal.id == anchor_id && !blocked_keys.contains(&proposal.key));
        let card_generation = anchor.row.identity_generation + i64::from(anchor_will_bump);
        let payload = serde_json::to_string(
            &livrarr_domain::identity_layer::SettlementReviewCard::GroupIdentity {
                work_ids: work_ids.clone(),
                proposed_identity: None,
                merge_choices: Vec::new(),
            },
        )
        .map_err(|error| format!("serialize title-policy review: {error}"))?;
        let inserted = sqlx::query(
            "INSERT INTO identity_review_cards \
                (user_id, work_id, kind, generation, status, payload, created_at) \
             SELECT ?1, ?2, ?3, ?4, 'pending', ?5, ?6 \
              WHERE NOT EXISTS (SELECT 1 FROM identity_review_cards \
                                 WHERE user_id=?1 AND work_id=?2 AND kind=?3 \
                                   AND status='pending' AND payload=?5)",
        )
        .bind(anchor.row.user_id)
        .bind(anchor.row.id)
        .bind(livrarr_domain::identity_layer::ReviewKind::GroupIdentity.storage_code())
        .bind(card_generation)
        .bind(payload)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(|error| format!("mint title-policy GroupIdentity card: {error}"))?;
        report.review_cards_minted += inserted.rows_affected() as usize;
    }

    for proposal in proposals
        .iter()
        .filter(|proposal| !blocked_keys.contains(&proposal.key))
    {
        let new_generation = proposal.generation + 1;
        let updated = sqlx::query(
            "UPDATE works SET title=?1, subtitle=?2, identity_volume=?3, \
                    normalized_title=?4, normalized_identity_main=?4, \
                    normalized_identity_subtitle=?5, normalized_identity_volume=?6, \
                    identity_generation=?7 \
              WHERE user_id=?8 AND id=?9 AND identity_generation=?10",
        )
        .bind(&proposal.title.main)
        .bind(&proposal.title.subtitle)
        .bind(&proposal.title.volume)
        .bind(&proposal.title.normalized_main)
        .bind(&proposal.title.normalized_subtitle)
        .bind(&proposal.title.normalized_volume)
        .bind(new_generation)
        .bind(proposal.user_id)
        .bind(proposal.id)
        .bind(proposal.generation)
        .execute(&mut *tx)
        .await
        .map_err(|error| format!("heal identity title for work {}: {error}", proposal.id))?;
        if updated.rows_affected() != 1 {
            return Err(format!(
                "identity title-policy generation changed for work {}",
                proposal.id
            ));
        }
        sqlx::query(
            "INSERT INTO identity_audit_events \
                (user_id, work_id, event_kind, actor, payload, created_at) \
             VALUES (?1, ?2, 'settlement', 'identity-title-policy-heal', ?3, ?4)",
        )
        .bind(proposal.user_id)
        .bind(proposal.id)
        .bind(format!("generation={new_generation}"))
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            format!(
                "audit identity title heal for work {}: {error}",
                proposal.id
            )
        })?;
        report.healed += 1;
    }

    // A fold whose winner already needed tuple normalization was claimed by
    // the update above. A bare-title winner still needs one generation/audit
    // for the graph absorption, but no tuple rewrite.
    let proposal_ids: BTreeSet<i64> = proposals
        .iter()
        .filter(|proposal| !blocked_keys.contains(&proposal.key))
        .map(|proposal| proposal.id)
        .collect();
    for (winner_id, loser_ids) in &folded_winners {
        if proposal_ids.contains(winner_id) {
            continue;
        }
        let winner = works
            .iter()
            .find(|work| work.row.id == *winner_id)
            .expect("fold winner came from selected works");
        let updated = sqlx::query(
            "UPDATE works SET identity_generation=identity_generation+1 \
              WHERE user_id=?1 AND id=?2 AND identity_generation=?3",
        )
        .bind(winner.row.user_id)
        .bind(winner.row.id)
        .bind(winner.row.identity_generation)
        .execute(&mut *tx)
        .await
        .map_err(|error| format!("bump article-fold winner {}: {error}", winner.row.id))?;
        if updated.rows_affected() != 1 {
            return Err(format!(
                "article-fold winner generation changed for work {}",
                winner.row.id
            ));
        }
        let new_generation = winner.row.identity_generation + 1;
        sqlx::query(
            "INSERT INTO identity_audit_events \
                (user_id, work_id, event_kind, actor, payload, created_at) \
             VALUES (?1, ?2, 'settlement', 'identity-title-policy-heal', ?3, ?4)",
        )
        .bind(winner.row.user_id)
        .bind(winner.row.id)
        .bind(format!(
            "generation={new_generation};folded_work_ids={}",
            loser_ids
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ))
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(|error| format!("audit article-fold winner {}: {error}", winner.row.id))?;
    }

    if blocked_keys.is_empty() {
        sqlx::query(
            "INSERT INTO _livrarr_meta (key, value) \
             VALUES ('identity_title_policy_generation', ?1) \
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        )
        .bind(IDENTITY_TITLE_POLICY_GENERATION.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|error| format!("stamp identity title-policy generation: {error}"))?;
    }
    tx.commit()
        .await
        .map_err(|error| format!("commit identity title-policy heal: {error}"))?;
    Ok(report)
}

/// Compiled-in generation for the PM seam-sweep route-taxonomy/data-debris
/// repair. This is runtime data healing, not an immutable schema migration.
const IDENTITY_SWEEP_HEAL_GENERATION: i64 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdentitySweepHealReport {
    pub routes_reowned: usize,
    pub editions_created: usize,
    pub works_bumped: usize,
    pub invalid_cards_dismissed: usize,
}

/// Repair edition-scoped routes left Work-owned by the first v2 cutover and
/// dismiss pending-route cards whose route is already owned by another Work.
/// Route re-ownership, one generation/audit per affected Work, card cleanup,
/// and the idempotence marker commit atomically.
pub async fn heal_identity_sweep_findings(
    pool: &SqlitePool,
) -> Result<IdentitySweepHealReport, String> {
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta WHERE key='identity_sweep_heal_generation'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("read identity_sweep_heal_generation: {error}"))?;
    if marker
        .and_then(|value| value.parse::<i64>().ok())
        .is_some_and(|generation| generation >= IDENTITY_SWEEP_HEAL_GENERATION)
    {
        return Ok(IdentitySweepHealReport::default());
    }

    type RouteRow = (i64, i64, i64, String, String);
    let mut tx = crate::pool::begin_write(pool)
        .await
        .map_err(|error| format!("begin identity sweep heal: {error}"))?;
    let edition_kinds = [
        livrarr_domain::identity_layer::RouteKind::Isbn13Edition,
        livrarr_domain::identity_layer::RouteKind::AsinEdition,
        livrarr_domain::identity_layer::RouteKind::GoodreadsBookEdition,
    ]
    .map(|kind| serde_json::to_string(&kind).expect("RouteKind serialization"));
    let routes: Vec<RouteRow> = sqlx::query_as(
        "SELECT id, user_id, resolved_work_id, provider, provider_scoped_id \
           FROM identity_routes \
          WHERE owner_type='work' AND state='active' \
            AND kind IN (?1, ?2, ?3) ORDER BY user_id, resolved_work_id, id",
    )
    .bind(&edition_kinds[0])
    .bind(&edition_kinds[1])
    .bind(&edition_kinds[2])
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| format!("select work-owned edition routes: {error}"))?;
    let mut report = IdentitySweepHealReport::default();
    let mut affected_works = BTreeSet::new();
    for (route_id, user_id, work_id, provider, provider_scoped_id) in routes {
        let edition_id: i64 = if let Some(existing) = sqlx::query_scalar(
            "SELECT id FROM editions WHERE user_id=?1 AND work_id=?2 AND state='active' \
               AND source_provider=?3 AND provider_edition_id=?4 ORDER BY id LIMIT 1",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(&provider)
        .bind(&provider_scoped_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|error| format!("find edition for route {route_id}: {error}"))?
        {
            existing
        } else {
            let inserted = sqlx::query(
                "INSERT INTO editions \
                    (user_id, work_id, format, source_provider, provider_edition_id, state) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 'active')",
            )
            .bind(user_id)
            .bind(work_id)
            .bind(
                serde_json::to_string(&livrarr_domain::identity_layer::EditionFormat::Unknown)
                    .expect("EditionFormat serialization"),
            )
            .bind(&provider)
            .bind(&provider_scoped_id)
            .execute(&mut *tx)
            .await
            .map_err(|error| format!("create edition for route {route_id}: {error}"))?;
            report.editions_created += 1;
            inserted.last_insert_rowid()
        };
        let updated = sqlx::query(
            "UPDATE identity_routes SET owner_type='edition', work_id=NULL, edition_id=?1 \
              WHERE id=?2 AND user_id=?3 AND owner_type='work'",
        )
        .bind(edition_id)
        .bind(route_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|error| format!("re-own edition route {route_id}: {error}"))?;
        report.routes_reowned += updated.rows_affected() as usize;
        if updated.rows_affected() == 1 {
            affected_works.insert((user_id, work_id));
        }
    }

    for (user_id, work_id) in affected_works {
        let updated = sqlx::query(
            "UPDATE works SET identity_generation=identity_generation+1 \
              WHERE user_id=?1 AND id=?2",
        )
        .bind(user_id)
        .bind(work_id)
        .execute(&mut *tx)
        .await
        .map_err(|error| format!("bump route-taxonomy work {work_id}: {error}"))?;
        if updated.rows_affected() != 1 {
            return Err(format!("route-taxonomy Work {work_id} disappeared"));
        }
        let generation: i64 =
            sqlx::query_scalar("SELECT identity_generation FROM works WHERE user_id=?1 AND id=?2")
                .bind(user_id)
                .bind(work_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|error| format!("read healed generation for work {work_id}: {error}"))?;
        sqlx::query(
            "INSERT INTO identity_audit_events \
                (user_id, work_id, event_kind, actor, payload, created_at) \
             VALUES (?1, ?2, 'settlement', 'identity-route-taxonomy-heal', ?3, ?4)",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(format!("generation={generation}"))
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(|error| format!("audit route-taxonomy work {work_id}: {error}"))?;
        report.works_bumped += 1;
    }

    type CardRow = (i64, i64, Option<i64>, String);
    let cards: Vec<CardRow> = sqlx::query_as(
        "SELECT id, user_id, work_id, payload FROM identity_review_cards \
          WHERE status='pending' AND kind=?1 ORDER BY id",
    )
    .bind(livrarr_domain::identity_layer::ReviewKind::PendingRoute.storage_code())
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| format!("select pending-route debris: {error}"))?;
    for (card_id, user_id, work_id, payload) in cards {
        let card: livrarr_domain::identity_layer::SettlementReviewCard =
            serde_json::from_str(&payload)
                .map_err(|error| format!("decode pending-route card {card_id}: {error}"))?;
        let livrarr_domain::identity_layer::SettlementReviewCard::PendingRoute {
            candidate, ..
        } = card
        else {
            continue;
        };
        let provider = serde_json::to_string(&candidate.route.provider)
            .map_err(|error| format!("encode card {card_id} provider: {error}"))?;
        let kind = serde_json::to_string(&candidate.route.kind)
            .map_err(|error| format!("encode card {card_id} kind: {error}"))?;
        let owner: Option<i64> = sqlx::query_scalar(
            "SELECT resolved_work_id FROM identity_routes \
              WHERE user_id=?1 AND provider=?2 AND kind=?3 AND provider_scoped_id=?4 \
                AND state='active' LIMIT 1",
        )
        .bind(user_id)
        .bind(provider)
        .bind(kind)
        .bind(&candidate.route.value)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|error| format!("check owner for pending card {card_id}: {error}"))?;
        if owner.is_none() || owner == work_id {
            continue;
        }
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE identity_review_cards SET status='cancelled', resolved_at=?1 \
              WHERE id=?2 AND user_id=?3 AND status='pending'",
        )
        .bind(&now)
        .bind(card_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|error| format!("dismiss invalid pending card {card_id}: {error}"))?;
        sqlx::query(
            "INSERT INTO identity_audit_events \
                (user_id, work_id, event_kind, actor, payload, created_at) \
             VALUES (?1, ?2, 'review-dismissal', 'identity-sweep-heal', ?3, ?4)",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(format!(
            "card_id={card_id};reason=route-owned-by-another-work"
        ))
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| format!("audit invalid pending card {card_id}: {error}"))?;
        report.invalid_cards_dismissed += 1;
    }

    sqlx::query(
        "INSERT INTO _livrarr_meta (key, value) \
         VALUES ('identity_sweep_heal_generation', ?1) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
    )
    .bind(IDENTITY_SWEEP_HEAL_GENERATION.to_string())
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("stamp identity sweep heal generation: {error}"))?;
    tx.commit()
        .await
        .map_err(|error| format!("commit identity sweep heal: {error}"))?;
    Ok(report)
}

/// Clear Goodreads-work anchor dead ends created before the subtitle matching
/// rule changed, and make the affected works immediately convergence-eligible.
///
/// The completion marker is checked before opening the transaction, so every
/// later startup performs only that one read and a debug log. On the first
/// startup, clock clearing, provider-scoped dead-end deletion, and marker
/// stamping commit atomically.
pub async fn clear_subtitle_rule_deadends(pool: &SqlitePool) -> Result<(), String> {
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta WHERE key = 'subtitle_rule_deadend_clear_v1'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("read subtitle_rule_deadend_clear_v1 marker: {e}"))?;

    if marker.as_deref() == Some("1") {
        tracing::debug!("subtitle-rule dead-end clear: already complete");
        return Ok(());
    }

    let mut tx = crate::pool::begin_write(pool)
        .await
        .map_err(|e| format!("begin subtitle-rule dead-end clear transaction: {e}"))?;

    let clock_result = sqlx::query(
        "UPDATE works SET next_convergence_at = NULL \
         WHERE id IN ( \
             SELECT work_id FROM work_anchor_dead_ends WHERE anchor_type = 'gr_work' \
         )",
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("clear convergence clocks for gr_work dead ends: {e}"))?;

    let delete_result =
        sqlx::query("DELETE FROM work_anchor_dead_ends WHERE anchor_type = 'gr_work'")
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("delete gr_work anchor dead ends: {e}"))?;

    sqlx::query(
        "INSERT INTO _livrarr_meta (key, value) \
         VALUES ('subtitle_rule_deadend_clear_v1', '1') \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("stamp subtitle_rule_deadend_clear_v1 marker: {e}"))?;

    tx.commit()
        .await
        .map_err(|e| format!("commit subtitle-rule dead-end clear transaction: {e}"))?;

    tracing::info!(
        clocks_cleared = clock_result.rows_affected(),
        dead_ends_deleted = delete_result.rows_affected(),
        "subtitle-rule dead-end clear complete"
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

        // A user-confirmed anchor on the loser of a type the keeper has NO
        // anchor for at all (non-contested) — must also move onto the keeper,
        // not be lost when the loser row is deleted.
        sqlx::query(
            "INSERT INTO work_identity_anchors \
             (work_id, anchor_type, anchor_value, confidence, setter, set_at, user_id) \
             VALUES (?, 'isbn_13', 'USERISBN', 'confirmed', 'user', '2026-01-01', ?)",
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

        // work_identity_anchors: BOTH of the loser's user anchors survive
        // onto the keeper — the contested gr_key wins over the keeper's own
        // auto_search one, and the non-contested isbn_13 (a type the keeper
        // lacked) is repointed rather than lost.
        let anchors: Vec<(i64, String, String, String, String)> = sqlx::query_as(
            "SELECT work_id, anchor_type, anchor_value, confidence, setter \
             FROM work_identity_anchors ORDER BY anchor_type",
        )
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(
            anchors,
            vec![
                (
                    keeper_id,
                    "gr_key".to_string(),
                    "USER456".to_string(),
                    "confirmed".to_string(),
                    "user".to_string()
                ),
                (
                    keeper_id,
                    "isbn_13".to_string(),
                    "USERISBN".to_string(),
                    "confirmed".to_string(),
                    "user".to_string()
                ),
            ],
            "both the contested (gr_key) and non-contested (isbn_13) loser user anchors \
             must survive onto the keeper: {anchors:?}"
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

/// REQ-003/REQ-004 (issue #175): `backfill_author_identity` must be one
/// atomic, marker-guarded, idempotent transaction that repairs duplicate
/// authors through the shared merge contract BEFORE arming the unique
/// index. Tests seed the REAL legacy precondition — `authors` rows with
/// NULL `normalized_name`, exactly what migration 077 leaves on an upgraded
/// install — via direct SQL, not `create_author()`, whose named ON CONFLICT
/// target requires the very unique index this function is responsible for
/// creating (the same justification as `backfill_normalized_identity_tests`
/// above; spec AC-003).
#[cfg(test)]
mod backfill_author_identity_tests {
    use super::*;
    use crate::sqlite::SqliteDb;
    use crate::test_helpers::create_test_db;
    use crate::{CreateUserDbRequest, UserDb, UserRole};
    use livrarr_domain::identity_matching::canonical_author_key;

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

    /// The test harness bootstraps the post-repair author index (so
    /// `create_author`'s conflict target always resolves); this function's
    /// whole job runs BEFORE that index exists, so repair tests drop it to
    /// seed the legacy state.
    async fn drop_harness_author_index(pool: &SqlitePool) {
        sqlx::query("DROP INDEX IF EXISTS idx_authors_identity")
            .execute(pool)
            .await
            .unwrap();
    }

    /// Raw-insert an author in the legacy pre-backfill state: NULL
    /// `normalized_name`, exactly what migration 077 leaves behind.
    async fn seed_legacy_author(
        pool: &SqlitePool,
        user_id: i64,
        name: &str,
        ol_key: Option<&str>,
        gr_key: Option<&str>,
        monitored: bool,
    ) -> i64 {
        sqlx::query(
            "INSERT INTO authors (user_id, name, ol_key, gr_key, monitored, added_at) \
             VALUES (?, ?, ?, ?, ?, '2026-01-01T00:00:00Z')",
        )
        .bind(user_id)
        .bind(name)
        .bind(ol_key)
        .bind(gr_key)
        .bind(monitored)
        .execute(pool)
        .await
        .unwrap()
        .last_insert_rowid()
    }

    async fn seed_work(
        pool: &SqlitePool,
        user_id: i64,
        author_id: i64,
        title: &str,
        series_id: Option<i64>,
    ) -> i64 {
        sqlx::query(
            "INSERT INTO works (user_id, title, author_name, normalized_title, \
             normalized_author, author_id, series_id, enrichment_status, added_at) \
             VALUES (?, ?, 'seed', ?, 'seed-author', ?, ?, 'unenriched', '2026-01-01')",
        )
        .bind(user_id)
        .bind(title)
        .bind(title.to_lowercase())
        .bind(author_id)
        .bind(series_id)
        .execute(pool)
        .await
        .unwrap()
        .last_insert_rowid()
    }

    async fn seed_series(
        pool: &SqlitePool,
        user_id: i64,
        author_id: i64,
        name: &str,
        gr_key: &str,
        monitor_ebook: bool,
    ) -> i64 {
        sqlx::query(
            "INSERT INTO series (user_id, author_id, name, gr_key, monitor_ebook) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(author_id)
        .bind(name)
        .bind(gr_key)
        .bind(monitor_ebook)
        .execute(pool)
        .await
        .unwrap()
        .last_insert_rowid()
    }

    async fn seed_caches(pool: &SqlitePool, author_id: i64) {
        sqlx::query("INSERT INTO author_series_cache (author_id, entries) VALUES (?, '[]')")
            .bind(author_id)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO author_bibliography (author_id, entries) VALUES (?, '[]')")
            .bind(author_id)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn marker_value(pool: &SqlitePool) -> Option<String> {
        sqlx::query_scalar(
            "SELECT value FROM _livrarr_meta WHERE key = 'author_identity_backfill_complete'",
        )
        .fetch_optional(pool)
        .await
        .unwrap()
    }

    async fn author_index_exists(pool: &SqlitePool) -> bool {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master \
             WHERE type = 'index' AND name = 'idx_authors_identity'",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        n > 0
    }

    async fn authors_snapshot(
        pool: &SqlitePool,
    ) -> Vec<(
        i64,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        bool,
    )> {
        sqlx::query_as(
            "SELECT id, name, normalized_name, ol_key, gr_key, monitored \
             FROM authors ORDER BY id",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    // ---- (a) Full repair: D-5 keeper, fold/move arms, caches, junk pair,
    //      display rewrite, monotonic fields, index, marker, idempotency ----

    #[tokio::test]
    async fn repair_merges_groups_via_shared_contract_with_d5_keeper_and_arms_index() {
        let db = create_test_db().await;
        drop_harness_author_index(db.pool()).await;
        let user_id = seed_user(&db, "author-repair-user").await;

        // Group 1 — byte-identical names, the exact #175 field shape. The
        // D-5 keeper (most works) is deliberately NOT the oldest id.
        let a1 = seed_legacy_author(db.pool(), user_id, "Anne Rice", None, None, false).await;
        let a2 =
            seed_legacy_author(db.pool(), user_id, "Anne Rice", Some("OL-A2"), None, false).await;
        let a3 =
            seed_legacy_author(db.pool(), user_id, "Anne Rice", None, Some("GR-A3"), false).await;

        // Same-gr_key series on keeper AND loser -> FOLD arm (flags OR);
        // loser-only series -> MOVE arm.
        let s_keeper = seed_series(db.pool(), user_id, a2, "Vampire Chronicles", "VC", false).await;
        let s_loser = seed_series(db.pool(), user_id, a1, "Vampire Chronicles", "VC", true).await;
        seed_series(db.pool(), user_id, a3, "Mayfair Witches", "MW", false).await;

        // Works scattered across the duplicates; a1's work rides the loser
        // series so the fold must repoint it without unlinking.
        let w1 = seed_work(db.pool(), user_id, a1, "Interview", Some(s_loser)).await;
        let w2 = seed_work(db.pool(), user_id, a2, "Lestat", None).await;
        let w3 = seed_work(db.pool(), user_id, a2, "Queen of the Damned", None).await;

        seed_caches(db.pool(), a1).await;
        seed_caches(db.pool(), a3).await;

        // Group 2 — display variants converging on one canonical key:
        // keeper by most works; the loser carries the only ol_key and the
        // only monitored=true (monotonic fill + OR).
        let b1 = seed_legacy_author(db.pool(), user_id, "J.K. Rowling", None, None, false).await;
        let b2 = seed_legacy_author(
            db.pool(),
            user_id,
            "J. K. Rowling",
            Some("OL-B2"),
            None,
            true,
        )
        .await;
        seed_work(db.pool(), user_id, b1, "Casual Vacancy", None).await;
        seed_work(db.pool(), user_id, b1, "Ickabog", None).await;
        let bw = seed_work(db.pool(), user_id, b2, "Christmas Pig", None).await;

        // Group 3 — works-tied: the key-count tiebreak decides, again NOT
        // the oldest id.
        let c1 = seed_legacy_author(db.pool(), user_id, "Ursula Vernon", None, None, false).await;
        let c2 = seed_legacy_author(
            db.pool(),
            user_id,
            "Ursula Vernon",
            Some("OL-C2"),
            Some("GR-C2"),
            false,
        )
        .await;

        // ST-010 pair: two distinct junk-named authors stay separate.
        let j1 = seed_legacy_author(db.pool(), user_id, "Jr.", None, None, false).await;
        let j2 = seed_legacy_author(db.pool(), user_id, "(Editor)", None, None, false).await;

        backfill_author_identity(db.pool()).await.unwrap();

        // Group 1: keeper a2 (most works, not oldest), losers gone.
        let anne_rows: Vec<(i64, Option<String>)> =
            sqlx::query_as("SELECT id, normalized_name FROM authors WHERE name = 'Anne Rice'")
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(
            anne_rows.len(),
            1,
            "exactly one Anne Rice row must survive: {anne_rows:?}"
        );
        assert_eq!(anne_rows[0].0, a2, "D-5 keeper is most-works, not oldest");
        assert_eq!(
            anne_rows[0].1.as_deref(),
            Some(canonical_author_key("Anne Rice").as_str()),
            "survivor carries the recipe's stored key"
        );

        // All three works on the keeper; w1 repointed to the folded series.
        let work_rows: Vec<(i64, i64, Option<i64>)> =
            sqlx::query_as("SELECT id, author_id, series_id FROM works WHERE id IN (?, ?, ?)")
                .bind(w1)
                .bind(w2)
                .bind(w3)
                .fetch_all(db.pool())
                .await
                .unwrap();
        for (wid, author_id, _) in &work_rows {
            assert_eq!(*author_id, a2, "work {wid} must be parented to the keeper");
        }
        let w1_series: Option<i64> = work_rows.iter().find(|r| r.0 == w1).unwrap().2;
        assert_eq!(
            w1_series,
            Some(s_keeper),
            "fold must repoint the loser-series work, never unlink it"
        );

        // FOLD: one VC row under the keeper with OR'd flag + the language
        // backstop; MOVE: MW travels intact to the keeper.
        let vc: Vec<(i64, i64, bool, Option<String>)> = sqlx::query_as(
            "SELECT id, author_id, monitor_ebook, monitor_language FROM series WHERE gr_key = 'VC'",
        )
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(vc.len(), 1, "fold leaves exactly one VC series: {vc:?}");
        assert_eq!(vc[0].0, s_keeper);
        assert_eq!(vc[0].1, a2);
        assert!(vc[0].2, "loser's monitor_ebook must OR into the fold");
        assert_eq!(
            vc[0].3.as_deref(),
            Some("en"),
            "monitored fold with no language gets the invariant backstop"
        );
        let mw_author: i64 = sqlx::query_scalar("SELECT author_id FROM series WHERE gr_key = 'MW'")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(mw_author, a2, "loser-only series must MOVE to the keeper");

        // Group 2: display rewrite + monotonic fields.
        let jk: Vec<(i64, Option<String>, bool)> = sqlx::query_as(
            "SELECT id, ol_key, monitored FROM authors WHERE name LIKE 'J.%Rowling'",
        )
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(jk.len(), 1, "one Rowling row must survive: {jk:?}");
        assert_eq!(jk[0].0, b1, "keeper by most works");
        // The frozen scalar column is not filled by a merge any more (FP-031).
        // The loser's provider linkage survives instead as its *staged* legacy
        // value moving to the keeper, which is what the later cutover ingestion
        // reads; that move is pinned in
        // `sqlite_author_link::route_history_and_variant_tests`.
        assert_eq!(
            jk[0].1, None,
            "the merge must not write the frozen scalar column"
        );
        assert!(jk[0].2, "loser's monitored must OR onto the keeper");
        let bw_display: String = sqlx::query_scalar("SELECT author_name FROM works WHERE id = ?")
            .bind(bw)
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(
            bw_display, "J.K. Rowling",
            "merged work's display author must be rewritten to the keeper spelling"
        );

        // Group 3: works tie -> most external keys wins (again not oldest).
        let ursula: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM authors WHERE name = 'Ursula Vernon'")
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(ursula, vec![c2], "key-count tiebreak keeps {c2}, not {c1}");

        // ST-010: junk-named rows stay separate with NULL keys.
        let junk: Vec<(i64, Option<String>)> = sqlx::query_as(
            "SELECT id, normalized_name FROM authors WHERE id IN (?, ?) ORDER BY id",
        )
        .bind(j1)
        .bind(j2)
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(
            junk,
            vec![(j1, None), (j2, None)],
            "junk-named authors keep NULL keys and stay separate"
        );

        // Zero references to any deleted row in ANY referencing table.
        for table in [
            "works",
            "series",
            "author_series_cache",
            "author_bibliography",
        ] {
            let n: i64 = sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM {table} WHERE author_id IN (?, ?, ?, ?)"
            ))
            .bind(a1)
            .bind(a3)
            .bind(b2)
            .bind(c1)
            .fetch_one(db.pool())
            .await
            .unwrap();
            assert_eq!(n, 0, "{table} must hold no reference to a deleted row");
        }

        assert!(author_index_exists(db.pool()).await, "index must be armed");
        assert_eq!(marker_value(db.pool()).await.as_deref(), Some("1"));

        // Re-run: marker short-circuits, state byte-identical.
        let before = authors_snapshot(db.pool()).await;
        backfill_author_identity(db.pool()).await.unwrap();
        assert_eq!(
            before,
            authors_snapshot(db.pool()).await,
            "a re-run after completion must be a no-op"
        );
    }

    // ---- (b) AC-004: DB-level enforcement + NULL exemption ----

    #[tokio::test]
    async fn db_rejects_duplicate_key_after_repair_and_null_keys_stay_exempt() {
        let db = create_test_db().await;
        drop_harness_author_index(db.pool()).await;
        let user_id = seed_user(&db, "author-enforce-user").await;
        seed_legacy_author(db.pool(), user_id, "Carl Sagan", None, None, false).await;

        backfill_author_identity(db.pool()).await.unwrap();
        assert!(author_index_exists(db.pool()).await);

        // A direct duplicate insert — bypassing every service layer — must
        // fail at the database itself.
        let key = canonical_author_key("Carl Sagan");
        let dup = sqlx::query(
            "INSERT INTO authors (user_id, name, normalized_name, added_at) \
             VALUES (?, 'Carl Sagan Copy', ?, '2026-01-02T00:00:00Z')",
        )
        .bind(user_id)
        .bind(&key)
        .execute(db.pool())
        .await;
        assert!(
            dup.is_err(),
            "a raw duplicate (user, key) insert must fail at the DB"
        );
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM authors WHERE normalized_name = ?")
                .bind(&key)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(count, 1, "no second row may exist for the key");

        // NULL keys are exempt: two junk rows both insert (ST-010).
        for name in ["Jr.", "(Editor)"] {
            sqlx::query(
                "INSERT INTO authors (user_id, name, normalized_name, added_at) \
                 VALUES (?, ?, NULL, '2026-01-02T00:00:00Z')",
            )
            .bind(user_id)
            .bind(name)
            .execute(db.pool())
            .await
            .unwrap();
        }
        let nulls: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM authors WHERE user_id = ? AND normalized_name IS NULL",
        )
        .bind(user_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(nulls, 2, "NULL-key rows are exempt from the unique index");
    }

    // ---- (c) AC-005: mid-transaction failpoint ----

    #[tokio::test]
    async fn mid_transaction_failure_rolls_back_keys_merges_index_and_marker_together() {
        let db = create_test_db().await;
        drop_harness_author_index(db.pool()).await;
        let user_id = seed_user(&db, "author-failpoint-user").await;

        // A singleton row proves Step 1's key backfill rolls back too.
        let singleton =
            seed_legacy_author(db.pool(), user_id, "Solo Writer", None, None, false).await;

        // A duplicate pair whose merge must touch author_bibliography.
        let keeper = seed_legacy_author(db.pool(), user_id, "Anne Rice", None, None, false).await;
        let loser = seed_legacy_author(db.pool(), user_id, "Anne Rice", None, None, false).await;
        seed_work(db.pool(), user_id, keeper, "Interview", None).await;
        seed_caches(db.pool(), loser).await;

        // The failpoint: drop a table the merge must touch, forcing a real,
        // naturally triggered SQL error partway through the repair.
        sqlx::query("DROP TABLE author_bibliography")
            .execute(db.pool())
            .await
            .unwrap();

        let result = backfill_author_identity(db.pool()).await;
        assert!(
            result.is_err(),
            "a mid-transaction SQL error must surface as Err"
        );

        let singleton_key: Option<String> =
            sqlx::query_scalar("SELECT normalized_name FROM authors WHERE id = ?")
                .bind(singleton)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(
            singleton_key, None,
            "Step 1's key backfill must roll back with the rest"
        );
        let pair_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM authors WHERE id IN (?, ?)")
            .bind(keeper)
            .bind(loser)
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(pair_count, 2, "no merge may survive the rollback");
        assert_eq!(marker_value(db.pool()).await, None, "marker rolls back too");
        assert!(
            !author_index_exists(db.pool()).await,
            "index rolls back too"
        );

        // A subsequent clean run completes the repair fully (the dropped
        // table is restored to its migrated shape: 003 + 033).
        sqlx::query(
            "CREATE TABLE author_bibliography ( \
                 author_id INTEGER PRIMARY KEY REFERENCES authors(id) ON DELETE CASCADE, \
                 entries TEXT NOT NULL DEFAULT '[]', \
                 fetched_at TEXT NOT NULL DEFAULT (datetime('now')), \
                 raw_entries TEXT \
             )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        backfill_author_identity(db.pool()).await.unwrap();

        let anne_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM authors WHERE name = 'Anne Rice'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(anne_count, 1, "the clean run must complete the merge");
        let survivor: i64 = sqlx::query_scalar("SELECT id FROM authors WHERE name = 'Anne Rice'")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(survivor, keeper, "D-5: the row with works survives");
        assert!(author_index_exists(db.pool()).await);
        assert_eq!(marker_value(db.pool()).await.as_deref(), Some("1"));
    }
}

#[cfg(test)]
mod subtitle_rule_deadend_clear_tests {
    use super::*;
    use crate::sqlite::SqliteDb;
    use crate::test_helpers::create_test_db;
    use crate::{CreateUserDbRequest, CreateWorkDbRequest, UserDb, UserRole, WorkDb, WorkDbCreate};
    use chrono::{Duration, Utc};

    async fn seed_user(db: &SqliteDb) -> i64 {
        db.create_user(CreateUserDbRequest {
            username: "subtitle-repair-user".into(),
            password_hash: "hash".into(),
            role: UserRole::User,
            api_key_hash: "subtitle-repair-key".into(),
        })
        .await
        .unwrap()
        .id
    }

    async fn seed_work(db: &SqliteDb, user_id: i64) -> i64 {
        let (work, created) = db
            .create_work(CreateWorkDbRequest {
                user_id,
                title: "Einstein".into(),
                author_name: "Walter Isaacson".into(),
                normalized_title: "einstein".into(),
                normalized_author: "walter isaacson".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(created);
        work.id
    }

    async fn seed_gr_dead_end(
        db: &SqliteDb,
        work_id: i64,
        user_id: i64,
        next_convergence_at: &str,
    ) {
        sqlx::query(
            "UPDATE works \
             SET identity_status = 'confirmed', identity_status_v2 = 'connected', \
                 enrichment_status = 'enriched', \
                 ol_key = '/works/OL1W', hc_key = '1', isbn_13 = '9780743264747', \
                 asin = 'B000000001', next_convergence_at = ? \
             WHERE id = ?",
        )
        .bind(next_convergence_at)
        .bind(work_id)
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO work_anchor_dead_ends \
             (work_id, anchor_type, attempt_count, last_attempt_at, user_id) \
             VALUES (?, 'gr_work', 3, ?, ?)",
        )
        .bind(work_id)
        .bind(Utc::now().to_rfc3339())
        .bind(user_id)
        .execute(db.pool())
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn subtitle_rule_deadend_clear_unsticks_gr_work_once_then_marker_noops() {
        let db = create_test_db().await;
        // This heal is deliberately pre-activation legacy work. Active v2
        // selection reads typed routes/attempts and never consults this scalar
        // dead-end ledger.
        sqlx::query(
            "UPDATE _livrarr_meta SET value='inactive' \
              WHERE key='identity_authority_v2'",
        )
        .execute(db.pool())
        .await
        .unwrap();
        let user_id = seed_user(&db).await;
        let work_id = seed_work(&db, user_id).await;
        let future_clock = (Utc::now() + Duration::hours(1)).to_rfc3339();
        seed_gr_dead_end(&db, work_id, user_id, &future_clock).await;

        assert!(
            db.list_convergence_due(user_id, Utc::now(), 3, 100)
                .await
                .unwrap()
                .is_empty(),
            "the threshold dead end plus future clock must exclude the work before repair"
        );

        clear_subtitle_rule_deadends(db.pool()).await.unwrap();

        let dead_end_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM work_anchor_dead_ends \
             WHERE work_id = ? AND anchor_type = 'gr_work'",
        )
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(dead_end_count, 0, "the gr_work dead end must be deleted");

        let repaired_clock: Option<String> =
            sqlx::query_scalar("SELECT next_convergence_at FROM works WHERE id = ?")
                .bind(work_id)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(
            repaired_clock, None,
            "the same work's convergence clock must be cleared"
        );
        assert_eq!(
            db.list_convergence_due(user_id, Utc::now(), 3, 100)
                .await
                .unwrap(),
            vec![work_id],
            "the real convergence selector must see the repaired work"
        );

        seed_gr_dead_end(&db, work_id, user_id, &future_clock).await;
        clear_subtitle_rule_deadends(db.pool()).await.unwrap();

        let second_dead_end_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM work_anchor_dead_ends \
             WHERE work_id = ? AND anchor_type = 'gr_work'",
        )
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(
            second_dead_end_count, 1,
            "the marker must make every later boot a strict no-op"
        );
        let second_clock: Option<String> =
            sqlx::query_scalar("SELECT next_convergence_at FROM works WHERE id = ?")
                .bind(work_id)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(
            second_clock,
            Some(future_clock),
            "the marker-gated no-op must leave the re-seeded clock untouched"
        );
    }
}
