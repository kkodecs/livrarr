use std::path::{Path, PathBuf};

use crate::state::AppState;
use crate::{ApiError, GrabStatus, MediaType};
use livrarr_db::{
    ConfigDb, CreateHistoryEventDbRequest, CreateLibraryItemDbRequest, DownloadClientDb, GrabDb,
    HistoryDb, LibraryItemDb, RemotePathMappingDb, RootFolderDb, WorkDb,
};
use livrarr_domain::EventType;
use livrarr_domain::{classify_file, sanitize_path_component};
use livrarr_tagwrite::TagWriteStatus;

/// Result of an import attempt, returned to the caller (retry handler).
pub struct ImportGrabResult {
    pub final_status: GrabStatus,
    pub imported_count: usize,
    pub failed_count: usize,
    pub skipped_count: usize,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

/// Run the import pipeline for a grab. Called by the retry handler (Phase 3a)
/// and later by the download poller (Phase 4).
///
/// Precondition: grab status already atomically set to `importing` by caller.
pub async fn import_grab(
    state: &AppState,
    user_id: i64,
    grab_id: i64,
) -> Result<ImportGrabResult, ApiError> {
    let grab = state.db.get_grab(user_id, grab_id).await?;
    let work = state.db.get_work(user_id, grab.work_id).await?;

    // Resolve source path: grab.download_url has the torrent name,
    // but we need content_path from the grab record. For Phase 3a,
    // the source path comes from the download client + remote path mapping.
    // Since we don't have qBit content_path stored yet (that's poller territory),
    // we need the download client to query qBit for the torrent's content_path.
    let client = state
        .db
        .get_download_client(grab.download_client_id)
        .await?;

    // Resolve source path based on client type.
    let source_path = match &grab.download_id {
        Some(id) => {
            let content_path = if client.client_type() == "sabnzbd" {
                // For SABnzbd: query history for the storage path.
                fetch_sabnzbd_storage_path(state, &client, id).await?
            } else {
                // For qBit: query for content_path by hash.
                fetch_qbit_content_path(state, &client, id).await?
            };
            // Apply remote path mapping.
            apply_remote_path_mapping(state, &client.host, &content_path).await?
        }
        None => {
            let error = "no download_id — download not confirmed in client".to_string();
            state
                .db
                .update_grab_status(user_id, grab_id, GrabStatus::ImportFailed, Some(&error))
                .await?;
            return Ok(ImportGrabResult {
                final_status: GrabStatus::ImportFailed,
                imported_count: 0,
                failed_count: 0,
                skipped_count: 0,
                warnings: vec![],
                error: Some(error),
            });
        }
    };

    let source = PathBuf::from(&source_path);

    // Check source exists.
    if !source.exists() {
        let error = format!("source path not found: {source_path}");
        state
            .db
            .update_grab_status(user_id, grab_id, GrabStatus::ImportFailed, Some(&error))
            .await?;
        return Ok(ImportGrabResult {
            final_status: GrabStatus::ImportFailed,
            imported_count: 0,
            failed_count: 0,
            skipped_count: 0,
            warnings: vec![],
            error: Some(error),
        });
    }

    // Enumerate files from source (single file or directory).
    let source_files = tokio::task::spawn_blocking({
        let source = source.clone();
        move || enumerate_source_files(&source)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking join error: {e}")))?
    .map_err(|e| ApiError::Internal(format!("source enumeration failed: {e}")))?;

    if source_files.is_empty() {
        let error = "no recognized media files found".to_string();
        state
            .db
            .update_grab_status(user_id, grab_id, GrabStatus::ImportFailed, Some(&error))
            .await?;
        return Ok(ImportGrabResult {
            final_status: GrabStatus::ImportFailed,
            imported_count: 0,
            failed_count: 0,
            skipped_count: 0,
            warnings: vec![],
            error: Some(error),
        });
    }

    // Get root folders for routing.
    let root_folders = state.db.list_root_folders().await?;

    // Get CWA config.
    let media_mgmt = state.db.get_media_management_config().await?;

    let author_name = work.author_name.clone();
    let title = work.title.clone();
    let work_id = work.id;

    // Filter ebooks to best preferred format when multiple formats exist.
    // E.g., if a torrent has epub+mobi+pdf and preference is [epub], only import the epub.
    let source_files = filter_preferred_formats(source_files, &media_mgmt);

    let mut imported_count = 0usize;
    let mut failed_count = 0usize;
    let mut skipped_count = 0usize;
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    // Validate and plan all files first, separating MP3s for batch handling.
    let mut validated: Vec<ValidatedFile> = Vec::new();

    for sf in &source_files {
        let media_type = sf.media_type;

        let root_folder = match root_folders.iter().find(|rf| rf.media_type == media_type) {
            Some(rf) => rf,
            None => {
                warnings.push(format!(
                    "no root folder for {:?}, skipping: {}",
                    media_type,
                    sf.path.display()
                ));
                skipped_count += 1;
                continue;
            }
        };

        let target_path = build_target_path(
            &root_folder.path,
            user_id,
            &author_name,
            &title,
            media_type,
            &sf.path,
            &source,
        );

        let target = PathBuf::from(&target_path);

        if target.components().any(|c| c.as_os_str() == "..") {
            errors.push(format!(
                "path traversal blocked: target {} contains '..'",
                target_path
            ));
            failed_count += 1;
            continue;
        }

        // Verify target is within the root folder.
        let root_path = Path::new(&root_folder.path);
        let canonical_root = std::fs::canonicalize(root_path).unwrap_or(root_path.to_path_buf());
        let canonical_target = match target.strip_prefix(root_path) {
            Ok(relative) => {
                if relative.has_root() {
                    errors.push(format!(
                        "path traversal blocked: relative path is rooted: {}",
                        target_path
                    ));
                    failed_count += 1;
                    continue;
                }
                canonical_root.join(relative)
            }
            Err(_) => {
                if let Some(parent) = target.parent() {
                    match std::fs::canonicalize(parent) {
                        Ok(p) => p.join(target.file_name().unwrap_or_default()),
                        Err(_) => target.clone(),
                    }
                } else {
                    target.clone()
                }
            }
        };

        if !canonical_target.starts_with(&canonical_root) {
            errors.push(format!(
                "path traversal blocked: target {} not within {}",
                target_path,
                canonical_root.display()
            ));
            failed_count += 1;
            continue;
        }

        if target.exists() {
            let relative = target_path
                .strip_prefix(&root_folder.path)
                .unwrap_or(&target_path)
                .trim_start_matches('/');
            let existing_items = state
                .db
                .list_library_items_by_work(user_id, work_id)
                .await?;
            if existing_items.iter().any(|li| li.path == relative) {
                skipped_count += 1;
                continue;
            }
            errors.push(format!("target path already exists: {target_path}"));
            failed_count += 1;
            continue;
        }

        let is_mp3 = sf
            .path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("mp3"))
            .unwrap_or(false);

        validated.push(ValidatedFile {
            source: sf.path.clone(),
            target_path,
            root_folder_id: root_folder.id,
            root_folder_path: root_folder.path.clone(),
            media_type,
            is_mp3,
        });
    }

    // Separate MP3 and non-MP3 files.
    let (mp3_files, other_files): (Vec<_>, Vec<_>) = validated.into_iter().partition(|f| f.is_mp3);

    // Read tag metadata + cover once (shared across all files).
    let has_enrichment = work.enrichment_status != livrarr_domain::EnrichmentStatus::Pending;
    let tag_metadata = if has_enrichment {
        Some(build_tag_metadata(&work))
    } else {
        None
    };
    let cover_data = if has_enrichment {
        read_cover_bytes(state, work_id).await
    } else {
        None
    };

    // --- Process non-MP3 files (per-file .tmp → tag → rename) ---
    for vf in &other_files {
        let result = import_single_file(
            state,
            &vf.source,
            &vf.target_path,
            &vf.root_folder_path,
            vf.root_folder_id,
            vf.media_type,
            user_id,
            work_id,
            tag_metadata.as_ref(),
            cover_data.as_deref(),
            &media_mgmt,
            &author_name,
            &title,
        )
        .await;
        match result {
            Ok(()) => imported_count += 1,
            Err(ImportFileError::Warning(w)) => {
                warnings.push(w);
                imported_count += 1; // file imported, just tag warning
            }
            Err(ImportFileError::Failed(e)) => {
                errors.push(e);
                failed_count += 1;
            }
        }
    }

    // --- Process MP3 files (batch .tmp → tag → rename, all-or-nothing) ---
    if !mp3_files.is_empty() {
        let batch_result = import_mp3_batch(
            state,
            &mp3_files,
            user_id,
            work_id,
            tag_metadata.as_ref(),
            cover_data.as_deref(),
            &media_mgmt,
            &author_name,
            &title,
        )
        .await;
        match batch_result {
            Ok((count, batch_warnings)) => {
                imported_count += count;
                warnings.extend(batch_warnings);
            }
            Err(e) => {
                errors.push(e);
                failed_count += mp3_files.len();
            }
        }
    }

    // Determine final status per IMPORT-014.
    let final_status = if imported_count > 0 && failed_count == 0 {
        GrabStatus::Imported
    } else {
        GrabStatus::ImportFailed
    };

    let error_msg = if errors.is_empty() {
        None
    } else {
        Some(errors.join("; "))
    };

    state
        .db
        .update_grab_status(user_id, grab_id, final_status, error_msg.as_deref())
        .await?;

    // Record history event.
    let event_type = if final_status == GrabStatus::Imported {
        EventType::Imported
    } else {
        EventType::ImportFailed
    };
    if let Err(e) = state
        .db
        .create_history_event(CreateHistoryEventDbRequest {
            user_id,
            work_id: Some(grab.work_id),
            event_type,
            data: serde_json::json!({
                "title": grab.title,
                "imported": imported_count,
                "failed": failed_count,
                "error": error_msg,
            }),
        })
        .await
    {
        tracing::warn!("create_history_event failed: {e}");
    }

    Ok(ImportGrabResult {
        final_status,
        imported_count,
        failed_count,
        skipped_count,
        warnings,
        error: error_msg,
    })
}

// ---------------------------------------------------------------------------
// Per-file import (non-MP3)
// ---------------------------------------------------------------------------

pub enum ImportFileError {
    Warning(String), // file imported but tag failed
    Failed(String),  // file not imported
}

#[allow(clippy::too_many_arguments)]
pub async fn import_single_file(
    state: &AppState,
    source: &Path,
    target_path: &str,
    root_folder_path: &str,
    root_folder_id: i64,
    media_type: MediaType,
    user_id: i64,
    work_id: i64,
    tag_metadata: Option<&livrarr_tagwrite::TagMetadata>,
    cover: Option<&[u8]>,
    media_mgmt: &livrarr_db::MediaManagementConfig,
    author_name: &str,
    title: &str,
) -> Result<(), ImportFileError> {
    let tmp_path = format!("{target_path}.tmp");
    let tmp_target = PathBuf::from(&tmp_path);
    let target = PathBuf::from(target_path);

    // Copy source → .tmp.
    let src = source.to_path_buf();
    let tmp_clone = tmp_target.clone();
    let copy_result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        if let Some(parent) = tmp_clone.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create_dir_all failed: {e}"))?;
        }
        std::fs::copy(&src, &tmp_clone).map_err(|e| format!("copy failed: {e}"))?;
        Ok(())
    })
    .await
    .map_err(|e| ImportFileError::Failed(format!("spawn error: {e}")))?;

    if let Err(e) = copy_result {
        return Err(ImportFileError::Failed(format!(
            "{}: {e}",
            source.display()
        )));
    }

    // Tag the .tmp if enrichment data available.
    let mut tag_warning = None;
    if let Some(metadata) = tag_metadata {
        tracing::debug!(path = %tmp_path, title = %metadata.title, "writing tags to epub");
        match livrarr_tagwrite::write_tags(
            tmp_path.clone(),
            metadata.clone(),
            cover.map(|c| c.to_vec()),
        )
        .await
        {
            Ok(TagWriteStatus::Written) => {
                tracing::info!(path = %tmp_path, "tags written successfully");
            }
            Ok(_) => {
                tracing::info!(path = %tmp_path, "tag write skipped (unsupported/no data)");
            }
            Err(e) => {
                tracing::warn!(path = %tmp_path, error = %e, "tag write failed, using original file");
                tag_warning = Some(format!("tag write failed for {}: {e}", source.display()));
            }
        }
    }

    // Finalize: rename .tmp → final, or re-copy untagged on tag failure.
    let src2 = source.to_path_buf();
    let tmp_fin = tmp_target.clone();
    let final_t = target.clone();
    let tw = tag_warning.is_some();
    let fin_result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        if tw {
            let _ = std::fs::remove_file(&tmp_fin);
            std::fs::copy(&src2, &final_t).map_err(|e| format!("re-copy failed: {e}"))?;
        } else {
            std::fs::rename(&tmp_fin, &final_t).map_err(|e| format!("rename failed: {e}"))?;
        }
        Ok(())
    })
    .await
    .map_err(|e| ImportFileError::Failed(format!("spawn error: {e}")))?;

    if let Err(e) = fin_result {
        let _ = std::fs::remove_file(&tmp_target);
        return Err(ImportFileError::Failed(format!(
            "{}: {e}",
            source.display()
        )));
    }

    // Measure file size post-tag.
    let file_size = target.metadata().map(|m| m.len() as i64).unwrap_or(0);

    // Create library item.
    let relative = target_path
        .strip_prefix(root_folder_path)
        .unwrap_or(target_path)
        .trim_start_matches('/')
        .to_string();

    state
        .db
        .create_library_item(CreateLibraryItemDbRequest {
            user_id,
            work_id,
            root_folder_id,
            path: relative,
            media_type,
            file_size,
        })
        .await
        .map_err(|e| {
            let _ = std::fs::remove_file(&target);
            ImportFileError::Failed(format!("DB error: {e}"))
        })?;

    // CWA integration (ebooks only, non-fatal).
    if media_type == MediaType::Ebook {
        if let Some(ref cwa_path) = media_mgmt.cwa_ingest_path {
            let ext = source
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("epub")
                .to_string();
            let tp = target_path.to_string();
            let cwa = cwa_path.clone();
            let auth = author_name.to_string();
            let t = title.to_string();
            let cwa_result =
                tokio::task::spawn_blocking(move || cwa_copy(&tp, &cwa, user_id, &auth, &t, &ext))
                    .await
                    .ok()
                    .flatten();
            if let Some(warn) = cwa_result {
                // CWA warning doesn't fail the import.
                return Err(ImportFileError::Warning(warn));
            }
        }
    }

    // Auto-send to email/Kindle on import (ebooks only, non-fatal).
    if media_type == MediaType::Ebook {
        if let Ok(email_cfg) = state.db.get_email_config().await {
            if email_cfg.send_on_import && email_cfg.enabled {
                let ext = source
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if super::email::ACCEPTED_EXTENSIONS.contains(&ext.as_str())
                    && file_size <= super::email::MAX_EMAIL_SIZE
                {
                    let target_str = target_path.to_string();
                    match tokio::fs::read(&target_str).await {
                        Ok(bytes) => {
                            let filename = std::path::Path::new(&target_str)
                                .file_name()
                                .and_then(|f| f.to_str())
                                .unwrap_or("book");
                            if let Err(e) =
                                super::email::send_file(&email_cfg, bytes, filename, &ext).await
                            {
                                tracing::warn!(file = %target_str, "Auto-send email failed: {e}");
                            } else {
                                tracing::info!(file = %target_str, "Auto-sent to email on import");
                            }
                        }
                        Err(e) => {
                            tracing::warn!(file = %target_str, "Auto-send: failed to read file: {e}");
                        }
                    }
                }
            }
        }
    }

    match tag_warning {
        Some(w) => Err(ImportFileError::Warning(w)),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// MP3 batch import (TAG-006 all-or-nothing)
// ---------------------------------------------------------------------------

struct ValidatedFile {
    source: PathBuf,
    target_path: String,
    root_folder_id: i64,
    root_folder_path: String,
    media_type: MediaType,
    is_mp3: bool,
}

#[allow(clippy::too_many_arguments)]
async fn import_mp3_batch(
    state: &AppState,
    files: &[ValidatedFile],
    user_id: i64,
    work_id: i64,
    tag_metadata: Option<&livrarr_tagwrite::TagMetadata>,
    cover: Option<&[u8]>,
    _media_mgmt: &livrarr_db::MediaManagementConfig,
    _author_name: &str,
    _title: &str,
) -> Result<(usize, Vec<String>), String> {
    let mut tmp_paths: Vec<String> = Vec::new();
    let mut target_paths: Vec<String> = Vec::new();
    let mut warnings = Vec::new();

    // Step 1: Copy all source → .tmp files.
    for vf in files {
        let tmp_path = format!("{}.tmp", vf.target_path);
        let src = vf.source.clone();
        let tmp_clone = PathBuf::from(&tmp_path);
        let copy_result = tokio::task::spawn_blocking(move || -> Result<(), String> {
            if let Some(parent) = tmp_clone.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("create_dir_all failed: {e}"))?;
            }
            std::fs::copy(&src, &tmp_clone).map_err(|e| format!("copy failed: {e}"))?;
            Ok(())
        })
        .await
        .map_err(|e| format!("spawn error: {e}"))?;

        if let Err(e) = copy_result {
            // Clean up all .tmps created so far.
            for tmp in &tmp_paths {
                let _ = std::fs::remove_file(tmp);
            }
            return Err(format!(
                "MP3 batch copy failed for {}: {e}",
                vf.source.display()
            ));
        }

        tmp_paths.push(tmp_path);
        target_paths.push(vf.target_path.clone());
    }

    // Step 2: Batch tag all .tmp files.
    let mut tag_failed = false;
    if let Some(metadata) = tag_metadata {
        match livrarr_tagwrite::write_tags_batch(
            tmp_paths.clone(),
            metadata.clone(),
            cover.map(|c| c.to_vec()),
        )
        .await
        {
            Ok(_) => {}
            Err(e) => {
                warnings.push(format!("MP3 batch tag write failed: {e}"));
                tag_failed = true;
            }
        }
    }

    // Step 3: Finalize.
    // Track which files were successfully placed on disk.
    let mut file_placed = vec![false; files.len()];

    if tag_failed {
        // Delete all .tmps, re-copy all sources untagged.
        for (i, vf) in files.iter().enumerate() {
            let _ = std::fs::remove_file(&tmp_paths[i]);
            let src = vf.source.clone();
            let final_p = PathBuf::from(&target_paths[i]);
            match tokio::task::spawn_blocking(move || std::fs::copy(&src, &final_p)).await {
                Ok(Ok(_)) => {
                    file_placed[i] = true;
                }
                Ok(Err(e)) => {
                    warnings.push(format!(
                        "MP3 batch re-copy failed for {}: {e}",
                        vf.source.display()
                    ));
                }
                Err(e) => {
                    warnings.push(format!(
                        "MP3 batch re-copy spawn error for {}: {e}",
                        vf.source.display()
                    ));
                }
            }
        }
    } else {
        // Rename all .tmps → finals.
        for (i, (tmp, target)) in tmp_paths.iter().zip(target_paths.iter()).enumerate() {
            if let Err(e) = std::fs::rename(tmp, target) {
                warnings.push(format!("MP3 batch rename failed for {target}: {e}"));
                let _ = std::fs::remove_file(tmp);
            } else {
                file_placed[i] = true;
            }
        }
    }

    // Step 4: Measure sizes and create library items — only for files successfully placed.
    let mut count = 0;
    for (i, vf) in files.iter().enumerate() {
        if !file_placed[i] {
            continue;
        }
        let target = PathBuf::from(&target_paths[i]);
        let file_size = target.metadata().map(|m| m.len() as i64).unwrap_or(0);
        let relative = target_paths[i]
            .strip_prefix(&vf.root_folder_path)
            .unwrap_or(&target_paths[i])
            .trim_start_matches('/')
            .to_string();

        match state
            .db
            .create_library_item(CreateLibraryItemDbRequest {
                user_id,
                work_id,
                root_folder_id: vf.root_folder_id,
                path: relative,
                media_type: vf.media_type,
                file_size,
            })
            .await
        {
            Ok(_) => count += 1,
            Err(e) => {
                warnings.push(format!("DB error for {}: {e}", target_paths[i]));
                let _ = std::fs::remove_file(&target);
            }
        }
    }

    Ok((count, warnings))
}

