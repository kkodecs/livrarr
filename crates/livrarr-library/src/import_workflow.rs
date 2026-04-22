use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::atomic_copy;
use livrarr_db::{
    ConfigDb, CreateHistoryEventDbRequest, CreateLibraryItemDbRequest, GrabDb, HistoryDb,
    LibraryItemDb, RemotePathMappingDb, RootFolderDb, WorkDb,
};
use livrarr_domain::keyed_mutex::KeyedMutex;
use livrarr_domain::services::{
    FailedFile, ImportResult, ImportWorkflow, ImportWorkflowError, ImportedFile, ScanConfirmation,
    SkippedFile,
};
use livrarr_domain::{
    classify_file, sanitize_path_component, DbError, EventType, GrabId, GrabStatus, MediaType,
    UserId, WorkId,
};

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

pub struct ImportWorkflowImpl<D> {
    db: D,
    import_locks: KeyedMutex<(UserId, WorkId)>,
    _import_semaphore: Arc<tokio::sync::Semaphore>,
    _data_dir: Arc<PathBuf>,
}

impl<D> ImportWorkflowImpl<D> {
    pub fn new(
        db: D,
        import_semaphore: Arc<tokio::sync::Semaphore>,
        data_dir: Arc<PathBuf>,
    ) -> Self {
        Self {
            db,
            import_locks: KeyedMutex::new(),
            _import_semaphore: import_semaphore,
            _data_dir: data_dir,
        }
    }
}

// ---------------------------------------------------------------------------
// Source file enumeration
// ---------------------------------------------------------------------------

struct SourceFile {
    path: PathBuf,
    media_type: MediaType,
}

fn enumerate_source_files(source: &Path) -> Result<Vec<SourceFile>, String> {
    let mut files = Vec::new();
    if source.is_file() {
        if let Some(media_type) = classify_file(source) {
            files.push(SourceFile {
                path: source.to_path_buf(),
                media_type,
            });
        }
    } else if source.is_dir() {
        walk_dir(source, &mut files)?;
    } else {
        return Err(format!(
            "source is neither file nor directory: {}",
            source.display()
        ));
    }
    Ok(files)
}

