use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::context::{
    HasAppConfigService, HasAuthorService, HasImportService, HasManualImportScan,
    HasManualImportService, HasMatchingService, HasWorkService,
};

pub trait ManualImportHandlerContext:
    HasMatchingService
    + HasManualImportService
    + HasManualImportScan
    + HasAppConfigService
    + HasAuthorService
    + HasWorkService
    + HasImportService
    + Clone
    + Send
    + Sync
    + 'static
{
}

impl<T> ManualImportHandlerContext for T where
    T: HasMatchingService
        + HasManualImportService
        + HasManualImportScan
        + HasAppConfigService
        + HasAuthorService
        + HasWorkService
        + HasImportService
        + Clone
        + Send
        + Sync
        + 'static
{
}
use crate::middleware::RequireAdmin;
use crate::ApiError;
use livrarr_domain::services::{
    AppConfigService, AuthorService, ImportFileResult, ImportService, ImportSingleFileRequest,
    ManualImportService, MatchingService, WorkService,
};
use livrarr_domain::{classify_file, normalize_for_matching, MediaType};

// ---------------------------------------------------------------------------
// Types
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
    pub ol_total: usize,
    pub ol_completed: usize,
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
    pub size: i64,
    pub parsed: Option<ParsedFile>,
    #[serde(rename = "match")]
    pub ol_match: Option<OlMatch>,
    pub existing_work_id: Option<i64>,
    pub has_existing_media_type: bool,
    pub routable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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

/// Snapshot returned by the scan accessor.
pub struct ScanSnapshot {
    pub files: Vec<ScannedFile>,
    pub warnings: Vec<String>,
    pub ol_total: usize,
    pub ol_completed: usize,
    pub user_id: i64,
}

/// Update for a single scanned file's OL match.
pub struct ScanFileUpdate {
    pub ol_match: Option<OlMatch>,
    pub existing_work_id: Option<i64>,
}

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