// ---------------------------------------------------------------------------
// File enumeration helpers
// ---------------------------------------------------------------------------

struct SourceFile {
    path: PathBuf,
    media_type: MediaType,
}

/// Filter source files to only the best preferred format per media type.
/// For ebooks: if a torrent has epub+mobi+pdf and prefs are [epub], only keep the epub.
/// For audiobooks: if prefs are [m4b] and both m4b+mp3 exist, only keep m4b files.
/// Files with formats not in the preference list at all are kept (no preference = accept).
fn filter_preferred_formats(
    files: Vec<SourceFile>,
    config: &livrarr_db::MediaManagementConfig,
) -> Vec<SourceFile> {
    let ebook_prefs = &config.preferred_ebook_formats;
    let audio_prefs = &config.preferred_audiobook_formats;

    // Find the best (lowest-index) preferred format present for each media type.
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
                    None => true, // no preferred format found, keep all
                },
                MediaType::Audiobook => match best_audio_ext {
                    Some(best) => ext == *best,
                    None => true,
                },
            }
        })
        .collect()
}

/// Enumerate files from a source path (single file or directory).
/// Skips hidden files, symlinks, and unrecognized extensions.
fn enumerate_source_files(source: &Path) -> Result<Vec<SourceFile>, String> {
    let mut files = Vec::new();

    if source.is_file() {
        // Single-file torrent (IMPORT-006a).
        if let Some(media_type) = classify_file(source) {
            files.push(SourceFile {
                path: source.to_path_buf(),
                media_type,
            });
        }
    } else if source.is_dir() {
        // Multi-file torrent — recursive walk.
        walk_dir_recursive(source, &mut files)?;
    } else {
        return Err(format!(
            "source is neither file nor directory: {}",
            source.display()
        ));
    }

    Ok(files)
}

