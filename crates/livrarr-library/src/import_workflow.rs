use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::atomic_copy;
use livrarr_db::{
    record_history, ChapterDb, ConfigDb, CreateLibraryItemDbRequest, GrabDb, HistoryDb, KashLinkDb,
    LibraryItemDb, NewKashLink, RemotePathMappingDb, RootFolderDb, WorkDb,
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

static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct ImportWorkflowImpl<D> {
    db: D,
    import_locks: KeyedMutex<(UserId, WorkId)>,
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
        Self {
            db,
            import_locks: KeyedMutex::new(),
            _import_semaphore: import_semaphore,
            _data_dir: data_dir,
            extractor,
        }
    }
}

impl<D> ImportWorkflowImpl<D>
where
    D: ChapterDb + LibraryItemDb + KashLinkDb + RootFolderDb + Send + Sync,
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

        // Target absent: materialize per mode.
        let target_parent = target.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let target_file_name = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        // Bytes land at a unique per-attempt staging name and reach the final
        // path only after the DB row commits, so a cross-work collision never
        // leaves this work's bytes at a path another work's row claims; plain
        // rename is the finalize — the row committed first is the authority
        // for the path, and cross-process races are excluded by the data-dir
        // PID lock at startup. A crashed attempt's orphaned staging file is
        // inert (unique name, never enumerated or served) and is deliberately
        // NOT swept here: a pre-clean pass could race a concurrent
        // same-target import's active staging file.
        let staging = target_parent.join(format!(
            "{target_file_name}.stg{}",
            STAGING_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));

        let file_size: u64 = match req.materialization {
            Materialization::AdoptInPlace => {
                return Err(ImportWorkflowError::SourceNotResolved(format!(
                    "adopt-in-place target does not exist: {}",
                    target.display()
                )));
            }
            Materialization::Copy => atomic_copy(&req.source, &staging)
                .await
                .map_err(|e| ImportWorkflowError::ImportFailed(format!("copy failed: {e}")))?,
            Materialization::HardlinkFirst => materialize_hardlink_first(&req.source, &staging)
                .await
                .map_err(ImportWorkflowError::ImportFailed)?,
        };

        let item = match self
            .db
            .create_library_item(CreateLibraryItemDbRequest {
                user_id,
                work_id: req.work_id,
                root_folder_id: req.root_folder_id,
                path: req.target_relative.clone(),
                media_type: req.media_type,
                file_size: file_size as i64,
                import_id: req.import_id.clone(),
                tag_status: livrarr_db::TagStatus::Pending,
                tagged_at_generation: 0,
            })
            .await
        {
            Ok(item) => item,
            Err(DbError::Constraint { .. }) => {
                let staging_cleanup = staging.clone();
                let _ = tokio::task::spawn_blocking(move || std::fs::remove_file(&staging_cleanup))
                    .await;
                return Err(ImportWorkflowError::PathCollision(req.target_relative));
            }
            Err(e) => {
                let staging_cleanup = staging.clone();
                let _ = tokio::task::spawn_blocking(move || std::fs::remove_file(&staging_cleanup))
                    .await;
                return Err(ImportWorkflowError::Db(e));
            }
        };

        if let Err(e) = tokio::fs::rename(&staging, &target).await {
            let staging_cleanup = staging.clone();
            let _ =
                tokio::task::spawn_blocking(move || std::fs::remove_file(&staging_cleanup)).await;
            // Roll the row back so the failure shape is "no row, no file" —
            // never a committed row pointing at a path with nothing on disk.
            let _ = self.db.delete_library_item(user_id, item.id).await;
            tracing::warn!(
                staging = %staging.display(),
                target = %target.display(),
                error = %e,
                "finalize rename failed after library item commit; row rolled back"
            );
            return Err(ImportWorkflowError::ImportFailed(format!(
                "finalize rename failed: {e}"
            )));
        }

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
