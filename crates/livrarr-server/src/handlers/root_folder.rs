use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::Json;

use crate::middleware::RequireAdmin;
use crate::state::AppState;
use crate::{
    ApiError, CreateRootFolderRequest, MediaType, RootFolderResponse, ScanErrorEntry, ScanResult,
    ScanUnmatchedFile,
};
use livrarr_db::{CreateLibraryItemDbRequest, LibraryItemDb, RootFolderDb, WorkDb};
use livrarr_domain::{classify_file, normalize_for_matching};

fn disk_space(path: &str) -> (Option<i64>, Option<i64>) {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let c_path = match CString::new(path) {
        Ok(p) => p,
        Err(_) => return (None, None),
    };

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

fn to_response(rf: livrarr_domain::RootFolder) -> RootFolderResponse {
    let (free_space, total_space) = disk_space(&rf.path);
    RootFolderResponse {
        id: rf.id,
        path: rf.path,
        media_type: rf.media_type,
        free_space,
        total_space,
    }
}

/// GET /api/v1/rootfolder
pub async fn list(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<Json<Vec<RootFolderResponse>>, ApiError> {
    let folders = state.db.list_root_folders().await?;
    Ok(Json(folders.into_iter().map(to_response).collect()))
}

/// POST /api/v1/rootfolder
pub async fn create(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Json(req): Json<CreateRootFolderRequest>,
) -> Result<Json<RootFolderResponse>, ApiError> {
    if req.path.is_empty() {
        return Err(ApiError::BadRequest("path is required".into()));
    }
    let rf = state
        .db
        .create_root_folder(&req.path, req.media_type)
        .await?;
    Ok(Json(to_response(rf)))
}

/// DELETE /api/v1/rootfolder/:id
pub async fn delete(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    state.db.delete_root_folder(id).await?;
    Ok(())
}

/// POST /api/v1/rootfolder/:id/scan
///
/// Satisfies: SCAN-001, SCAN-002, SCAN-003, SCAN-004, SCAN-005
pub async fn scan(
    State(state): State<AppState>,
    RequireAdmin(auth): RequireAdmin,
    Path(id): Path<i64>,
) -> Result<Json<ScanResult>, ApiError> {
    let rf = state.db.get_root_folder(id).await?;
    let user_id = auth.user.id;

    // Check root folder path exists (P0 from review: path gone at scan time).
    let root_path = PathBuf::from(&rf.path);
    if !root_path.exists() {
        return Err(ApiError::BadRequest(
            "root folder path does not exist".into(),
        ));
    }

    let media_type = rf.media_type;

    // Walk root folder directly (no user_id subdirectory).
    let scan_files = tokio::task::spawn_blocking({
        let scan_dir = root_path.clone();
        move || scan_walk_user_dir(&scan_dir, media_type)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking join error: {e}")))?;

    let (files, scan_errors) = scan_files;

    if files.is_empty() && scan_errors.is_empty() {
        return Ok(Json(ScanResult {
            matched: 0,
            unmatched: vec![],
            errors: vec![],
        }));
    }

    // Load user's works for matching.
    let works = state.db.list_works(user_id).await?;

    // Load existing library items to avoid duplicates.
    let existing_items = state.db.list_library_items(user_id).await?;

    let mut matched: i64 = 0;
    let mut unmatched = Vec::new();
    let mut errors: Vec<ScanErrorEntry> = scan_errors;

    for sf in &files {
        // Parse path relative to root folder for author/title extraction and DB storage.
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

        let components: Vec<&str> = relative_to_root
            .components()
            .map(|c| c.as_os_str().to_str().unwrap_or(""))
            .collect();
        let depth = components.len();

        // SCAN-005: depth handling.
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
                // SCAN-002: strip extension, normalize for matching.
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
                // SCAN-003: {author}/{title}/{files...}
                let author = components[0];
                let title_dir = components[1];
                (author.to_string(), title_dir.to_string())
            }
        };

        // Normalize both sides for matching (SCAN-002/003).
        let norm_title = normalize_for_matching(&parsed_title);
        let norm_author = normalize_for_matching(&parsed_author);

        // Find matching work.
        let matched_work = works.iter().find(|w| {
            normalize_for_matching(&w.title) == norm_title
                && normalize_for_matching(&w.author_name) == norm_author
        });

        match matched_work {
            Some(work) => {
                // SCAN-004: create library item if not already tracked.
                // Store path relative to root folder (includes user_id) — matches import pipeline.
                let relative_str = relative_to_root.to_string_lossy().to_string();
                let already_tracked = existing_items
                    .iter()
                    .any(|li| li.root_folder_id == rf.id && li.path == relative_str);

                if !already_tracked {
                    let file_size = sf.path.metadata().map(|m| m.len() as i64).unwrap_or(0);
                    match state
                        .db
                        .create_library_item(CreateLibraryItemDbRequest {
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
                    matched += 1; // Already tracked counts as matched.
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

/// POST /api/v1/unmapped/scan — scan arbitrary path for unmapped files
pub async fn scan_path(
    State(state): State<AppState>,
    RequireAdmin(auth): RequireAdmin,
    Json(req): Json<ScanPathRequest>,
) -> Result<Json<ScanResult>, ApiError> {
    let path = PathBuf::from(&req.path);
    let user_id = auth.user.id;

    // All directory probing + walking happens on the blocking pool — no sync fs
    // calls on the async runtime.
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

    let works = state.db.list_works(user_id).await?;

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

        // Try to extract author/title from path structure.
        let parsed = if components.len() >= 2 {
            let stem = std::path::Path::new(components.last().unwrap_or(&""))
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            // Try Author/Title.ext or Author/Title/file.ext
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

                let matched_work = works.iter().find(|w| {
                    normalize_for_matching(&w.title) == norm_title
                        && normalize_for_matching(&w.author_name) == norm_author
                });

                if matched_work.is_some() {
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

#[derive(Debug, serde::Deserialize)]
pub struct ScanPathRequest {
    pub path: String,
}

// ---------------------------------------------------------------------------
// Scan Helpers
// ---------------------------------------------------------------------------

/// Maximum recursion depth for directory scans.
const SCAN_MAX_DEPTH: usize = 20;
/// Maximum entries traversed before stopping a scan.
const SCAN_MAX_ENTRIES: usize = 100_000;

struct ScanFile {
    path: PathBuf,
}

struct ScanFileTyped {
    path: PathBuf,
    media_type: MediaType,
}

/// Walk any directory recursively, collecting all recognized media files regardless of type.
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

/// Walk user directory recursively, collecting classifiable files.
/// Returns (files, errors).
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

        // Skip hidden files/dirs.
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

        // Skip symlinks.
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
            // Only include files matching root folder's media type.
            if let Some(file_mt) = classify_file(&path) {
                if file_mt == root_media_type {
                    files.push(ScanFile { path });
                }
            }
        }
    }
}
