use std::path::{Path, PathBuf};

use livrarr_db::{ConfigDb, LibraryItemDb, PlaybackProgressDb, RootFolderDb};
use livrarr_domain::services::{EmailPayload, FileService, FileServiceError};
use livrarr_domain::{DbError, LibraryItem, PlaybackProgress, UserId};

/// Accepted file extensions for email delivery (mirrors handler constant).
const ACCEPTED_EXTENSIONS: &[&str] = &["epub", "pdf", "docx", "doc", "rtf", "htm", "html", "txt"];

/// Maximum file size for email attachments (50 MB).
const MAX_EMAIL_SIZE: i64 = 50 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

pub struct FileServiceImpl<D> {
    db: D,
}

impl<D> FileServiceImpl<D> {
    pub fn new(db: D) -> Self {
        Self { db }
    }
}

impl<D> FileService for FileServiceImpl<D>
where
    D: LibraryItemDb + RootFolderDb + ConfigDb + PlaybackProgressDb + Send + Sync + 'static,
{
    async fn list_paginated(
        &self,
        user_id: UserId,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<LibraryItem>, i64), FileServiceError> {
        self.db
            .list_library_items_paginated(user_id, page, page_size)
            .await
            .map_err(map_db_err)
    }

    async fn get(&self, user_id: UserId, item_id: i64) -> Result<LibraryItem, FileServiceError> {
        self.db
            .get_library_item(user_id, item_id)
            .await
            .map_err(map_db_err)
    }

    async fn delete(&self, user_id: UserId, item_id: i64) -> Result<(), FileServiceError> {
        self.db
            .delete_library_item(user_id, item_id)
            .await
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn resolve_path(
        &self,
        user_id: UserId,
        item_id: i64,
    ) -> Result<PathBuf, FileServiceError> {
        let item = self
            .db
            .get_library_item(user_id, item_id)
            .await
            .map_err(map_db_err)?;
        let root_folder =
            self.db
                .get_root_folder(item.root_folder_id)
                .await
                .map_err(|e| match e {
                    DbError::NotFound { .. } => FileServiceError::RootFolderNotFound,
                    other => FileServiceError::Db(other),
                })?;

        let root = Path::new(&root_folder.path);
        let abs_path = root.join(&item.path);

        // Canonicalize and verify containment (path traversal protection).
        let canonical = abs_path
            .canonicalize()
            .map_err(|_| FileServiceError::NotFound)?;
        let canonical_root = root.canonicalize().map_err(|e| {
            FileServiceError::Io(std::io::Error::other(format!(
                "Root folder not accessible: {e}"
            )))
        })?;
        if !canonical.starts_with(&canonical_root) {
            return Err(FileServiceError::Forbidden);
        }

        Ok(canonical)
    }

    async fn prepare_email(
        &self,
        user_id: UserId,
        item_id: i64,
    ) -> Result<EmailPayload, FileServiceError> {
        let item = self
            .db
            .get_library_item(user_id, item_id)
            .await
            .map_err(map_db_err)?;
        let root_folder =
            self.db
                .get_root_folder(item.root_folder_id)
                .await
                .map_err(|e| match e {
                    DbError::NotFound { .. } => FileServiceError::RootFolderNotFound,
                    other => FileServiceError::Db(other),
                })?;

        let abs_path = Path::new(&root_folder.path).join(&item.path);

        // Validate extension against allowlist.
        let ext = abs_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !ACCEPTED_EXTENSIONS.contains(&ext.as_str()) {
            return Err(FileServiceError::BadRequest(format!(
                "Format '.{ext}' not accepted. Supported: EPUB, PDF, DOCX, RTF, TXT, HTML."
            )));
        }

        // Validate size (50 MB limit).
        if item.file_size > MAX_EMAIL_SIZE {
            return Err(FileServiceError::BadRequest(format!(
                "File exceeds the 50 MB email limit ({})",
                format_bytes(item.file_size)
            )));
        }

        // Read file in spawn_blocking for blocking I/O safety.
        let path_clone = abs_path.clone();
        let file_bytes = tokio::task::spawn_blocking(move || std::fs::read(&path_clone))
            .await
            .expect("spawn_blocking panicked")
            .map_err(|e| {
                FileServiceError::Io(std::io::Error::new(
                    e.kind(),
                    format!("Failed to read file {}: {e}", abs_path.display()),
                ))
            })?;

        let filename = abs_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("book")
            .to_owned();

        Ok(EmailPayload {
            file_bytes,
            filename,
            extension: ext,
        })
    }

    async fn get_progress(
        &self,
        user_id: UserId,
        item_id: i64,
    ) -> Result<Option<PlaybackProgress>, FileServiceError> {
        self.db
            .get_progress(user_id, item_id)
            .await
            .map_err(FileServiceError::Db)
    }

    async fn update_progress(
        &self,
        user_id: UserId,
        item_id: i64,
        position: &str,
        progress_pct: f64,
    ) -> Result<(), FileServiceError> {
        // Validate the library item exists and belongs to the user.
        let _item = self
            .db
            .get_library_item(user_id, item_id)
            .await
            .map_err(map_db_err)?;

        let pct = progress_pct.clamp(0.0, 1.0);
        self.db
            .upsert_progress(user_id, item_id, position, pct)
            .await
            .map_err(FileServiceError::Db)
    }
}

fn map_db_err(e: DbError) -> FileServiceError {
    match e {
        DbError::NotFound { .. } => FileServiceError::NotFound,
        other => FileServiceError::Db(other),
    }
}

fn format_bytes(bytes: i64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}