fn walk_dir_recursive(dir: &Path, files: &mut Vec<SourceFile>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry error: {e}"))?;
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        // Skip hidden files/dirs.
        if name.starts_with('.') {
            continue;
        }

        let ft = entry
            .file_type()
            .map_err(|e| format!("file_type error: {e}"))?;

        // Skip symlinks (IMPORT-013).
        if ft.is_symlink() {
            continue;
        }

        if ft.is_dir() {
            walk_dir_recursive(&path, files)?;
        } else if ft.is_file() {
            if let Some(media_type) = classify_file(&path) {
                files.push(SourceFile { path, media_type });
            }
        }
    }

    Ok(())
}

/// Build target path: {root}/{author}/{title}.{ext} (ebook) or {root}/{author}/{title}/{relative} (audiobook).
pub fn build_target_path(
    root: &str,
    _user_id: i64,
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
            format!("{root}/{author_san}/{title_san}.{ext}")
        }
        MediaType::Audiobook => {
            // Preserve subdirectory structure from source.
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
            format!("{root}/{author_san}/{title_san}/{relative_str}")
        }
    }
}

/// Fetch torrent content_path from qBittorrent by hash.
pub async fn fetch_qbit_content_path(
    state: &AppState,
    client: &livrarr_domain::DownloadClient,
    hash: &str,
) -> Result<String, ApiError> {
    let base_url = super::release::qbit_base_url(client);
    let sid = super::release::qbit_login(state, &base_url, client).await?;

    let info_url = format!("{base_url}/api/v2/torrents/info");
    let resp = state
        .http_client
        .get(&info_url)
        .query(&[("hashes", hash)])
        .header("Cookie", format!("SID={sid}"))
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("qBittorrent request failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "qBittorrent returned {}",
            resp.status()
        )));
    }

    let torrents: Vec<serde_json::Value> = resp
        .json()
        .await
        .map_err(|e| ApiError::BadGateway(format!("qBittorrent parse error: {e}")))?;

    let torrent = torrents.first().ok_or(ApiError::NotFound)?;

    torrent
        .get("content_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ApiError::BadGateway("qBittorrent torrent missing content_path".to_string()))
}