fn walk_dir(dir: &Path, files: &mut Vec<SourceFile>) -> Result<(), String> {
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
            walk_dir(&path, files)?;
        } else if ft.is_file() {
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

fn build_target_path(
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

impl<D> ImportWorkflow for ImportWorkflowImpl<D>
where
    D: GrabDb
        + WorkDb
        + LibraryItemDb
        + RootFolderDb
        + HistoryDb
        + RemotePathMappingDb
        + ConfigDb
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

        // Check source exists
        let source_clone = source.clone();
        let exists = tokio::task::spawn_blocking(move || source_clone.exists())
            .await
            .unwrap_or(false);

        if !exists {
            return Err(ImportWorkflowError::SourceInaccessible);
        }

        // Enumerate files
        let source_clone = source.clone();
        let source_files =
            tokio::task::spawn_blocking(move || enumerate_source_files(&source_clone))
                .await
                .map_err(|e| ImportWorkflowError::SourceNotResolved(format!("spawn error: {e}")))?
                .map_err(|e| {
                    ImportWorkflowError::SourceNotResolved(format!("enumeration failed: {e}"))
                })?;

        // Filter to preferred formats (e.g., epub over mobi when both exist)
        let media_mgmt = self
            .db
            .get_media_management_config()
            .await
            .map_err(ImportWorkflowError::Db)?;
        let source_files = filter_preferred_formats(source_files, &media_mgmt);

        if source_files.is_empty() {
            self.db
                .update_grab_status(
                    user_id,
                    grab_id,
                    GrabStatus::ImportFailed,
                    Some("no recognized media files"),
                )
                .await
                .ok();
            return Ok(ImportResult {
                grab_id,
                final_status: GrabStatus::ImportFailed,
                imported_files: vec![],
                failed_files: vec![],
                skipped_files: vec![],
                warnings: vec!["no recognized media files found".into()],
            });
        }

        // File size pre-check: local files must be >= 90% of grab.size
        if let Some(expected_size) = grab.size {
            if expected_size > 0 {
                let paths: Vec<PathBuf> = source_files.iter().map(|f| f.path.clone()).collect();
                let local_total: i64 = tokio::task::spawn_blocking(move || {
                    paths
                        .iter()
                        .filter_map(|p| std::fs::metadata(p).ok())
                        .map(|m| m.len() as i64)
                        .sum()
                })
                .await
                .unwrap_or(0);

                if local_total < expected_size * 9 / 10 {
                    let error = format!(
                        "files not fully synced: local {:.1}MB vs expected {:.1}MB",
                        local_total as f64 / 1_048_576.0,
                        expected_size as f64 / 1_048_576.0,
                    );
                    self.db
                        .update_grab_status(
                            user_id,
                            grab_id,
                            GrabStatus::ImportFailed,
                            Some(&error),
                        )
                        .await
                        .ok();
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

        // Pre-load existing library items for this work to avoid N+1 dedup queries.
        let existing_items = self
            .db
            .list_library_items_by_work(user_id, work_id)
            .await
            .unwrap_or_default();
        let existing_paths: std::collections::HashSet<&str> =
            existing_items.iter().map(|li| li.path.as_str()).collect();

        // Process each file
        for sf in &source_files {
            let media_type = sf.media_type;

            // Find root folder for this media type
            let root_folder = match root_folders.iter().find(|rf| rf.media_type == media_type) {
                Some(rf) => rf,
                None => {
                    failed_files.push(FailedFile {
                        source_name: sf
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
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

            let target = PathBuf::from(&target_path);

            // Validate path (no traversal)
            if let Err(e) = validate_target_path(&target, &root_folder.path) {
                failed_files.push(FailedFile {
                    source_name: sf
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    error: e,
                });
                continue;
            }

            // Compute relative path for DB storage
            let relative = target_path
                .strip_prefix(&root_folder.path)
                .unwrap_or(&target_path)
                .trim_start_matches('/')
                .to_string();

            // Check if target already exists
            let target_clone = target.clone();
            let target_exists = tokio::task::spawn_blocking(move || target_clone.exists())
                .await
                .unwrap_or(false);

            if target_exists {
                // Check for existing library item (dedup) using pre-loaded set.
                if existing_paths.contains(relative.as_str()) {
                    // Already imported — skip
                    skipped_files.push(SkippedFile {
                        source_name: sf
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        reason: "already imported (dedup)".into(),
                    });
                    continue;
                }

                // Orphan adoption: file exists on disk but no DB record
                let target_for_meta = target.clone();
                let file_size = tokio::task::spawn_blocking(move || {
                    target_for_meta
                        .metadata()
                        .map(|m| m.len() as i64)
                        .unwrap_or(0)
                })
                .await
                .unwrap_or(0);

                match self
                    .db
                    .create_library_item(CreateLibraryItemDbRequest {
                        user_id,
                        work_id,
                        root_folder_id: root_folder.id,
                        path: relative.clone(),
                        media_type,
                        file_size,
                        import_id: None,
                    })
                    .await
                {
                    Ok(item) => {
                        imported_files.push(ImportedFile {
                            source_name: sf
                                .path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned(),
                            target_relative_path: relative,
                            media_type,
                            file_size: file_size as u64,
                            library_item_id: item.id,
                            tags_written: false,
                            cwa_copied: false,
                        });
                        warnings.push(format!("adopted orphaned file: {}", target_path));
                    }
                    Err(e) => {
                        failed_files.push(FailedFile {
                            source_name: sf
                                .path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned(),
                            error: format!("orphan adoption DB error: {e}"),
                        });
                    }
                }
                continue;
            }

            // Copy file to target (atomic copy)
            let src_path = sf.path.clone();
            let dst_path = target.clone();
            match atomic_copy(&src_path, &dst_path).await {
                Ok(copied) => {
                    // Create library item
                    match self
                        .db
                        .create_library_item(CreateLibraryItemDbRequest {
                            user_id,
                            work_id,
                            root_folder_id: root_folder.id,
                            path: relative.clone(),
                            media_type,
                            file_size: copied as i64,
                            import_id: None,
                        })
                        .await
                    {
                        Ok(item) => {
                            imported_files.push(ImportedFile {
                                source_name: sf
                                    .path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned(),
                                target_relative_path: relative,
                                media_type,
                                file_size: copied,
                                library_item_id: item.id,
                                tags_written: false,
                                cwa_copied: false,
                            });
                        }
                        Err(e) => {
                            // File copied but DB failed — leave file on disk for retry recovery
                            failed_files.push(FailedFile {
                                source_name: sf
                                    .path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned(),
                                error: format!("DB error after copy: {e}"),
                            });
                        }
                    }
                }
                Err(e) => {
                    failed_files.push(FailedFile {
                        source_name: sf
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        error: format!("copy failed: {e}"),
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
        self.db
            .update_grab_status(user_id, grab_id, final_status, error_msg.as_deref())
            .await
            .ok();

        // Record history event
        let event_type = if final_status == GrabStatus::Imported {
            EventType::Imported
        } else {
            EventType::ImportFailed
        };
        let _ = self
            .db
            .create_history_event(CreateHistoryEventDbRequest {
                user_id,
                work_id: Some(work_id),
                event_type,
                data: serde_json::json!({
                    "title": grab.title,
                    "imported": imported_files.len(),
                    "failed": failed_files.len(),
                    "skipped": skipped_files.len(),
                }),
            })
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

    async fn retry_import(
        &self,
        user_id: UserId,
        grab_id: GrabId,
    ) -> Result<ImportResult, ImportWorkflowError> {
        // Set grab status back to Importing
        self.db
            .update_grab_status(user_id, grab_id, GrabStatus::Importing, None)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => ImportWorkflowError::GrabNotFound,
                other => ImportWorkflowError::Db(other),
            })?;

        // Re-run import pipeline — dedup inside import_grab handles already-imported files
        self.import_grab(user_id, grab_id).await
    }

    async fn confirm_scan(
        &self,
        _user_id: UserId,
        _scan_id: &str,
        _selections: Vec<ScanConfirmation>,
    ) -> Result<ImportResult, ImportWorkflowError> {
        // Scan-based import will move to ManualImportService (deferred).
        Err(ImportWorkflowError::ScanExpired)
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
