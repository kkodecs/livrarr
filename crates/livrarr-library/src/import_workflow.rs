use std::path::{Path, PathBuf};
use std::sync::Arc;

use livrarr_db::{
    record_history, ChapterDb, ConfigDb, CreateImportIntentDbRequest, CreateLibraryItemDbRequest,
    GrabDb, HistoryDb, ImportIntent, ImportIntentDb, ImportIntentState, KashLinkDb, LibraryItemDb,
    NewKashLink, RemotePathMappingDb, RootFolderDb, WorkDb,
};
use livrarr_domain::history_events;
use livrarr_domain::keyed_mutex::KeyedMutex;
use livrarr_domain::services::{
    ChapterExtractionError, ChapterExtractor, FailedFile, ImportFileOutcome, ImportFileRequest,
    ImportResult, ImportWorkflow, ImportWorkflowError, ImportedFile, Materialization, SkipReason,
    SkippedFile,
};
use livrarr_domain::{
    classify_file, sanitize_path_component, DbError, GrabId, GrabStatus, MediaType, UserId, WorkId,
};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

/// Prefix identifying a Unit D2 staging reservation (`tempfile::Builder`, a
/// safe non-predictable name in the destination directory — standards.md:295).
/// The leading dot hides it from directory listings/library scans, matching
/// tempfile's own default convention. Recognized only by this exact prefix so
/// the recovery sweep never touches `atomic_copy`'s own unrelated `.tmp`
/// fallback files (those are covered by the separate `sweep_stale_temp_files`
/// startup pass).
const STAGING_PREFIX: &str = ".livrarr-import-";

/// Minimum age before an unreferenced staging file is swept at startup —
/// matches the 1-hour cutoff `sweep_stale_temp_files` already uses.
const STAGING_SWEEP_MIN_AGE: std::time::Duration = std::time::Duration::from_secs(3600);

/// Outcome of one startup pass reconciling import intents (Unit D2).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ImportRecoveryReport {
    /// The rename had not (verifiably) happened: staging file removed,
    /// intent cleared, no `LibraryItem` created.
    pub rolled_back: u32,
    /// The target file exists: `LibraryItem` finalized (or confirmed
    /// already finalized) and the intent cleared.
    pub completed: u32,
    /// A `Renamed` intent whose target file is unexpectedly missing — left
    /// in place for investigation rather than silently discarded.
    pub anomalous: u32,
    /// An aged, unreferenced staging file removed by the sweep phase.
    pub swept: u32,
}

pub struct ImportWorkflowImpl<D> {
    db: D,
    import_locks: Arc<KeyedMutex<(UserId, WorkId)>>,
    _import_semaphore: Arc<tokio::sync::Semaphore>,
    _data_dir: Arc<PathBuf>,
    /// Injected M4B chapter extraction (REQ-005): the library crate holds no
    /// tagwrite edge; the composition root supplies the delegate.
    extractor: Arc<dyn ChapterExtractor>,
}

impl<D> ImportWorkflowImpl<D> {
    pub fn new(
        db: D,
        import_semaphore: Arc<tokio::sync::Semaphore>,
        data_dir: Arc<PathBuf>,
        extractor: Arc<dyn ChapterExtractor>,
    ) -> Self {
        let import_locks = Arc::new(KeyedMutex::new());
        spawn_import_locks_sweeper(&import_locks);
        Self {
            db,
            import_locks,
            _import_semaphore: import_semaphore,
            _data_dir: data_dir,
            extractor,
        }
    }
}

/// D3 #8 / R-5: `KeyedMutex::sweep()` is the backstop for permits `Drop`'s
/// opportunistic per-guard prune skips (only when the map is contended at
/// release) — it existed with zero production callers. This spawns a 300s
/// periodic sweep of `import_locks` for the life of the process, sharing
/// ownership via the `Arc` clone captured in the task. A no-op (never
/// panics) when no Tokio runtime is current — `ImportWorkflowImpl::new` is a
/// plain constructor called from many test contexts, and the sweep is a
/// backstop nothing depends on synchronously.
fn spawn_import_locks_sweeper(locks: &Arc<KeyedMutex<(UserId, WorkId)>>) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let locks = Arc::clone(locks);
        handle.spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                ticker.tick().await;
                locks.sweep().await;
            }
        });
    }
}

