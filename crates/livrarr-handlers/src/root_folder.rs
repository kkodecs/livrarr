use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::Json;

use crate::context::AppContext;
use crate::middleware::RequireAdmin;
use crate::{
    ApiError, CreateRootFolderRequest, RootFolderResponse, ScanErrorEntry, ScanResult,
    ScanUnmatchedFile,
};
use livrarr_domain::services::{FileService, ImportIoService, SettingsService, WorkService};
use livrarr_domain::{classify_file, normalize_for_matching, MediaType};

fn disk_space(path: &str) -> (Option<i64>, Option<i64>) {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let c_path = match CString::new(path) {
        Ok(p) => p,
        Err(_) => return (None, None),
    };

    // SAFETY: `c_path` is a valid NUL-terminated C string (CString::new succeeded above).
    // `stat` is written to by `statvfs` before we read it (return value == 0 confirms success).
    // `statvfs` is a POSIX function that writes a plain-data struct with no pointers requiring
    // lifetime management. The `assume_init` call is safe because `statvfs` fully initialises
    // the struct on success.
    unsafe {
        let mut stat = MaybeUninit::<libc::statvfs>::uninit();
        if libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) != 0 {
            return (None, None);
        }
        let stat = stat.assume_init();
        let free = (stat.f_bavail as i64) * (stat.f_frsize as i64);
        let total = (stat.f_blocks as i64) * (stat.f_frsize as i64);
        (Some(free), Some(total))
    }
}

fn to_response_sync(rf: livrarr_domain::RootFolder) -> RootFolderResponse {
    let (free_space, total_space) = disk_space(&rf.path);
    RootFolderResponse {
        id: rf.id,
        path: rf.path,
        media_type: rf.media_type,
        free_space,
        total_space,
    }
}

pub async fn list<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
) -> Result<Json<Vec<RootFolderResponse>>, ApiError> {
    let folders = state.settings_service().list_root_folders().await?;
    let responses = tokio::task::spawn_blocking(move || {
        folders
            .into_iter()
            .map(to_response_sync)
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking join error: {e}")))?;
    Ok(Json(responses))
}

pub async fn create<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Json(req): Json<CreateRootFolderRequest>,
) -> Result<Json<RootFolderResponse>, ApiError> {
    if req.path.is_empty() {
        return Err(ApiError::BadRequest("path is required".into()));
    }
    let rf = state
        .settings_service()
        .create_root_folder(&req.path, req.media_type)
        .await?;
    let response = tokio::task::spawn_blocking(move || to_response_sync(rf))
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking join error: {e}")))?;
    Ok(Json(response))
}

pub async fn delete<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    state.settings_service().delete_root_folder(id).await?;
    Ok(())
}

