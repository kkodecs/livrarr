use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use livrarr_db::{LibraryItemDb, RootFolderDb, WorkDb};
use livrarr_domain::services::{
    FileService, FileServiceError, FileStream, ScanResult, ScannedFile,
};
use livrarr_domain::{classify_file, DbError, LibraryItem, UserId, WorkId};
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Scan state (in-memory, ephemeral)
// ---------------------------------------------------------------------------

struct ScanState {
    user_id: UserId,
    result: ScanResult,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

pub struct FileServiceImpl<D> {
    db: D,
    scan_states: Arc<RwLock<HashMap<String, ScanState>>>,
}

impl<D> FileServiceImpl<D> {
    pub fn new(db: D) -> Self {
        Self {
            db,
            scan_states: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

/// Validate that a relative path has no `..` components and is not absolute.
fn validate_relative_path(path: &str) -> Result<(), FileServiceError> {
    let p = Path::new(path);
    if p.is_absolute() {
        return Err(FileServiceError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path traversal: absolute path not allowed",
        )));
    }
    for component in p.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(FileServiceError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path traversal: '..' component not allowed",
            )));
        }
    }
    Ok(())
}

/// Resolve a library item to its full filesystem path.
fn resolve_full_path(root_path: &str, relative_path: &str) -> PathBuf {
    Path::new(root_path).join(relative_path)
}

fn clone_scan_result(r: &ScanResult) -> ScanResult {
    ScanResult {
        scan_id: r.scan_id.clone(),
        files: r
            .files
            .iter()
            .map(|f| ScannedFile {
                relative_path: f.relative_path.clone(),
                filename: f.filename.clone(),
                media_type: f.media_type,
                size: f.size,
                matched_work_id: f.matched_work_id,
                has_existing_item: f.has_existing_item,
            })
            .collect(),
        warnings: r.warnings.clone(),
    }
}

impl<D> FileService for FileServiceImpl<D>
where
    D: LibraryItemDb + RootFolderDb + WorkDb + Clone + Send + Sync + 'static,
{
    async fn list(
        &self,
        user_id: UserId,
        work_id: Option<WorkId>,
    ) -> Result<Vec<LibraryItem>, FileServiceError> {
        let items = match work_id {
            Some(wid) => self
                .db
                .list_library_items_by_work(user_id, wid)
                .await
                .map_err(map_db_err)?,
            None => self
                .db
                .list_library_items(user_id)
                .await
                .map_err(map_db_err)?,
        };
        Ok(items)
    }

    async fn get(&self, user_id: UserId, item_id: i64) -> Result<LibraryItem, FileServiceError> {
        self.db
            .get_library_item(user_id, item_id)
            .await
            .map_err(map_db_err)
    }

    async fn delete(&self, user_id: UserId, item_id: i64) -> Result<(), FileServiceError> {
        let item = self
            .db
            .get_library_item(user_id, item_id)
            .await
            .map_err(map_db_err)?;

        // Validate path before any filesystem operation
        validate_relative_path(&item.path)?;

        // Resolve full path and best-effort delete physical file
        let root = self
            .db
            .get_root_folder(item.root_folder_id)
            .await
            .map_err(map_db_err)?;
        let full_path = resolve_full_path(&root.path, &item.path);

        // Best-effort delete — ignore missing file
        let _ = tokio::task::spawn_blocking(move || std::fs::remove_file(&full_path))
            .await
            .expect("spawn_blocking panicked");

        // Delete DB record
        self.db
            .delete_library_item(user_id, item_id)
            .await
            .map_err(map_db_err)?;

        Ok(())
    }

    async fn retag(&self, _user_id: UserId, _item_id: i64) -> Result<(), FileServiceError> {
        Err(FileServiceError::TagWrite(
            "retag not yet implemented".into(),
        ))
    }

    async fn read_file(
        &self,
        user_id: UserId,
        item_id: i64,
    ) -> Result<FileStream, FileServiceError> {
        let item = self
            .db
            .get_library_item(user_id, item_id)
            .await
            .map_err(map_db_err)?;

        validate_relative_path(&item.path)?;

        let root = self
            .db
            .get_root_folder(item.root_folder_id)
            .await
            .map_err(map_db_err)?;
        let full_path = resolve_full_path(&root.path, &item.path);

        let file = tokio::fs::File::open(&full_path).await?;
        let metadata = file.metadata().await?;
        let size = metadata.len();

        let filename = Path::new(&item.path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "download".into());

        Ok(FileStream {
            reader: Box::new(file),
            size,
            media_type: item.media_type,
            filename,
        })
    }

    async fn scan_root_folder(
        &self,
        user_id: UserId,
        root_folder_id: i64,
    ) -> Result<ScanResult, FileServiceError> {
        let root = self
            .db
            .get_root_folder(root_folder_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => FileServiceError::RootFolderNotFound,
                other => FileServiceError::Db(other),
            })?;

        // User-scoped scan directory
        let scan_dir = PathBuf::from(&root.path).join(user_id.to_string());

        // Walk filesystem in spawn_blocking
        let scan_dir_clone = scan_dir.clone();
        let paths: Vec<PathBuf> = tokio::task::spawn_blocking(move || {
            let mut result = Vec::new();
            if !scan_dir_clone.exists() {
                return result;
            }
            fn walk_dir(dir: &Path, result: &mut Vec<PathBuf>) {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            walk_dir(&path, result);
                        } else {
                            result.push(path);
                        }
                    }
                }
            }
            walk_dir(&scan_dir_clone, &mut result);
            result
        })
        .await
        .expect("spawn_blocking panicked");

        let mut files = Vec::new();
        let mut warnings = Vec::new();
        let root_path = PathBuf::from(&root.path);

        for path in &paths {
            let media_type = match classify_file(path) {
                Some(mt) => mt,
                None => continue,
            };

            let relative = match path.strip_prefix(&root_path) {
                Ok(r) => r.to_string_lossy().into_owned(),
                Err(_) => {
                    warnings.push(format!("path outside root: {}", path.display()));
                    continue;
                }
            };

            if validate_relative_path(&relative).is_err() {
                warnings.push(format!("invalid path skipped: {relative}"));
                continue;
            }

            let filename = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();

            let size = path.metadata().map(|m| m.len()).unwrap_or(0);

            files.push(ScannedFile {
                relative_path: relative,
                filename,
                media_type,
                size,
                matched_work_id: None,
                has_existing_item: false,
            });
        }

        let scan_id = generate_scan_id();

        let result = ScanResult {
            scan_id: scan_id.clone(),
            files,
            warnings,
        };

        // Store scan state
        let state = ScanState {
            user_id,
            result: clone_scan_result(&result),
        };
        self.scan_states.write().await.insert(scan_id, state);

        Ok(result)
    }

    async fn get_scan(
        &self,
        user_id: UserId,
        scan_id: &str,
    ) -> Result<ScanResult, FileServiceError> {
        let states = self.scan_states.read().await;
        let state = states.get(scan_id).ok_or(FileServiceError::ScanExpired)?;

        if state.user_id != user_id {
            return Err(FileServiceError::ScanForbidden);
        }

        Ok(clone_scan_result(&state.result))
    }
}

fn map_db_err(e: DbError) -> FileServiceError {
    match e {
        DbError::NotFound { .. } => FileServiceError::NotFound,
        other => FileServiceError::Db(other),
    }
}

fn generate_scan_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tid = format!("{:?}", std::thread::current().id());
    format!("{t:032x}-{tid}")
}
