pub mod bookmark_service;
pub mod chapter_service;
pub mod cross_format_service;
pub mod file_service;
pub mod import_workflow;

use livrarr_domain::{DbError, GrabStatus, MediaType, RootFolderId, UserId, WorkId};
// Re-export classify_file from domain.
pub use livrarr_domain::classify_file;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("source path does not exist: {path}")]
    SourceNotFound { path: String },
    #[error("mapped local path does not exist: {path}")]
    MappedPathNotFound { path: String },
    #[error("no recognized media files in download")]
    NoRecognizedFiles,
    #[error("no root folder configured for media type: {media_type:?}")]
    NoRootFolder { media_type: MediaType },
    #[error("grab not found")]
    GrabNotFound,
    #[error("invalid grab status for import: {status:?}")]
    InvalidGrabStatus { status: GrabStatus },
    #[error("path conflict: {path} already claimed by work {existing_work_id}")]
    PathConflict {
        path: String,
        existing_work_id: WorkId,
    },
    #[error("duplicate ebook extension in same download: {extension}")]
    DuplicateEbookExtension { extension: String },
    #[error("disk full")]
    DiskFull,
    #[error("path too long: {length} bytes")]
    PathTooLong { length: usize },
    #[error("source enumeration failed: {0}")]
    EnumerationFailed(String),
    #[error("file copy failed: {0}")]
    CopyFailed(String),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

// ---------------------------------------------------------------------------
// Path Builder
// ---------------------------------------------------------------------------

/// Build target paths for imported files.
pub trait PathBuilder: Send + Sync {
    /// Build ebook target path.
    /// Layout: {root}/{user_id}/{sanitized_author}/{sanitized_title}.{ext}
    fn build_ebook_path(
        &self,
        root: &str,
        user_id: UserId,
        author: &str,
        title: &str,
        extension: &str,
    ) -> Result<String, ImportError>;

    /// Build audiobook target path.
    /// Layout: {root}/{user_id}/{sanitized_author}/{sanitized_title}/{relative_path}
    fn build_audiobook_path(
        &self,
        root: &str,
        user_id: UserId,
        author: &str,
        title: &str,
        relative_path: &str,
    ) -> Result<String, ImportError>;
}

// ---------------------------------------------------------------------------
// Scan Service
// ---------------------------------------------------------------------------

/// Manual library scan.
#[trait_variant::make(Send)]
pub trait ScanService: Send + Sync {
    /// Scan a root folder for the requesting user's files.
    async fn scan_root_folder(
        &self,
        user_id: UserId,
        root_folder_id: RootFolderId,
    ) -> Result<ScanResult, ScanError>;
}

pub struct ScanResult {
    pub matched: Vec<ScanMatch>,
    pub unmatched: Vec<ScanUnmatched>,
    pub errors: Vec<String>,
}

pub struct ScanMatch {
    pub path: String,
    pub work_id: WorkId,
    pub media_type: MediaType,
}

pub struct ScanUnmatched {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("root folder not found")]
    RootFolderNotFound,
    #[error("scan already running on this root folder")]
    AlreadyRunning,
    #[error("I/O error: {0}")]
    Io(String),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

// ---------------------------------------------------------------------------
// Atomic File Operations
// ---------------------------------------------------------------------------

pub async fn atomic_copy(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<u64> {
    let src = src.to_path_buf();
    let dst = dst.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let parent = dst.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path has no parent directory",
            )
        })?;
        std::fs::create_dir_all(parent)?;
        let mut src_file = std::fs::File::open(&src)?;
        let tmp = tempfile::NamedTempFile::new_in(parent)?;
        let mut dst_file = tmp.as_file().try_clone()?;
        let copied = std::io::copy(&mut src_file, &mut dst_file)?;
        dst_file.sync_all()?;
        drop(dst_file);
        tmp.persist(&dst).map_err(|e| e.error)?;
        Ok(copied)
    })
    .await
    .expect("spawn_blocking panicked")
}

#[cfg(test)]
mod playback_service_tests;