/// Fetch SABnzbd storage path from history by nzo_id.
async fn fetch_sabnzbd_storage_path(
    state: &AppState,
    client: &livrarr_domain::DownloadClient,
    nzo_id: &str,
) -> Result<String, ApiError> {
    let base_url = super::download_client::client_base_url(client);
    let api_key = client.api_key.as_deref().unwrap_or("");

    // SABnzbd search param searches by name, not nzo_id. Fetch recent history and match client-side.
    let url = format!("{base_url}/api?mode=history&apikey={api_key}&output=json&limit=200");
    let resp = state
        .http_client
        .get(&url)
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("SABnzbd history request failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "SABnzbd history returned {}",
            resp.status()
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ApiError::BadGateway(format!("SABnzbd history parse error: {e}")))?;

    let entry = body
        .get("history")
        .and_then(|h| h.get("slots"))
        .and_then(|s| s.as_array())
        .and_then(|slots| {
            slots.iter().find(|e| {
                e.get("nzo_id")
                    .and_then(|n| n.as_str())
                    .is_some_and(|n| n == nzo_id)
            })
        })
        .ok_or_else(|| {
            ApiError::BadGateway(format!(
                "SABnzbd history entry not found for nzo_id={nzo_id}"
            ))
        })?;

    entry
        .get("storage")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            ApiError::BadGateway("SABnzbd history entry missing storage path".to_string())
        })
}

