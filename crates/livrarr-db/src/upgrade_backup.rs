//! One protected pre-upgrade copy per upgrade.
//!
//! An upgrade is a start whose binary does different startup work than the
//! binary that last finished starting on this database: its startup
//! generation (the highest embedded migration plus each marker-gated startup
//! repair's key and target) differs from `_livrarr_meta.startup_generation`.
//! Before any database write of such a start, the live database is copied with
//! `VACUUM INTO` to a `.partial` work file, published by rename, and only then
//! recorded as the attempt in `_livrarr_meta.upgrade_snapshot`. No migration or
//! repair runs until that record commits. A copy published but never recorded
//! is named by a pending file beside the database and reclaimed on the next
//! start. The copy itself carries its own name, so a database restored from
//! it, which predates the attempt's record, still keeps that copy protected
//! as the next attempt's previous rollback copy until that attempt finishes.
//!
//! An attempt stays `in_progress` until every applicable repair has stamped
//! its marker, including the Goodreads cover repair that runs after serving
//! starts. While it is in progress its copy, and the rollback copy of the
//! attempt before it, are protected from pruning. A restart during an attempt
//! reuses its copy once anything has changed since the copy was taken, and
//! retakes it in place only when nothing has; if something has changed and the
//! copy is gone, startup stops rather than save a later state under its name.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// File-name prefix of every pre-upgrade copy.
pub const COPY_PREFIX: &str = "livrarr.db.pre-migrate-";
/// Suffix of a copy's work file before publication.
pub const PARTIAL_SUFFIX: &str = ".partial";
/// Names the copy a new attempt is publishing, until its record commits.
const PENDING_FILE: &str = "livrarr.db.upgrade-copy-pending";
const RECORD_KEY: &str = "upgrade_snapshot";
/// Written only inside a new attempt's copy, naming that copy: a database
/// restored from it names the copy it came from.
const ROLLBACK_COPY_KEY: &str = "upgrade_rollback_copy";
const GENERATION_KEY: &str = "startup_generation";
/// Activation is a startup write the upgrade makes outside any migration.
const AUTHORITY_MARKER: &str = "identity_authority_v2";

/// A marker-gated startup repair: its `_livrarr_meta` key, the generation it
/// stamps when complete, and whether this binary runs it at all.
struct StartupRepair {
    marker: &'static str,
    target: i64,
    applies: bool,
}

/// The repairs `init_database` runs after migrations, in order, followed by
/// the Goodreads cover repair, which runs in the background after serving
/// starts and can commit queue and slot changes before stamping its marker.
fn startup_repairs() -> [StartupRepair; 9] {
    use crate::identity_layer as il;
    // The title-policy and dedup-residue repairs return before reading or
    // stamping their markers while work merging is unavailable.
    let merging = livrarr_domain::identity_layer::WORK_MERGING_AVAILABLE;
    let repair = |marker, target, applies| StartupRepair {
        marker,
        target,
        applies,
    };
    [
        repair("identity_review_dismissal_adoption_v1", 1, true),
        repair(
            "identity_title_policy_generation",
            crate::pool::IDENTITY_TITLE_POLICY_GENERATION,
            merging,
        ),
        repair(
            "identity_sweep_heal_generation",
            crate::pool::IDENTITY_SWEEP_HEAL_GENERATION,
            true,
        ),
        repair(
            "identity_dedup_residue_heal_generation",
            il::IDENTITY_DEDUP_RESIDUE_HEAL_GENERATION,
            merging,
        ),
        repair(
            "identity_round10_residue_heal_generation",
            il::IDENTITY_ROUND10_RESIDUE_HEAL_GENERATION,
            true,
        ),
        repair(
            "identity_round11_attempt_reheal",
            il::IDENTITY_ROUND11_ATTEMPT_REHEAL_GENERATION,
            true,
        ),
        repair(
            "identity_round15_search_ledger_reset",
            il::IDENTITY_ROUND15_SEARCH_LEDGER_RESET_GENERATION,
            true,
        ),
        repair(
            "identity_round21_goodreads_book_namespace_heal",
            il::IDENTITY_ROUND21_GOODREADS_NAMESPACE_HEAL_GENERATION,
            true,
        ),
        repair(
            "identity_round15_gr_cover_reselect",
            il::IDENTITY_ROUND15_GR_COVER_RESELECT_GENERATION,
            true,
        ),
    ]
}