impl<D> ImportWorkflowImpl<D>
where
    D: ChapterDb + LibraryItemDb + KashLinkDb + RootFolderDb + ImportIntentDb + Send + Sync,
{
    /// Extracts and stores audiobook chapters for a just-imported item, then
    /// runs `.kash` link establishment for m4bs with a parsed duration.
    ///
    /// Mirrors the chapter-extraction hook in `import_grab` so that imports
    /// arriving through other paths (e.g. manual import) populate
    /// `audiobook_chapters` and kash links the same way. Failure is
    /// non-fatal: errors are logged and never fail the import.
    pub async fn extract_chapters_for_item(
        &self,
        item_id: livrarr_domain::LibraryItemId,
        target: &Path,
        media_type: MediaType,
        user_id: UserId,
        work_id: WorkId,
    ) {
        extract_chapters_and_kash(
            &self.db,
            &self.extractor,
            item_id,
            target,
            media_type,
            user_id,
            work_id,
        )
        .await;
    }

    /// Lock-free core for `ImportWorkflow::import_file`. `import_file` (the
    /// trait method) acquires the per-(user, work) lock first; `import_grab`
    /// already holds that lock for its whole run and calls this directly —
    /// `KeyedMutex` is not re-entrant, so it must never call `import_file`.
    ///
    /// Generalizes the orphan-adoption branch `import_grab` used to run
    /// inline: target-exists + no row for this work adopts the on-disk file
    /// (`Copy`/`HardlinkFirst` require the target's size to match the
    /// source's; `AdoptInPlace` never checks, since source and target are the
    /// same file by definition). A row for a different work at the same path
    /// is a `PathCollision`, whether detected directly (size mismatch) or via
    /// the database's own cross-work rejection.
    async fn import_file_locked(
        &self,
        user_id: UserId,
        req: ImportFileRequest,
    ) -> Result<ImportFileOutcome, ImportWorkflowError> {
        let root_folder = self
            .db
            .get_root_folder(req.root_folder_id)
            .await
            .map_err(ImportWorkflowError::Db)?;

        let target = Path::new(&root_folder.path).join(&req.target_relative);
        validate_target_path(&target, &root_folder.path)
            .map_err(ImportWorkflowError::ImportFailed)?;

        let target_clone = target.clone();
        let target_exists = tokio::task::spawn_blocking(move || target_clone.exists())
            .await
            .unwrap_or(false);

        if target_exists {
            let existing_items = self
                .db
                .list_library_items_by_work(user_id, req.work_id)
                .await
                .map_err(ImportWorkflowError::Db)?;

            if existing_items
                .iter()
                .any(|li| li.root_folder_id == req.root_folder_id && li.path == req.target_relative)
            {
                return Ok(ImportFileOutcome::Skipped {
                    reason: SkipReason::AlreadyImported,
                });
            }

            // Adoption: create the row from the on-disk file, no file I/O.
            // Copy/HardlinkFirst must confirm the file is actually ours —
            // a colliding different book virtually never matches size.
            let file_size: i64 = match req.materialization {
                Materialization::AdoptInPlace => {
                    let target_for_meta = target.clone();
                    tokio::task::spawn_blocking(move || {
                        target_for_meta
                            .metadata()
                            .map(|m| m.len() as i64)
                            .unwrap_or(0)
                    })
                    .await
                    .unwrap_or(0)
                }
                Materialization::Copy | Materialization::HardlinkFirst => {
                    let source_path = req.source.clone();
                    let target_for_meta = target.clone();
                    let (source_size, target_size) = tokio::task::spawn_blocking(move || {
                        (
                            std::fs::metadata(&source_path).map(|m| m.len()).ok(),
                            std::fs::metadata(&target_for_meta).map(|m| m.len()).ok(),
                        )
                    })
                    .await
                    .unwrap_or((None, None));

                    if source_size.is_none() || source_size != target_size {
                        return Err(ImportWorkflowError::PathCollision(req.target_relative));
                    }
                    target_size.unwrap_or(0) as i64
                }
            };

            let item = match self
                .db
                .create_library_item(CreateLibraryItemDbRequest {
                    user_id,
                    work_id: req.work_id,
                    root_folder_id: req.root_folder_id,
                    path: req.target_relative.clone(),
                    media_type: req.media_type,
                    file_size,
                    import_id: req.import_id.clone(),
                    tag_status: livrarr_db::TagStatus::Pending,
                    tagged_at_generation: 0,
                })
                .await
            {
                Ok(item) => item,
                Err(DbError::Constraint { .. }) => {
                    return Err(ImportWorkflowError::PathCollision(req.target_relative));
                }
                Err(e) => return Err(ImportWorkflowError::Db(e)),
            };

            if req.extract_chapters {
                extract_chapters_and_kash(
                    &self.db,
                    &self.extractor,
                    item.id,
                    &target,
                    req.media_type,
                    user_id,
                    req.work_id,
                )
                .await;
            }

            return Ok(ImportFileOutcome::Adopted {
                item_id: item.id,
                path: req.target_relative,
            });
        }

        // Target absent: materialize per mode via the crash-consistent state
        // machine (Unit D2): persist intent -> write+fsync staging (tempfile,
        // destination dir) -> atomic rename -> fsync parent dir -> finalize
        // DB -> clear intent (standards.md:81/295). A crash at any step is
        // reconciled by `recover_import_intents` at the next startup.
        if req.materialization == Materialization::AdoptInPlace {
            return Err(ImportWorkflowError::SourceNotResolved(format!(
                "adopt-in-place target does not exist: {}",
                target.display()
            )));
        }

        // A different work's row can already claim this exact path even
        // though its backing file is gone from disk (e.g. removed
        // out-of-band) — target.exists() alone can't see that. Catch it
        // here, before any staging I/O, so the row committed first stays
        // the authority for the path and no bytes ever reach target.
        match self
            .db
            .find_library_item_by_path(user_id, req.root_folder_id, &req.target_relative)
            .await
        {
            Ok(Some(existing)) if existing.work_id != req.work_id => {
                return Err(ImportWorkflowError::PathCollision(req.target_relative));
            }
            Ok(_) => {}
            Err(e) => return Err(ImportWorkflowError::Db(e)),
        }

        let target_parent = target.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        if let Err(e) = tokio::fs::create_dir_all(&target_parent).await {
            return Err(ImportWorkflowError::ImportFailed(format!(
                "failed to create destination directory: {e}"
            )));
        }

        let source_for_size = req.source.clone();
        let expected_size: u64 = tokio::task::spawn_blocking(move || {
            std::fs::metadata(&source_for_size)
                .map(|m| m.len())
                .unwrap_or(0)
        })
        .await
        .unwrap_or(0);

        // Reserve a safe, non-predictable staging name in the destination
        // directory (standards.md:295 — replaces the predictable
        // `.stg{counter}`).
        let staging_tmp = tempfile::Builder::new()
            .prefix(STAGING_PREFIX)
            .tempfile_in(&target_parent)
            .map_err(|e| {
                ImportWorkflowError::ImportFailed(format!("staging tempfile create failed: {e}"))
            })?;
        let staging_path = staging_tmp.path().to_path_buf();

        // Persist the intent BEFORE any content is written. Its
        // UNIQUE(user_id, root_folder_id, target_relative) is the collision
        // arbiter a concurrent second work targeting this same path now
        // hits (moving the LibraryItem row's creation to after the rename,
        // below, removed the old library_items-constraint arbiter that used
        // to run before any file I/O).
        let intent = match self
            .db
            .create_import_intent(CreateImportIntentDbRequest {
                user_id,
                work_id: req.work_id,
                root_folder_id: req.root_folder_id,
                media_type: req.media_type,
                target_relative: req.target_relative.clone(),
                staging_path: staging_path.to_string_lossy().into_owned(),
                expected_size: expected_size as i64,
                import_id: req.import_id.clone(),
            })
            .await
        {
            Ok(intent) => intent,
            Err(DbError::Constraint { .. }) => {
                let _ = tokio::fs::remove_file(&staging_path).await;
                return Err(ImportWorkflowError::PathCollision(req.target_relative));
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&staging_path).await;
                return Err(ImportWorkflowError::Db(e));
            }
        };

        // Write + fsync staging.
        let materialize_result: Result<u64, String> = match req.materialization {
            Materialization::AdoptInPlace => unreachable!("handled above"),
            Materialization::Copy => {
                let source = req.source.clone();
                tokio::task::spawn_blocking(move || -> Result<u64, String> {
                    let mut src_file = std::fs::File::open(&source)
                        .map_err(|e| format!("open source failed: {e}"))?;
                    let mut dst_file = staging_tmp
                        .as_file()
                        .try_clone()
                        .map_err(|e| format!("staging handle clone failed: {e}"))?;
                    let copied = std::io::copy(&mut src_file, &mut dst_file)
                        .map_err(|e| format!("copy failed: {e}"))?;
                    dst_file
                        .sync_all()
                        .map_err(|e| format!("fsync staging failed: {e}"))?;
                    drop(dst_file);
                    // Disarm delete-on-drop, keeping the fsynced content at
                    // staging_path — the shared rename below moves it to
                    // target.
                    staging_tmp
                        .into_temp_path()
                        .keep()
                        .map_err(|e| format!("failed to keep staging file: {e}"))?;
                    Ok(copied)
                })
                .await
                .unwrap_or_else(|e| Err(format!("spawn task panicked: {e}")))
            }
            Materialization::HardlinkFirst => {
                // hard_link requires the destination NOT exist — release
                // this reservation first. materialize_hardlink_first
                // re-materializes at the exact same path (hard_link, or its
                // own tempfile-copy fallback on EXDEV). The brief window
                // between close() and the hard_link attempt is a name-reuse
                // race only another Livrarr staging write could hit;
                // tempfile's random suffix makes that practically
                // impossible, and a collision would surface as an I/O error
                // here, never silent corruption.
                let close_result: Result<(), String> = tokio::task::spawn_blocking(move || {
                    staging_tmp.close().map_err(|e| e.to_string())
                })
                .await
                .unwrap_or_else(|e| Err(format!("spawn error: {e}")));
                match close_result {
                    Ok(()) => materialize_hardlink_first(&req.source, &staging_path).await,
                    Err(e) => Err(format!("failed to release staging reservation: {e}")),
                }
            }
        };

        let materialized_size = match materialize_result {
            Ok(size) => size,
            Err(e) => {
                let _ = tokio::fs::remove_file(&staging_path).await;
                let _ = self.db.delete_import_intent(intent.id).await;
                return Err(ImportWorkflowError::ImportFailed(format!(
                    "staging write failed: {e}"
                )));
            }
        };

        // Atomic rename: staging -> target.
        if let Err(e) = tokio::fs::rename(&staging_path, &target).await {
            let _ = tokio::fs::remove_file(&staging_path).await;
            let _ = self.db.delete_import_intent(intent.id).await;
            tracing::warn!(
                staging = %staging_path.display(),
                target = %target.display(),
                error = %e,
                "finalize rename failed; intent rolled back"
            );
            return Err(ImportWorkflowError::ImportFailed(format!(
                "finalize rename failed: {e}"
            )));
        }

        // Fsync the parent directory — durably persists the rename's
        // directory-entry change (and, for HardlinkFirst, the earlier
        // hard_link dentry too: one directory fsync flushes all pending
        // metadata changes for that directory). A real fsync failure here
        // means the rename isn't durable yet — must not advance to
        // mark_import_intent_renamed/create_library_item; the file and
        // intent are left exactly as they are for recovery to finish.
        let parent_for_fsync = target_parent.clone();
        match tokio::task::spawn_blocking(move || fsync_dir(&parent_for_fsync)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                return Err(ImportWorkflowError::ImportFailed(format!(
                    "dir fsync failed: {e}"
                )));
            }
            Err(join) => {
                return Err(ImportWorkflowError::ImportFailed(format!(
                    "fsync task panicked: {join}"
                )));
            }
        }

        if let Err(e) = self.db.mark_import_intent_renamed(intent.id).await {
            tracing::warn!(
                intent_id = intent.id,
                error = %e,
                "failed to persist Renamed intent state — recovery still completes via the on-disk rename"
            );
        }

        // Finalize DB + clear intent. Both are individually idempotent
        // (create_library_item upserts on (user_id, root_folder_id, path);
        // delete_import_intent on a missing row is a no-op), so a crash
        // between them is safely retried by recovery without a wrapping
        // transaction.
        let item = match self
            .db
            .create_library_item(CreateLibraryItemDbRequest {
                user_id,
                work_id: req.work_id,
                root_folder_id: req.root_folder_id,
                path: req.target_relative.clone(),
                media_type: req.media_type,
                file_size: materialized_size as i64,
                import_id: req.import_id.clone(),
                tag_status: livrarr_db::TagStatus::Pending,
                tagged_at_generation: 0,
            })
            .await
        {
            Ok(item) => item,
            Err(e) => {
                // The file is safely at target; only the DB write failed.
                // Leave the intent for recovery to finish next startup —
                // never roll back a durably-renamed import.
                tracing::warn!(
                    intent_id = intent.id,
                    error = %e,
                    "finalize DB write failed after durable rename; recovery will complete the LibraryItem write on the next startup"
                );
                return Err(ImportWorkflowError::Db(e));
            }
        };

        let _ = self.db.delete_import_intent(intent.id).await;

        if req.extract_chapters {
            extract_chapters_and_kash(
                &self.db,
                &self.extractor,
                item.id,
                &target,
                req.media_type,
                user_id,
                req.work_id,
            )
            .await;
        }

        Ok(ImportFileOutcome::Imported {
            item_id: item.id,
            path: req.target_relative,
        })
    }
}