const MAX_MEDIA_FILES: usize = 2_000;
const MAX_ENTRIES_TRAVERSED: usize = 50_000;

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub async fn scan<S: ManualImportHandlerContext>(
    State(state): State<S>,
    RequireAdmin(auth): RequireAdmin,
    Json(req): Json<ScanRequest>,
) -> Result<Json<ScanResponse>, ApiError> {
    use crate::accessors::ManualImportScanAccessor;

    let path = PathBuf::from(&req.path);
    let precheck: Result<(), &'static str> = tokio::task::spawn_blocking({
        let path = path.clone();
        move || {
            if !path.exists() || !path.is_dir() {
                return Err("The file system path specified was not found.");
            }
            if std::fs::read_dir(&path).is_err() {
                return Err("The file system path specified was not found.");
            }
            Ok(())
        }
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;
    precheck.map_err(|msg| ApiError::BadRequest(msg.into()))?;

    let mut warnings = Vec::new();

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

    // Group multi-file audiobooks by parent directory.
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

    // Local extraction via MatchingService.
    let scan_root_path = path.clone();
    let mut parsed_files: Vec<Option<ParsedFile>> = Vec::with_capacity(scan_items.len());
    for si in &scan_items {
        let input = livrarr_domain::services::MatchInput {
            file_path: Some(si.primary_path.clone()),
            grouped_paths: si.grouped_paths.clone(),
            parse_string: Some(si.display_name.clone()),
            media_type: Some(si.media_type),
            scan_root: Some(scan_root_path.clone()),
        };
        let clusters = state.matching_service().extract_and_reconcile(&input).await;
        let parsed = clusters.into_iter().next().map(|c| ParsedFile {
            author: c.author.unwrap_or_default(),
            title: c.title.unwrap_or_default(),
            series: c.series,
            series_position: c.series_position,
            language: c.language,
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

    let user_id = auth.user.id;
    let existing_works = state.manual_import_service().list_works(user_id).await?;
    let root_folders = state.manual_import_service().list_root_folders().await?;

    let pre_existing_work_ids: Vec<Option<i64>> = sort_indices
        .iter()
        .map(|&i| {
            let parsed = parsed_files.get(i).and_then(|p| p.as_ref());
            parsed.and_then(|p| {
                livrarr_matching::work_dedup::find_matching_work(
                    &existing_works,
                    &p.title,
                    &p.author,
                    &Default::default(),
                )
                .map(|w| w.id)
            })
        })
        .collect();

    let matched_work_ids: Vec<i64> = pre_existing_work_ids
        .iter()
        .flatten()
        .copied()
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let batch_items = if matched_work_ids.is_empty() {
        vec![]
    } else {
        state
            .manual_import_service()
            .list_library_items_by_work_ids(user_id, &matched_work_ids)
            .await
            .unwrap_or_default()
    };
    let mut items_by_work: std::collections::HashMap<i64, Vec<&livrarr_domain::LibraryItem>> =
        std::collections::HashMap::new();
    for item in &batch_items {
        items_by_work.entry(item.work_id).or_default().push(item);
    }

    let mut scanned_files = Vec::new();
    let mut ol_indices = Vec::new();

    for (loop_idx, &i) in sort_indices.iter().enumerate() {
        let si = &scan_items[i];
        let filename = si.display_name.clone();
        let parsed = parsed_files.get(i).cloned().flatten();

        let size: i64 = if let Some(ref paths) = si.grouped_paths {
            let mut total = 0u64;
            for p in paths {
                total += tokio::fs::metadata(p).await.map(|m| m.len()).unwrap_or(0);
            }
            total.try_into().unwrap_or(i64::MAX)
        } else {
            tokio::fs::metadata(&si.primary_path)
                .await
                .map(|m| m.len())
                .unwrap_or(0) as i64
        };

        let routable = root_folders.iter().any(|rf| rf.media_type == si.media_type);
        let existing_work_id = pre_existing_work_ids[loop_idx];

        let has_existing_media_type = existing_work_id
            .and_then(|wid| items_by_work.get(&wid))
            .map(|items| items.iter().any(|li| li.media_type == si.media_type))
            .unwrap_or(false);

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
        if parsed.is_some() {
            ol_indices.push(file_idx);
        }

        scanned_files.push(ScannedFile {
            path: si.primary_path.display().to_string(),
            filename: display_filename,
            media_type: si.media_type,
            size,
            parsed,
            ol_match: None,
            existing_work_id,
            has_existing_media_type,
            routable,
            error: file_error,
            grouped_paths: grouped_path_strings,
        });
    }

    let ol_total = ol_indices.len();
    let scan_id = uuid::Uuid::new_v4().to_string();

    state.manual_import_scan().insert_scan(
        scan_id.clone(),
        user_id,
        scanned_files.clone(),
        warnings.clone(),
        ol_total,
    );

    // Background OL lookups.
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

                st.manual_import_scan().acquire_ol_permit().await;

                // Skip OL lookup if the file already matched an existing work
                // from parsed metadata — don't let OL's potentially bad data
                // override a correct match.
                let already_matched = {
                    let scan = st.manual_import_scan().get_scan(&sid);
                    scan.and_then(|s| s.files.get(file_idx).and_then(|f| f.existing_work_id))
                        .is_some()
                };

                if already_matched {
                    st.manual_import_scan().increment_ol_completed(&sid);
                    return;
                }

                let search_term = {
                    let scan = st.manual_import_scan().get_scan(&sid);
                    let scan = match scan {
                        Some(s) => s,
                        None => return,
                    };
                    let f = match scan.files.get(file_idx) {
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

                let ol_results = st
                    .manual_import_scan()
                    .search_ol_works(&search_term, 10)
                    .await
                    .unwrap_or_default();

                if let Some(scan) = st.manual_import_scan().get_scan(&sid) {
                    let user_id = scan.user_id;
                    let existing_works = st
                        .manual_import_service()
                        .list_works(user_id)
                        .await
                        .unwrap_or_default();

                    if !ol_results.is_empty() {
                        let dup_match = ol_results.iter().find_map(|result| {
                            let matched = livrarr_matching::work_dedup::find_matching_work(
                                &existing_works,
                                &result.title,
                                &result.author_name,
                                &livrarr_matching::work_dedup::ProviderKeys {
                                    ol_key: result.ol_key.as_deref(),
                                    ..Default::default()
                                },
                            );
                            matched.map(|w| (result, w.id))
                        });

                        let update = if let Some((result, dup_id)) = dup_match {
                            ScanFileUpdate {
                                ol_match: Some(OlMatch {
                                    ol_key: result.ol_key.clone().unwrap_or_default(),
                                    title: result.title.clone(),
                                    author: result.author_name.clone(),
                                    cover_url: result.cover_url.clone(),
                                    existing_work_id: Some(dup_id),
                                }),
                                existing_work_id: Some(dup_id),
                            }
                        } else {
                            let result = &ol_results[0];
                            ScanFileUpdate {
                                ol_match: Some(OlMatch {
                                    ol_key: result.ol_key.clone().unwrap_or_default(),
                                    title: result.title.clone(),
                                    author: result.author_name.clone(),
                                    cover_url: result.cover_url.clone(),
                                    existing_work_id: None,
                                }),
                                existing_work_id: None,
                            }
                        };
                        st.manual_import_scan()
                            .update_scan_file(&sid, file_idx, update);
                    }
                    st.manual_import_scan().increment_ol_completed(&sid);
                }
            }));
        }

        for h in handles {
            let _ = h.await;
        }

        let st = bg_state.clone();
        let sid = bg_scan_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
            st.manual_import_scan().remove_scan(&sid);
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

pub async fn scan_progress<S: HasManualImportScan>(
    State(state): State<S>,
    RequireAdmin(auth): RequireAdmin,
    axum::extract::Path(scan_id): axum::extract::Path<String>,
) -> Result<Json<ScanProgressResponse>, ApiError> {
    use crate::accessors::ManualImportScanAccessor;

    let scan = state
        .manual_import_scan()
        .get_scan(&scan_id)
        .ok_or(ApiError::NotFound)?;

    // Verify the requesting user owns this scan.
    if scan.user_id != auth.user.id {
        return Err(ApiError::NotFound);
    }

    Ok(Json(ScanProgressResponse {
        files: scan.files,
        warnings: scan.warnings,
        ol_total: scan.ol_total,
        ol_completed: scan.ol_completed,
    }))
}

pub async fn search<S: HasManualImportScan + HasManualImportService>(
    State(state): State<S>,
    RequireAdmin(auth): RequireAdmin,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, ApiError> {
    use crate::accessors::ManualImportScanAccessor;

    let term = if let Some(ref author) = req.author {
        format!("{} {}", req.query, author)
    } else {
        req.query.clone()
    };

    tracing::info!(query = %req.query, author = ?req.author, term = %term, "manual import search");

    let results = state
        .manual_import_scan()
        .search_ol_works(&term, 10)
        .await
        .map_err(ApiError::BadGateway)?;

    tracing::info!(ol_results = results.len(), "OL search returned");

    let user_id = auth.user.id;
    let existing_works = state.manual_import_service().list_works(user_id).await?;

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

    Ok(Json(SearchResponse { results }))
}

pub async fn import<S: ManualImportHandlerContext>(
    State(state): State<S>,
    RequireAdmin(auth): RequireAdmin,
    Json(req): Json<ImportRequest>,
) -> Result<Json<ImportResponse>, ApiError> {
    let user_id = auth.user.id;
    let existing_works = state.manual_import_service().list_works(user_id).await?;
    let root_folders = state.manual_import_service().list_root_folders().await?;
    let media_mgmt = state
        .app_config_service()
        .get_media_management_config()
        .await?;

    let mut results = Vec::new();
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
            &mut author_ol_cache,
        )
        .await;
        results.push(result);
    }

    Ok(Json(ImportResponse { results }))
}

// ---------------------------------------------------------------------------
// Import helpers
// ---------------------------------------------------------------------------

async fn import_single_item<S: ManualImportHandlerContext>(
    state: &S,
    user_id: i64,
    item: &ImportItem,
    existing_works: &[livrarr_domain::Work],
    root_folders: &[livrarr_domain::RootFolder],
    _media_mgmt: &livrarr_domain::settings::MediaManagementConfig,
    author_ol_cache: &mut std::collections::HashMap<String, Option<String>>,
) -> ImportResult {
    let source = PathBuf::from(&item.path);

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

    let target_path = state.import_service().build_target_path(
        &root_folder.path,
        user_id,
        &item.author,
        &item.title,
        media_type,
        &source,
        &source,
    );

    let req = ImportSingleFileRequest {
        source,
        target_path,
        root_folder_path: root_folder.path.clone(),
        root_folder_id: root_folder.id,
        media_type,
        user_id,
        work_id,
        author_name: item.author.clone(),
        title: item.title.clone(),
    };

    match state.import_service().import_single_file(req).await {
        ImportFileResult::Ok => {
            info!("manual import: imported {} for work {}", item.path, work_id);
            ImportResult {
                path: item.path.clone(),
                status: ImportStatus::Imported,
                work_id: Some(work_id),
                error: None,
            }
        }
        ImportFileResult::Warning(w) => {
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
        ImportFileResult::Failed(e) => {
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

fn find_existing_work<'a>(
    works: &'a [livrarr_domain::Work],
    ol_key: &str,
    title: &str,
    author: &str,
) -> Option<&'a livrarr_domain::Work> {
    livrarr_matching::work_dedup::find_matching_work(
        works,
        title,
        author,
        &livrarr_matching::work_dedup::ProviderKeys {
            ol_key: if ol_key.is_empty() { None } else { Some(ol_key) },
            ..Default::default()
        },
    )
}

async fn find_or_create_work<S: HasAuthorService + HasWorkService + HasManualImportService>(
    state: &S,
    user_id: i64,
    item: &ImportItem,
    existing_works: &[livrarr_domain::Work],
    author_ol_cache: &mut std::collections::HashMap<String, Option<String>>,
) -> Result<i64, ApiError> {
    if let Some(work) = find_existing_work(existing_works, &item.ol_key, &item.title, &item.author)
    {
        return Ok(work.id);
    }

    let cache_key = item.author.to_lowercase();
    let author_ol_key = if let Some(cached) = author_ol_cache.get(&cache_key) {
        cached.clone()
    } else {
        let result = match state.author_service().lookup_authors(&item.author, 1).await {
            Ok(results) => results.into_iter().next().map(|r| r.ol_key),
            Err(e) => {
                tracing::warn!(author = %item.author, error = %e, "OL author lookup failed");
                None
            }
        };
        author_ol_cache.insert(cache_key, result.clone());
        result
    };

    let ol_key = if item.ol_key.is_empty() {
        None
    } else {
        Some(item.ol_key.clone())
    };
    let add_req = livrarr_domain::services::AddWorkRequest {
        ol_key,
        title: item.title.clone(),
        author_name: item.author.clone(),
        author_ol_key,
        gr_key: None,
        year: None,
        cover_url: None,
        metadata_source: None,
        language: item.language.clone(),
        detail_url: None,
        series_name: None,
        series_position: None,
        defer_enrichment: false,
        provenance_setter: None,
    };

    match state.work_service().add(user_id, add_req).await {
        Ok(result) => Ok(result.work.id),
        Err(e) => {
            let fresh_works = state
                .manual_import_service()
                .list_works(user_id)
                .await
                .map_err(ApiError::from)?;
            find_existing_work(&fresh_works, &item.ol_key, &item.title, &item.author)
                .map(|w| w.id)
                .ok_or_else(|| ApiError::Internal(format!("work creation failed: {e}")))
        }
    }
}

// ---------------------------------------------------------------------------
// File enumeration
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
                "Found more than {MAX_MEDIA_FILES} media files. Showing first {MAX_MEDIA_FILES}."
            ));
            return;
        }

        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

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