/// What this binary does at startup, as text. A disabled repair is listed as
/// `off`, so enabling it later is a new generation.
pub fn startup_generation() -> String {
    let highest = crate::pool::MIGRATOR
        .iter()
        .map(|migration| migration.version)
        .max()
        .unwrap_or(0);
    let mut generation = format!("migrations={highest}");
    for repair in startup_repairs() {
        if repair.applies {
            generation.push_str(&format!(";{}={}", repair.marker, repair.target));
        } else {
            generation.push_str(&format!(";{}=off", repair.marker));
        }
    }
    generation
}

/// Keys whose change since the copy means the upgrade has written the
/// database: every repair marker and the identity authority marker.
fn tracked_markers() -> Vec<&'static str> {
    let mut keys: Vec<&'static str> = startup_repairs()
        .iter()
        .map(|repair| repair.marker)
        .collect();
    keys.push(AUTHORITY_MARKER);
    keys
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AttemptStatus {
    InProgress,
    Complete,
}

/// The `upgrade_snapshot` record.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct UpgradeSnapshot {
    status: AttemptStatus,
    /// The copy taken before this attempt wrote anything.
    file: String,
    /// The highest applied migration when the copy was taken.
    migration: i64,
    /// Tracked marker values when the copy was taken.
    markers: BTreeMap<String, Option<String>>,
    /// The previous attempt's copy, protected until this attempt completes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous: Option<String>,
    /// Set when init finished with the cover repair still incomplete; the
    /// background pass may have committed changes since.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    repairs_done: bool,
}

/// The database as read before any write of this start.
struct StartState {
    migration: i64,
    markers: BTreeMap<String, Option<String>>,
    generation: Option<String>,
    record: Option<UpgradeSnapshot>,
    /// The copy this database was restored from, if it was.
    restored_from: Option<String>,
}

impl StartState {
    /// True once the upgrade has written anything since `record`'s copy was
    /// taken. Decided without looking at the copy file.
    fn moved_on_since(&self, record: &UpgradeSnapshot) -> bool {
        record.repairs_done
            || self.migration != record.migration
            || tracked_markers().into_iter().any(|key| {
                self.markers.get(key).cloned().flatten()
                    != record.markers.get(key).cloned().flatten()
            })
    }
}

/// The outcome of the pre-upgrade copy decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreUpgradeCopy {
    /// No migration has ever been applied.
    FreshInstall,
    /// This binary's startup work already finished on this database.
    NotNeeded,
    /// A new attempt: this copy was taken and recorded.
    Taken(String),
    /// The in-progress attempt's copy, kept because the database has moved on.
    Reused(String),
    /// The in-progress attempt's copy, retaken because nothing has changed.
    Retaken(String),
}

