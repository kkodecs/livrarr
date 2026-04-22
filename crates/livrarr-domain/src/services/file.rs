use crate::{DbError, LibraryItem, MediaType, PlaybackProgress, UserId, WorkId};

#[derive(Debug)]
pub struct ScanResult {
    pub scan_id: String,
    pub files: Vec<ScannedFile>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub struct ScannedFile {
    pub relative_path: String,
    pub filename: String,
    pub media_type: MediaType,
    pub size: i64,
    pub matched_work_id: Option<WorkId>,
    pub has_existing_item: bool,
}

/// Prepared email payload — contains validated file data for the handler to send via SMTP.
/// The handler is responsible for fetching `EmailConfig` and calling `email::send_file`.
#[derive(Debug)]
pub struct EmailPayload {
    pub file_bytes: Vec<u8>,
    pub filename: String,
    pub extension: String,
}

#[derive(Debug, thiserror::Error)]
pub enum FileServiceError {
    #[error("library item not found")]
    NotFound,
    #[error("root folder not found")]
    RootFolderNotFound,
    #[error("path traversal denied")]
    Forbidden,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

#[trait_variant::make(Send)]
pub trait FileService: Send + Sync {
    async fn list(&self, user_id: UserId) -> Result<Vec<LibraryItem>, FileServiceError>;

    async fn list_paginated(
        &self,
        user_id: UserId,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<LibraryItem>, i64), FileServiceError>;
    async fn get(&self, user_id: UserId, item_id: i64) -> Result<LibraryItem, FileServiceError>;
    async fn delete(&self, user_id: UserId, item_id: i64) -> Result<(), FileServiceError>;

    async fn resolve_path(
        &self,
        user_id: UserId,
        item_id: i64,
    ) -> Result<std::path::PathBuf, FileServiceError>;

    async fn prepare_email(
        &self,
        user_id: UserId,
        item_id: i64,
    ) -> Result<EmailPayload, FileServiceError>;

    async fn get_progress(
        &self,
        user_id: UserId,
        item_id: i64,
    ) -> Result<Option<PlaybackProgress>, FileServiceError>;
    async fn update_progress(
        &self,
        user_id: UserId,
        item_id: i64,
        position: &str,
        progress_pct: f64,
    ) -> Result<(), FileServiceError>;
}
