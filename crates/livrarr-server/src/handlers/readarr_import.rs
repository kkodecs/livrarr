//! Readarr Library Import handler.
//!
//! Endpoints: connect, preview, start, progress, history, undo.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use axum::extract::{Path as AxumPath, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use crate::readarr_client::{self, RdAuthor, RdBook, RdBookFile, RdRootFolder, ReadarrClient};
use crate::state::AppState;
use crate::{ApiError, AuthContext};
use livrarr_db::{
    AuthorDb, CreateAuthorDbRequest, CreateImportDbRequest, CreateLibraryItemDbRequest,
    CreateWorkDbRequest, ImportDb, LibraryItemDb, RootFolderDb, WorkDb,
};
use livrarr_domain::{
    derive_sort_name, normalize_for_matching, sanitize_path_component, Import, MediaType,
};

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Outcome of a single-file undo operation. Computed on a blocking thread so
/// log formatting and DB updates can run on the async side.
enum UndoOutcome {
    NotFound,
    Deleted,
    DeleteFailed(String),
    SizeMismatch { expected: i64, actual: u64 },
    StatFailed(String),
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectRequest {
    pub url: String,
    pub api_key: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectResponse {
    pub root_folders: Vec<RootFolderInfo>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RootFolderInfo {
    pub id: i64,
    pub name: Option<String>,
    pub path: String,
    pub accessible: Option<bool>,
    pub free_space: Option<i64>,
    pub total_space: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRequest {
    pub url: String,
    pub api_key: String,
    pub readarr_root_folder_id: i64,
    pub livrarr_root_folder_id: i64,
    #[serde(default)]
    pub files_only: bool,
    /// Path as seen inside Readarr's container (e.g. "/books").
    pub container_path: Option<String>,
    /// Equivalent path accessible to Livrarr (e.g. "/mnt/data/books").
    pub host_path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResponse {
    pub authors_to_create: i64,
    pub authors_existing: i64,
    pub works_to_create: i64,
    pub works_existing: i64,
    pub files_to_import: i64,
    pub files_to_skip: i64,
    pub skipped_items: Vec<SkippedItem>,
    pub import_files: Vec<PreviewFileItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedItem {
    pub title: String,
    pub author: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewFileItem {
    pub title: String,
    pub author: String,
    pub path: String,
    pub media_type: String,
    pub work_status: String, // "new" | "existing"
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRequest {
    pub url: String,
    pub api_key: String,
    pub readarr_root_folder_id: i64,
    pub livrarr_root_folder_id: i64,
    #[serde(default)]
    pub files_only: bool,
    pub container_path: Option<String>,
    pub host_path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartResponse {
    pub import_id: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProgress {
    pub running: bool,
    pub import_id: Option<String>,
    pub phase: String,
    pub authors_processed: i64,
    pub authors_total: i64,
    pub works_processed: i64,
    pub works_total: i64,
    pub files_processed: i64,
    pub files_total: i64,
    pub files_skipped: i64,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportHistoryResponse {
    pub imports: Vec<ImportRecord>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRecord {
    pub id: String,
    pub source: String,
    pub status: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub authors_created: i64,
    pub works_created: i64,
    pub files_imported: i64,
    pub files_skipped: i64,
    pub source_url: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoResponse {
    pub files_deleted: i64,
    pub files_skipped: i64,
    pub works_deleted: i64,
    pub authors_deleted: i64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn import_to_record(imp: &Import) -> ImportRecord {
    ImportRecord {
        id: imp.id.clone(),
        source: imp.source.clone(),
        status: imp.status.clone(),
        started_at: imp.started_at.to_rfc3339(),
        completed_at: imp.completed_at.map(|d| d.to_rfc3339()),
        authors_created: imp.authors_created,
        works_created: imp.works_created,
        files_imported: imp.files_imported,
        files_skipped: imp.files_skipped,
        source_url: imp.source_url.clone(),
    }
}

/// Determine media type from Readarr quality ID, falling back to file extension.
fn resolve_media_type(quality_id: Option<i32>, path: &str) -> Option<MediaType> {
    if let Some(qid) = quality_id {
        if let Some(mt_str) = readarr_client::quality_to_media_type(qid) {
            return match mt_str {
                "ebook" => Some(MediaType::Ebook),
                "audiobook" => Some(MediaType::Audiobook),
                _ => None,
            };
        }
    }
    // Fallback to extension.
    if let Some(mt_str) = readarr_client::media_type_from_extension(path) {
        return match mt_str {
            "ebook" => Some(MediaType::Ebook),
            "audiobook" => Some(MediaType::Audiobook),
            _ => None,
        };
    }
    None
}

/// Extract quality ID from Readarr book file.
fn extract_quality_id(bf: &RdBookFile) -> Option<i32> {
    bf.quality.as_ref()?.quality.as_ref().map(|q| q.id)
}

/// Parse series title: "Series Name #1.5; Other Series #2" -> (series_name, series_position)
/// Only uses the first semicolon-delimited segment.
fn parse_series_title(series_title: &str) -> (Option<String>, Option<f64>) {
    let segment = series_title
        .split(';')
        .next()
        .unwrap_or(series_title)
        .trim();
    if segment.is_empty() {
        return (None, None);
    }

    let re = regex::Regex::new(r"^(.*?)(?:\s+#([\d.]+))?$").unwrap();
    if let Some(caps) = re.captures(segment) {
        let name = caps.get(1).map(|m| m.as_str().trim().to_string());
        let pos = caps.get(2).and_then(|m| m.as_str().parse::<f64>().ok());
        let name = name.filter(|n| !n.is_empty());
        (name, pos)
    } else {
        (Some(segment.to_string()), None)
    }
}

/// Extract year from a date string like "2024-01-15T00:00:00Z" or "2024".
fn extract_year(date_str: &str) -> Option<i32> {
    date_str.get(..4)?.parse::<i32>().ok()
}

/// Get the cover URL from Readarr images, preferring remote_url for covers.
fn extract_cover_url(images: &Option<Vec<readarr_client::RdImage>>) -> Option<String> {
    let imgs = images.as_ref()?;
    // Prefer a "cover" type image.
    for img in imgs {
        if img.cover_type.as_deref() == Some("cover") {
            if let Some(ref url) = img.remote_url {
                if !url.is_empty() {
                    return Some(url.clone());
                }
            }
            if let Some(ref url) = img.url {
                if !url.is_empty() {
                    return Some(url.clone());
                }
            }
        }
    }
    // Fallback: first image with a URL.
    for img in imgs {
        if let Some(ref url) = img.remote_url {
            if !url.is_empty() {
                return Some(url.clone());
            }
        }
        if let Some(ref url) = img.url {
            if !url.is_empty() {
                return Some(url.clone());
            }
        }
    }
    None
}

/// Build destination path.
///
/// Ebooks:     `{root}/{user_id}/{Author}/{Title}.{ext}`
/// Audiobooks: `{root}/{user_id}/{Author}/{Title}/{filename}`
fn build_dest_path(
    root: &str,
    user_id: i64,
    author_name: &str,
    title: &str,
    source_path: &str,
    media_type: MediaType,
) -> PathBuf {
    let author_dir = sanitize_path_component(author_name, "Unknown Author");
    let title_dir = sanitize_path_component(title, "Unknown Title");
    let base = PathBuf::from(root)
        .join(user_id.to_string())
        .join(author_dir);

    match media_type {
        MediaType::Audiobook => {
            let filename = Path::new(source_path)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    let ext = Path::new(source_path)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("bin");
                    format!("{title_dir}.{ext}")
                });
            base.join(title_dir).join(filename)
        }
        MediaType::Ebook => {
            let ext = Path::new(source_path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("bin");
            base.join(format!("{title_dir}.{ext}"))
        }
    }
}

/// Canonicalize source path and verify it's under the expected root.
fn validate_source_path(source: &str, readarr_root: &str) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(source)
        .map_err(|e| format!("cannot canonicalize source path: {e}"))?;
    let root_canonical = std::fs::canonicalize(readarr_root)
        .map_err(|e| format!("cannot canonicalize readarr root: {e}"))?;
    if !canonical.starts_with(&root_canonical) {
        return Err(format!(
            "source path {canonical:?} is not under readarr root {root_canonical:?}"
        ));
    }
    Ok(canonical)
}

/// Translate a path from container-space to host-space using the optional
/// container_path / host_path mapping. Returns the path unchanged if either
/// side is absent or the path doesn't start with container_path.
fn apply_path_translation(
    path: &str,
    container_path: Option<&str>,
    host_path: Option<&str>,
) -> String {
    match (container_path, host_path) {
        (Some(cp), Some(hp)) if !cp.is_empty() && !hp.is_empty() => {
            let cp = cp.trim_end_matches('/');
            let hp = hp.trim_end_matches('/');
            if let Some(suffix) = path.strip_prefix(cp) {
                format!("{}{}", hp, suffix)
            } else {
                path.to_string()
            }
        }
        _ => path.to_string(),
    }
}

/// Try hardlink, fall back to copy with temp+rename.
fn materialize_file(source: &Path, dest: &Path) -> Result<(), String> {
    // Ensure parent directory exists.
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {e}"))?;
    }

    // Try hardlink first.
    if std::fs::hard_link(source, dest).is_ok() {
        return Ok(());
    }

    // Fallback: copy to temp, verify size, atomic rename.
    let temp = dest.with_extension(format!("tmp.{}", uuid::Uuid::new_v4()));
    match std::fs::copy(source, &temp) {
        Ok(copied) => {
            let source_size = std::fs::metadata(source)
                .map_err(|e| format!("cannot stat source: {e}"))?
                .len();
            if copied != source_size {
                let _ = std::fs::remove_file(&temp);
                return Err(format!(
                    "copy size mismatch: copied {copied} vs source {source_size}"
                ));
            }
            // fsync the temp file before atomic rename — ensures data is durable.
            if let Ok(f) = std::fs::File::open(&temp) {
                let _ = f.sync_all();
            }
            std::fs::rename(&temp, dest).map_err(|e| {
                let _ = std::fs::remove_file(&temp);
                format!("rename failed: {e}")
            })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            Err(format!("copy failed: {e}"))
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /api/v1/import/readarr/connect
pub async fn connect(
    State(state): State<AppState>,
    _auth: AuthContext,
    Json(req): Json<ConnectRequest>,
) -> Result<Json<ConnectResponse>, ApiError> {
    if req.url.is_empty() {
        return Err(ApiError::BadRequest("url is required".into()));
    }
    if req.api_key.is_empty() {
        return Err(ApiError::BadRequest("apiKey is required".into()));
    }

    let client = ReadarrClient::new(&req.url, &req.api_key, state.http_client.inner().clone());
    let folders = client
        .root_folders()
        .await
        .map_err(|e| ApiError::BadGateway(format!("Failed to connect to Readarr: {e}")))?;

    let root_folders = folders
        .into_iter()
        .map(|f| RootFolderInfo {
            id: f.id,
            name: f.name,
            path: f.path,
            accessible: f.accessible,
            free_space: f.free_space,
            total_space: f.total_space,
        })
        .collect();

    Ok(Json(ConnectResponse { root_folders }))
}

/// POST /api/v1/import/readarr/preview
pub async fn preview(
    State(state): State<AppState>,
    auth: AuthContext,
    Json(req): Json<PreviewRequest>,
) -> Result<Json<PreviewResponse>, ApiError> {
    let user_id = auth.user.id;

    let client = ReadarrClient::new(&req.url, &req.api_key, state.http_client.inner().clone());

    let (authors, books, book_files, rd_folders) = fetch_all_readarr_data(&client).await?;

    let _readarr_root = rd_folders
        .iter()
        .find(|f| f.id == req.readarr_root_folder_id)
        .map(|f| f.path.clone())
        .ok_or_else(|| ApiError::BadRequest("Invalid Readarr root folder ID".into()))?;

    let livrarr_root = state.db.get_root_folder(req.livrarr_root_folder_id).await?;

    let author_map: HashMap<i64, &RdAuthor> = authors.iter().map(|a| (a.id, a)).collect();
    let mut book_files_by_book: HashMap<i64, Vec<&RdBookFile>> = HashMap::new();
    for bf in &book_files {
        book_files_by_book.entry(bf.book_id).or_default().push(bf);
    }

    let existing_authors = state.db.list_authors(user_id).await?;
    let existing_works = state.db.list_works(user_id).await?;

    let mut skipped_items: Vec<SkippedItem> = Vec::new();
    let mut import_files: Vec<PreviewFileItem> = Vec::new();
    let mut authors_to_create = 0i64;
    let mut works_to_create = 0i64;
    let mut works_existing = 0i64;
    let mut files_to_skip = 0i64;

    let mut author_names_seen: HashMap<String, bool> = HashMap::new();
    for a in &existing_authors {
        author_names_seen.insert(normalize_for_matching(&a.name), true);
    }

    for book in &books {
        let author_name = author_map
            .get(&book.author_id)
            .and_then(|a| a.author_name.as_deref())
            .unwrap_or("");
        let title = book.title.as_deref().unwrap_or("");

        if author_name.is_empty() {
            skipped_items.push(SkippedItem {
                title: title.to_string(),
                author: String::new(),
                reason: "No author".to_string(),
            });
            continue;
        }

        // files_only mode: skip books that have no files in Readarr at all.
        if req.files_only && !book_files_by_book.contains_key(&book.id) {
            continue;
        }

        let norm_author = normalize_for_matching(author_name);
        if !author_names_seen.contains_key(&norm_author) {
            author_names_seen.insert(norm_author.clone(), false);
            authors_to_create += 1;
        }

        // Compute work status before file loop so we can annotate each file.
        let edition = book.monitored_edition();
        let isbn = edition
            .and_then(|e| e.isbn13.as_deref())
            .filter(|s| !s.is_empty());
        let asin = edition
            .and_then(|e| e.asin.as_deref())
            .filter(|s| !s.is_empty());
        let year = book.release_date.as_deref().and_then(extract_year);
        let norm_title = normalize_for_matching(title);

        let is_existing = if let Some(isbn_val) = isbn {
            existing_works
                .iter()
                .any(|w| w.isbn_13.as_deref() == Some(isbn_val))
        } else if let Some(asin_val) = asin {
            existing_works
                .iter()
                .any(|w| w.asin.as_deref() == Some(asin_val))
        } else {
            existing_works.iter().any(|w| {
                normalize_for_matching(&w.author_name) == norm_author
                    && normalize_for_matching(&w.title) == norm_title
                    && w.year == year
            })
        };

        let work_status = if is_existing { "existing" } else { "new" };
        if is_existing {
            works_existing += 1;
        } else {
            works_to_create += 1;
        }

        let files = book_files_by_book.get(&book.id);
        let file_list: Vec<&&RdBookFile> = files.map(|f| f.iter().collect()).unwrap_or_default();

        let audiobook_count = file_list
            .iter()
            .filter(|f| {
                let qid = extract_quality_id(f);
                resolve_media_type(qid, &f.path) == Some(MediaType::Audiobook)
            })
            .count();

        for f in &file_list {
            let qid = extract_quality_id(f);
            let mt = match resolve_media_type(qid, &f.path) {
                None => {
                    files_to_skip += 1;
                    skipped_items.push(SkippedItem {
                        title: title.to_string(),
                        author: author_name.to_string(),
                        reason: format!("Unknown format: {}", f.path),
                    });
                    continue;
                }
                Some(mt) => mt,
            };
            // For multi-part audiobooks, use the source filename stem to avoid path collisions.
            let effective_title = if mt == MediaType::Audiobook && audiobook_count > 1 {
                Path::new(&f.path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(title)
                    .to_string()
            } else {
                title.to_string()
            };
            let dest = build_dest_path(
                &livrarr_root.path,
                user_id,
                author_name,
                &effective_title,
                &f.path,
                mt,
            );
            let dest_exists = {
                let d = dest.clone();
                tokio::task::spawn_blocking(move || d.exists())
                    .await
                    .unwrap_or(false)
            };
            if dest_exists {
                files_to_skip += 1;
                skipped_items.push(SkippedItem {
                    title: title.to_string(),
                    author: author_name.to_string(),
                    reason: "Destination already exists".to_string(),
                });
            } else {
                import_files.push(PreviewFileItem {
                    title: title.to_string(),
                    author: author_name.to_string(),
                    path: f.path.clone(),
                    media_type: match mt {
                        MediaType::Ebook => "ebook".to_string(),
                        MediaType::Audiobook => "audiobook".to_string(),
                    },
                    work_status: work_status.to_string(),
                });
            }
        }
    }

    let authors_existing = authors
        .iter()
        .filter(|a| {
            let name = a.author_name.as_deref().unwrap_or("");
            let norm = normalize_for_matching(name);
            author_names_seen.get(&norm) == Some(&true)
        })
        .count() as i64;

    let files_to_import = import_files.len() as i64;

    Ok(Json(PreviewResponse {
        authors_to_create,
        authors_existing,
        works_to_create,
        works_existing,
        files_to_import,
        files_to_skip,
        skipped_items,
        import_files,
    }))
}

/// POST /api/v1/import/readarr/start
pub async fn start(
    State(state): State<AppState>,
    auth: AuthContext,
    Json(req): Json<StartRequest>,
) -> Result<Json<StartResponse>, ApiError> {
    let user_id = auth.user.id;
    let import_id = uuid::Uuid::new_v4().to_string();

    state
        .db
        .create_import(CreateImportDbRequest {
            id: import_id.clone(),
            user_id,
            source: "readarr".to_string(),
            source_url: Some(req.url.clone()),
            target_root_folder_id: Some(req.livrarr_root_folder_id),
        })
        .await
        .map_err(|e| match e {
            livrarr_domain::DbError::Constraint { .. } => ApiError::Conflict {
                reason: "An import is already running for this user".to_string(),
            },
            other => ApiError::from(other),
        })?;

    {
        let mut prog = state.readarr_import_progress.lock().await;
        *prog = ImportProgress {
            running: true,
            import_id: Some(import_id.clone()),
            phase: "fetching".to_string(),
            ..Default::default()
        };
    }

    let state2 = state.clone();
    let import_id2 = import_id.clone();
    let url = req.url.clone();
    let api_key = req.api_key.clone();
    let readarr_root_id = req.readarr_root_folder_id;
    let livrarr_root_id = req.livrarr_root_folder_id;
    let files_only = req.files_only;
    let container_path = req.container_path.clone();
    let host_path = req.host_path.clone();

    tokio::spawn(async move {
        if let Err(e) = run_import(
            state2.clone(),
            user_id,
            &import_id2,
            &url,
            &api_key,
            readarr_root_id,
            livrarr_root_id,
            files_only,
            container_path,
            host_path,
        )
        .await
        {
            error!(import_id = %import_id2, "Readarr import failed: {e}");
            let _ = state2.db.update_import_status(&import_id2, "failed").await;
        }

        let mut prog = state2.readarr_import_progress.lock().await;
        prog.running = false;
        prog.phase = "done".to_string();
    });

    Ok(Json(StartResponse { import_id }))
}

/// GET /api/v1/import/readarr/progress
pub async fn progress(
    State(state): State<AppState>,
    _auth: AuthContext,
) -> Result<Json<ImportProgress>, ApiError> {
    let prog = state.readarr_import_progress.lock().await;
    Ok(Json(prog.clone()))
}

/// GET /api/v1/import/readarr/history
pub async fn history(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<ImportHistoryResponse>, ApiError> {
    let imports = state.db.list_imports(auth.user.id).await?;
    let records = imports.iter().map(import_to_record).collect();
    Ok(Json(ImportHistoryResponse { imports: records }))
}

/// DELETE /api/v1/import/readarr/{import_id}
pub async fn undo(
    State(state): State<AppState>,
    auth: AuthContext,
    AxumPath(import_id): AxumPath<String>,
) -> Result<Json<UndoResponse>, ApiError> {
    let user_id = auth.user.id;

    // Verify import exists and belongs to user.
    let imp = state
        .db
        .get_import(&import_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    if imp.user_id != user_id {
        return Err(ApiError::Forbidden);
    }

    if imp.status == "running" {
        return Err(ApiError::BadRequest(
            "Cannot undo a running import. Wait for it to complete.".into(),
        ));
    }

    // 1. Query all library items with this import_id.
    let items = state.db.list_library_items_by_import(&import_id).await?;

    // Look up root folder path for full-path resolution (library_items store relative paths).
    let root_folder_path: Option<String> = if let Some(rf_id) = imp.target_root_folder_id {
        state.db.get_root_folder(rf_id).await.ok().map(|rf| rf.path)
    } else {
        None
    };

    let mut files_deleted = 0i64;
    let mut files_skipped = 0i64;

    // 2. For each item, try to delete the destination file.
    for item in &items {
        let full_path = if let Some(ref root) = root_folder_path {
            PathBuf::from(root).join(&item.path)
        } else {
            PathBuf::from(&item.path)
        };
        let expected_size = item.file_size;
        let fp = full_path.clone();
        let undo_outcome: UndoOutcome = tokio::task::spawn_blocking(move || {
            let path = fp.as_path();
            if !path.exists() {
                return UndoOutcome::NotFound;
            }
            match std::fs::metadata(path) {
                Ok(meta) => {
                    if meta.len() as i64 == expected_size {
                        match std::fs::remove_file(path) {
                            Ok(()) => UndoOutcome::Deleted,
                            Err(e) => UndoOutcome::DeleteFailed(e.to_string()),
                        }
                    } else {
                        UndoOutcome::SizeMismatch {
                            expected: expected_size,
                            actual: meta.len(),
                        }
                    }
                }
                Err(e) => UndoOutcome::StatFailed(e.to_string()),
            }
        })
        .await
        .unwrap_or_else(|e| UndoOutcome::StatFailed(format!("join: {e}")));
        match undo_outcome {
            UndoOutcome::Deleted => {
                files_deleted += 1;
                info!(path = %item.path, "Undo: deleted file");
            }
            UndoOutcome::DeleteFailed(e) => {
                warn!(path = %item.path, "Undo: failed to delete file: {e}");
                files_skipped += 1;
            }
            UndoOutcome::SizeMismatch { expected, actual } => {
                warn!(
                    path = %item.path,
                    expected = expected,
                    actual = actual,
                    "Undo: skipping file with size mismatch"
                );
                files_skipped += 1;
            }
            UndoOutcome::StatFailed(e) => {
                warn!(path = %item.path, "Undo: cannot stat file: {e}");
                files_skipped += 1;
            }
            UndoOutcome::NotFound => {}
        }

        // 3. Delete the library item DB row regardless of file deletion outcome.
        if let Err(e) = state.db.delete_library_item_by_id(item.id).await {
            warn!(id = item.id, "Undo: failed to delete library item: {e}");
        }
    }

    // 4. Delete orphan works (import_id match, zero library items).
    let works_deleted = state
        .db
        .delete_orphan_works_by_import(&import_id)
        .await
        .unwrap_or(0);

    // 5. Delete orphan authors (import_id match, zero works).
    let authors_deleted = state
        .db
        .delete_orphan_authors_by_import(&import_id)
        .await
        .unwrap_or(0);

    // 6. Mark import as undone.
    state.db.update_import_status(&import_id, "undone").await?;

    Ok(Json(UndoResponse {
        files_deleted,
        files_skipped,
        works_deleted,
        authors_deleted,
    }))
}

// ---------------------------------------------------------------------------
// Fetch helpers
// ---------------------------------------------------------------------------

async fn fetch_all_readarr_data(
    client: &ReadarrClient,
) -> Result<
    (
        Vec<RdAuthor>,
        Vec<RdBook>,
        Vec<RdBookFile>,
        Vec<RdRootFolder>,
    ),
    ApiError,
> {
    // Fetch all data before any mutations.
    let folders = client
        .root_folders()
        .await
        .map_err(|e| ApiError::BadGateway(format!("Readarr root folders: {e}")))?;
    let authors = client
        .authors()
        .await
        .map_err(|e| ApiError::BadGateway(format!("Readarr authors: {e}")))?;
    let books = client
        .books()
        .await
        .map_err(|e| ApiError::BadGateway(format!("Readarr books: {e}")))?;
    let author_ids: Vec<i64> = authors.iter().map(|a| a.id).collect();
    let file_results = futures::future::join_all(
        author_ids
            .iter()
            .map(|&aid| client.book_files_by_author(aid)),
    )
    .await;
    let mut book_files: Vec<RdBookFile> = Vec::new();
    for (aid, res) in author_ids.iter().zip(file_results) {
        match res {
            Ok(files) => book_files.extend(files),
            Err(e) => {
                return Err(ApiError::BadGateway(format!(
                    "Readarr book files (author {aid}): {e}"
                )));
            }
        }
    }

    Ok((authors, books, book_files, folders))
}

// ---------------------------------------------------------------------------
// Import execution
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)] // 11 params: state, user, import-id, url, key, 2 root-ids, flag, 2 path-translation opts
async fn run_import(
    state: AppState,
    user_id: i64,
    import_id: &str,
    url: &str,
    api_key: &str,
    readarr_root_id: i64,
    livrarr_root_id: i64,
    files_only: bool,
    container_path: Option<String>,
    host_path: Option<String>,
) -> Result<(), String> {
    // Phase 1: Fetch.
    let client = ReadarrClient::new(url, api_key, state.http_client.inner().clone());
    let (rd_authors, rd_books, rd_book_files, rd_folders) =
        fetch_all_readarr_data(&client)
            .await
            .map_err(|e| format!("fetch failed: {e}"))?;

    let readarr_root_raw = rd_folders
        .iter()
        .find(|f| f.id == readarr_root_id)
        .map(|f| f.path.clone())
        .ok_or_else(|| "Invalid Readarr root folder ID".to_string())?;
    // Translate the root folder path to the local equivalent if a mapping is configured.
    let readarr_root = apply_path_translation(
        &readarr_root_raw,
        container_path.as_deref(),
        host_path.as_deref(),
    );

    let livrarr_root = state
        .db
        .get_root_folder(livrarr_root_id)
        .await
        .map_err(|e| format!("get livrarr root folder: {e}"))?;

    // Build maps.
    let author_map: HashMap<i64, &RdAuthor> = rd_authors.iter().map(|a| (a.id, a)).collect();
    let mut book_files_by_book: HashMap<i64, Vec<&RdBookFile>> = HashMap::new();
    for bf in &rd_book_files {
        book_files_by_book.entry(bf.book_id).or_default().push(bf);
    }

    // When files_only: filter books to only those with files before processing.
    let active_book_ids: std::collections::HashSet<i64> = if files_only {
        book_files_by_book.keys().copied().collect()
    } else {
        rd_books.iter().map(|b| b.id).collect()
    };

    // Load existing data for dedup.
    let existing_authors = state
        .db
        .list_authors(user_id)
        .await
        .map_err(|e| format!("list authors: {e}"))?;

    // Update progress — totals reflect only active books.
    let active_books: Vec<&RdBook> = rd_books
        .iter()
        .filter(|b| active_book_ids.contains(&b.id))
        .collect();
    {
        let mut prog = state.readarr_import_progress.lock().await;
        prog.phase = "processing".to_string();
        prog.authors_total = rd_authors.len() as i64;
        prog.works_total = active_books.len() as i64;
        prog.files_total = rd_book_files
            .iter()
            .filter(|f| active_book_ids.contains(&f.book_id))
            .count() as i64;
    }

    // Phase 2: Process authors.
    // Map Readarr author ID -> Livrarr author ID.
    let mut rd_to_livrarr_author: HashMap<i64, i64> = HashMap::new();
    let mut authors_created = 0i64;

    for rd_author in &rd_authors {
        let name = rd_author.author_name.as_deref().unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }

        // files_only: skip authors with no books that have files.
        if files_only {
            let has_files = rd_books
                .iter()
                .filter(|b| b.author_id == rd_author.id)
                .any(|b| active_book_ids.contains(&b.id));
            if !has_files {
                let mut prog = state.readarr_import_progress.lock().await;
                prog.authors_processed += 1;
                continue;
            }
        }

        let norm = normalize_for_matching(name);

        // Conservative: only merge if exactly one name match.
        let matches: Vec<_> = existing_authors
            .iter()
            .filter(|a| normalize_for_matching(&a.name) == norm)
            .collect();

        let livrarr_author_id = if matches.len() == 1 {
            matches[0].id
        } else {
            // Create new author.
            let sort_name = rd_author
                .sort_name
                .as_deref()
                .map(|s| s.to_string())
                .unwrap_or_else(|| derive_sort_name(name));

            match state
                .db
                .create_author(CreateAuthorDbRequest {
                    user_id,
                    name: name.to_string(),
                    sort_name: Some(sort_name),
                    ol_key: None,
                    gr_key: rd_author.foreign_author_id.clone(),
                    hc_key: None,
                    import_id: Some(import_id.to_string()),
                })
                .await
            {
                Ok(a) => {
                    authors_created += 1;
                    a.id
                }
                Err(e) => {
                    warn!(name = %name, "Failed to create author: {e}");
                    let mut prog = state.readarr_import_progress.lock().await;
                    prog.errors.push(format!("Author '{name}': {e}"));
                    continue;
                }
            }
        };

        rd_to_livrarr_author.insert(rd_author.id, livrarr_author_id);

        {
            let mut prog = state.readarr_import_progress.lock().await;
            prog.authors_processed += 1;
        }
    }

    // Phase 3: Process books/works.
    // Map Readarr book ID -> Livrarr work ID.
    let mut rd_to_livrarr_work: HashMap<i64, i64> = HashMap::new();
    let mut works_created = 0i64;
    let mut files_imported = 0i64;
    let mut files_skipped = 0i64;

    // Refresh existing works to include newly created ones.
    let all_works = state
        .db
        .list_works(user_id)
        .await
        .map_err(|e| format!("list works after authors: {e}"))?;

    for rd_book in &active_books {
        let rd_author_id = rd_book.author_id;
        let author_name = author_map
            .get(&rd_author_id)
            .and_then(|a| a.author_name.as_deref())
            .unwrap_or("");
        let title = rd_book.title.as_deref().unwrap_or("").trim();

        // Skip books with no author.
        if author_name.is_empty() {
            let mut prog = state.readarr_import_progress.lock().await;
            prog.works_processed += 1;
            prog.errors
                .push(format!("Book '{title}': skipped (no author)"));
            continue;
        }

        let livrarr_author_id = rd_to_livrarr_author.get(&rd_author_id).copied();

        // Edition metadata.
        let edition = rd_book.monitored_edition();
        let isbn = edition
            .and_then(|e| e.isbn13.as_deref())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let asin = edition
            .and_then(|e| e.asin.as_deref())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let publisher = edition
            .and_then(|e| e.publisher.as_deref())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let language = edition
            .and_then(|e| e.language.as_deref())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let year = rd_book.release_date.as_deref().and_then(extract_year);

        // Dedup: ISBN/ASIN first, then author+title+year.
        let norm_title = normalize_for_matching(title);
        let norm_author = normalize_for_matching(author_name);

        let existing_work = if let Some(ref isbn_val) = isbn {
            all_works
                .iter()
                .find(|w| w.isbn_13.as_deref() == Some(isbn_val))
        } else if let Some(ref asin_val) = asin {
            all_works
                .iter()
                .find(|w| w.asin.as_deref() == Some(asin_val))
        } else {
            all_works.iter().find(|w| {
                normalize_for_matching(&w.author_name) == norm_author
                    && normalize_for_matching(&w.title) == norm_title
                    && w.year == year
            })
        };

        let work_id = if let Some(ew) = existing_work {
            // Merge into existing — only add library items, never update metadata.
            ew.id
        } else {
            // Create new work.
            let description = rd_book
                .overview
                .as_deref()
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    edition
                        .and_then(|e| e.overview.as_deref())
                        .filter(|s| !s.is_empty())
                })
                .map(|s| s.to_string());

            let (series_name, series_position) = rd_book
                .series_title
                .as_deref()
                .map(parse_series_title)
                .unwrap_or((None, None));

            let genres = rd_book.genres.clone();
            let page_count = rd_book
                .page_count
                .or_else(|| edition.and_then(|e| e.page_count));

            let rating = rd_book.ratings.as_ref().and_then(|r| r.value);
            let rating_count = rd_book.ratings.as_ref().and_then(|r| r.votes);

            let cover_url = extract_cover_url(&rd_book.images);

            // Check if this book has any files to determine monitor flags.
            let book_files_list = book_files_by_book.get(&rd_book.id);
            let has_ebook_file = book_files_list
                .map(|fs| {
                    fs.iter().any(|f| {
                        resolve_media_type(extract_quality_id(f), &f.path) == Some(MediaType::Ebook)
                    })
                })
                .unwrap_or(false);
            let has_audiobook_file = book_files_list
                .map(|fs| {
                    fs.iter().any(|f| {
                        resolve_media_type(extract_quality_id(f), &f.path)
                            == Some(MediaType::Audiobook)
                    })
                })
                .unwrap_or(false);

            // If no files, use Readarr monitored state.
            let monitor_ebook = has_ebook_file || rd_book.monitored.unwrap_or(false);
            let monitor_audiobook = has_audiobook_file;

            // Cleanup title + author at import-time so the locked identity
            // anchor stores canonical form (Readarr's titles are typically
            // clean but cleanup is cheap and uniform).
            let cleaned_title = livrarr_metadata::title_cleanup::clean_title(title);
            let cleaned_author = livrarr_metadata::title_cleanup::clean_author(author_name);
            match state
                .db
                .create_work(CreateWorkDbRequest {
                    user_id,
                    title: cleaned_title,
                    author_name: cleaned_author,
                    author_id: livrarr_author_id,
                    ol_key: None,
                    gr_key: rd_book.foreign_book_id.clone(),
                    year,
                    cover_url,
                    metadata_source: Some("readarr".to_string()),
                    detail_url: None,
                    language,
                    import_id: Some(import_id.to_string()),
                    series_id: None,
                    series_name: None,
                    series_position: None,
                    monitor_ebook: false,
                    monitor_audiobook: false,
                })
                .await
            {
                Ok(w) => {
                    works_created += 1;
                    // Readarr import is user-validated (Pete's Readarr
                    // instance picked these works for import) — lock as
                    // setter=User.
                    crate::handlers::work::write_addtime_provenance(
                        &state.db,
                        user_id,
                        &w,
                        livrarr_domain::ProvenanceSetter::User,
                    )
                    .await;

                    let _ = state
                        .db
                        .update_work_enrichment(
                            user_id,
                            w.id,
                            livrarr_db::UpdateWorkEnrichmentDbRequest {
                                title: None,
                                subtitle: None,
                                original_title: None,
                                author_name: None,
                                description,
                                year: None,
                                series_name,
                                series_position,
                                genres,
                                language: None,
                                page_count,
                                duration_seconds: None,
                                publisher,
                                publish_date: rd_book.release_date.clone(),
                                ol_key: None,
                                gr_key: None,
                                hc_key: None,
                                isbn_13: isbn.clone(),
                                asin: asin.clone(),
                                narrator: None,
                                narration_type: None,
                                abridged: None,
                                rating,
                                rating_count,
                                enrichment_status: livrarr_domain::EnrichmentStatus::Skipped,
                                enrichment_source: Some("readarr".to_string()),
                                cover_url: None,
                            },
                        )
                        .await;

                    let _ = state
                        .db
                        .update_work_user_fields(
                            user_id,
                            w.id,
                            livrarr_db::UpdateWorkUserFieldsDbRequest {
                                title: None,
                                author_name: None,
                                series_name: None,
                                series_position: None,
                                monitor_ebook: Some(monitor_ebook),
                                monitor_audiobook: Some(monitor_audiobook),
                            },
                        )
                        .await;

                    // Download the Readarr cover to disk so mediacover can serve it.
                    if let Some(ref url) = w.cover_url {
                        let covers_dir = state.data_dir.join("covers");
                        let _ = tokio::fs::create_dir_all(&covers_dir).await;
                        // Cover URL came from a remote Readarr response — use SSRF-safe client.
                        if let Ok(resp) = state.http_client_safe.get(url).send().await {
                            if resp.status().is_success() {
                                if let Ok(bytes) = resp.bytes().await {
                                    let path = covers_dir.join(format!("{}.jpg", w.id));
                                    // Atomic cover write: .tmp → fsync → rename.
                                    let tmp_path = path.with_extension("jpg.tmp");
                                    let tmp_b = tmp_path.clone();
                                    let target = path.clone();
                                    let bytes_vec = bytes.to_vec();
                                    let write_res = tokio::task::spawn_blocking(
                                        move || -> std::io::Result<()> {
                                            use std::io::Write;
                                            let mut f = std::fs::File::create(&tmp_b)?;
                                            f.write_all(&bytes_vec)?;
                                            f.sync_all()?;
                                            drop(f);
                                            std::fs::rename(&tmp_b, &target)
                                        },
                                    )
                                    .await;
                                    if !matches!(write_res, Ok(Ok(()))) {
                                        let _ = tokio::fs::remove_file(&tmp_path).await;
                                    }
                                }
                            }
                        }
                    }

                    w.id
                }
                Err(e) => {
                    warn!(title = %title, "Failed to create work: {e}");
                    let mut prog = state.readarr_import_progress.lock().await;
                    prog.works_processed += 1;
                    prog.errors.push(format!("Work '{title}': {e}"));
                    continue;
                }
            }
        };

        rd_to_livrarr_work.insert(rd_book.id, work_id);

        {
            let mut prog = state.readarr_import_progress.lock().await;
            prog.works_processed += 1;
        }
    }

    // Phase 4: Process files (only those belonging to active books).
    for rd_file in rd_book_files
        .iter()
        .filter(|f| active_book_ids.contains(&f.book_id))
    {
        let work_id = match rd_to_livrarr_work.get(&rd_file.book_id) {
            Some(id) => *id,
            None => {
                // Book was skipped.
                files_skipped += 1;
                let mut prog = state.readarr_import_progress.lock().await;
                prog.files_processed += 1;
                prog.files_skipped += 1;
                continue;
            }
        };

        let author_name = rd_file
            .author_id
            .and_then(|aid| author_map.get(&aid))
            .and_then(|a| a.author_name.as_deref())
            .unwrap_or("Unknown Author");

        let title = rd_books
            .iter()
            .find(|b| b.id == rd_file.book_id)
            .and_then(|b| b.title.as_deref())
            .unwrap_or("Unknown Title");

        // Determine media type.
        let qid = extract_quality_id(rd_file);
        let media_type = match resolve_media_type(qid, &rd_file.path) {
            Some(mt) => mt,
            None => {
                files_skipped += 1;
                let mut prog = state.readarr_import_progress.lock().await;
                prog.files_processed += 1;
                prog.files_skipped += 1;
                continue;
            }
        };

        // For multi-part audiobooks, use the source filename stem as the destination name
        // to avoid path collisions between parts.
        let book_audio_count = if media_type == MediaType::Audiobook {
            book_files_by_book
                .get(&rd_file.book_id)
                .map(|fs| {
                    fs.iter()
                        .filter(|f| {
                            resolve_media_type(extract_quality_id(f), &f.path)
                                == Some(MediaType::Audiobook)
                        })
                        .count()
                })
                .unwrap_or(0)
        } else {
            1
        };
        let effective_title = if book_audio_count > 1 {
            Path::new(&rd_file.path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(title)
                .to_string()
        } else {
            title.to_string()
        };

        // Validate source path.
        let translated_path = apply_path_translation(
            &rd_file.path,
            container_path.as_deref(),
            host_path.as_deref(),
        );
        let source = {
            let tp = translated_path.clone();
            let rr = readarr_root.clone();
            tokio::task::spawn_blocking(move || validate_source_path(&tp, &rr))
                .await
                .unwrap_or_else(|e| Err(format!("spawn_blocking join: {e}")))
        };
        let source = match source {
            Ok(p) => p,
            Err(e) => {
                warn!(path = %rd_file.path, "Source path validation failed: {e}");
                files_skipped += 1;
                let mut prog = state.readarr_import_progress.lock().await;
                prog.files_processed += 1;
                prog.files_skipped += 1;
                prog.errors.push(format!("File '{}': {e}", rd_file.path));
                continue;
            }
        };

        // Build destination path.
        let dest = build_dest_path(
            &livrarr_root.path,
            user_id,
            author_name,
            &effective_title,
            &rd_file.path,
            media_type,
        );

        // Check if destination already exists.
        let dest_exists = {
            let d = dest.clone();
            tokio::task::spawn_blocking(move || d.exists())
                .await
                .unwrap_or(false)
        };
        if dest_exists {
            files_skipped += 1;
            let mut prog = state.readarr_import_progress.lock().await;
            prog.files_processed += 1;
            prog.files_skipped += 1;
            continue;
        }

        // Materialize file (hardlink or copy).
        let mat_result = {
            let src = source.clone();
            let dst = dest.clone();
            tokio::task::spawn_blocking(move || materialize_file(&src, &dst))
                .await
                .unwrap_or_else(|e| Err(format!("spawn_blocking join: {e}")))
        };
        if let Err(e) = mat_result {
            warn!(src = %rd_file.path, dest = %dest.display(), "File materialization failed: {e}");
            files_skipped += 1;
            let mut prog = state.readarr_import_progress.lock().await;
            prog.files_processed += 1;
            prog.files_skipped += 1;
            prog.errors.push(format!("File '{}': {e}", rd_file.path));
            continue;
        }

        // Get the relative path from root folder.
        let rel_path = dest
            .strip_prefix(&livrarr_root.path)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| dest.to_string_lossy().to_string());

        // Create library item.
        match state
            .db
            .create_library_item(CreateLibraryItemDbRequest {
                user_id,
                work_id,
                root_folder_id: livrarr_root_id,
                path: rel_path,
                media_type,
                file_size: rd_file.size,
                import_id: Some(import_id.to_string()),
            })
            .await
        {
            Ok(_) => {
                files_imported += 1;
            }
            Err(livrarr_domain::DbError::Constraint { .. }) => {
                // Path already claimed by another work — destination exists in DB
                // but not on disk. Skip silently; this is an expected collision.
                files_skipped += 1;
            }
            Err(e) => {
                warn!(path = %rd_file.path, "Failed to create library item: {e}");
                files_skipped += 1;
                let mut prog = state.readarr_import_progress.lock().await;
                prog.errors
                    .push(format!("LibraryItem for '{}': {e}", rd_file.path));
            }
        }

        {
            let mut prog = state.readarr_import_progress.lock().await;
            prog.files_processed += 1;
        }
    }

    // Update counters and mark completed.
    let _ = state
        .db
        .update_import_counts(
            import_id,
            authors_created,
            works_created,
            files_imported,
            files_skipped,
        )
        .await;

    state
        .db
        .set_import_completed(import_id)
        .await
        .map_err(|e| format!("set completed: {e}"))?;

    info!(
        import_id = %import_id,
        authors_created,
        works_created,
        files_imported,
        files_skipped,
        "Readarr import completed"
    );

    Ok(())
}
