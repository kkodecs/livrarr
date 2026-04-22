use std::path::{Path, PathBuf};

use crate::infra::email;
use crate::services::settings_service::SettingsService;
use crate::state::AppState;
use crate::{ApiError, MediaType};
use livrarr_domain::sanitize_path_component;
use livrarr_domain::services::{ImportGrabResult, ImportIoService, ImportWorkflow};
use livrarr_tagwrite::TagWriteStatus;

/// Run the import pipeline for a grab. Called by the retry handler
/// and by the download poller via spawn_import.
///
/// Precondition: grab status already atomically set to `importing` by caller.
pub async fn import_grab(
    state: &AppState,
    user_id: i64,
    grab_id: i64,
) -> Result<ImportGrabResult, ApiError> {
    // Pre-service: ensure content_path is populated.
    // The download poller persists content_path when confirming a download.
    // For manual retries, content_path may be missing — resolve from the
    // download client.
    let grab = state.import_io_service.get_grab(user_id, grab_id).await?;
    if grab.content_path.is_none() {
        if let Some(ref download_id) = grab.download_id {
            let client = state
                .import_io_service
                .get_download_client(grab.download_client_id)
                .await?;
            let content_path = if client.client_type() == "sabnzbd" {
                fetch_sabnzbd_storage_path(state, &client, download_id).await?
            } else {
                fetch_qbit_content_path(state, &client, download_id).await?
            };
            state
                .import_io_service
                .set_grab_content_path(user_id, grab_id, &content_path)
                .await?;
        }
    }

    // Service handles: source resolution, enumeration, format filtering,
    // file copy, library item creation, status update, history event.
    let result = state.import_workflow.import_grab(user_id, grab_id).await?;

    let mut warnings = result.warnings;

    // Post-service I/O: tag imported files + CWA copy + email.
    if !result.imported_files.is_empty() {
        let work = state
            .import_io_service
            .get_work(user_id, grab.work_id)
            .await?;

        // Tag writing — retag the just-imported files if enrichment data available.
        if work.enrichment_status != livrarr_domain::EnrichmentStatus::Pending {
            let items = state
                .import_io_service
                .list_library_items_by_work(user_id, work.id)
                .await
                .unwrap_or_default();
            let imported_ids: std::collections::HashSet<i64> = result
                .imported_files
                .iter()
                .map(|f| f.library_item_id)
                .collect();
            let matching: Vec<_> = items
                .iter()
                .filter(|i| imported_ids.contains(&i.id))
                .cloned()
                .collect();
            if !matching.is_empty() {
                use livrarr_domain::services::TagService;
                let tag_warnings = state
                    .tag_service
                    .retag_library_items(&work, &matching)
                    .await;
                warnings.extend(tag_warnings);
            }
        }

        // CWA copy + email — fire-and-forget for ebooks.
        let media_mgmt = state
            .settings_service
            .get_media_management_config()
            .await
            .ok();
        let root_folders = state
            .import_io_service
            .list_root_folders()
            .await
            .unwrap_or_default();
        for imp in &result.imported_files {
            if imp.media_type != MediaType::Ebook {
                continue;
            }
            let rf = match root_folders
                .iter()
                .find(|rf| rf.media_type == MediaType::Ebook)
            {
                Some(rf) => rf,
                None => continue,
            };
            let abs_path = format!("{}/{}", rf.path, imp.target_relative_path);

            // CWA
            if let Some(ref mgmt) = media_mgmt {
                if let Some(ref cwa_path) = mgmt.cwa_ingest_path {
                    let ext = Path::new(&imp.target_relative_path)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("epub")
                        .to_string();
                    let work = state
                        .import_io_service
                        .get_work(user_id, grab.work_id)
                        .await
                        .ok();
                    if let Some(work) = work {
                        let tp = abs_path.clone();
                        let cwa = cwa_path.clone();
                        let auth = work.author_name.clone();
                        let t = work.title.clone();
                        let cwa_result = tokio::task::spawn_blocking(move || {
                            cwa_copy(&tp, &cwa, user_id, &auth, &t, &ext)
                        })
                        .await
                        .ok()
                        .flatten();
                        if let Some(warn) = cwa_result {
                            warnings.push(warn);
                        }
                    }
                }
            }

            // Auto-send to email/Kindle
            if let Ok(email_cfg) = state.settings_service.get_email_config().await {
                if email_cfg.send_on_import && email_cfg.enabled {
                    let ext = Path::new(&imp.target_relative_path)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    if email::ACCEPTED_EXTENSIONS.contains(&ext.as_str())
                        && (imp.file_size as i64) <= email::MAX_EMAIL_SIZE
                    {
                        match tokio::fs::read(&abs_path).await {
                            Ok(bytes) => {
                                let filename = Path::new(&abs_path)
                                    .file_name()
                                    .and_then(|f| f.to_str())
                                    .unwrap_or("book");
                                if let Err(e) =
                                    email::send_file(&email_cfg, bytes, filename, &ext).await
                                {
                                    tracing::warn!(file = %abs_path, "Auto-send email failed: {e}");
                                } else {
                                    tracing::info!(file = %abs_path, "Auto-sent to email on import");
                                }
                            }
                            Err(e) => {
                                tracing::warn!(file = %abs_path, "Auto-send: failed to read file: {e}");
                            }
                        }
                    }
                }
            }
        }
    }

    let error_msg = if result.failed_files.is_empty() {
        None
    } else {
        Some(
            result
                .failed_files
                .iter()
                .map(|f| f.error.as_str())
                .collect::<Vec<_>>()
                .join("; "),
        )
    };

    Ok(ImportGrabResult {
        final_status: result.final_status,
        imported_count: result.imported_files.len(),
        failed_count: result.failed_files.len(),
        skipped_count: result.skipped_files.len(),
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
        let _ = std::fs::remove_file(&tmp_target); // clean partial .tmp
        return Err(ImportFileError::Failed(format!(
            "{}: {e}",
            source.display()
        )));
    }

    // Tag the .tmp if enrichment data available.
    let mut tag_warning = None;
    if let Some(metadata) = tag_metadata {
        tracing::debug!(path = %tmp_path, "writing tags");
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
            // Tag failed — re-copy source to a temp file, fsync, then rename atomically.
            let _ = std::fs::remove_file(&tmp_fin);
            let fallback = tmp_fin.with_extension("fallback.tmp");
            std::fs::copy(&src2, &fallback).map_err(|e| {
                let _ = std::fs::remove_file(&fallback);
                format!("fallback copy failed: {e}")
            })?;
            if let Ok(f) = std::fs::File::open(&fallback) {
                let _ = f.sync_all();
            }
            std::fs::rename(&fallback, &final_t).map_err(|e| {
                let _ = std::fs::remove_file(&fallback);
                format!("fallback rename failed: {e}")
            })?;
        } else {
            // Fsync the tagged .tmp to disk before atomic rename so partial writes
            // (e.g., tag crate buffered data) can't be lost on power failure.
            if let Ok(f) = std::fs::File::open(&tmp_fin) {
                let _ = f.sync_all();
            }
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
        .import_io_service
        .create_library_item(livrarr_domain::services::CreateLibraryItemRequest {
            user_id,
            work_id,
            root_folder_id,
            path: relative,
            media_type,
            file_size,
            import_id: None,
        })
        .await
        .map_err(|e| {
            // Do NOT delete the file — leave on disk for retry recovery.
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
        if let Ok(email_cfg) = state.settings_service.get_email_config().await {
            if email_cfg.send_on_import && email_cfg.enabled {
                let ext = source
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if email::ACCEPTED_EXTENSIONS.contains(&ext.as_str())
                    && file_size <= email::MAX_EMAIL_SIZE
                {
                    let target_str = target_path.to_string();
                    match tokio::fs::read(&target_str).await {
                        Ok(bytes) => {
                            let filename = std::path::Path::new(&target_str)
                                .file_name()
                                .and_then(|f| f.to_str())
                                .unwrap_or("book");
                            if let Err(e) =
                                email::send_file(&email_cfg, bytes, filename, &ext).await
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

/// Build target path: `{root}/{user_id}/{author}/{title}.{ext}` (ebook)
/// or `{root}/{user_id}/{author}/{title}/{relative}` (audiobook).
pub fn build_target_path(
    root: &str,
    user_id: i64,
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
            format!("{root}/{user_id}/{author_san}/{title_san}/{relative_str}")
        }
    }
}

/// Fetch torrent content_path from qBittorrent by hash.
pub async fn fetch_qbit_content_path(
    state: &AppState,
    client: &livrarr_domain::DownloadClient,
    hash: &str,
) -> Result<String, ApiError> {
    let base_url = crate::infra::release_helpers::qbit_base_url(client);
    let sid = crate::infra::release_helpers::qbit_login(state, &base_url, client).await?;

    let info_url = format!("{base_url}/api/v2/torrents/info");
    // Admin-configured endpoint — use SSRF-safe client for redirect protection.
    let resp = state
        .http_client_safe
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
    let base_url = livrarr_handlers::download_client::client_base_url(client);
    let api_key = client.api_key.as_deref().unwrap_or("");

    // SABnzbd search param searches by name, not nzo_id. Fetch recent history and match client-side.
    let url = format!("{base_url}/api?mode=history&apikey={api_key}&output=json&limit=200");
    // Admin-configured endpoint — use SSRF-safe client so a redirect to an
    // internal address is blocked.
    let resp = state.http_client_safe.get(&url).send().await.map_err(|e| {
        ApiError::BadGateway(format!(
            "SABnzbd history request failed: {}",
            e.without_url()
        ))
    })?;

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

#[derive(Clone)]
pub struct PathMappingResult {
    pub local_path: String,
    pub configured_remote_path: Option<String>,
    pub configured_local_path: Option<String>,
}

pub async fn apply_remote_path_mapping(
    state: &AppState,
    client_host: &str,
    content_path: &str,
) -> Result<PathMappingResult, ApiError> {
    // Normalize Windows backslashes — download clients on Windows report paths
    // like C:\Downloads\book.epub that need to match Linux forward-slash mappings.
    let content_path = &content_path.replace('\\', "/");

    let mappings = state.import_io_service.list_remote_path_mappings().await?;

    // Extract hostname from client_host URL (strip scheme, port, path).
    let client_hostname = client_host
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split(':')
        .next()
        .unwrap_or(client_host);

    // Filter to mappings that match this host.
    let host_matches: Vec<_> = mappings
        .iter()
        .filter(|m| {
            let mh = m.host.to_ascii_lowercase();
            let ch = client_hostname.to_ascii_lowercase();
            ch == mh || ch.ends_with(&format!(".{mh}"))
        })
        .collect();

    // Find longest matching remote_path prefix for this host.
    // Enforce directory boundary: remote_path must match at a `/` boundary
    // to prevent partial matches (e.g., /data/downloads matching /data/downloads_new).
    let best_match = host_matches
        .iter()
        .filter(|m| {
            let rp = m.remote_path.replace('\\', "/");
            if content_path.starts_with(&rp) {
                // Exact match or next char is '/' (directory boundary).
                content_path.len() == rp.len()
                    || content_path.as_bytes().get(rp.len()) == Some(&b'/')
                    || rp.ends_with('/')
            } else {
                false
            }
        })
        .max_by_key(|m| m.remote_path.len());

    match best_match {
        Some(mapping) => {
            let rp = mapping.remote_path.replace('\\', "/");
            let local = content_path.replacen(&rp, &mapping.local_path, 1);
            // Normalize double slashes from trailing/leading slash mismatches.
            Ok(PathMappingResult {
                local_path: local.replace("//", "/"),
                configured_remote_path: Some(mapping.remote_path.clone()),
                configured_local_path: Some(mapping.local_path.clone()),
            })
        }
        None => {
            // No path-prefix match, but include host-matched mapping config
            // for diagnostics (so the user/AI can see what's configured).
            let (cfg_remote, cfg_local) = host_matches
                .first()
                .map(|m| (Some(m.remote_path.clone()), Some(m.local_path.clone())))
                .unwrap_or((None, None));
            Ok(PathMappingResult {
                local_path: content_path.to_string(),
                configured_remote_path: cfg_remote,
                configured_local_path: cfg_local,
            })
        }
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

/// Read cover image bytes for tag embedding.
/// Checks new tenant-aware path first, falls back to old flat layout.
/// Returns None if the file doesn't exist (not an error per TAG-V21-003).
pub async fn read_cover_bytes(state: &AppState, user_id: i64, work_id: i64) -> Option<Vec<u8>> {
    // Try new tenant-aware path: covers/{user_id}/{work_id}.jpg
    let new_path = state
        .data_dir
        .join("covers")
        .join(user_id.to_string())
        .join(format!("{work_id}.jpg"));
    if let Ok(bytes) = tokio::fs::read(&new_path).await {
        return Some(bytes);
    }
    // Fallback to old flat layout: covers/{work_id}.jpg
    let old_path = state.data_dir.join("covers").join(format!("{work_id}.jpg"));
    tokio::fs::read(&old_path).await.ok()
}