/// Apply remote path mapping (longest prefix match on host).
pub async fn apply_remote_path_mapping(
    state: &AppState,
    client_host: &str,
    content_path: &str,
) -> Result<String, ApiError> {
    let mappings = state.db.list_remote_path_mappings().await?;

    // Extract hostname from client_host URL (strip scheme, port, path).
    let client_hostname = client_host
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split(':')
        .next()
        .unwrap_or(client_host);

    // Find longest matching remote_path prefix for this host.
    // Enforce directory boundary: remote_path must match at a `/` boundary
    // to prevent partial matches (e.g., /data/downloads matching /data/downloads_new).
    let best_match = mappings
        .iter()
        .filter(|m| {
            // Match if mapping host equals the hostname, or if hostname ends with the mapping host
            // (e.g., "host.example.com" ends with "example.com").
            let mh = m.host.to_ascii_lowercase();
            let ch = client_hostname.to_ascii_lowercase();
            ch == mh || ch.ends_with(&format!(".{mh}"))
        })
        .filter(|m| {
            if content_path.starts_with(&m.remote_path) {
                // Exact match or next char is '/' (directory boundary).
                content_path.len() == m.remote_path.len()
                    || content_path.as_bytes().get(m.remote_path.len()) == Some(&b'/')
                    || m.remote_path.ends_with('/')
            } else {
                false
            }
        })
        .max_by_key(|m| m.remote_path.len());

    match best_match {
        Some(mapping) => {
            let local = content_path.replacen(&mapping.remote_path, &mapping.local_path, 1);
            // Normalize double slashes from trailing/leading slash mismatches.
            Ok(local.replace("//", "/"))
        }
        None => Ok(content_path.to_string()),
    }
}

