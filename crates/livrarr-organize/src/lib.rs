use livrarr_domain::{
    sanitize_path_component, DbError, Grab, GrabId, GrabStatus, LibraryItemId, MediaType,
    RootFolderId, UserId, WorkId,
};
// Re-export classify_file from domain.
pub use livrarr_domain::classify_file;

// =============================================================================
// CRATE: livrarr-organize
// =============================================================================
// Naming, path building, file copy, import pipeline, CWA copy, manual scan.

// ---------------------------------------------------------------------------
// Import Service
// ---------------------------------------------------------------------------

/// Import pipeline -- processes completed downloads.
#[trait_variant::make(Send)]
pub trait ImportService: Send + Sync {
    /// Run import for a completed grab.
    async fn import_grab(&self, grab: &Grab) -> Result<ImportResult, ImportError>;

    /// Retry a failed import.
    async fn retry_import(&self, user_id: UserId, grab_id: GrabId) -> Result<(), ImportError>;
}

pub struct ImportResult {
    pub grab_id: GrabId,
    pub final_status: GrabStatus,
    pub imported_files: Vec<ImportedFile>,
    pub skipped_files: Vec<SkippedFile>,
    pub failed_files: Vec<FailedFile>,
    pub warnings: Vec<String>,
}

pub struct ImportedFile {
    pub source_path: String,
    pub target_path: String,
    pub media_type: MediaType,
    pub file_size: i64,
    pub library_item_id: LibraryItemId,
    pub tags_written: bool,
    pub cwa_copied: bool,
}

pub struct SkippedFile {
    pub source_path: String,
    pub reason: String,
}

pub struct FailedFile {
    pub source_path: String,
    pub error: String,
}

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
#[trait_variant::make(Send)]
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
// CWA Integration
// ---------------------------------------------------------------------------

/// CWA downstream integration (non-fatal).
pub fn copy_to_cwa(
    source_path: &str,
    cwa_ingest_path: &str,
    user_id: UserId,
    author: &str,
    title: &str,
    extension: &str,
) -> CwaResult {
    let author_san = sanitize_path_component(author, "Unknown Author");
    let title_san = sanitize_path_component(title, "Unknown Title");
    let dst_dir = std::path::Path::new(cwa_ingest_path)
        .join(user_id.to_string())
        .join(&author_san);
    let dst = dst_dir.join(format!("{}.{}", title_san, extension));

    if dst.exists() {
        return CwaResult {
            success: false,
            warning: Some(format!("destination already exists: {}", dst.display())),
        };
    }

    if let Err(e) = std::fs::create_dir_all(&dst_dir) {
        return CwaResult {
            success: false,
            warning: Some(format!("failed to create CWA directory: {e}")),
        };
    }

    // Try hardlink first
    match std::fs::hard_link(source_path, &dst) {
        Ok(()) => CwaResult {
            success: true,
            warning: None,
        },
        Err(e) if e.raw_os_error() == Some(18) => {
            // EXDEV — cross-filesystem, fallback to copy
            match std::fs::copy(source_path, &dst) {
                Ok(_) => CwaResult {
                    success: true,
                    warning: None,
                },
                Err(e) => CwaResult {
                    success: false,
                    warning: Some(format!("CWA copy failed: {e}")),
                },
            }
        }
        Err(e) => CwaResult {
            success: false,
            warning: Some(format!("CWA hardlink failed: {e}")),
        },
    }
}

pub struct CwaResult {
    pub success: bool,
    pub warning: Option<String>,
}