/// Decide and take the pre-upgrade copy. Runs before migrations; writes the
/// database only to record a new attempt, after its copy is published.
pub async fn prepare_pre_upgrade_copy(
    pool: &SqlitePool,
    data_dir: &Path,
) -> Result<PreUpgradeCopy, String> {
    let Some(state) = read_start_state(pool).await? else {
        return Ok(PreUpgradeCopy::FreshInstall);
    };
    reclaim_unrecorded_copy(data_dir, &state).await?;

    if let Some(record) = state
        .record
        .as_ref()
        .filter(|record| record.status == AttemptStatus::InProgress)
    {
        let copy_exists = tokio::fs::try_exists(data_dir.join(&record.file))
            .await
            .map_err(|error| format!("check pre-upgrade copy {}: {error}", record.file))?;
        if state.moved_on_since(record) {
            if !copy_exists {
                return Err(format!(
                    "the original rollback copy {} is unavailable: the upgrade it protects has \
                     already changed this database (migration {} when the copy was taken, {} now). \
                     Put that file back in the data directory and restart; no further upgrade \
                     step runs until then",
                    record.file, record.migration, state.migration
                ));
            }
            tracing::info!("pre-upgrade copy already saved: {}", record.file);
            return Ok(PreUpgradeCopy::Reused(record.file.clone()));
        }
        write_copy(pool, data_dir, &record.file, false).await?;
        tracing::info!("pre-upgrade copy retaken: {}", record.file);
        return Ok(PreUpgradeCopy::Retaken(record.file.clone()));
    }

    if state.generation.as_deref() == Some(startup_generation().as_str()) {
        return Ok(PreUpgradeCopy::NotNeeded);
    }

    remove_stale_partials(data_dir).await?;
    let name = new_copy_name(data_dir, state.migration).await?;
    write_pending(data_dir, &name).await?;
    write_copy(pool, data_dir, &name, true).await?;
    // The previous rollback copy: the copy this database was restored from,
    // else the completed attempt's copy.
    let completed = state
        .record
        .as_ref()
        .filter(|record| record.status == AttemptStatus::Complete)
        .map(|record| record.file.clone());
    let mut previous = None;
    for candidate in [state.restored_from.clone(), completed]
        .into_iter()
        .flatten()
    {
        let exists = tokio::fs::try_exists(data_dir.join(&candidate))
            .await
            .map_err(|error| format!("check rollback copy {candidate}: {error}"))?;
        if exists && is_copy_name(&candidate) {
            previous = Some(candidate);
            break;
        }
    }
    let record = UpgradeSnapshot {
        status: AttemptStatus::InProgress,
        file: name.clone(),
        migration: state.migration,
        markers: state.markers,
        previous,
        repairs_done: false,
    };
    let mut tx = crate::pool::begin_write(pool)
        .await
        .map_err(|error| format!("begin recording pre-upgrade copy {name}: {error}"))?;
    write_meta(&mut tx, RECORD_KEY, &encode_record(&record)?)
        .await
        .map_err(|error| format!("record pre-upgrade copy {name}: {error}"))?;
    sqlx::query("DELETE FROM _livrarr_meta WHERE key = ?1")
        .bind(ROLLBACK_COPY_KEY)
        .execute(&mut *tx)
        .await
        .map_err(|error| format!("record pre-upgrade copy {name}: {error}"))?;
    tx.commit()
        .await
        .map_err(|error| format!("commit pre-upgrade copy record {name}: {error}"))?;
    remove_file_if_present(&data_dir.join(PENDING_FILE)).await?;
    sync_dir(data_dir).await?;
    tracing::info!("pre-upgrade backup: {name}");
    Ok(PreUpgradeCopy::Taken(name))
}

/// Record that this binary's synchronous startup work finished: stamp the
/// startup generation, and complete an in-progress attempt once every
/// applicable repair (the background Goodreads cover repair included) has
/// stamped its marker. Until then the attempt is marked `repairs_done`, which
/// makes any later start reuse its copy rather than retake it.
pub async fn record_startup_completion(pool: &SqlitePool) -> Result<(), String> {
    let mut tx = crate::pool::begin_write(pool)
        .await
        .map_err(|error| format!("begin startup completion: {error}"))?;
    write_meta(&mut tx, GENERATION_KEY, &startup_generation())
        .await
        .map_err(|error| format!("record startup generation: {error}"))?;
    let record = read_meta(&mut *tx, RECORD_KEY)
        .await
        .map_err(|error| format!("read upgrade record: {error}"))?
        .map(|value| decode_record(&value))
        .transpose()?;
    if let Some(mut record) = record.filter(|record| record.status == AttemptStatus::InProgress) {
        let mut complete = true;
        for repair in startup_repairs()
            .into_iter()
            .filter(|repair| repair.applies)
        {
            let value = read_meta(&mut *tx, repair.marker)
                .await
                .map_err(|error| format!("read {}: {error}", repair.marker))?;
            let stamped = value
                .and_then(|value| value.parse::<i64>().ok())
                .is_some_and(|generation| generation >= repair.target);
            complete &= stamped;
        }
        if complete {
            record.status = AttemptStatus::Complete;
            record.previous = None;
            record.repairs_done = false;
        } else {
            record.repairs_done = true;
        }
        write_meta(&mut tx, RECORD_KEY, &encode_record(&record)?)
            .await
            .map_err(|error| format!("record upgrade progress: {error}"))?;
        if complete {
            tracing::info!("upgrade finished; rollback copy: {}", record.file);
        }
    }
    tx.commit()
        .await
        .map_err(|error| format!("commit startup completion: {error}"))
}

