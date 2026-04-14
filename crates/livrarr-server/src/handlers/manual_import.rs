use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::middleware::RequireAdmin;
use crate::state::AppState;
use crate::{AddWorkRequest, ApiError, WorkSearchResult};
use livrarr_db::{
    ConfigDb, CreateHistoryEventDbRequest, HistoryDb, LibraryItemDb, RootFolderDb, WorkDb,
};
use livrarr_domain::EventType;
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
    pub scan_id: String,
    pub files: Vec<ScannedFile>,
    pub warnings: Vec<String>,
    /// Total files found; OL lookups proceed in background.
    pub ol_total: usize,
    pub ol_completed: usize,
}

/// In-memory state for a progressive scan. OL lookups update files in place.
pub struct ScanState {
    pub files: tokio::sync::RwLock<Vec<ScannedFile>>,
    pub warnings: Vec<String>,
    pub ol_total: usize,
    pub ol_completed: std::sync::atomic::AtomicUsize,
    pub user_id: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgressResponse {
    pub files: Vec<ScannedFile>,
    pub warnings: Vec<String>,
    pub ol_total: usize,
    pub ol_completed: usize,
}

#[derive(Debug, Clone, Serialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
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

const MAX_MEDIA_FILES: usize = 2_000;
const MAX_ENTRIES_TRAVERSED: usize = 50_000;

/// POST /api/v1/manualimport/scan
///
/// Two-phase scan: returns files immediately with parsed metadata + local dedup.
/// OL lookups run in background — poll GET /api/v1/manualimport/progress/:scan_id.
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
            scan_id: String::new(),
            files: vec![],
            warnings,
            ol_total: 0,
            ol_completed: 0,
        }));
    }

    // ── Group multi-file audiobooks by parent directory ──
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

    struct ScanItem {
        display_name: String,
        primary_path: PathBuf,
        media_type: MediaType,
        grouped_paths: Option<Vec<PathBuf>>,
    }

    let grouped_dirs: std::collections::HashSet<PathBuf> = dir_audio_files
        .iter()
        .filter(|(_, indices)| indices.len() >= 2)
        .map(|(dir, _)| dir.clone())
        .collect();

    let mut scan_items: Vec<ScanItem> = Vec::new();

    for (dir, indices) in &dir_audio_files {
        if indices.len() < 2 {
            continue;
        }
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

    for sf in source_files.iter() {
        if sf.media_type == MediaType::Audiobook {
            if let Some(parent) = sf.path.parent() {
                if grouped_dirs.contains(parent) {
                    continue;
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

    // Local extraction via matching engine (M1+M2+M3).
    let scan_root = path.clone();
    let mut parsed_files: Vec<Option<ParsedFile>> = Vec::with_capacity(scan_items.len());
    for si in &scan_items {
        let input = crate::matching::types::MatchInput {
            file_path: Some(si.primary_path.clone()),
            grouped_paths: si.grouped_paths.clone(),
            parse_string: Some(si.display_name.clone()),
            media_type: Some(si.media_type),
            scan_root: Some(scan_root.clone()),
        };
        let clusters = crate::matching::extract_and_reconcile(&input).await;
        let parsed = clusters.into_iter().next().map(|cluster| {
            let e = &cluster.primary;
            ParsedFile {
                author: e.author.clone().unwrap_or_default(),
                title: e.title.clone().unwrap_or_default(),
                series: e.series.clone(),
                series_position: e.series_position,
                language: e.language.clone(),
            }
        });
        parsed_files.push(parsed);
    }

    // Sort by (author, series, series_position, title).
    let mut sort_indices: Vec<usize> = (0..scan_items.len()).collect();
    sort_indices.sort_by(|&a, &b| {
        let pa = parsed_files[a].as_ref();
        let pb = parsed_files[b].as_ref();
        let author_a = pa.map(|p| p.author.as_str()).unwrap_or("");
        let author_b = pb.map(|p| p.author.as_str()).unwrap_or("");
        let series_a = pa.and_then(|p| p.series.as_deref()).unwrap_or("");
        let series_b = pb.and_then(|p| p.series.as_deref()).unwrap_or("");
        let pos_a = pa.and_then(|p| p.series_position).unwrap_or(f64::MAX);
        let pos_b = pb.and_then(|p| p.series_position).unwrap_or(f64::MAX);
        let title_a = pa.map(|p| p.title.as_str()).unwrap_or("");
        let title_b = pb.map(|p| p.title.as_str()).unwrap_or("");
        author_a
            .cmp(author_b)
            .then(series_a.cmp(series_b))
            .then(
                pos_a
                    .partial_cmp(&pos_b)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(title_a.cmp(title_b))
    });

    // Get user's existing works for local dedup.
    let user_id = auth.user.id;
    let existing_works = state.db.list_works(user_id).await?;
    let root_folders = state.db.list_root_folders().await?;

    // Phase 1: build files with parsed metadata + local dedup (no OL).
    let mut scanned_files = Vec::new();
    let mut ol_indices = Vec::new(); // indices into scanned_files that need OL lookup

    for &i in &sort_indices {
        let si = &scan_items[i];
        let filename = si.display_name.clone();
        let parsed = parsed_files.get(i).cloned().flatten();

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

        // Local-only dedup (normalized title+author match).
        let existing_work_id = parsed.as_ref().and_then(|p| {
            existing_works
                .iter()
                .find(|w| {
                    normalize_for_matching(&w.title) == normalize_for_matching(&p.title)
                        && normalize_for_matching(&w.author_name)
                            == normalize_for_matching(&p.author)
                })
                .map(|w| w.id)
        });

        let has_existing_media_type = if let Some(wid) = existing_work_id {
            state
                .db
                .list_library_items_by_work(user_id, wid)
                .await
                .map(|items| items.iter().any(|li| li.media_type == si.media_type))
                .unwrap_or(false)
        } else {
            false
        };

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

        let grouped_path_strings = si
            .grouped_paths
            .as_ref()
            .map(|paths| paths.iter().map(|p| p.display().to_string()).collect());

        let display_filename = if let Some(ref paths) = si.grouped_paths {
            format!("{}/ ({} files)", filename, paths.len())
        } else {
            filename
        };

        let file_idx = scanned_files.len();
        // Track files that have parsed metadata for OL lookup.
        if parsed.is_some() {
            ol_indices.push(file_idx);
        }

        scanned_files.push(ScannedFile {
            path: si.primary_path.display().to_string(),
            filename: display_filename,
            media_type: si.media_type,
            size,
            parsed,
            ol_match: None, // Filled by background OL lookup
            existing_work_id,
            has_existing_media_type,
            routable,
            error: file_error,
            grouped_paths: grouped_path_strings,
        });
    }

    let ol_total = ol_indices.len();
    let scan_id = uuid::Uuid::new_v4().to_string();

    // Store scan state for progressive polling.
    let scan_state = ScanState {
        files: tokio::sync::RwLock::new(scanned_files.clone()),
        warnings: warnings.clone(),
        ol_total,
        ol_completed: std::sync::atomic::AtomicUsize::new(0),
        user_id,
    };
    state
        .manual_import_scans
        .insert(scan_id.clone(), scan_state);

    // Phase 2: spawn background OL lookups (3 concurrent).
    let bg_state = state.clone();
    let bg_scan_id = scan_id.clone();
    tokio::spawn(async move {
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(3));
        let mut handles = Vec::new();

        for file_idx in ol_indices {
            let sem = semaphore.clone();
            let st = bg_state.clone();
            let sid = bg_scan_id.clone();

            handles.push(tokio::spawn(async move {
                let _permit = match sem.acquire().await {
                    Ok(p) => p,
                    Err(_) => return,
                };

                // Rate limit via OL leaky bucket.
                st.ol_rate_limiter.acquire().await;

                // Read the file's parsed metadata.
                let search_term = {
                    let scan = st.manual_import_scans.get(&sid);
                    let scan = match scan {
                        Some(s) => s,
                        None => return,
                    };
                    let files = scan.files.read().await;
                    let f = match files.get(file_idx) {
                        Some(f) => f,
                        None => return,
                    };
                    let p = match &f.parsed {
                        Some(p) => p,
                        None => return,
                    };
                    let mut clean_title = if let Some(paren) = p.title.find('(') {
                        p.title[..paren].trim().to_string()
                    } else {
                        p.title.trim().to_string()
                    };
                    if clean_title.len() > 60 {
                        if let Some(colon) = clean_title.find(':') {
                            if colon > 5 {
                                clean_title = clean_title[..colon].trim().to_string();
                            }
                        }
                    }
                    format!("{} {}", clean_title, p.author)
                };

                let ol_results = search_ol_batch(&st, &search_term)
                    .await
                    .unwrap_or_default();

                // Update the scan state with OL results.
                if let Some(scan) = st.manual_import_scans.get(&sid) {
                    let user_id = scan.user_id;
                    let existing_works = st.db.list_works(user_id).await.unwrap_or_default();

                    let mut files = scan.files.write().await;
                    if let Some(f) = files.get_mut(file_idx) {
                        if !ol_results.is_empty() {
                            // Prefer OL result that matches existing work (dedup).
                            let dup_match = ol_results.iter().find_map(|result| {
                                let dup = existing_works.iter().find(|w| {
                                    (result.ol_key.is_some()
                                        && w.ol_key.as_deref() == result.ol_key.as_deref())
                                        || (normalize_for_matching(&w.title)
                                            == normalize_for_matching(&result.title)
                                            && normalize_for_matching(&w.author_name)
                                                == normalize_for_matching(&result.author_name))
                                });
                                dup.map(|w| (result, w.id))
                            });

                            if let Some((result, dup_id)) = dup_match {
                                f.ol_match = Some(OlMatch {
                                    ol_key: result.ol_key.clone().unwrap_or_default(),
                                    title: result.title.clone(),
                                    author: result.author_name.clone(),
                                    cover_url: result.cover_url.clone(),
                                    existing_work_id: Some(dup_id),
                                });
                                f.existing_work_id = Some(dup_id);
                            } else {
                                let result = &ol_results[0];
                                f.ol_match = Some(OlMatch {
                                    ol_key: result.ol_key.clone().unwrap_or_default(),
                                    title: result.title.clone(),
                                    author: result.author_name.clone(),
                                    cover_url: result.cover_url.clone(),
                                    existing_work_id: None,
                                });
                            }
                        }
                        // If no OL match and no existing_work_id yet, local dedup was
                        // already done in phase 1.
                    }
                    scan.ol_completed
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }));
        }

        // Wait for all lookups to complete.
        for h in handles {
            let _ = h.await;
        }

        // Clean up scan state after 10 minutes.
        let st = bg_state.clone();
        let sid = bg_scan_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
            st.manual_import_scans.remove(&sid);
        });
    });

    Ok(Json(ScanResponse {
        scan_id,
        files: scanned_files,
        warnings,
        ol_total,
        ol_completed: 0,
    }))
}

/// GET /api/v1/manualimport/progress/:scan_id
pub async fn scan_progress(
    State(state): State<AppState>,
    RequireAdmin(_auth): RequireAdmin,
    axum::extract::Path(scan_id): axum::extract::Path<String>,
) -> Result<Json<ScanProgressResponse>, ApiError> {
    let scan = state
        .manual_import_scans
        .get(&scan_id)
        .ok_or(ApiError::NotFound)?;

    let files = scan.files.read().await.clone();
    let ol_completed = scan
        .ol_completed
        .load(std::sync::atomic::Ordering::Relaxed);

    Ok(Json(ScanProgressResponse {
        files,
        warnings: scan.warnings.clone(),
        ol_total: scan.ol_total,
        ol_completed,
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
                (r.ol_key.is_some() && w.ol_key.as_deref() == r.ol_key.as_deref())
                    || (normalize_for_matching(&w.title) == normalize_for_matching(&r.title)
                        && normalize_for_matching(&w.author_name)
                            == normalize_for_matching(&r.author_name))
            });
            OlMatch {
                ol_key: r.ol_key.unwrap_or_default(),
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
    #[serde(default)]
    pub language: Option<String>,
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
    // Cache author OL key lookups to avoid N+1 API calls for same author.
    let mut author_ol_cache: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();

    for item in &req.items {
        let result = import_single_item(
            &state,
            user_id,
            item,
            &existing_works,
            &root_folders,
            &media_mgmt,
            &existing_items_snapshot,
            &mut author_ol_cache,
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
    author_ol_cache: &mut std::collections::HashMap<String, Option<String>>,
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
    let work_id =
        match find_or_create_work(state, user_id, item, existing_works, author_ol_cache).await {
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
                import_id: None,
            })
            .await
        {
            Ok(_) => {
                info!(
                    "manual import: shared file {} for work {}",
                    item.path, work_id
                );
                log_manual_import_history(state, user_id, work_id, &item.path, &work.title, None)
                    .await;
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
            info!("manual import: imported {} for work {}", item.path, work_id);
            log_manual_import_history(state, user_id, work_id, &item.path, &work.title, None).await;
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
            log_manual_import_history(state, user_id, work_id, &item.path, &work.title, None).await;
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
            log_manual_import_history(state, user_id, work_id, &item.path, &work.title, Some(&e))
                .await;
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

async fn log_manual_import_history(
    state: &AppState,
    user_id: i64,
    work_id: i64,
    path: &str,
    title: &str,
    error: Option<&str>,
) {
    let event_type = if error.is_some() {
        EventType::ImportFailed
    } else {
        EventType::Imported
    };
    if let Err(e) = state
        .db
        .create_history_event(CreateHistoryEventDbRequest {
            user_id,
            work_id: Some(work_id),
            event_type,
            data: serde_json::json!({
                "source": "manual_import",
                "path": path,
                "title": title,
                "error": error,
            }),
        })
        .await
    {
        tracing::warn!("create_history_event failed: {e}");
    }
}

/// Find an existing work by ol_key (with fallback), or create a new one via the same flow as work::add.
async fn find_or_create_work(
    state: &AppState,
    user_id: i64,
    item: &ImportItem,
    existing_works: &[livrarr_domain::Work],
    author_ol_cache: &mut std::collections::HashMap<String, Option<String>>,
) -> Result<i64, ApiError> {
    if let Some(work) = find_existing_work(existing_works, &item.ol_key, &item.title, &item.author)
    {
        return Ok(work.id);
    }

    // Resolve author OL key with cache to avoid N+1 lookups for same author.
    let cache_key = item.author.to_lowercase();
    let author_ol_key = if let Some(cached) = author_ol_cache.get(&cache_key) {
        cached.clone()
    } else {
        let result = match super::author::lookup_ol_authors(&state.http_client, &item.author, 1)
            .await
        {
            Ok(results) => results.into_iter().next().map(|r| r.ol_key),
            Err(e) => {
                tracing::warn!(author = %item.author, error = %e, "OL author lookup failed during import, proceeding without ol_key");
                None
            }
        };
        author_ol_cache.insert(cache_key, result.clone());
        result
    };

    // Create via the same flow as work::add.
    let ol_key = if item.ol_key.is_empty() {
        None
    } else {
        Some(item.ol_key.clone())
    };
    let add_req = AddWorkRequest {
        ol_key,
        title: item.title.clone(),
        author_name: item.author.clone(),
        author_ol_key,
        year: None,
        cover_url: None,
        metadata_source: None,
        language: item.language.clone(),
        detail_url: None,
        defer_enrichment: true,
    };

    match super::work::add_work_internal(state, user_id, add_req).await {
        Ok(resp) => Ok(resp.work.id),
        Err(ApiError::Conflict { .. }) => {
            // Work was created by a prior item in the same batch (stale snapshot).
            // Re-query to find it.
            let fresh_works = state.db.list_works(user_id).await?;
            find_existing_work(&fresh_works, &item.ol_key, &item.title, &item.author)
                .map(|w| w.id)
                .ok_or_else(|| {
                    ApiError::Internal("work conflict but not found on re-query".into())
                })
        }
        Err(e) => Err(e),
    }
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
                ol_key: Some(ol_key),
                title: title.to_string(),
                author_name,
                author_ol_key,
                year,
                cover_url,
                description: None,
                series_name: None,
                series_position: None,
                source: None,
                source_type: None,
                language: None,
                detail_url: None,
                rating: None,
            })
        })
        .collect();

    Ok(results)
}