impl<D> ImportWorkflowImpl<D>
where
    D: ImportIntentDb + LibraryItemDb + RootFolderDb + Send + Sync,
{
    /// Startup recovery for the import crash-consistency state machine
    /// (Unit D2). Call once at startup, before any import can be in flight
    /// (`recover_interrupted_state`). Under each affected work's import
    /// lock — the same `import_locks` the live import path holds — every
    /// outstanding intent is reconciled (completed or rolled back per its
    /// persisted state plus the on-disk ground truth); only then does the
    /// sweep phase remove aged staging files no remaining intent
    /// references.
    pub async fn recover_import_intents(&self) -> Result<ImportRecoveryReport, DbError> {
        let mut report = ImportRecoveryReport::default();

        // Propagate rather than default: a listing failure must never look
        // identical to "nothing to recover" — the caller needs to be able
        // to tell the two apart and escalate loudly (Unit D2 hardening).
        let intents = self.db.list_import_intents().await?;

        let mut by_work: std::collections::HashMap<(UserId, WorkId), Vec<ImportIntent>> =
            std::collections::HashMap::new();
        for intent in intents {
            by_work
                .entry((intent.user_id, intent.work_id))
                .or_default()
                .push(intent);
        }

        for ((user_id, work_id), group) in by_work {
            let _guard = self.import_locks.lock((user_id, work_id)).await;
            for intent in group {
                self.reconcile_one_intent(intent, &mut report).await;
            }
        }

        // Re-list after reconciliation: the "referenced" set for the sweep
        // must reflect intents still outstanding now, never a stale
        // pre-reconcile snapshot — never sweep a staging file an intent
        // still references.
        let referenced: std::collections::HashSet<PathBuf> = match self
            .db
            .list_import_intents()
            .await
        {
            Ok(v) => v
                .into_iter()
                .map(|i| PathBuf::from(i.staging_path))
                .collect(),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "import intent recovery: failed to re-list intents before sweep — skipping sweep"
                );
                return Ok(report);
            }
        };

        let root_folders = match self.db.list_root_folders().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "import intent recovery: failed to list root folders — sweep skipped"
                );
                return Ok(report);
            }
        };

        report.swept =
            sweep_unreferenced_staging_files(&root_folders, &referenced, STAGING_SWEEP_MIN_AGE)
                .await as u32;

        if report != ImportRecoveryReport::default() {
            tracing::info!(
                rolled_back = report.rolled_back,
                completed = report.completed,
                anomalous = report.anomalous,
                swept = report.swept,
                "import intent recovery: complete"
            );
        }

        Ok(report)
    }

    async fn reconcile_one_intent(&self, intent: ImportIntent, report: &mut ImportRecoveryReport) {
        let root_folder = match self.db.get_root_folder(intent.root_folder_id).await {
            Ok(rf) => rf,
            Err(e) => {
                tracing::warn!(
                    intent_id = intent.id,
                    error = %e,
                    "import intent recovery: root folder unreadable — leaving intent for a later pass"
                );
                return;
            }
        };
        let target = Path::new(&root_folder.path).join(&intent.target_relative);
        let target_exists = tokio::fs::try_exists(&target).await.unwrap_or(false);

        if !target_exists {
            match intent.state {
                ImportIntentState::Staging => {
                    // Normal, expected crash shape: the rename never
                    // happened (or happened to a different, unrelated
                    // path) — roll back.
                    let _ = tokio::fs::remove_file(&intent.staging_path).await;
                    match self.db.delete_import_intent(intent.id).await {
                        Ok(()) => report.rolled_back += 1,
                        Err(e) => tracing::warn!(
                            intent_id = intent.id,
                            error = %e,
                            "import intent recovery: rollback cleanup failed — will retry next pass"
                        ),
                    }
                }
                ImportIntentState::Renamed => {
                    // Renamed is only persisted after a successful rename +
                    // parent fsync — the target should exist. Something
                    // external removed it; escalate rather than silently
                    // discarding the tracking record.
                    tracing::error!(
                        intent_id = intent.id,
                        target = %target.display(),
                        "import intent recovery: intent state says the rename completed, but \
                         the target file is missing — leaving the intent for investigation"
                    );
                    report.anomalous += 1;
                }
            }
            return;
        }

        // Target exists: the rename durably happened (the atomic-rename +
        // parent-fsync guarantee holds regardless of which state this row
        // still shows — a crash can land between the rename and the
        // Renamed state write). Finalize the LibraryItem row (idempotent
        // on (user_id, root_folder_id, path)) and clear the intent.
        let real_size = tokio::fs::metadata(&target)
            .await
            .map(|m| m.len() as i64)
            .unwrap_or(intent.expected_size);

        if real_size != intent.expected_size {
            // The intent's expected_size is the crash-consistency contract
            // between what was staged and what's being finalized. A target
            // present at the right path but the wrong size is never
            // trustworthy enough to finalize — escalate instead, matching
            // the sibling anomalous branch above.
            tracing::error!(
                intent_id = intent.id,
                target = %target.display(),
                expected_size = intent.expected_size,
                real_size,
                "import intent recovery: target file size does not match the intent's expected size — leaving intent for investigation"
            );
            report.anomalous += 1;
            return;
        }

        let finalize_result = self
            .db
            .create_library_item(CreateLibraryItemDbRequest {
                user_id: intent.user_id,
                work_id: intent.work_id,
                root_folder_id: intent.root_folder_id,
                path: intent.target_relative.clone(),
                media_type: intent.media_type,
                file_size: real_size,
                import_id: intent.import_id.clone(),
                tag_status: livrarr_db::TagStatus::Pending,
                tagged_at_generation: 0,
            })
            .await;

        match finalize_result {
            Ok(_) => match self.db.delete_import_intent(intent.id).await {
                Ok(()) => report.completed += 1,
                Err(e) => tracing::warn!(
                    intent_id = intent.id,
                    error = %e,
                    "import intent recovery: finalize succeeded but clearing the intent failed — will retry next pass"
                ),
            },
            Err(e) => {
                tracing::warn!(
                    intent_id = intent.id,
                    error = %e,
                    "import intent recovery: LibraryItem finalize failed — leaving intent for a later pass"
                );
            }
        }
    }
}

/// Test-only failpoint forcing a durability fsync to fail for an exact,
/// pre-armed path (Unit D2 hardening). A real fsync failure isn't
/// reproducible on demand from a portable test, so this seam lets tests
/// exercise the two error-handling arms it guards (`fsync_dir`'s inner
/// `Err`, and the hardlink-first cross-fs copy fallback's data fsync)
/// deterministically. Entirely `#[cfg(test)]`-gated — compiles out
/// completely in non-test builds, so production behavior is unchanged.
/// Keyed by the exact path being fsynced (never a bare on/off switch) so
/// concurrent tests using distinct tempdirs can never interfere with each
/// other.
#[cfg(test)]
mod fsync_test_failpoint {
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    static ARMED: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

    pub(super) fn arm(path: PathBuf) {
        ARMED
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
            .unwrap()
            .insert(path);
    }

    pub(super) fn is_armed(path: &Path) -> bool {
        ARMED
            .get()
            .map(|set| set.lock().unwrap().contains(path))
            .unwrap_or(false)
    }
}

/// Fsync a directory so its pending metadata changes (a rename, a
/// hard_link) are durable before the caller proceeds. Always run inside
/// `spawn_blocking` — this is a blocking syscall.
fn fsync_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if fsync_test_failpoint::is_armed(dir) {
        return Err(std::io::Error::other("injected dir fsync failure (test)"));
    }
    std::fs::File::open(dir)?.sync_all()
}

/// Delete staging files this module's crash-consistent import path created
/// (Unit D2) that (a) no live intent references and (b) are older than
/// `min_age`. Recognized purely by the [`STAGING_PREFIX`] this module's
/// tempfile reservations use — never touches `atomic_copy`'s own unrelated
/// `.tmp` fallback files (those are covered by the separate
/// `sweep_stale_temp_files` startup pass in livrarr-server).
async fn sweep_unreferenced_staging_files(
    root_folders: &[livrarr_domain::RootFolder],
    referenced: &std::collections::HashSet<PathBuf>,
    min_age: std::time::Duration,
) -> usize {
    let cutoff = std::time::SystemTime::now().checked_sub(min_age);
    let mut removed = 0usize;
    for rf in root_folders {
        removed += sweep_dir_for_staging(Path::new(&rf.path), referenced, cutoff).await;
    }
    removed
}

fn sweep_dir_for_staging<'a>(
    dir: &'a Path,
    referenced: &'a std::collections::HashSet<PathBuf>,
    cutoff: Option<std::time::SystemTime>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = usize> + Send + 'a>> {
    Box::pin(async move {
        let mut removed = 0usize;
        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(e) => e,
            Err(_) => return 0,
        };
        let mut subdirs = Vec::new();
        loop {
            let entry = match entries.next_entry().await {
                Ok(Some(e)) => e,
                Ok(None) => break,
                Err(_) => break,
            };
            let file_type = match entry.file_type().await {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                subdirs.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            if !entry
                .file_name()
                .to_string_lossy()
                .starts_with(STAGING_PREFIX)
            {
                continue;
            }
            if referenced.contains(&path) {
                continue;
            }
            let is_aged = match cutoff {
                Some(cutoff) => entry
                    .metadata()
                    .await
                    .and_then(|m| m.modified())
                    .map(|mtime| mtime < cutoff)
                    .unwrap_or(false),
                None => true,
            };
            if is_aged && tokio::fs::remove_file(&path).await.is_ok() {
                removed += 1;
            }
        }
        for sub in subdirs {
            removed += sweep_dir_for_staging(&sub, referenced, cutoff).await;
        }
        removed
    })
}

#[cfg(test)]
mod import_recovery_tests {
    use super::*;
    use livrarr_domain::RootFolder;
    use std::time::Duration;

    struct NoopChapterExtractor;
    impl ChapterExtractor for NoopChapterExtractor {
        fn extract_m4b_chapters(
            &self,
            _path: &Path,
        ) -> Result<
            livrarr_domain::services::ChapterExtractionResult,
            livrarr_domain::services::ChapterExtractionError,
        > {
            unreachable!("recovery tests never write .m4b fixtures")
        }
    }

    fn make_workflow(
        db: livrarr_db::sqlite::SqliteDb,
    ) -> ImportWorkflowImpl<livrarr_db::sqlite::SqliteDb> {
        ImportWorkflowImpl::new(
            db,
            Arc::new(tokio::sync::Semaphore::new(2)),
            Arc::new(PathBuf::new()),
            Arc::new(NoopChapterExtractor),
        )
    }