/// CWA downstream integration: hardlink first, copy fallback, then touch to trigger inotify.
/// CWA expects flat files in the ingest root, no subdirectories.
/// Returns Some(warning) on failure, None on success.
fn cwa_copy(
    source_path: &str,
    cwa_ingest_path: &str,
    _user_id: i64,
    author: &str,
    title: &str,
    extension: &str,
) -> Option<String> {
    let author_san = sanitize_path_component(author, "Unknown Author");
    let title_san = sanitize_path_component(title, "Unknown Title");
    let dst_dir = Path::new(cwa_ingest_path);
    let dst = dst_dir.join(format!("{author_san} - {title_san}.{extension}"));

    if dst.exists() {
        return Some(format!("CWA destination already exists: {}", dst.display()));
    }

    if let Err(e) = std::fs::create_dir_all(dst_dir) {
        return Some(format!("CWA create_dir_all failed: {e}"));
    }

    // Hardlink first (zero extra disk space on same filesystem).
    let result = match std::fs::hard_link(source_path, &dst) {
        Ok(()) => None,
        Err(e) if e.raw_os_error() == Some(18) => {
            // EXDEV — cross-filesystem, fallback to copy.
            match std::fs::copy(source_path, &dst) {
                Ok(_) => None,
                Err(e) => Some(format!("CWA copy failed: {e}")),
            }
        }
        Err(e) => Some(format!("CWA hardlink failed: {e}")),
    };

    // Touch the file to trigger inotify (hardlinks don't fire IN_CREATE).
    // Open for writing and close — triggers IN_CLOSE_WRITE which CWA watches.
    if result.is_none() {
        let _ = std::fs::OpenOptions::new().append(true).open(&dst);
    }

    result
}