/// Copies the prune must keep: the current attempt's copy, the previous
/// attempt's copy while that attempt is in progress, and the copy a restored
/// database came from until its next attempt is recorded.
pub async fn protected_copies(pool: &SqlitePool) -> Result<BTreeSet<String>, String> {
    let mut connection = pool
        .acquire()
        .await
        .map_err(|error| format!("read upgrade record: {error}"))?;
    let record = read_meta(&mut *connection, RECORD_KEY)
        .await
        .map_err(|error| format!("read upgrade record: {error}"))?
        .map(|value| decode_record(&value))
        .transpose()?;
    let mut protected = BTreeSet::new();
    protected.extend(
        read_meta(&mut *connection, ROLLBACK_COPY_KEY)
            .await
            .map_err(|error| format!("read upgrade record: {error}"))?,
    );
    if let Some(record) = record {
        if record.status == AttemptStatus::InProgress {
            protected.extend(record.previous.clone());
        }
        protected.insert(record.file);
    }
    Ok(protected)
}

// ── reading ─────────────────────────────────────────────────────────────────

async fn read_start_state(pool: &SqlitePool) -> Result<Option<StartState>, String> {
    let read = |error: sqlx::Error| format!("read startup state: {error}");
    let mut connection = pool.acquire().await.map_err(read)?;
    let migration: Option<i64> = if table_exists(&mut connection, "_sqlx_migrations").await? {
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(&mut *connection)
            .await
            .map_err(read)?
    } else {
        None
    };
    let Some(migration) = migration else {
        return Ok(None);
    };
    let mut state = StartState {
        migration,
        markers: BTreeMap::new(),
        generation: None,
        record: None,
        restored_from: None,
    };
    if !table_exists(&mut connection, "_livrarr_meta").await? {
        return Ok(Some(state));
    }
    for key in tracked_markers() {
        let value = read_meta(&mut *connection, key).await.map_err(read)?;
        state.markers.insert(key.to_owned(), value);
    }
    state.generation = read_meta(&mut *connection, GENERATION_KEY)
        .await
        .map_err(read)?;
    state.restored_from = read_meta(&mut *connection, ROLLBACK_COPY_KEY)
        .await
        .map_err(read)?;
    state.record = read_meta(&mut *connection, RECORD_KEY)
        .await
        .map_err(read)?
        .map(|value| decode_record(&value))
        .transpose()?;
    Ok(Some(state))
}

async fn table_exists(connection: &mut sqlx::SqliteConnection, name: &str) -> Result<bool, String> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
    )
    .bind(name)
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| format!("read startup state: {error}"))
}

async fn read_meta<'c, E>(executor: E, key: &str) -> Result<Option<String>, sqlx::Error>
where
    E: sqlx::Executor<'c, Database = sqlx::Sqlite>,
{
    sqlx::query_scalar("SELECT value FROM _livrarr_meta WHERE key = ?1")
        .bind(key)
        .fetch_optional(executor)
        .await
}

async fn write_meta(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    key: &str,
    value: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO _livrarr_meta (key, value) VALUES (?1, ?2) \
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(&mut **tx)
    .await
    .map(|_| ())
}

fn encode_record(record: &UpgradeSnapshot) -> Result<String, String> {
    serde_json::to_string(record).map_err(|error| format!("encode upgrade record: {error}"))
}

fn decode_record(value: &str) -> Result<UpgradeSnapshot, String> {
    serde_json::from_str(value).map_err(|error| format!("unreadable upgrade record: {error}"))
}

// ── files ───────────────────────────────────────────────────────────────────

fn is_copy_name(name: &str) -> bool {
    name.starts_with(COPY_PREFIX) && !name.contains(['/', '\\']) && !name.contains("..")
}

/// A copy published by an earlier start but never recorded is reclaimed. A
/// copy the live database names (the record's copy or previous copy, or the
/// copy it was restored from) is kept; only its pending and work files go.
async fn reclaim_unrecorded_copy(data_dir: &Path, state: &StartState) -> Result<(), String> {
    let pending = data_dir.join(PENDING_FILE);
    let name = match tokio::fs::read_to_string(&pending).await {
        Ok(name) => name.trim().to_owned(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("read {PENDING_FILE}: {error}")),
    };
    // A copy the live database names is associated: the recorded attempt's
    // copy, its previous copy, or the copy this database was restored from.
    let associated = state.record.as_ref().is_some_and(|record| {
        record.file == name || record.previous.as_deref() == Some(name.as_str())
    }) || state.restored_from.as_deref() == Some(name.as_str());
    if is_copy_name(&name) {
        remove_file_if_present(&partial_path(data_dir, &name)).await?;
        if !associated {
            remove_file_if_present(&data_dir.join(&name)).await?;
            tracing::info!("reclaimed unrecorded pre-upgrade copy {name}");
        }
    }
    remove_file_if_present(&pending).await?;
    sync_dir(data_dir).await
}

