use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::middleware::RequireAdmin;
use crate::state::AppState;
use crate::{AddWorkRequest, ApiError, WorkSearchResult};
use livrarr_db::{ConfigDb, LibraryItemDb, RootFolderDb, WorkDb};
use livrarr_domain::{classify_file, normalize_for_matching, MediaType};

// ---------------------------------------------------------------------------
// Scan
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ScanRequest {
    pub path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResponse {
    pub files: Vec<ScannedFile>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedFile {
    pub path: String,
    pub filename: String,
    pub media_type: MediaType,
    pub size: u64,
    pub parsed: Option<ParsedFile>,
    #[serde(rename = "match")]
    pub ol_match: Option<OlMatch>,
    pub existing_work_id: Option<i64>,
    /// True only if a library item of the same media type already exists for this work.
    pub has_existing_media_type: bool,
    pub routable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// For multi-file audiobooks: all file paths in the group.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grouped_paths: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedFile {
    pub author: String,
    pub title: String,
    pub series: Option<String>,
    pub series_position: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OlMatch {
    pub ol_key: String,
    pub title: String,
    pub author: String,
    pub cover_url: Option<String>,
    pub existing_work_id: Option<i64>,
}

const MAX_MEDIA_FILES: usize = 50;
const MAX_ENTRIES_TRAVERSED: usize = 10_000;
const LLM_BATCH_SIZE: usize = 50;

/// POST /api/v1/manualimport/scan
pub async fn scan(
    State(state): State<AppState>,
    RequireAdmin(auth): RequireAdmin,
    Json(req): Json<ScanRequest>,
) -> Result<Json<ScanResponse>, ApiError> {
    let path = PathBuf::from(&req.path);
    if !path.exists() || !path.is_dir() {
        return Err(ApiError::BadRequest(
            "The file system path specified was not found.".into(),
        ));
    }
    // Check readability.
    if std::fs::read_dir(&path).is_err() {
        return Err(ApiError::BadRequest(
            "The file system path specified was not found.".into(),
        ));
    }

    let mut warnings = Vec::new();

    // Enumerate files with limits.
    let (source_files, enumeration_warning) = tokio::task::spawn_blocking({
        let path = path.clone();
        move || enumerate_with_limits(&path)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;

    if let Some(w) = enumeration_warning {
        warnings.push(w);
    }

    if source_files.is_empty() {
        return Ok(Json(ScanResponse {
            files: vec![],
            warnings,
        }));
    }

    // ── Group multi-file audiobooks by parent directory ──
    // When 2+ audio files share the same parent dir, collapse into a single entry
    // using the folder path for identification instead of individual filenames.
    let scan_root = &path;
    let mut dir_audio_files: std::collections::HashMap<PathBuf, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, sf) in source_files.iter().enumerate() {
        if sf.media_type == MediaType::Audiobook {
            if let Some(parent) = sf.path.parent() {
                dir_audio_files
                    .entry(parent.to_path_buf())
                    .or_default()
                    .push(i);
            }
        }
    }

    // Build scan items: either individual files or folder groups.
    struct ScanItem {
        /// Display name for LLM parsing.
        display_name: String,
        /// Primary path (directory for groups, file for singles).
        primary_path: PathBuf,
        media_type: MediaType,
        /// Individual file paths (None = single file, Some = group).
        grouped_paths: Option<Vec<PathBuf>>,
    }

    let grouped_dirs: std::collections::HashSet<PathBuf> = dir_audio_files
        .iter()
        .filter(|(_, indices)| indices.len() >= 2)
        .map(|(dir, _)| dir.clone())
        .collect();

    let mut scan_items: Vec<ScanItem> = Vec::new();

    // Add grouped audiobook directories first.
    for (dir, indices) in &dir_audio_files {
        if indices.len() < 2 {
            continue;
        }
        // Use folder path relative to scan root for display.
        let rel = dir
            .strip_prefix(scan_root)
            .unwrap_or(dir)
            .to_string_lossy()
            .to_string();
        let display = if rel.is_empty() {
            dir.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        } else {
            rel
        };
        let file_paths: Vec<PathBuf> = indices
            .iter()
            .map(|&i| source_files[i].path.clone())
            .collect();
        scan_items.push(ScanItem {
            display_name: display,
            primary_path: dir.clone(),
            media_type: MediaType::Audiobook,
            grouped_paths: Some(file_paths),
        });
    }

    // Add individual files (skip those already in a group).
    for sf in source_files.iter() {
        if sf.media_type == MediaType::Audiobook {
            if let Some(parent) = sf.path.parent() {
                if grouped_dirs.contains(parent) {
                    continue; // Part of a group.
                }
            }
        }
        let filename = sf
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        scan_items.push(ScanItem {
            display_name: filename,
            primary_path: sf.path.clone(),
            media_type: sf.media_type,
            grouped_paths: None,
        });
    }

    // LLM batch parse display names.
    let display_names: Vec<String> = scan_items
        .iter()
        .map(|si| si.display_name.clone())
        .collect();
    let (parsed_files, sort_order) = llm_parse_filenames(&state, &display_names).await;

    // Get user's existing works for duplicate detection.
    let user_id = auth.user.id;
    let existing_works = state.db.list_works(user_id).await?;
    let root_folders = state.db.list_root_folders().await?;

    // Search OL for each item (throttled), in LLM sort order.
    let mut scanned_files = Vec::new();

    for &i in &sort_order {
        let si = &scan_items[i];
        let filename = display_names[i].clone();
        let parsed = parsed_files.get(i).cloned().flatten();

        // Compute total size.
        let size = if let Some(ref paths) = si.grouped_paths {
            let mut total = 0u64;
            for p in paths {
                total += tokio::fs::metadata(p).await.map(|m| m.len()).unwrap_or(0);
            }
            total
        } else {
            tokio::fs::metadata(&si.primary_path)
                .await
                .map(|m| m.len())
                .unwrap_or(0)
        };

        let routable = root_folders.iter().any(|rf| rf.media_type == si.media_type);

        let (ol_match, existing_work_id) = if let Some(ref p) = parsed {
            let search_term = format!("{} {}", p.title, p.author);
            let ol = search_ol_single(&state, &search_term).await;

            let (matched, dup_id) = match ol {
                Some(result) => {
                    let dup = existing_works
                        .iter()
                        .find(|w| {
                            w.ol_key.as_deref() == Some(&result.ol_key)
                                || (normalize_for_matching(&w.title)
                                    == normalize_for_matching(&result.title)
                                    && normalize_for_matching(&w.author_name)
                                        == normalize_for_matching(&result.author_name))
                        })
                        .map(|w| w.id);

                    (
                        Some(OlMatch {
                            ol_key: result.ol_key,
                            title: result.title,
                            author: result.author_name,
                            cover_url: result.cover_url,
                            existing_work_id: dup,
                        }),
                        dup,
                    )
                }
                None => {
                    let dup = existing_works
                        .iter()
                        .find(|w| {
                            normalize_for_matching(&w.title) == normalize_for_matching(&p.title)
                                && normalize_for_matching(&w.author_name)
                                    == normalize_for_matching(&p.author)
                        })
                        .map(|w| w.id);
                    (None, dup)
                }
            };

            (matched, dup_id)
        } else {
            (None, None)
        };

        // OL throttle.
        if parsed.is_some() && scanned_files.len() + 1 < sort_order.len() {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        // Check readability (for groups, check first file).
        let check_path = si
            .grouped_paths
            .as_ref()
            .and_then(|p| p.first())
            .unwrap_or(&si.primary_path);
        let file_error = if tokio::fs::File::open(check_path).await.is_err() {
            Some("file not readable".to_string())
        } else {
            None
        };

        let has_existing_media_type = if let Some(wid) = existing_work_id {
            let items = state.db.list_library_items_by_work(user_id, wid).await?;
            items.iter().any(|li| li.media_type == si.media_type)
        } else {
            false
        };

        let grouped_path_strings = si
            .grouped_paths
            .as_ref()
            .map(|paths| paths.iter().map(|p| p.display().to_string()).collect());

        // For groups, show folder name with file count.
        let display_filename = if let Some(ref paths) = si.grouped_paths {
            format!("{}/ ({} files)", filename, paths.len())
        } else {
            filename
        };

        scanned_files.push(ScannedFile {
            path: si.primary_path.display().to_string(),
            filename: display_filename,
            media_type: si.media_type,
            size,
            parsed,
            ol_match,
            existing_work_id,
            has_existing_media_type,
            routable,
            error: file_error,
            grouped_paths: grouped_path_strings,
        });
    }

    Ok(Json(ScanResponse {
        files: scanned_files,
        warnings,
    }))
}

// ---------------------------------------------------------------------------
// Search (inline correction)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub author: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub results: Vec<OlMatch>,
}

/// POST /api/v1/manualimport/search
pub async fn search(
    State(state): State<AppState>,
    RequireAdmin(auth): RequireAdmin,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, ApiError> {
    let term = if let Some(ref author) = req.author {
        format!("{} {}", req.query, author)
    } else {
        req.query.clone()
    };

    tracing::info!(query = %req.query, author = ?req.author, term = %term, "manual import search");

    let results = search_ol_batch(&state, &term).await?;

    tracing::info!(ol_results = results.len(), "OL search returned");

    // Check duplicates against user's library.
    let user_id = auth.user.id;
    let existing_works = state.db.list_works(user_id).await?;

    let results: Vec<OlMatch> = results
        .into_iter()
        .map(|r| {
            let dup = existing_works.iter().find(|w| {
                w.ol_key.as_deref() == Some(&r.ol_key)
                    || (normalize_for_matching(&w.title) == normalize_for_matching(&r.title)
                        && normalize_for_matching(&w.author_name)
                            == normalize_for_matching(&r.author_name))
            });
            OlMatch {
                ol_key: r.ol_key,
                title: r.title,
                author: r.author_name,
                cover_url: r.cover_url,
                existing_work_id: dup.map(|w| w.id),
            }
        })
        .collect();

    tracing::info!(
        final_results = results.len(),
        "manual import search complete"
    );

    Ok(Json(SearchResponse { results }))
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub items: Vec<ImportItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportItem {
    pub path: String,
    pub ol_key: String,
    pub title: String,
    pub author: String,
    pub delete_existing: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResponse {
    pub results: Vec<ImportResult>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub path: String,
    pub status: ImportStatus,
    pub work_id: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportStatus {
    Imported,
    Skipped,
    Failed,
}

/// POST /api/v1/manualimport/import
pub async fn import(
    State(state): State<AppState>,
    RequireAdmin(auth): RequireAdmin,
    Json(req): Json<ImportRequest>,
) -> Result<Json<ImportResponse>, ApiError> {
    let user_id = auth.user.id;

    // Validate all items have ol_key.
    for item in &req.items {
        if item.ol_key.is_empty() {
            return Err(ApiError::BadRequest(format!(
                "missing olKey for file: {}",
                item.path
            )));
        }
    }

    // Fetch works and root folders once (avoid N+1 queries).
    let existing_works = state.db.list_works(user_id).await?;
    let root_folders = state.db.list_root_folders().await?;

    // Snapshot existing library items for deletion safety.
    // Uses same matching logic as scan: olKey first, then normalized title+author fallback.
    let existing_items_snapshot: Vec<_> = {
        let mut all = Vec::new();
        for item in &req.items {
            if item.delete_existing {
                let work =
                    find_existing_work(&existing_works, &item.ol_key, &item.title, &item.author);
                if let Some(work) = work {
                    // Only delete items of the same media type as the file being imported.
                    let source_media_type = classify_file(std::path::Path::new(&item.path));
                    let items = state
                        .db
                        .list_library_items_by_work(user_id, work.id)
                        .await?;
                    for li in items {
                        // Skip items of a different media type. If source type is unknown, skip all
                        // (don't delete blindly).
                        match source_media_type {
                            Some(mt) if mt == li.media_type => {}
                            _ => continue,
                        }
                        all.push((
                            li.id,
                            work.id,
                            li.path.clone(),
                            li.media_type,
                            li.root_folder_id,
                        ));
                    }
                }
            }
        }
        all
    };
    let media_mgmt = state.db.get_media_management_config().await?;
    let mut results = Vec::new();

    for item in &req.items {
        let result = import_single_item(
            &state,
            user_id,
            item,
            &existing_works,
            &root_folders,
            &media_mgmt,
            &existing_items_snapshot,
        )
        .await;
        results.push(result);
    }

    Ok(Json(ImportResponse { results }))
}

/// DeletionTarget: (library_item_id, work_id, relative_path, media_type, root_folder_id)
type DeletionTarget = (i64, i64, String, MediaType, i64);

async fn import_single_item(
    state: &AppState,
    user_id: i64,
    item: &ImportItem,
    existing_works: &[livrarr_domain::Work],
    root_folders: &[livrarr_domain::RootFolder],
    media_mgmt: &livrarr_db::MediaManagementConfig,
    deletion_snapshot: &[DeletionTarget],
) -> ImportResult {
    let source = PathBuf::from(&item.path);

    // Classify the file.
    let media_type = match classify_file(&source) {
        Some(mt) => mt,
        None => {
            return ImportResult {
                path: item.path.clone(),
                status: ImportStatus::Failed,
                work_id: None,
                error: Some("unrecognized media type".into()),
            };
        }
    };

    // Check routability.
    let root_folder = match root_folders.iter().find(|rf| rf.media_type == media_type) {
        Some(rf) => rf,
        None => {
            return ImportResult {
                path: item.path.clone(),
                status: ImportStatus::Failed,
                work_id: None,
                error: Some(format!("no root folder configured for {:?}", media_type)),
            };
        }
    };

    // Find or create the work (reuses the same pattern as work::add).
    let work_id = match find_or_create_work(state, user_id, item, existing_works).await {
        Ok(id) => id,
        Err(e) => {
            warn!("manual import: work creation failed for {}: {e}", item.path);
            return ImportResult {
                path: item.path.clone(),
                status: ImportStatus::Failed,
                work_id: None,
                error: Some(format!("work creation failed: {e}")),
            };
        }
    };

    // Handle delete existing (same media type only, from snapshot).
    if item.delete_existing {
        for (li_id, snap_work_id, li_path, li_media_type, li_rf_id) in deletion_snapshot {
            if *snap_work_id == work_id && *li_media_type == media_type {
                // Use the root folder from the existing item, not the new import's root folder.
                let rf_path = root_folders
                    .iter()
                    .find(|rf| rf.id == *li_rf_id)
                    .map(|rf| rf.path.as_str())
                    .unwrap_or(&root_folder.path);
                let full_path = PathBuf::from(rf_path).join(li_path);
                match std::fs::remove_file(&full_path) {
                    Ok(()) => {
                        if let Err(e) = state.db.delete_library_item(user_id, *li_id).await {
                            tracing::warn!("delete_library_item failed: {e}");
                        }
                        info!(
                            "manual import: deleted existing {} for work {}",
                            full_path.display(),
                            work_id
                        );
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        // Already deleted (by another file in the batch) — success.
                        if let Err(e) = state.db.delete_library_item(user_id, *li_id).await {
                            tracing::warn!("delete_library_item failed: {e}");
                        }
                    }
                    Err(e) => {
                        warn!(
                            "manual import: failed to delete {} for work {}: {e}",
                            full_path.display(),
                            work_id
                        );
                        return ImportResult {
                            path: item.path.clone(),
                            status: ImportStatus::Failed,
                            work_id: Some(work_id),
                            error: Some(format!("failed to delete existing: {e}")),
                        };
                    }
                }
            }
        }
    }

    // Get work for tag metadata.
    let work = match state.db.get_work(user_id, work_id).await {
        Ok(w) => w,
        Err(e) => {
            return ImportResult {
                path: item.path.clone(),
                status: ImportStatus::Failed,
                work_id: Some(work_id),
                error: Some(format!("failed to load work: {e}")),
            };
        }
    };

    let has_enrichment = work.enrichment_status != livrarr_domain::EnrichmentStatus::Pending;
    let tag_metadata = if has_enrichment {
        Some(super::import::build_tag_metadata(&work))
    } else {
        None
    };
    let cover_data = if has_enrichment {
        super::import::read_cover_bytes(state, work_id).await
    } else {
        None
    };

    // Build target path.
    let target_path = super::import::build_target_path(
        &root_folder.path,
        user_id,
        &work.author_name,
        &work.title,
        media_type,
        &source,
        &source, // source_root = source for single-file
    );

    // Check if target already exists.
    let target = PathBuf::from(&target_path);
    if target.exists() {
        let relative = target_path
            .strip_prefix(root_folder.path.trim_end_matches('/'))
            .unwrap_or(&target_path)
            .trim_start_matches('/');
        // Already tracked by this user = skipped.
        let my_items = match state.db.list_library_items_by_work(user_id, work_id).await {
            Ok(items) => items,
            Err(e) => {
                return ImportResult {
                    path: item.path.clone(),
                    status: ImportStatus::Failed,
                    work_id: Some(work_id),
                    error: Some(format!("failed to query library items: {e}")),
                };
            }
        };
        if my_items.iter().any(|li| li.path == relative) {
            return ImportResult {
                path: item.path.clone(),
                status: ImportStatus::Skipped,
                work_id: Some(work_id),
                error: None,
            };
        }
        // File exists but not tracked by this user — shared file.
        // Create a library item pointing to the existing file without copying.
        let file_size = std::fs::metadata(&target)
            .map(|m| m.len() as i64)
            .unwrap_or(0);
        match state
            .db
            .create_library_item(livrarr_db::CreateLibraryItemDbRequest {
                user_id,
                work_id,
                root_folder_id: root_folder.id,
                path: relative.to_string(),
                media_type,
                file_size,
            })
            .await
        {
            Ok(_) => {
                info!(
                    "manual import: shared file {} for work {} ({})",
                    item.path, work_id, work.title
                );
                return ImportResult {
                    path: item.path.clone(),
                    status: ImportStatus::Imported,
                    work_id: Some(work_id),
                    error: None,
                };
            }
            Err(e) => {
                return ImportResult {
                    path: item.path.clone(),
                    status: ImportStatus::Failed,
                    work_id: Some(work_id),
                    error: Some(format!("failed to create library item: {e}")),
                };
            }
        }
    }

    // Import the file.
    match super::import::import_single_file(
        state,
        &source,
        &target_path,
        &root_folder.path,
        root_folder.id,
        media_type,
        user_id,
        work_id,
        tag_metadata.as_ref(),
        cover_data.as_deref(),
        media_mgmt,
        &work.author_name,
        &work.title,
    )
    .await
    {
        Ok(()) => {
            info!(
                "manual import: imported {} for work {} ({})",
                item.path, work_id, work.title
            );
            ImportResult {
                path: item.path.clone(),
                status: ImportStatus::Imported,
                work_id: Some(work_id),
                error: None,
            }
        }
        Err(super::import::ImportFileError::Warning(w)) => {
            info!(
                "manual import: imported {} for work {} with warning: {w}",
                item.path, work_id
            );
            ImportResult {
                path: item.path.clone(),
                status: ImportStatus::Imported,
                work_id: Some(work_id),
                error: None,
            }
        }
        Err(super::import::ImportFileError::Failed(e)) => {
            warn!(
                "manual import: failed {} for work {}: {e}",
                item.path, work_id
            );
            ImportResult {
                path: item.path.clone(),
                status: ImportStatus::Failed,
                work_id: Some(work_id),
                error: Some(e),
            }
        }
    }
}

/// Find an existing work using same matching as scan: olKey first, then normalized title+author.
fn find_existing_work<'a>(
    works: &'a [livrarr_domain::Work],
    ol_key: &str,
    title: &str,
    author: &str,
) -> Option<&'a livrarr_domain::Work> {
    works
        .iter()
        .find(|w| w.ol_key.as_deref() == Some(ol_key))
        .or_else(|| {
            works.iter().find(|w| {
                normalize_for_matching(&w.title) == normalize_for_matching(title)
                    && normalize_for_matching(&w.author_name) == normalize_for_matching(author)
            })
        })
}

/// Find an existing work by ol_key (with fallback), or create a new one via the same flow as work::add.
async fn find_or_create_work(
    state: &AppState,
    user_id: i64,
    item: &ImportItem,
    existing_works: &[livrarr_domain::Work],
) -> Result<i64, ApiError> {
    if let Some(work) = find_existing_work(existing_works, &item.ol_key, &item.title, &item.author)
    {
        return Ok(work.id);
    }

    // Create via the same flow as work::add.
    let add_req = AddWorkRequest {
        ol_key: item.ol_key.clone(),
        title: item.title.clone(),
        author_name: item.author.clone(),
        author_ol_key: None, // We don't have this from the scan match; enrichment will fill it.
        year: None,
        cover_url: None,
    };

    let resp = super::work::add_work_internal(state, user_id, add_req).await?;
    Ok(resp.work.id)
}

// ---------------------------------------------------------------------------
// File enumeration with limits
// ---------------------------------------------------------------------------

struct EnumeratedFile {
    path: PathBuf,
    media_type: MediaType,
}

fn enumerate_with_limits(dir: &Path) -> (Vec<EnumeratedFile>, Option<String>) {
    let mut files = Vec::new();
    let mut entries_traversed = 0usize;
    let mut warning = None;

    enumerate_recursive(dir, &mut files, &mut entries_traversed, &mut warning);

    (files, warning)
}

fn enumerate_recursive(
    dir: &Path,
    files: &mut Vec<EnumeratedFile>,
    entries_traversed: &mut usize,
    warning: &mut Option<String>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        *entries_traversed += 1;

        if *entries_traversed > MAX_ENTRIES_TRAVERSED {
            *warning = Some(format!(
                "Traversal limit reached ({MAX_ENTRIES_TRAVERSED} entries). Some files may not be shown."
            ));
            return;
        }

        if files.len() >= MAX_MEDIA_FILES {
            *warning = Some(format!(
                "Found more than {MAX_MEDIA_FILES} media files. Showing first {MAX_MEDIA_FILES} — scan a subdirectory for remaining files."
            ));
            return;
        }

        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden.
        if name.starts_with('.') {
            continue;
        }

        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        if ft.is_dir() {
            enumerate_recursive(&path, files, entries_traversed, warning);
            if warning.is_some() {
                return;
            }
        } else if ft.is_file() {
            if let Some(media_type) = classify_file(&path) {
                files.push(EnumeratedFile { path, media_type });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// LLM filename parsing
// ---------------------------------------------------------------------------

/// Returns (parsed_files_by_index, sort_order).
/// sort_order is the LLM's recommended display order as a vec of original indices.
async fn llm_parse_filenames(
    state: &AppState,
    filenames: &[String],
) -> (Vec<Option<ParsedFile>>, Vec<usize>) {
    let default_order: Vec<usize> = (0..filenames.len()).collect();

    let cfg: livrarr_db::MetadataConfig = match state.db.get_metadata_config().await {
        Ok(c) => c,
        Err(_) => return (vec![None; filenames.len()], default_order),
    };

    let endpoint = match cfg.llm_endpoint.as_deref().filter(|s| !s.is_empty()) {
        Some(e) => e.to_string(),
        None => return (vec![None; filenames.len()], (0..filenames.len()).collect()),
    };
    let api_key = match cfg.llm_api_key.as_deref().filter(|s| !s.is_empty()) {
        Some(k) => k.to_string(),
        None => return (vec![None; filenames.len()], (0..filenames.len()).collect()),
    };
    let model = match cfg.llm_model.as_deref().filter(|s| !s.is_empty()) {
        Some(m) => m.to_string(),
        None => return (vec![None; filenames.len()], (0..filenames.len()).collect()),
    };

    let mut all_parsed: Vec<Option<ParsedFile>> = vec![None; filenames.len()];
    let mut sort_order: Vec<usize> = Vec::new();

    // Process in batches.
    for chunk_start in (0..filenames.len()).step_by(LLM_BATCH_SIZE) {
        let chunk_end = (chunk_start + LLM_BATCH_SIZE).min(filenames.len());
        let chunk = &filenames[chunk_start..chunk_end];

        match llm_parse_batch(&state.http_client, &endpoint, &api_key, &model, chunk).await {
            Some(parsed) => {
                for (idx, p) in parsed {
                    let abs_idx = chunk_start + idx;
                    if abs_idx < all_parsed.len() {
                        all_parsed[abs_idx] = Some(p);
                        sort_order.push(abs_idx);
                    }
                }
            }
            None => {
                warn!(
                    "manual import: LLM batch failed for files {}-{}",
                    chunk_start, chunk_end
                );
            }
        }
    }

    // Add any files the LLM didn't return (failed parse) at the end.
    for i in 0..filenames.len() {
        if !sort_order.contains(&i) {
            sort_order.push(i);
        }
    }

    (all_parsed, sort_order)
}

async fn llm_parse_batch(
    http: &livrarr_http::HttpClient,
    endpoint: &str,
    api_key: &str,
    model: &str,
    filenames: &[String],
) -> Option<Vec<(usize, ParsedFile)>> {
    let mut listing = String::new();
    for (i, name) in filenames.iter().enumerate() {
        listing.push_str(&format!("{i}: \"{name}\"\n"));
    }

    let prompt = format!(
        "These are ebook/audiobook filenames:\n\n\
         {listing}\n\
         Extract the author name, title, and series info from each filename.\n\
         Order the results in the most logical way for a reader — \
         group by author, then by series order, then standalone works alphabetically.\n\n\
         Return a JSON array. Each entry: {{\"idx\": <original index>, \"author\": \"<author name>\", \
         \"title\": \"<book title>\", \"series\": \"<series name or null>\", \
         \"position\": <number or null>}}\n\n\
         Return ONLY the JSON array, no other text."
    );

    let url = format!(
        "{}chat/completions",
        endpoint.trim_end_matches('/').to_owned() + "/"
    );

    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 4000,
        "temperature": 0.0,
    });

    let resp = http
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let data: serde_json::Value = resp.json().await.ok()?;
    let answer = data
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    // Robust JSON extraction: find first '[' and last ']' to handle preamble text.
    let start = answer.find('[').unwrap_or(0);
    let end = answer.rfind(']').map(|e| e + 1).unwrap_or(answer.len());
    let json_str = if start < end {
        &answer[start..end]
    } else {
        answer
    };

    let entries: Vec<serde_json::Value> = serde_json::from_str(json_str).ok()?;

    let parsed: Vec<(usize, ParsedFile)> = entries
        .iter()
        .filter_map(|entry| {
            let idx = entry.get("idx")?.as_u64()? as usize;
            let author = entry.get("author")?.as_str()?.to_string();
            let title = entry.get("title")?.as_str()?.to_string();
            let series = entry
                .get("series")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let position = entry.get("position").and_then(|v| v.as_f64());

            Some((
                idx,
                ParsedFile {
                    author,
                    title,
                    series,
                    series_position: position,
                },
            ))
        })
        .collect();

    if parsed.is_empty() {
        return None;
    }

    Some(parsed)
}

// ---------------------------------------------------------------------------
// OL search helpers
// ---------------------------------------------------------------------------

/// Search OL for a single term, return best match.
async fn search_ol_single(state: &AppState, term: &str) -> Option<WorkSearchResult> {
    let results = search_ol_batch(state, term).await.ok()?;
    results.into_iter().next()
}

/// Search OL and return cleaned results (reuses existing lookup_openlibrary + LLM cleanup).
async fn search_ol_batch(state: &AppState, term: &str) -> Result<Vec<WorkSearchResult>, ApiError> {
    let resp = state
        .http_client
        .get("https://openlibrary.org/search.json")
        .query(&[
            ("q", term),
            ("limit", "10"),
            (
                "fields",
                "key,title,author_name,author_key,first_publish_year,cover_i",
            ),
        ])
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("OpenLibrary request failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "OpenLibrary returned {}",
            resp.status()
        )));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ApiError::BadGateway(format!("OpenLibrary parse error: {e}")))?;

    let docs = data
        .get("docs")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();

    let results: Vec<WorkSearchResult> = docs
        .iter()
        .filter_map(|doc| {
            let key = doc.get("key")?.as_str()?;
            let title = doc.get("title")?.as_str()?;
            let ol_key = key.trim_start_matches("/works/").to_string();

            let author_name = doc
                .get("author_name")
                .and_then(|a| a.as_array())
                .and_then(|a| a.first())
                .and_then(|a| a.as_str())
                .unwrap_or("Unknown")
                .to_string();

            let author_ol_key = doc
                .get("author_key")
                .and_then(|a| a.as_array())
                .and_then(|a| a.first())
                .and_then(|a| a.as_str())
                .map(|k| k.trim_start_matches("/authors/").to_string());

            let year = doc
                .get("first_publish_year")
                .and_then(|y| y.as_i64())
                .map(|y| y as i32);

            let cover_url = doc
                .get("cover_i")
                .and_then(|c| c.as_i64())
                .map(|c| format!("https://covers.openlibrary.org/b/id/{c}-L.jpg"));

            Some(WorkSearchResult {
                ol_key,
                title: title.to_string(),
                author_name,
                author_ol_key,
                year,
                cover_url,
                description: None,
                series_name: None,
                series_position: None,
            })
        })
        .collect();

    Ok(results)
}