/// Build TagMetadata from a Work record for tag writing.
pub fn build_tag_metadata(work: &livrarr_domain::Work) -> livrarr_tagwrite::TagMetadata {
    livrarr_tagwrite::TagMetadata {
        title: work.title.clone(),
        subtitle: work.subtitle.clone(),
        author: work.author_name.clone(),
        narrator: work.narrator.clone(),
        year: work.year,
        genre: work.genres.clone(),
        description: work.description.clone(),
        publisher: work.publisher.clone(),
        isbn: work.isbn_13.clone(),
        language: work.language.clone(),
        series_name: work.series_name.clone(),
        series_position: work.series_position,
    }
}

/// Read cover image bytes from covers/{work_id}.jpg.
/// Returns None if the file doesn't exist (not an error per TAG-V21-003).
pub async fn read_cover_bytes(state: &AppState, work_id: i64) -> Option<Vec<u8>> {
    let cover_path = state.data_dir.join("covers").join(format!("{work_id}.jpg"));
    tokio::fs::read(&cover_path).await.ok()
}

/// Re-tag existing library items after re-enrichment (TAG-V21-004, TAG-007).
/// Uses .tmp lifecycle: copy item → .tmp, tag .tmp, rename over original.
/// Returns warnings for any failures (non-fatal).
pub async fn retag_library_items(
    state: &AppState,
    work: &livrarr_domain::Work,
    items: &[livrarr_domain::LibraryItem],
) -> Vec<String> {
    let tag_metadata = build_tag_metadata(work);
    let cover_data = read_cover_bytes(state, work.id).await;

    let mut warnings = Vec::new();

    // Separate MP3 items from non-MP3 for batch handling.
    let mut mp3_items = Vec::new();
    let mut other_items = Vec::new();
    for item in items {
        let ext = Path::new(&item.path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        if ext == "mp3" {
            mp3_items.push(item);
        } else {
            other_items.push(item);
        }
    }

    // Non-MP3: per-file .tmp lifecycle.
    for item in &other_items {
        let root = match state.db.get_root_folder(item.root_folder_id).await {
            Ok(rf) => rf,
            Err(e) => {
                warnings.push(format!(
                    "root folder lookup failed for item {}: {e}",
                    item.id
                ));
                continue;
            }
        };
        let abs_path = format!("{}/{}", root.path, item.path);
        if !Path::new(&abs_path).exists() {
            warnings.push(format!("file not found, skipping: {abs_path}"));
            continue;
        }

        let tmp_path = format!("{abs_path}.tmp");

        // Copy original → .tmp.
        if let Err(e) = tokio::task::spawn_blocking({
            let src = abs_path.clone();
            let dst = tmp_path.clone();
            move || std::fs::copy(&src, &dst)
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r.map_err(|e| e.to_string()))
        {
            warnings.push(format!("copy to .tmp failed for {abs_path}: {e}"));
            continue;
        }

        // Tag the .tmp.
        match livrarr_tagwrite::write_tags(
            tmp_path.clone(),
            tag_metadata.clone(),
            cover_data.clone(),
        )
        .await
        {
            Ok(TagWriteStatus::Written) => {
                // Rename .tmp over original.
                if let Err(e) = std::fs::rename(&tmp_path, &abs_path) {
                    warnings.push(format!("rename failed for {abs_path}: {e}"));
                    let _ = std::fs::remove_file(&tmp_path);
                } else {
                    // Update file_size in DB after tag write changed it (TAG-V21-004).
                    let new_size = Path::new(&abs_path)
                        .metadata()
                        .map(|m| m.len() as i64)
                        .unwrap_or(0);
                    if let Err(e) = state
                        .db
                        .update_library_item_size(item.user_id, item.id, new_size)
                        .await
                    {
                        tracing::warn!("update_library_item_size failed: {e}");
                    }
                }
            }
            Ok(_) => {
                // Unsupported or NoData — remove .tmp, leave original.
                let _ = std::fs::remove_file(&tmp_path);
            }
            Err(e) => {
                warnings.push(format!("tag write failed for {abs_path}: {e}"));
                let _ = std::fs::remove_file(&tmp_path);
            }
        }
    }

    // MP3 batch: .tmp lifecycle for all MP3 items.
    if !mp3_items.is_empty() {
        let root = match state.db.get_root_folder(mp3_items[0].root_folder_id).await {
            Ok(rf) => rf,
            Err(e) => {
                warnings.push(format!("root folder lookup failed: {e}"));
                return warnings;
            }
        };

        let mut abs_paths = Vec::new();
        let mut tmp_paths = Vec::new();
        for item in &mp3_items {
            let abs = format!("{}/{}", root.path, item.path);
            let tmp = format!("{abs}.tmp");
            abs_paths.push(abs);
            tmp_paths.push(tmp);
        }

        // Copy all originals → .tmp files.
        let mut copy_ok = true;
        for (abs, tmp) in abs_paths.iter().zip(tmp_paths.iter()) {
            let src = abs.clone();
            let dst = tmp.clone();
            let result = tokio::task::spawn_blocking(move || std::fs::copy(&src, &dst)).await;
            if result.is_err() || result.unwrap().is_err() {
                warnings.push(format!("MP3 batch: copy to .tmp failed for {abs}"));
                copy_ok = false;
                break;
            }
        }

        if !copy_ok {
            // Clean up any .tmp files created so far.
            for tmp in &tmp_paths {
                let _ = std::fs::remove_file(tmp);
            }
        } else {
            // Tag all .tmp files as a batch.
            match livrarr_tagwrite::write_tags_batch(
                tmp_paths.clone(),
                tag_metadata.clone(),
                cover_data.clone(),
            )
            .await
            {
                Ok(_) => {
                    // Rename all .tmps over originals and update file_sizes.
                    for (i, (tmp, abs)) in tmp_paths.iter().zip(abs_paths.iter()).enumerate() {
                        if let Err(e) = std::fs::rename(tmp, abs) {
                            warnings.push(format!("MP3 batch rename failed for {abs}: {e}"));
                            let _ = std::fs::remove_file(tmp);
                        } else {
                            // Update file_size in DB (TAG-V21-004).
                            let new_size = Path::new(abs)
                                .metadata()
                                .map(|m| m.len() as i64)
                                .unwrap_or(0);
                            if let Err(e) = state
                                .db
                                .update_library_item_size(
                                    mp3_items[i].user_id,
                                    mp3_items[i].id,
                                    new_size,
                                )
                                .await
                            {
                                tracing::warn!("update_library_item_size failed: {e}");
                            }
                        }
                    }
                }
                Err(e) => {
                    warnings.push(format!("MP3 batch tag write failed: {e}"));
                    for tmp in &tmp_paths {
                        let _ = std::fs::remove_file(tmp);
                    }
                }
            }
        }
    }

    warnings
}