    /// Real user + work + ebook root folder — every recovery test's shared
    /// starting point. The returned `TempDir` must stay alive for the
    /// test's duration (it owns the root folder's backing directory).
    async fn seed() -> (
        livrarr_db::sqlite::SqliteDb,
        ImportWorkflowImpl<livrarr_db::sqlite::SqliteDb>,
        UserId,
        WorkId,
        i64,
        tempfile::TempDir,
    ) {
        use livrarr_db::{
            CreateUserDbRequest, CreateWorkDbRequest, RootFolderDb, UserDb, WorkDbCreate,
        };

        let db = livrarr_db::create_test_db().await;
        let user = db
            .create_user(CreateUserDbRequest {
                username: "d2-recovery".into(),
                password_hash: "hash".into(),
                role: livrarr_domain::UserRole::Admin,
                api_key_hash: "d2-recovery-key".into(),
            })
            .await
            .unwrap();
        let (work, _) = db
            .create_work(CreateWorkDbRequest {
                user_id: user.id,
                title: "D2 Test Book".into(),
                author_name: "D2 Author".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let root_dir = tempfile::tempdir().unwrap();
        let rf = db
            .create_root_folder(root_dir.path().to_str().unwrap(), MediaType::Ebook)
            .await
            .unwrap();
        let workflow = make_workflow(db.clone());
        (db, workflow, user.id, work.id, rf.id, root_dir)
    }

    // -----------------------------------------------------------------
    // Reconcile-phase tests: one per crash transition + idempotency.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn recovery_rolls_back_when_rename_never_happened() {
        // Failpoint: crash any time between "intent persisted" and "rename
        // attempted" — the staging file may be partial, complete, or absent,
        // but the target never received it.
        let (db, workflow, user_id, work_id, root_folder_id, root_dir) = seed().await;

        let staging_path = root_dir.path().join(".livrarr-import-crash1");
        tokio::fs::write(&staging_path, b"partial bytes")
            .await
            .unwrap();

        let intent = db
            .create_import_intent(CreateImportIntentDbRequest {
                user_id,
                work_id,
                root_folder_id,
                media_type: MediaType::Ebook,
                target_relative: "D2 Author/D2 Test Book.epub".into(),
                staging_path: staging_path.to_string_lossy().into_owned(),
                expected_size: 14,
                import_id: None,
            })
            .await
            .unwrap();
        assert_eq!(intent.state, ImportIntentState::Staging);

        let report = workflow.recover_import_intents().await.unwrap();

        assert_eq!(report.rolled_back, 1, "{report:?}");
        assert_eq!(report.completed, 0, "{report:?}");
        assert!(
            !tokio::fs::try_exists(&staging_path).await.unwrap(),
            "orphaned staging file must be removed on rollback"
        );
        assert!(
            db.list_import_intents().await.unwrap().is_empty(),
            "rolled-back intent must be cleared"
        );
        assert!(
            db.list_library_items_by_work(user_id, work_id)
                .await
                .unwrap()
                .is_empty(),
            "a rolled-back import must never create a LibraryItem"
        );
    }

    #[tokio::test]
    async fn recovery_completes_when_rename_already_happened_but_state_still_staging() {
        // Failpoint: crash after the atomic rename (and even after the
        // parent-dir fsync) but before the intent's state is advanced to
        // Renamed — the ambiguous window this whole design accounts for.
        // Recovery must trust the filesystem over the stale `Staging` value.
        let (db, workflow, user_id, work_id, root_folder_id, root_dir) = seed().await;

        let target_relative = "D2 Author/D2 Test Book.epub";
        let target_abs = root_dir.path().join(target_relative);
        tokio::fs::create_dir_all(target_abs.parent().unwrap())
            .await
            .unwrap();
        let contents = b"the rename already completed";
        tokio::fs::write(&target_abs, contents).await.unwrap();

        // staging_path no longer exists post-rename — recovery must not
        // require it to.
        let ghost_staging = root_dir.path().join(".livrarr-import-ghost1");

        let intent = db
            .create_import_intent(CreateImportIntentDbRequest {
                user_id,
                work_id,
                root_folder_id,
                media_type: MediaType::Ebook,
                target_relative: target_relative.into(),
                staging_path: ghost_staging.to_string_lossy().into_owned(),
                expected_size: contents.len() as i64,
                import_id: None,
            })
            .await
            .unwrap();
        assert_eq!(intent.state, ImportIntentState::Staging);

        let report = workflow.recover_import_intents().await.unwrap();

        assert_eq!(report.completed, 1, "{report:?}");
        assert_eq!(report.rolled_back, 0, "{report:?}");
        let items = db
            .list_library_items_by_work(user_id, work_id)
            .await
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].path, target_relative);
        assert_eq!(items[0].file_size, contents.len() as i64);
        assert!(db.list_import_intents().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn recovery_completes_when_state_is_renamed() {
        // Failpoint: crash after the Renamed state write but before the
        // LibraryItem finalize.
        let (db, workflow, user_id, work_id, root_folder_id, root_dir) = seed().await;

        let target_relative = "D2 Author/D2 Test Book.epub";
        let target_abs = root_dir.path().join(target_relative);
        tokio::fs::create_dir_all(target_abs.parent().unwrap())
            .await
            .unwrap();
        let contents = b"renamed and durable";
        tokio::fs::write(&target_abs, contents).await.unwrap();

        let intent = db
            .create_import_intent(CreateImportIntentDbRequest {
                user_id,
                work_id,
                root_folder_id,
                media_type: MediaType::Ebook,
                target_relative: target_relative.into(),
                staging_path: root_dir
                    .path()
                    .join(".livrarr-import-ghost2")
                    .to_string_lossy()
                    .into_owned(),
                expected_size: contents.len() as i64,
                import_id: None,
            })
            .await
            .unwrap();
        db.mark_import_intent_renamed(intent.id).await.unwrap();

        let report = workflow.recover_import_intents().await.unwrap();

        assert_eq!(report.completed, 1, "{report:?}");
        let items = db
            .list_library_items_by_work(user_id, work_id)
            .await
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].path, target_relative);
        assert!(db.list_import_intents().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn recovery_is_idempotent_when_finalize_already_ran_but_intent_never_cleared() {
        // Failpoint: crash strictly between the LibraryItem commit and the
        // intent delete — both individually-idempotent calls must tolerate
        // a re-run without creating a duplicate row.
        let (db, workflow, user_id, work_id, root_folder_id, root_dir) = seed().await;

        let target_relative = "D2 Author/D2 Test Book.epub";
        let target_abs = root_dir.path().join(target_relative);
        tokio::fs::create_dir_all(target_abs.parent().unwrap())
            .await
            .unwrap();
        let contents = b"already finalized once";
        tokio::fs::write(&target_abs, contents).await.unwrap();

        // The LibraryItem row already exists (finalize already ran)...
        db.create_library_item(CreateLibraryItemDbRequest {
            user_id,
            work_id,
            root_folder_id,
            path: target_relative.into(),
            media_type: MediaType::Ebook,
            file_size: contents.len() as i64,
            import_id: None,
            tag_status: livrarr_db::TagStatus::Pending,
            tagged_at_generation: 0,
        })
        .await
        .unwrap();

        // ...but the intent row was never cleared.
        let intent = db
            .create_import_intent(CreateImportIntentDbRequest {
                user_id,
                work_id,
                root_folder_id,
                media_type: MediaType::Ebook,
                target_relative: target_relative.into(),
                staging_path: root_dir
                    .path()
                    .join(".livrarr-import-ghost3")
                    .to_string_lossy()
                    .into_owned(),
                expected_size: contents.len() as i64,
                import_id: None,
            })
            .await
            .unwrap();
        db.mark_import_intent_renamed(intent.id).await.unwrap();

        let report = workflow.recover_import_intents().await.unwrap();

        assert_eq!(report.completed, 1, "{report:?}");
        let items = db
            .list_library_items_by_work(user_id, work_id)
            .await
            .unwrap();
        assert_eq!(items.len(), 1, "must not create a duplicate LibraryItem");
        assert!(db.list_import_intents().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn recovery_rerun_is_a_noop() {
        // Explicit idempotent-rerun requirement: calling recovery again once
        // everything is already reconciled must be a safe no-op.
        let (db, workflow, user_id, work_id, root_folder_id, root_dir) = seed().await;

        let target_relative = "D2 Author/D2 Test Book.epub";
        let target_abs = root_dir.path().join(target_relative);
        tokio::fs::create_dir_all(target_abs.parent().unwrap())
            .await
            .unwrap();
        let contents = b"idempotent rerun";
        tokio::fs::write(&target_abs, contents).await.unwrap();

        db.create_import_intent(CreateImportIntentDbRequest {
            user_id,
            work_id,
            root_folder_id,
            media_type: MediaType::Ebook,
            target_relative: target_relative.into(),
            staging_path: root_dir
                .path()
                .join(".livrarr-import-ghost4")
                .to_string_lossy()
                .into_owned(),
            expected_size: contents.len() as i64,
            import_id: None,
        })
        .await
        .unwrap();

        let first = workflow.recover_import_intents().await.unwrap();
        assert_eq!(first.completed, 1, "{first:?}");

        let second = workflow.recover_import_intents().await.unwrap();
        assert_eq!(
            second,
            ImportRecoveryReport::default(),
            "a second run with nothing left to do must be a total no-op: {second:?}"
        );

        let items = db
            .list_library_items_by_work(user_id, work_id)
            .await
            .unwrap();
        assert_eq!(items.len(), 1, "rerun must not duplicate the LibraryItem");
    }

    #[tokio::test]
    async fn recovery_flags_renamed_intent_with_missing_target_as_anomalous() {
        // Defensive branch: `Renamed` is only persisted after a successful
        // rename, so a missing target at that state is unexpected — recovery
        // must escalate (leave the intent for investigation), never
        // silently roll back a state that claims the rename succeeded.
        let (db, workflow, user_id, work_id, root_folder_id, _root_dir) = seed().await;

        let intent = db
            .create_import_intent(CreateImportIntentDbRequest {
                user_id,
                work_id,
                root_folder_id,
                media_type: MediaType::Ebook,
                target_relative: "D2 Author/Never There.epub".into(),
                staging_path: "/nonexistent/.livrarr-import-ghost5".into(),
                expected_size: 5,
                import_id: None,
            })
            .await
            .unwrap();
        db.mark_import_intent_renamed(intent.id).await.unwrap();

        let report = workflow.recover_import_intents().await.unwrap();

        assert_eq!(report.anomalous, 1, "{report:?}");
        assert_eq!(report.completed, 0, "{report:?}");
        assert_eq!(report.rolled_back, 0, "{report:?}");
        assert_eq!(
            db.list_import_intents().await.unwrap().len(),
            1,
            "an anomalous intent is left in place, not silently discarded"
        );
    }

    #[tokio::test]
    async fn recovery_flags_size_mismatched_renamed_intent_as_anomalous_and_does_not_finalize() {
        // A Renamed intent's expected_size is the crash-consistency contract
        // between what was staged and what recovery finalizes. A target
        // file present at the right path but the WRONG size (e.g. a
        // name-reuse race, or something external replacing the file) must
        // never be silently finalized as if it were the completed import.
        let (db, workflow, user_id, work_id, root_folder_id, root_dir) = seed().await;

        let target_relative = "D2 Author/D2 Test Book.epub";
        let target_abs = root_dir.path().join(target_relative);
        tokio::fs::create_dir_all(target_abs.parent().unwrap())
            .await
            .unwrap();
        // The file actually on disk is a different size than the intent
        // expects.
        tokio::fs::write(&target_abs, b"wrong size entirely")
            .await
            .unwrap();

        let intent = db
            .create_import_intent(CreateImportIntentDbRequest {
                user_id,
                work_id,
                root_folder_id,
                media_type: MediaType::Ebook,
                target_relative: target_relative.into(),
                staging_path: root_dir
                    .path()
                    .join(".livrarr-import-sizemismatch")
                    .to_string_lossy()
                    .into_owned(),
                expected_size: 999_999,
                import_id: None,
            })
            .await
            .unwrap();
        db.mark_import_intent_renamed(intent.id).await.unwrap();

        let report = workflow.recover_import_intents().await.unwrap();

        assert_eq!(report.anomalous, 1, "{report:?}");
        assert_eq!(report.completed, 0, "{report:?}");
        assert!(
            db.list_library_items_by_work(user_id, work_id)
                .await
                .unwrap()
                .is_empty(),
            "a size-mismatched target must never be finalized as a LibraryItem"
        );
        assert_eq!(
            db.list_import_intents().await.unwrap().len(),
            1,
            "a size-mismatched intent is left in place, not silently discarded"
        );
    }

    #[tokio::test]
    async fn concurrent_import_to_same_path_is_path_collision_not_a_double_write() {
        // Moving the LibraryItem row's creation to after the rename (the
        // crash-safety fix itself) removed the old collision arbiter
        // (library_items' own UNIQUE constraint, checked before any file
        // I/O). import_intents' own UNIQUE(user_id, root_folder_id,
        // target_relative) must now catch this instead: a second work whose
        // import is already in flight (intent outstanding, not yet
        // rename/finalized — find_library_item_by_path sees nothing yet)
        // targeting the exact same path must be rejected, never allowed to
        // stage/rename over the first one.
        use livrarr_db::{CreateWorkDbRequest, WorkDbCreate};

        let (db, workflow, user_id, work_id, root_folder_id, root_dir) = seed().await;
        let (other_work, _) = db
            .create_work(CreateWorkDbRequest {
                user_id,
                title: "Second In Flight".into(),
                author_name: "Second Author".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        let target_relative = "Shared/Path.epub";

        // work_id's import is already in flight: an intent is outstanding,
        // but nothing has been renamed or finalized yet.
        db.create_import_intent(CreateImportIntentDbRequest {
            user_id,
            work_id,
            root_folder_id,
            media_type: MediaType::Ebook,
            target_relative: target_relative.into(),
            staging_path: root_dir
                .path()
                .join(".livrarr-import-inflight")
                .to_string_lossy()
                .into_owned(),
            expected_size: 4,
            import_id: None,
        })
        .await
        .unwrap();

        let source_dir = tempfile::tempdir().unwrap();
        let source_path = source_dir.path().join("incoming.epub");
        tokio::fs::write(&source_path, b"racing bytes")
            .await
            .unwrap();

        let result = workflow
            .import_file(
                user_id,
                ImportFileRequest {
                    work_id: other_work.id,
                    root_folder_id,
                    source: source_path,
                    target_relative: target_relative.into(),
                    media_type: MediaType::Ebook,
                    materialization: Materialization::Copy,
                    import_id: None,
                    extract_chapters: false,
                },
            )
            .await;

        assert!(
            matches!(&result, Err(ImportWorkflowError::PathCollision(p)) if p == target_relative),
            "expected PathCollision, got: {result:?}"
        );
        assert!(
            !tokio::fs::try_exists(root_dir.path().join(target_relative))
                .await
                .unwrap(),
            "the second work's bytes must never reach the shared target path"
        );
        assert!(
            db.list_library_items_by_work(user_id, other_work.id)
                .await
                .unwrap()
                .is_empty(),
            "the rejected import must not create a LibraryItem"
        );
        assert_eq!(
            db.list_import_intents().await.unwrap().len(),
            1,
            "only the original in-flight intent remains — the rejected attempt left nothing behind"
        );
    }

    // -----------------------------------------------------------------
    // Durability-failure tests: a real fsync failure (dir or data) must
    // fail the import / recovery pass rather than being silently ignored,
    // and a DB error listing intents must be surfaced, not defaulted away.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn import_fails_and_leaves_intent_for_recovery_when_parent_dir_fsync_errors() {
        // A real fsync failure on the parent directory means the rename's
        // durability is NOT confirmed. The prior code only caught the
        // spawn_blocking JoinError (a task panic) and silently ignored a
        // genuine `Ok(Err(io_error))` from fsync_dir itself, finalizing the
        // import anyway. This must fail the import instead, leaving the
        // already-renamed file + intent in place for recovery to finish.
        let (db, workflow, user_id, work_id, root_folder_id, root_dir) = seed().await;

        let target_relative = "D2 Author/D2 Test Book.epub";
        let target_parent = root_dir.path().join("D2 Author");
        fsync_test_failpoint::arm(target_parent.clone());

        let source_dir = tempfile::tempdir().unwrap();
        let source_path = source_dir.path().join("incoming.epub");
        tokio::fs::write(&source_path, b"some real bytes")
            .await
            .unwrap();

        let result = workflow
            .import_file(
                user_id,
                ImportFileRequest {
                    work_id,
                    root_folder_id,
                    source: source_path,
                    target_relative: target_relative.into(),
                    media_type: MediaType::Ebook,
                    materialization: Materialization::Copy,
                    import_id: None,
                    extract_chapters: false,
                },
            )
            .await;

        assert!(
            matches!(&result, Err(ImportWorkflowError::ImportFailed(msg)) if msg.contains("fsync")),
            "expected a dir-fsync failure to fail the import, got: {result:?}"
        );
        assert!(
            db.list_library_items_by_work(user_id, work_id)
                .await
                .unwrap()
                .is_empty(),
            "must not finalize a LibraryItem when the durability fsync failed"
        );
        assert_eq!(
            db.list_import_intents().await.unwrap().len(),
            1,
            "the intent must survive for recovery to finish the job next startup"
        );
        assert!(
            tokio::fs::try_exists(root_dir.path().join(target_relative))
                .await
                .unwrap(),
            "the rename itself already happened durably on disk before the fsync step"
        );
    }

    #[tokio::test]
    async fn hardlink_first_copy_fallback_never_persists_when_data_fsync_fails() {
        // Force the hard_link attempt to fail (EEXIST — a file already sits
        // at `dst`) so the cross-fs copy fallback runs: the exact branch
        // this bug lives in, regardless of *why* hard_link failed (the code
        // never distinguishes EXDEV from any other hard_link error).
        let src_dir = tempfile::tempdir().unwrap();
        let src = src_dir.path().join("source.epub");
        tokio::fs::write(&src, b"cross-fs fallback bytes")
            .await
            .unwrap();

        let dst_dir = tempfile::tempdir().unwrap();
        let dst = dst_dir.path().join("target.epub");
        tokio::fs::write(&dst, b"pre-existing decoy").await.unwrap();

        fsync_test_failpoint::arm(dst.clone());

        let result = materialize_hardlink_first(&src, &dst).await;

        assert!(
            matches!(&result, Err(msg) if msg.contains("data fsync")),
            "expected the injected data-fsync failure to be surfaced, got: {result:?}"
        );
        assert_eq!(
            tokio::fs::read(&dst).await.unwrap(),
            b"pre-existing decoy",
            "persist must never run when the pre-persist data fsync failed"
        );
    }

    #[tokio::test]
    async fn recovery_surfaces_list_intents_failure_instead_of_a_default_report() {
        // A DB error while listing intents must never look identical to
        // "there was nothing to recover" — the caller (startup) needs to be
        // able to tell the two apart and escalate loudly.
        let (db, workflow, _user_id, _work_id, _root_folder_id, _root_dir) = seed().await;

        // A genuine sqlx failure — no mock/seam needed: closing the real
        // pool makes the next query fail for real.
        db.pool().close().await;

        let result = workflow.recover_import_intents().await;

        assert!(
            result.is_err(),
            "a list_import_intents failure must propagate, not collapse into Ok(ImportRecoveryReport::default())"
        );
    }

    // -----------------------------------------------------------------
    // Sweep-phase tests: aged+unreferenced vs. referenced vs. too-fresh.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn sweep_removes_aged_unreferenced_staging_file() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join(format!("{STAGING_PREFIX}orphan"));
        tokio::fs::write(&staging, b"orphaned").await.unwrap();
        // Guarantee real elapsed wall-clock time has passed so a coarse
        // mtime resolution can never make "aged since zero" a false tie.
        tokio::time::sleep(Duration::from_millis(20)).await;

        let root_folders = [RootFolder {
            id: 1,
            path: dir.path().to_string_lossy().into_owned(),
            media_type: MediaType::Ebook,
        }];
        let referenced = std::collections::HashSet::new();

        let removed =
            sweep_unreferenced_staging_files(&root_folders, &referenced, Duration::ZERO).await;

        assert_eq!(removed, 1);
        assert!(!tokio::fs::try_exists(&staging).await.unwrap());
    }

    #[tokio::test]
    async fn sweep_preserves_referenced_staging_file_regardless_of_age() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join(format!("{STAGING_PREFIX}referenced"));
        tokio::fs::write(&staging, b"still owned by an intent")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;

        let root_folders = [RootFolder {
            id: 1,
            path: dir.path().to_string_lossy().into_owned(),
            media_type: MediaType::Ebook,
        }];
        let mut referenced = std::collections::HashSet::new();
        referenced.insert(staging.clone());

        // Duration::ZERO means "aged" would otherwise sweep it immediately —
        // only the reference should protect it.
        let removed =
            sweep_unreferenced_staging_files(&root_folders, &referenced, Duration::ZERO).await;

        assert_eq!(
            removed, 0,
            "an intent-referenced staging file is never swept"
        );
        assert!(tokio::fs::try_exists(&staging).await.unwrap());
    }

    #[tokio::test]
    async fn sweep_preserves_fresh_unreferenced_staging_file() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join(format!("{STAGING_PREFIX}fresh"));
        tokio::fs::write(&staging, b"just created").await.unwrap();

        let root_folders = [RootFolder {
            id: 1,
            path: dir.path().to_string_lossy().into_owned(),
            media_type: MediaType::Ebook,
        }];
        let referenced = std::collections::HashSet::new();

        // A huge min_age means "fresh" is nowhere near old enough to sweep.
        let removed = sweep_unreferenced_staging_files(
            &root_folders,
            &referenced,
            Duration::from_secs(999_999),
        )
        .await;

        assert_eq!(removed, 0, "a fresh unreferenced staging file must survive");
        assert!(tokio::fs::try_exists(&staging).await.unwrap());
    }
}

// ---------------------------------------------------------------------------
// Source file enumeration
// ---------------------------------------------------------------------------

struct SourceFile {
    path: PathBuf,
    media_type: MediaType,
}

fn enumerate_source_files(source: &Path) -> Result<(Vec<SourceFile>, u64), String> {
    let mut files = Vec::new();
    let mut total_size = 0u64;
    if source.is_file() {
        total_size = std::fs::metadata(source).map(|m| m.len()).unwrap_or(0);
        if let Some(media_type) = classify_file(source) {
            files.push(SourceFile {
                path: source.to_path_buf(),
                media_type,
            });
        }
    } else if source.is_dir() {
        walk_dir(source, &mut files, &mut total_size)?;
    } else {
        return Err(format!(
            "source is neither file nor directory: {}",
            source.display()
        ));
    }
    Ok((files, total_size))
}

#[cfg(test)]
mod enumerate_source_files_tests {
    use super::*;
    use std::io::Write;