async fn remove_stale_partials(data_dir: &Path) -> Result<(), String> {
    let mut entries = tokio::fs::read_dir(data_dir)
        .await
        .map_err(|error| format!("read data directory: {error}"))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| format!("read data directory: {error}"))?
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(COPY_PREFIX) && name.ends_with(PARTIAL_SUFFIX) {
            remove_file_if_present(&entry.path()).await?;
            tracing::info!("removed stale partial copy {name}");
        }
    }
    Ok(())
}

async fn new_copy_name(data_dir: &Path, migration: i64) -> Result<String, String> {
    let base = format!(
        "{COPY_PREFIX}v{migration:03}-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );
    let mut name = base.clone();
    let mut suffix = 1;
    while tokio::fs::try_exists(data_dir.join(&name))
        .await
        .map_err(|error| format!("check copy name {name}: {error}"))?
    {
        suffix += 1;
        name = format!("{base}-{suffix}");
    }
    Ok(name)
}

async fn write_pending(data_dir: &Path, name: &str) -> Result<(), String> {
    let pending = data_dir.join(PENDING_FILE);
    tokio::fs::write(&pending, name)
        .await
        .map_err(|error| format!("write {PENDING_FILE}: {error}"))?;
    sync_file(&pending).await?;
    sync_dir(data_dir).await
}

/// `VACUUM INTO` the work file, name the copy inside it when `self_named`,
/// make it durable, then publish it by rename, atomically replacing any
/// earlier copy of the same name.
async fn write_copy(
    pool: &SqlitePool,
    data_dir: &Path,
    name: &str,
    self_named: bool,
) -> Result<(), String> {
    let partial = partial_path(data_dir, name);
    remove_file_if_present(&partial).await?;
    let target = partial
        .to_str()
        .ok_or_else(|| format!("copy path is not UTF-8: {}", partial.display()))?;
    sqlx::query("VACUUM INTO ?1")
        .bind(target)
        .execute(pool)
        .await
        .map_err(|error| format!("VACUUM INTO {name}{PARTIAL_SUFFIX} failed: {error}"))?;
    if self_named {
        name_copy_inside(&partial, name).await?;
    }
    sync_file(&partial).await?;
    tokio::fs::rename(&partial, data_dir.join(name))
        .await
        .map_err(|error| format!("publish pre-upgrade copy {name}: {error}"))?;
    sync_dir(data_dir).await
}

/// Write the copy's own name into the unpublished work file's
/// `_livrarr_meta`, so a database restored from it knows its rollback copy.
/// The `VACUUM INTO` output uses a rollback journal; with journaling off no
/// file appears beside it, and a crash here leaves only the `.partial` file
/// for the next attempt to remove.
async fn name_copy_inside(partial: &Path, name: &str) -> Result<(), String> {
    use sqlx::{ConnectOptions, Connection};
    let fail = |error: sqlx::Error| format!("name the copy inside its work file: {error}");
    let mut copy = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(partial)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Off)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Off)
        .connect()
        .await
        .map_err(fail)?;
    sqlx::query(
        "INSERT INTO _livrarr_meta (key, value) VALUES (?1, ?2) \
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
    )
    .bind(ROLLBACK_COPY_KEY)
    .bind(name)
    .execute(&mut copy)
    .await
    .map_err(fail)?;
    copy.close().await.map_err(fail)
}

fn partial_path(data_dir: &Path, name: &str) -> PathBuf {
    data_dir.join(format!("{name}{PARTIAL_SUFFIX}"))
}

async fn remove_file_if_present(path: &Path) -> Result<(), String> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("remove {}: {error}", path.display())),
    }
}

async fn sync_file(path: &Path) -> Result<(), String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || std::fs::File::open(&path)?.sync_all())
        .await
        .map_err(|error| format!("sync copy: {error}"))?
        .map_err(|error| format!("sync copy: {error}"))
}

/// Make a rename or removal in the data directory durable. Directories can be
/// opened and synced only on Unix.
async fn sync_dir(data_dir: &Path) -> Result<(), String> {
    if cfg!(unix) {
        sync_file(data_dir).await
    } else {
        Ok(())
    }
}