pub async fn scan<S: AppContext>(
    State(state): State<S>,
    RequireAdmin(auth): RequireAdmin,
    Path(id): Path<i64>,
) -> Result<Json<ScanResult>, ApiError> {
    let rf = state.settings_service().get_root_folder(id).await?;
    let user_id = auth.user.id;

    let root_path = PathBuf::from(&rf.path);
    if !tokio::fs::try_exists(&root_path).await.unwrap_or(false) {
        return Err(ApiError::BadRequest(
            "root folder path does not exist".into(),
        ));
    }

    let media_type = rf.media_type;

    // Scan new tenant-aware layout: {root}/{user_id}/...
    let user_dir = root_path.join(user_id.to_string());
    let scan_files = tokio::task::spawn_blocking({
        let user_dir_clone = user_dir.clone();
        move || scan_walk_user_dir(&user_dir_clone, media_type)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking join error: {e}")))?;

    let (mut files, mut scan_errors) = scan_files;

    // Backward compat: also scan old layout (files directly under root without
    // user_id segment). Skip any top-level directory whose name is purely numeric
    // (those are user_id directories from the new layout).
    let old_layout_files = tokio::task::spawn_blocking({
        let root_clone = root_path.clone();
        move || scan_walk_user_dir_skip_numeric_dirs(&root_clone, media_type)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking join error: {e}")))?;

    let (old_files, old_errors) = old_layout_files;
    files.extend(old_files);
    scan_errors.extend(old_errors);

    if files.is_empty() && scan_errors.is_empty() {
        return Ok(Json(ScanResult {
            matched: 0,
            unmatched: vec![],
            errors: vec![],
        }));
    }

    let works = state
        .work_service()
        .list(
            user_id,
            livrarr_domain::services::WorkFilter {
                author_id: None,
                monitored: None,
                enrichment_status: None,
                media_type: None,
                sort_by: None,
                sort_dir: None,
            },
        )
        .await?;

    let existing_items = state.file_service().list(user_id).await?;

    // Pre-compute normalized lookup HashMap to avoid O(files×works).
    let work_lookup: std::collections::HashMap<(String, String), &livrarr_domain::Work> = works
        .iter()
        .map(|w| {
            (
                (
                    normalize_for_matching(&w.title),
                    normalize_for_matching(&w.author_name),
                ),
                w,
            )
        })
        .collect();

    let mut matched: i64 = 0;
    let mut unmatched = Vec::new();
    let mut errors: Vec<ScanErrorEntry> = scan_errors;

    for sf in &files {
        // Paths from user_dir scan are prefixed with {root}/{user_id}/...
        // Paths from old layout scan are prefixed with {root}/...
        // Store relative to root_path in both cases.
        let relative_to_root = match sf.path.strip_prefix(&root_path) {
            Ok(r) => r,
            Err(_) => {
                errors.push(ScanErrorEntry {
                    path: sf.path.display().to_string(),
                    message: "path not within root folder".into(),
                });
                continue;
            }
        };

        // Determine the effective base for author/title parsing.
        // New layout: {user_id}/{author}/{title}.ext — strip user_id prefix.
        // Old layout: {author}/{title}.ext — use as-is.
        let user_id_str = user_id.to_string();
        let parse_relative = if relative_to_root
            .components()
            .next()
            .and_then(|c| c.as_os_str().to_str())
            == Some(&user_id_str)
        {
            relative_to_root
                .strip_prefix(&user_id_str)
                .unwrap_or(relative_to_root)
        } else {
            relative_to_root
        };

        let components: Vec<&str> = parse_relative
            .components()
            .map(|c| c.as_os_str().to_str().unwrap_or(""))
            .collect();
        let depth = components.len();

        let (parsed_author, parsed_title) = match media_type {
            MediaType::Ebook => {
                if depth != 2 {
                    unmatched.push(ScanUnmatchedFile {
                        path: sf.path.display().to_string(),
                        media_type,
                    });
                    continue;
                }
                let author = components[0];
                let filename = components[1];
                let stem = std::path::Path::new(filename)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(filename);
                (author.to_string(), stem.to_string())
            }
            MediaType::Audiobook => {
                if depth < 3 {
                    unmatched.push(ScanUnmatchedFile {
                        path: sf.path.display().to_string(),
                        media_type,
                    });
                    continue;
                }
                let author = components[0];
                let title_dir = components[1];
                (author.to_string(), title_dir.to_string())
            }
        };

        let norm_title = normalize_for_matching(&parsed_title);
        let norm_author = normalize_for_matching(&parsed_author);

        let matched_work = work_lookup.get(&(norm_title, norm_author)).copied();

        match matched_work {
            Some(work) => {
                let relative_str = relative_to_root.to_string_lossy().to_string();
                let already_tracked = existing_items
                    .iter()
                    .any(|li| li.root_folder_id == rf.id && li.path == relative_str);

                if !already_tracked {
                    let file_size = sf.path.metadata().map(|m| m.len() as i64).unwrap_or(0);
                    match state
                        .import_io_service()
                        .create_library_item(livrarr_domain::services::CreateLibraryItemRequest {
                            user_id,
                            work_id: work.id,
                            root_folder_id: rf.id,
                            path: relative_str,
                            media_type,
                            file_size,
                            import_id: None,
                        })
                        .await
                    {
                        Ok(_) => matched += 1,
                        Err(e) => {
                            errors.push(ScanErrorEntry {
                                path: sf.path.display().to_string(),
                                message: format!("failed to create library item: {e}"),
                            });
                        }
                    }
                } else {
                    matched += 1;
                }
            }
            None => {
                unmatched.push(ScanUnmatchedFile {
                    path: sf.path.display().to_string(),
                    media_type,
                });
            }
        }
    }

    Ok(Json(ScanResult {
        matched,
        unmatched,
        errors,
    }))
}

#[derive(Debug, serde::Deserialize)]
pub struct ScanPathRequest {
    pub path: String,
}

pub async fn scan_path<S: AppContext>(
    State(state): State<S>,
    RequireAdmin(auth): RequireAdmin,
    Json(req): Json<ScanPathRequest>,
) -> Result<Json<ScanResult>, ApiError> {
    let path = PathBuf::from(&req.path);
    let user_id = auth.user.id;

    let probe_and_walk = tokio::task::spawn_blocking({
        let path = path.clone();
        move || -> Result<(Vec<ScanFileTyped>, Vec<ScanErrorEntry>), String> {
            if !path.exists() || !path.is_dir() {
                return Err("The file system path specified was not found.".into());
            }
            if std::fs::read_dir(&path).is_err() {
                return Err("The file system path specified was not found.".into());
            }
            Ok(scan_walk_all_types(&path))
        }
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking join error: {e}")))?;

    let scan_files = match probe_and_walk {
        Ok(r) => r,
        Err(msg) => return Err(ApiError::BadRequest(msg)),
    };

    let (files, scan_errors) = scan_files;

    if files.is_empty() && scan_errors.is_empty() {
        return Ok(Json(ScanResult {
            matched: 0,
            unmatched: vec![],
            errors: vec![],
        }));
    }

    let works = state
        .work_service()
        .list(
            user_id,
            livrarr_domain::services::WorkFilter {
                author_id: None,
                monitored: None,
                enrichment_status: None,
                media_type: None,
                sort_by: None,
                sort_dir: None,
            },
        )
        .await?;

    // Pre-compute normalized lookup HashMap to avoid O(files×works).
    let work_lookup2: std::collections::HashMap<(String, String), bool> = works
        .iter()
        .map(|w| {
            (
                (
                    normalize_for_matching(&w.title),
                    normalize_for_matching(&w.author_name),
                ),
                true,
            )
        })
        .collect();

    let mut matched: i64 = 0;
    let mut unmatched = Vec::new();
    let mut errors: Vec<ScanErrorEntry> = scan_errors;

    for sf in &files {
        let relative = match sf.path.strip_prefix(&path) {
            Ok(r) => r,
            Err(_) => {
                errors.push(ScanErrorEntry {
                    path: sf.path.display().to_string(),
                    message: "path not within scan directory".into(),
                });
                continue;
            }
        };

        let components: Vec<&str> = relative
            .components()
            .map(|c| c.as_os_str().to_str().unwrap_or(""))
            .collect();

        let parsed = if components.len() >= 2 {
            let stem = std::path::Path::new(components.last().unwrap_or(&""))
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if components.len() == 2 {
                Some((components[0].to_string(), stem.to_string()))
            } else {
                Some((components[0].to_string(), components[1].to_string()))
            }
        } else {
            None
        };

        match parsed {
            Some((author, title)) => {
                let norm_title = normalize_for_matching(&title);
                let norm_author = normalize_for_matching(&author);

                if work_lookup2.contains_key(&(norm_title, norm_author)) {
                    matched += 1;
                } else {
                    unmatched.push(ScanUnmatchedFile {
                        path: sf.path.display().to_string(),
                        media_type: sf.media_type,
                    });
                }
            }
            None => {
                unmatched.push(ScanUnmatchedFile {
                    path: sf.path.display().to_string(),
                    media_type: sf.media_type,
                });
            }
        }
    }

    Ok(Json(ScanResult {
        matched,
        unmatched,
        errors,
    }))
}

// ---------------------------------------------------------------------------
// Scan Helpers
// ---------------------------------------------------------------------------

const SCAN_MAX_DEPTH: usize = 20;
const SCAN_MAX_ENTRIES: usize = 100_000;

struct ScanFile {
    path: PathBuf,
}

struct ScanFileTyped {
    path: PathBuf,
    media_type: MediaType,
}

fn scan_walk_all_types(dir: &std::path::Path) -> (Vec<ScanFileTyped>, Vec<ScanErrorEntry>) {
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let mut entries_traversed = 0usize;

    if !dir.exists() {
        return (files, errors);
    }

    scan_walk_all_recursive(dir, &mut files, &mut errors, 0, &mut entries_traversed);
    (files, errors)
}

fn scan_walk_all_recursive(
    dir: &std::path::Path,
    files: &mut Vec<ScanFileTyped>,
    errors: &mut Vec<ScanErrorEntry>,
    depth: usize,
    entries_traversed: &mut usize,
) {
    if depth > SCAN_MAX_DEPTH {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            errors.push(ScanErrorEntry {
                path: dir.display().to_string(),
                message: format!("permission denied or read error: {e}"),
            });
            return;
        }
    };

    for entry in entries {
        *entries_traversed += 1;
        if *entries_traversed > SCAN_MAX_ENTRIES {
            return;
        }

        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                errors.push(ScanErrorEntry {
                    path: dir.display().to_string(),
                    message: format!("dir entry error: {e}"),
                });
                continue;
            }
        };

        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if name.starts_with('.') {
            continue;
        }

        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        if ft.is_symlink() {
            continue;
        }

        if ft.is_dir() {
            scan_walk_all_recursive(&path, files, errors, depth + 1, entries_traversed);
        } else if ft.is_file() {
            if let Some(media_type) = classify_file(&path) {
                files.push(ScanFileTyped { path, media_type });
            }
        }
    }
}

fn scan_walk_user_dir(
    user_dir: &std::path::Path,
    media_type: MediaType,
) -> (Vec<ScanFile>, Vec<ScanErrorEntry>) {
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let mut entries_traversed = 0usize;

    if !user_dir.exists() {
        return (files, errors);
    }

    scan_walk_recursive(
        user_dir,
        media_type,
        &mut files,
        &mut errors,
        0,
        &mut entries_traversed,
    );
    (files, errors)
}

/// Scan old-layout files in root, skipping top-level directories whose names
/// are purely numeric (those are user_id directories from the new tenant layout).
fn scan_walk_user_dir_skip_numeric_dirs(
    root_dir: &std::path::Path,
    media_type: MediaType,
) -> (Vec<ScanFile>, Vec<ScanErrorEntry>) {
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let mut entries_traversed = 0usize;

    if !root_dir.exists() {
        return (files, errors);
    }

    let entries = match std::fs::read_dir(root_dir) {
        Ok(e) => e,
        Err(e) => {
            errors.push(ScanErrorEntry {
                path: root_dir.display().to_string(),
                message: format!("permission denied or read error: {e}"),
            });
            return (files, errors);
        }
    };

    for entry in entries {
        entries_traversed += 1;
        if entries_traversed > SCAN_MAX_ENTRIES {
            break;
        }

        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                errors.push(ScanErrorEntry {
                    path: root_dir.display().to_string(),
                    message: format!("dir entry error: {e}"),
                });
                continue;
            }
        };

        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if name.starts_with('.') {
            continue;
        }

        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        if ft.is_symlink() {
            continue;
        }

        if ft.is_dir() {
            // Skip numeric directories (user_id dirs from new layout)
            if name.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            scan_walk_recursive(
                &path,
                media_type,
                &mut files,
                &mut errors,
                1,
                &mut entries_traversed,
            );
        } else if ft.is_file() {
            if let Some(file_mt) = classify_file(&path) {
                if file_mt == media_type {
                    files.push(ScanFile { path });
                }
            }
        }
    }

    (files, errors)
}

fn scan_walk_recursive(
    dir: &std::path::Path,
    root_media_type: MediaType,
    files: &mut Vec<ScanFile>,
    errors: &mut Vec<ScanErrorEntry>,
    depth: usize,
    entries_traversed: &mut usize,
) {
    if depth > SCAN_MAX_DEPTH {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            errors.push(ScanErrorEntry {
                path: dir.display().to_string(),
                message: format!("permission denied or read error: {e}"),
            });
            return;
        }
    };

    for entry in entries {
        *entries_traversed += 1;
        if *entries_traversed > SCAN_MAX_ENTRIES {
            return;
        }

        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                errors.push(ScanErrorEntry {
                    path: dir.display().to_string(),
                    message: format!("dir entry error: {e}"),
                });
                continue;
            }
        };

        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        if name.starts_with('.') {
            continue;
        }

        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(e) => {
                errors.push(ScanErrorEntry {
                    path: path.display().to_string(),
                    message: format!("file_type error: {e}"),
                });
                continue;
            }
        };

        if ft.is_symlink() {
            continue;
        }

        if ft.is_dir() {
            scan_walk_recursive(
                &path,
                root_media_type,
                files,
                errors,
                depth + 1,
                entries_traversed,
            );
        } else if ft.is_file() {
            if let Some(file_mt) = classify_file(&path) {
                if file_mt == root_media_type {
                    files.push(ScanFile { path });
                }
            }
        }
    }
}