    fn write_file(path: &Path, byte_len: usize) {
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(&vec![0u8; byte_len]).unwrap();
    }

    #[test]
    fn total_size_includes_unrecognized_files_alongside_recognized_ones() {
        let dir = tempfile::tempdir().unwrap();
        write_file(&dir.path().join("A Time of Dread.epub"), 3_900_000);
        write_file(&dir.path().join("A Time of Dread.mobi"), 3_700_000);
        write_file(&dir.path().join("cover.jpg"), 1_300_000);

        let (files, total_size) = enumerate_source_files(dir.path()).unwrap();

        assert_eq!(
            files.len(),
            2,
            "only the two recognized ebook files are importable"
        );
        assert_eq!(
            total_size,
            3_900_000 + 3_700_000 + 1_300_000,
            "total on-disk size must include the unrecognized cover image too, \
             or a fully-downloaded release with extra files reads as partial"
        );
    }

    #[test]
    fn total_size_matches_files_when_nothing_unrecognized_present() {
        let dir = tempfile::tempdir().unwrap();
        write_file(&dir.path().join("book.epub"), 1_000_000);

        let (files, total_size) = enumerate_source_files(dir.path()).unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(total_size, 1_000_000);
    }
}

fn walk_dir(dir: &Path, files: &mut Vec<SourceFile>, total_size: &mut u64) -> Result<(), String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("skipping unreadable directory {}: {e}", dir.display());
            return Ok(());
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("skipping unreadable dir entry in {}: {e}", dir.display());
                continue;
            }
        };
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(e) => {
                tracing::warn!("skipping {}: {e}", path.display());
                continue;
            }
        };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            walk_dir(&path, files, total_size)?;
        } else if ft.is_file() {
            // Every file counts toward the on-disk total — including ones
            // classify_file won't recognize (cover art, NFO, samples) —
            // so the size-completeness check below compares against what
            // actually downloaded, not just the subset Livrarr imports.
            *total_size += entry.metadata().map(|m| m.len()).unwrap_or(0);
            if let Some(media_type) = classify_file(&path) {
                files.push(SourceFile { path, media_type });
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Path building
// ---------------------------------------------------------------------------

pub fn build_target_path(
    root: &str,
    user_id: UserId,
    author: &str,
    title: &str,
    media_type: MediaType,
    source_file: &Path,
    source_root: &Path,
) -> String {
    let author_san = sanitize_path_component(author, "Unknown Author");
    let title_san = sanitize_path_component(title, "Unknown Title");
    let root = root.trim_end_matches('/');

    match media_type {
        MediaType::Ebook => {
            let ext = source_file
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("epub");
            format!("{root}/{user_id}/{author_san}/{title_san}.{ext}")
        }
        MediaType::Audiobook => {
            let relative = if source_file == source_root {
                Path::new(
                    source_file
                        .file_name()
                        .unwrap_or(std::ffi::OsStr::new("unknown")),
                )
            } else {
                source_file.strip_prefix(source_root).unwrap_or(source_file)
            };
            let relative_str = relative.to_string_lossy();
            format!("{root}/{user_id}/{author_san}/{title_san}/{relative_str}")
        }
    }
}

// ---------------------------------------------------------------------------
// Path validation
// ---------------------------------------------------------------------------

fn validate_target_path(target: &Path, root_folder_path: &str) -> Result<(), String> {
    // Reject .. components
    if target.components().any(|c| c.as_os_str() == "..") {
        return Err(format!(
            "path traversal blocked: target {} contains '..'",
            target.display()
        ));
    }
    // Verify target is within root folder
    let root_path = Path::new(root_folder_path);
    if !target.starts_with(root_path) {
        return Err(format!(
            "path traversal blocked: target {} not within {}",
            target.display(),
            root_folder_path
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Materialization
// ---------------------------------------------------------------------------

/// Hard-links the source into place; falls back to a size-verified copy when
/// the link fails (e.g. the source and target are on different filesystems).
/// Lifted from the Readarr import road's `materialize_file` so grab import,
/// Readarr import, and future doors share one implementation. Returns the
/// final file size in bytes (needed by callers to populate `file_size` on
/// the created `LibraryItem` row).
async fn materialize_hardlink_first(src: &Path, dst: &Path) -> Result<u64, String> {
    let src = src.to_path_buf();
    let dst = dst.to_path_buf();
    tokio::task::spawn_blocking(move || {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {e}"))?;
        }
        if std::fs::hard_link(&src, &dst).is_ok() {
            let size = std::fs::metadata(&dst)
                .map_err(|e| format!("cannot stat hardlinked file: {e}"))?
                .len();
            return Ok(size);
        }
        let parent = dst.parent().ok_or("dest has no parent")?;
        let mut tmp = tempfile::NamedTempFile::new_in(parent)
            .map_err(|e| format!("tempfile create failed: {e}"))?;
        let source_size = std::fs::metadata(&src)
            .map_err(|e| format!("cannot stat source: {e}"))?
            .len();
        let copied = std::io::copy(
            &mut std::fs::File::open(&src).map_err(|e| format!("open source failed: {e}"))?,
            &mut tmp,
        )
        .map_err(|e| format!("copy failed: {e}"))?;
        if copied != source_size {
            return Err(format!(
                "copy size mismatch: copied {copied} vs source {source_size}"
            ));
        }
        // The parent-dir fsync the caller runs after this returns only
        // flushes the rename's directory-entry change — it says nothing
        // about this file's own data blocks. Sync those explicitly before
        // persist (the atomic rename), or a crash right after could durably
        // rename in zero-length or partially-flushed content.
        #[cfg(test)]
        if fsync_test_failpoint::is_armed(&dst) {
            return Err("data fsync failed: injected (test)".to_string());
        }
        tmp.as_file()
            .sync_all()
            .map_err(|e| format!("data fsync failed: {e}"))?;
        tmp.persist(&dst)
            .map_err(|e| format!("rename failed: {e}"))?;
        Ok(copied)
    })
    .await
    .expect("spawn_blocking panicked")
}

// ---------------------------------------------------------------------------
// Format filtering
// ---------------------------------------------------------------------------

fn filter_preferred_formats(
    files: Vec<SourceFile>,
    config: &livrarr_db::MediaManagementConfig,
) -> Vec<SourceFile> {
    let ebook_prefs = &config.preferred_ebook_formats;
    let audio_prefs = &config.preferred_audiobook_formats;

    let ext_of = |f: &SourceFile| -> String {
        f.path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase()
    };

    let best_ebook_ext = ebook_prefs.iter().find(|pref| {
        files
            .iter()
            .any(|f| f.media_type == MediaType::Ebook && ext_of(f) == **pref)
    });

    let best_audio_ext = audio_prefs.iter().find(|pref| {
        files
            .iter()
            .any(|f| f.media_type == MediaType::Audiobook && ext_of(f) == **pref)
    });

    files
        .into_iter()
        .filter(|f| {
            let ext = f
                .path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            match f.media_type {
                MediaType::Ebook => match best_ebook_ext {
                    Some(best) => ext == *best,
                    None => true,
                },
                MediaType::Audiobook => match best_audio_ext {
                    Some(best) => ext == *best,
                    None => true,
                },
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Trait implementation
// ---------------------------------------------------------------------------

/// Returns the m4b container duration when the header parsed successfully
/// (`None` otherwise) so callers can feed `.kash` link establishment without
/// re-reading the file.
async fn try_extract_chapters<D: ChapterDb>(
    item_id: livrarr_domain::LibraryItemId,
    target: &Path,
    media_type: MediaType,
    db: &D,
    extractor: &Arc<dyn ChapterExtractor>,
) -> Option<f64> {
    let mut container_duration: Option<f64> = None;
    let ext = target
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if ext.as_str() == "m4b" {
        let path = target.to_path_buf();
        let extractor = extractor.clone();
        let result =
            tokio::task::spawn_blocking(move || extractor.extract_m4b_chapters(&path)).await;

        match result {
            Ok(Ok(extraction)) => {
                let dur = extraction.duration_secs;
                container_duration = dur;
                if extraction.chapters.is_empty() {
                    if let Err(e) = db
                        .update_chapter_scan_result(item_id, "no_chapters", dur)
                        .await
                    {
                        tracing::warn!(
                            item_id,
                            "chapter scan: failed to persist no_chapters status: {e}"
                        );
                    }
                } else {
                    let mut chapters = Vec::new();
                    let extracted = &extraction.chapters;
                    for (i, ch) in extracted.iter().enumerate() {
                        let title = if ch.title.is_empty() {
                            format!("Chapter {}", i + 1)
                        } else {
                            ch.title.clone()
                        };
                        let end_time = if i + 1 < extracted.len() {
                            extracted[i + 1].start_time_secs
                        } else {
                            match dur {
                                Some(d) if d > ch.start_time_secs => d,
                                _ => {
                                    tracing::warn!(
                                        item_id,
                                        "last chapter has no valid end time — dropping"
                                    );
                                    continue;
                                }
                            }
                        };
                        chapters.push(livrarr_domain::AudiobookChapter {
                            id: 0,
                            library_item_id: item_id,
                            chapter_index: i as i32,
                            title,
                            start_time_secs: ch.start_time_secs,
                            end_time_secs: end_time,
                        });
                    }
                    if chapters.is_empty() {
                        if let Err(e) = db
                            .update_chapter_scan_result(item_id, "no_chapters", dur)
                            .await
                        {
                            tracing::warn!(
                                item_id,
                                "chapter scan: failed to persist no_chapters status: {e}"
                            );
                        }
                    } else {
                        match db.replace_chapters(item_id, &chapters).await {
                            Ok(()) => {
                                if let Err(e) =
                                    db.update_chapter_scan_result(item_id, "scanned", dur).await
                                {
                                    tracing::warn!(
                                        item_id,
                                        "chapter scan: chapters saved but failed to persist scanned status (will rescan): {e}"
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    item_id,
                                    error = %e,
                                    "chapter extraction: replace_chapters failed — leaving scan_status NULL for retry"
                                );
                            }
                        }
                    }
                }
            }
            Ok(Err(ChapterExtractionError::ParseError(_))) => {
                tracing::warn!(item_id, "corrupt M4B — marking parse_error");
                if let Err(e) = db
                    .update_chapter_scan_result(item_id, "parse_error", None)
                    .await
                {
                    tracing::warn!(
                        item_id,
                        "chapter scan: failed to persist parse_error status: {e}"
                    );
                }
            }
            Ok(Err(ChapterExtractionError::IoError(e))) => {
                tracing::warn!(item_id, error = %e, "chapter extraction I/O error — will retry");
            }
            Err(e) => {
                tracing::warn!(item_id, error = %e, "chapter extraction task panicked");
            }
        }
    } else if media_type == MediaType::Audiobook {
        if let Err(e) = db
            .update_chapter_scan_result(item_id, "no_chapters", None)
            .await
        {
            tracing::warn!(
                item_id,
                "chapter scan: failed to persist no_chapters status: {e}"
            );
        }
    }
    container_duration
}

/// Errors from `.kash` link establishment. Warn-and-continue at every call
/// site — kash problems never fail or abort an import.
#[derive(Debug)]
pub enum KashLinkError {
    KashUnreadable,
    NoMatchingEbook,
    Db(String),
}

/// Detect a sibling `<stem>.kash` for a just-imported m4b and reconcile the
/// kash link: a matching sidecar upserts the link (identity changes reset its
/// per-user state); an absent or duration-mismatched sidecar deletes any
/// stale link; an unreadable sidecar leaves the link intact (a transient IO
/// failure must not destroy state). The m4b itself is never read or hashed
/// here — audio identity is the already-extracted container duration only
/// (REQ-009/REQ-014).
pub async fn establish_kash_link<D>(
    db: &D,
    user_id: UserId,
    audio_item_id: livrarr_domain::LibraryItemId,
    audio_path: &Path,
    work_id: WorkId,
    duration_secs: f64,
) -> Result<(), KashLinkError>
where
    D: ChapterDb + LibraryItemDb + KashLinkDb + RootFolderDb + Send + Sync,
{
    // --- Step 1: sidecar existence check ---
    // No sidecar = unlink (reconciliation); idempotent no-op for the common
    // case where no row ever existed.
    let kash_path = audio_path.with_extension("kash");
    let sidecar_exists = tokio::fs::try_exists(&kash_path).await.unwrap_or(false);
    if !sidecar_exists {
        db.delete_link_for_audio(audio_item_id)
            .await
            .map_err(|e| KashLinkError::Db(e.to_string()))?;
        return Ok(());
    }

    // --- Step 2: read sidecar bytes ---
    // Transient IO must not destroy an existing link — return Err and leave
    // the link row intact.
    let bytes = tokio::fs::read(&kash_path)
        .await
        .map_err(|_| KashLinkError::KashUnreadable)?;

    // --- Step 3: parse the sidecar ---
    // Same conservative posture as an IO error: leave link intact on parse
    // failure.
    let kash =
        livrarr_domain::kash::parse_kash(&bytes).map_err(|_| KashLinkError::KashUnreadable)?;

    // --- Step 4: duration identity check ---
    // The sidecar must describe this audio cut. Drift beyond tolerance means
    // a different rip/edition — delete any stale link to close the R-003
    // poison window, then return Ok (not an import failure).
    if (kash.duration_seconds - duration_secs).abs() > livrarr_domain::kash::DURATION_TOLERANCE_SECS
    {
        db.delete_link_for_audio(audio_item_id)
            .await
            .map_err(|e| KashLinkError::Db(e.to_string()))?;
        tracing::info!(
            audio_item_id,
            kash_duration = kash.duration_seconds,
            container_duration = duration_secs,
            "kash duration drift beyond tolerance — stale link deleted"
        );
        return Ok(());
    }

    // --- Step 5: enumerate EPUB candidates for this work ---
    let candidates = db
        .list_library_items_by_work(user_id, work_id)
        .await
        .map_err(|e| KashLinkError::Db(e.to_string()))?;

    let epub_candidates: Vec<_> = candidates
        .into_iter()
        .filter(|item| {
            item.media_type == livrarr_domain::MediaType::Ebook
                && Path::new(&item.path)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("epub"))
        })
        .collect();

    // --- Step 6: resolve absolute paths and hash each candidate ---
    // First candidate whose SHA-256 hex matches kash.epub_hash is the link
    // target (first-link-wins within the candidate set). Candidates that
    // fail to read are skipped with a warning — do not abort.
    let target_hash = kash.epub_hash.clone();
    let mut matched_id: Option<livrarr_domain::LibraryItemId> = None;

    for candidate in epub_candidates {
        let root = db
            .get_root_folder(candidate.root_folder_id)
            .await
            .map_err(|e| KashLinkError::Db(e.to_string()))?;

        let abs_path = Path::new(&root.path).join(&candidate.path);
        let candidate_id = candidate.id;
        let hash_target = target_hash.clone();

        let hash_result = tokio::task::spawn_blocking(move || -> Option<String> {
            let bytes = std::fs::read(&abs_path).ok()?;
            let hex = format!("{:x}", Sha256::digest(&bytes));
            Some(hex)
        })
        .await
        .unwrap_or(None);

        match hash_result {
            Some(hex) if hex == hash_target => {
                matched_id = Some(candidate_id);
                break;
            }
            None => {
                tracing::warn!(
                    audio_item_id,
                    candidate_id,
                    "kash link: could not read epub candidate — skipping"
                );
            }
            _ => {}
        }
    }

    // --- Step 7: upsert the link or error ---
    let ebook_item_id = matched_id.ok_or(KashLinkError::NoMatchingEbook)?;

    match db
        .upsert_link(NewKashLink {
            audio_item_id,
            ebook_item_id,
            container_duration_secs: duration_secs,
            epub_hash: kash.epub_hash,
        })
        .await
    {
        Ok(_) => Ok(()),
        Err(livrarr_domain::DbError::Constraint { .. }) => {
            tracing::warn!(
                audio_item_id,
                ebook_item_id,
                "kash link: first link wins (v1 1:1 limitation) — ebook already linked"
            );
            Ok(())
        }
        Err(e) => Err(KashLinkError::Db(e.to_string())),
    }
}

/// Shared post-import hook: chapter extraction + kash link establishment.
/// Used by all three import paths (grab import ×2 call sites, manual import
/// via `extract_chapters_for_item`).
///
/// When the m4b header cannot be parsed (file absent or corrupt) but the item
/// already has a stored `duration_seconds` (e.g. set by an earlier scan or a
/// manual import that pre-populated the field), that stored duration is used
/// for kash link establishment so that `.kash` sidecars are wired up even on
/// paths that do not re-parse the container.
async fn extract_chapters_and_kash<D>(
    db: &D,
    extractor: &Arc<dyn ChapterExtractor>,
    item_id: livrarr_domain::LibraryItemId,
    target: &Path,
    media_type: MediaType,
    user_id: UserId,
    work_id: WorkId,
) where
    D: ChapterDb + LibraryItemDb + KashLinkDb + RootFolderDb + Send + Sync,
{
    let extracted_duration = try_extract_chapters(item_id, target, media_type, db, extractor).await;
    let is_m4b = target
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("m4b"));
    if media_type == MediaType::Audiobook && is_m4b {
        // Prefer the freshly-extracted duration; fall back to the item's
        // stored duration_seconds so that manual-import paths (where the m4b
        // may not be present but the DB was pre-populated) still wire up the
        // kash link.
        let duration = if extracted_duration.is_some() {
            extracted_duration
        } else {
            db.get_library_item(user_id, item_id)
                .await
                .ok()
                .and_then(|item| item.duration_seconds)
        };
        if let Some(d) = duration {
            if let Err(e) = establish_kash_link(db, user_id, item_id, target, work_id, d).await {
                tracing::warn!(
                    item_id,
                    error = ?e,
                    "kash link establishment failed — import unaffected"
                );
            }
        }
    }
}

impl<D> ImportWorkflow for ImportWorkflowImpl<D>
where
    D: GrabDb
        + WorkDb
        + LibraryItemDb
        + RootFolderDb
        + HistoryDb
        + RemotePathMappingDb
        + ConfigDb
        + ChapterDb
        + KashLinkDb
        + ImportIntentDb
        + Clone
        + Send
        + Sync
        + 'static,
{
    async fn import_grab(
        &self,
        user_id: UserId,
        grab_id: GrabId,
    ) -> Result<ImportResult, ImportWorkflowError> {
        // Look up grab
        let grab = self
            .db
            .get_grab(user_id, grab_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => ImportWorkflowError::GrabNotFound,
                other => ImportWorkflowError::Db(other),
            })?;

        // Look up work
        let work = self
            .db
            .get_work(user_id, grab.work_id)
            .await
            .map_err(ImportWorkflowError::Db)?;

        // Acquire per-work lock
        let _guard = self.import_locks.lock((user_id, work.id)).await;

        // Resolve source path from grab.content_path
        let source_path = match &grab.content_path {
            Some(path) => {
                // Apply remote path mapping
                let mappings = self
                    .db
                    .list_remote_path_mappings()
                    .await
                    .map_err(ImportWorkflowError::Db)?;
                apply_path_mapping(path, &mappings)
            }
            None => {
                return Err(ImportWorkflowError::SourceNotResolved(
                    "no content_path on grab — download not confirmed".to_string(),
                ));
            }
        };

        let source = PathBuf::from(&source_path);
        tracing::info!(
            grab_id = grab_id,
            raw_path = grab.content_path.as_deref().unwrap_or("(none)"),
            resolved_path = %source.display(),
            "import: resolved source path"
        );

        // Check source exists
        let source_clone = source.clone();
        let exists = tokio::task::spawn_blocking(move || source_clone.exists())
            .await
            .unwrap_or(false);

        if !exists {
            tracing::warn!(
                grab_id = grab_id,
                path = %source.display(),
                "import: source path does not exist"
            );
            return Err(ImportWorkflowError::SourceInaccessible);
        }

        let source_for_meta = source.clone();
        let (is_file, is_dir) = tokio::task::spawn_blocking(move || {
            (source_for_meta.is_file(), source_for_meta.is_dir())
        })
        .await
        .unwrap_or((false, false));
        tracing::info!(
            grab_id = grab_id,
            path = %source.display(),
            is_file = is_file,
            is_dir = is_dir,
            "import: source type"
        );

        // Enumerate files
        let source_clone = source.clone();
        let (source_files, total_size) =
            tokio::task::spawn_blocking(move || enumerate_source_files(&source_clone))
                .await
                .map_err(|e| ImportWorkflowError::SourceNotResolved(format!("spawn error: {e}")))?
                .map_err(|e| {
                    ImportWorkflowError::SourceNotResolved(format!("enumeration failed: {e}"))
                })?;

        tracing::info!(
            grab_id = grab_id,
            file_count = source_files.len(),
            files = ?source_files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>(),
            "import: enumerated source files"
        );

        if source_files.is_empty() {
            if let Err(e) = self
                .db
                .update_grab_status(
                    user_id,
                    grab_id,
                    GrabStatus::ImportFailed,
                    Some("no recognized media files"),
                )
                .await
            {
                tracing::warn!(
                    grab_id = grab_id,
                    "import: failed to persist ImportFailed status (no recognized media files): {e}"
                );
            }
            return Ok(ImportResult {
                grab_id,
                final_status: GrabStatus::ImportFailed,
                imported_files: vec![],
                failed_files: vec![],
                skipped_files: vec![],
                warnings: vec!["no recognized media files found".into()],
            });
        }

        // File size pre-check BEFORE format filtering — compares the full
        // on-disk size (every file under the source path, not just the
        // ones Livrarr recognizes as importable) against grab.size, so a
        // bundled cover image, NFO, or sample file doesn't read as a
        // partial download.
        if let Some(expected_size) = grab.size {
            if expected_size > 0 {
                let local_total = total_size as i64;

                if local_total < expected_size * 9 / 10 {
                    let error = format!(
                        "files not fully synced: local {:.1}MB vs expected {:.1}MB",
                        local_total as f64 / 1_048_576.0,
                        expected_size as f64 / 1_048_576.0,
                    );
                    if let Err(e) = self
                        .db
                        .update_grab_status(
                            user_id,
                            grab_id,
                            GrabStatus::ImportFailed,
                            Some(&error),
                        )
                        .await
                    {
                        tracing::warn!(
                            grab_id = grab_id,
                            "import: failed to persist ImportFailed status (size mismatch): {e}"
                        );
                    }
                    return Ok(ImportResult {
                        grab_id,
                        final_status: GrabStatus::ImportFailed,
                        imported_files: vec![],
                        failed_files: vec![],
                        skipped_files: vec![],
                        warnings: vec![error],
                    });
                }
            }
        }

        // Filter to preferred formats AFTER size check
        let media_mgmt = self
            .db
            .get_media_management_config()
            .await
            .map_err(ImportWorkflowError::Db)?;
        let source_files = filter_preferred_formats(source_files, &media_mgmt);

        // Get root folders
        let root_folders = self
            .db
            .list_root_folders()
            .await
            .map_err(ImportWorkflowError::Db)?;

        let author_name = &work.author_name;
        let title = &work.title;
        let work_id = work.id;

        let mut imported_files = Vec::new();
        let mut failed_files = Vec::new();
        let mut skipped_files = Vec::new();
        let mut warnings = Vec::new();

        // Process each file
        for sf in &source_files {
            let media_type = sf.media_type;
            let source_name = sf
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();

            // Find root folder for this media type
            let root_folder = match root_folders.iter().find(|rf| rf.media_type == media_type) {
                Some(rf) => rf,
                None => {
                    failed_files.push(FailedFile {
                        source_name,
                        error: format!("no root folder for {:?}", media_type),
                    });
                    continue;
                }
            };

            // Build target path
            let target_path = build_target_path(
                &root_folder.path,
                user_id,
                author_name,
                title,
                media_type,
                &sf.path,
                &source,
            );

            // Compute relative path — the request into the shared import core
            // takes a root-relative path, not the door's absolute one.
            let relative = target_path
                .strip_prefix(&root_folder.path)
                .unwrap_or(&target_path)
                .trim_start_matches('/')
                .to_string();

            let req = ImportFileRequest {
                work_id,
                root_folder_id: root_folder.id,
                source: sf.path.clone(),
                target_relative: relative,
                media_type,
                materialization: Materialization::Copy,
                import_id: None,
                extract_chapters: true,
            };

            match self.import_file_locked(user_id, req).await {
                Ok(ImportFileOutcome::Imported { item_id, path }) => {
                    let file_size = self
                        .db
                        .get_library_item(user_id, item_id)
                        .await
                        .map(|item| item.file_size)
                        .unwrap_or(0);
                    imported_files.push(ImportedFile {
                        source_name,
                        target_relative_path: path,
                        media_type,
                        file_size: file_size as u64,
                        library_item_id: item_id,
                        tags_written: false,
                        cwa_copied: false,
                    });
                }
                Ok(ImportFileOutcome::Adopted { item_id, path }) => {
                    let file_size = self
                        .db
                        .get_library_item(user_id, item_id)
                        .await
                        .map(|item| item.file_size)
                        .unwrap_or(0);
                    imported_files.push(ImportedFile {
                        source_name,
                        target_relative_path: path,
                        media_type,
                        file_size: file_size as u64,
                        library_item_id: item_id,
                        tags_written: false,
                        cwa_copied: false,
                    });
                    warnings.push(format!("adopted orphaned file: {}", target_path));
                }
                Ok(ImportFileOutcome::Skipped {
                    reason: SkipReason::AlreadyImported,
                }) => {
                    skipped_files.push(SkippedFile {
                        source_name,
                        reason: "already imported (dedup)".into(),
                    });
                }
                Err(ImportWorkflowError::PathCollision(path)) => {
                    failed_files.push(FailedFile {
                        source_name,
                        error: format!(
                            "path collision: {path} already claimed by a different work"
                        ),
                    });
                }
                Err(e) => {
                    failed_files.push(FailedFile {
                        source_name,
                        error: e.to_string(),
                    });
                }
            }
        }

        // Determine final status. Any successful import or dedup-skip counts as Imported.
        // The GrabStatus enum doesn't have an ImportedWithErrors variant — partial
        // failures are reported via failed_files in the result.
        let final_status = if !imported_files.is_empty() || !skipped_files.is_empty() {
            GrabStatus::Imported
        } else {
            GrabStatus::ImportFailed
        };

        // Update grab status
        let error_msg = if failed_files.is_empty() {
            None
        } else {
            let errors: Vec<&str> = failed_files.iter().map(|f| f.error.as_str()).collect();
            Some(errors.join("; "))
        };
        if let Err(e) = self
            .db
            .update_grab_status(user_id, grab_id, final_status, error_msg.as_deref())
            .await
        {
            tracing::warn!(
                grab_id = grab_id,
                final_status = ?final_status,
                "import: failed to persist final grab status: {e}"
            );
        }

        // Record history event
        record_history(
            &self.db,
            user_id,
            history_events::imported_batch(
                work_id,
                &work.title,
                Some(&work.author_name),
                &grab.title,
                final_status == GrabStatus::Imported,
                imported_files.len(),
                failed_files.len(),
                skipped_files.len(),
            ),
        )
        .await;

        Ok(ImportResult {
            grab_id,
            final_status,
            imported_files,
            failed_files,
            skipped_files,
            warnings,
        })
    }

    async fn import_file(
        &self,
        user_id: UserId,
        req: ImportFileRequest,
    ) -> Result<ImportFileOutcome, ImportWorkflowError> {
        let _guard = self.import_locks.lock((user_id, req.work_id)).await;
        self.import_file_locked(user_id, req).await
    }
}

// ---------------------------------------------------------------------------
// Remote path mapping helper
// ---------------------------------------------------------------------------

fn path_starts_with(path: &str, prefix: &str) -> bool {
    let prefix = prefix.strip_suffix('/').unwrap_or(prefix);
    path == prefix || path.starts_with(&format!("{}/", prefix))
}

fn apply_path_mapping(
    content_path: &str,
    mappings: &[livrarr_domain::RemotePathMapping],
) -> String {
    let content_path = &content_path.replace('\\', "/");
    // Find longest matching remote_path prefix
    let best = mappings
        .iter()
        .filter(|m| {
            let rp = m.remote_path.replace('\\', "/");
            path_starts_with(content_path, &rp)
        })
        .max_by_key(|m| m.remote_path.len());

    match best {
        Some(mapping) => {
            let rp = mapping.remote_path.replace('\\', "/");
            content_path
                .replacen(&rp, &mapping.local_path, 1)
                .replace("//", "/")
        }
        None => content_path.to_string(),
    }
}

#[cfg(test)]
mod import_locks_sweeper_tests {
    use super::*;

    /// Never invoked — `ImportWorkflowImpl::new`'s inherent constructor
    /// requires a concrete `Arc<dyn ChapterExtractor>`, but this test never
    /// exercises chapter extraction, only the sweep wiring.
    struct UnusedChapterExtractor;
    impl livrarr_domain::services::ChapterExtractor for UnusedChapterExtractor {
        fn extract_m4b_chapters(
            &self,
            _path: &Path,
        ) -> Result<
            livrarr_domain::services::ChapterExtractionResult,
            livrarr_domain::services::ChapterExtractionError,
        > {
            unreachable!("test double: sweep-wiring test never extracts chapters")
        }
    }

    // The inherent constructor imposes no trait bounds on `D` — `()` stands
    // in for `db` since this test never calls a trait method on it.
    fn new_test_workflow() -> ImportWorkflowImpl<()> {
        ImportWorkflowImpl::new(
            (),
            Arc::new(tokio::sync::Semaphore::new(1)),
            Arc::new(PathBuf::from("unused")),
            Arc::new(UnusedChapterExtractor),
        )
    }

    /// D3 #8 / R-5: `sweep()` existed with zero production callers. This
    /// proves the constructor now spawns a task holding a live `Arc` clone
    /// of the SAME `import_locks` the workflow locks against — strong_count
    /// is 2 (the struct's own field + the spawned task's clone) only if a
    /// task was actually spawned and targets this instance; it stays 1 if
    /// the wiring regresses or spawns an unrelated instance.
    #[tokio::test]
    async fn constructor_spawns_a_sweeper_holding_the_live_import_locks_arc() {
        let wf = new_test_workflow();
        assert_eq!(
            Arc::strong_count(&wf.import_locks),
            2,
            "constructor must spawn exactly one sweep task holding its own \
             Arc clone of import_locks"
        );
    }

    /// The sweeper must be a recurring loop, not a one-shot: after the real
    /// 300s production interval elapses (via tokio's mock clock — no real
    /// waiting), the task must still be alive and holding its Arc clone.
    #[tokio::test(start_paused = true)]
    async fn the_spawned_sweeper_survives_past_one_full_interval_without_dying() {
        let wf = new_test_workflow();
        assert_eq!(Arc::strong_count(&wf.import_locks), 2);

        tokio::time::advance(std::time::Duration::from_secs(301)).await;
        // Let the woken task actually run its tick and re-arm the next one.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        assert_eq!(
            Arc::strong_count(&wf.import_locks),
            2,
            "the sweep loop must still be alive (looping, not one-shot) \
             after a full interval elapses"
        );
    }

    /// Defensive guard regression test: constructing a workflow outside any
    /// Tokio runtime (a handful of the many call sites across the workspace
    /// are test fixtures; this crate cannot prove every one of them runs
    /// under `#[tokio::test]`) must never panic. If the guard is ever
    /// weakened to an unconditional `tokio::spawn`, this turns into a panic.
    #[test]
    fn constructor_does_not_panic_outside_a_tokio_runtime() {
        let wf = new_test_workflow();
        assert_eq!(
            Arc::strong_count(&wf.import_locks),
            1,
            "no runtime is current here, so no sweep task should be spawned"
        );
    }
}
