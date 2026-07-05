use crate::{DbError, GrabId, GrabStatus, LibraryItemId, MediaType, RootFolderId, UserId, WorkId};

#[derive(Debug)]
pub struct ImportResult {
    pub grab_id: GrabId,
    pub final_status: GrabStatus,
    pub imported_files: Vec<ImportedFile>,
    pub failed_files: Vec<FailedFile>,
    pub skipped_files: Vec<SkippedFile>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub struct ImportedFile {
    pub source_name: String,
    pub target_relative_path: String,
    pub media_type: MediaType,
    pub file_size: u64,
    pub library_item_id: i64,
    pub tags_written: bool,
    pub cwa_copied: bool,
}

#[derive(Debug)]
pub struct FailedFile {
    pub source_name: String,
    pub error: String,
}

#[derive(Debug)]
pub struct SkippedFile {
    pub source_name: String,
    pub reason: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportWorkflowError {
    #[error("grab not found")]
    GrabNotFound,
    #[error("source path not resolved: {0}")]
    SourceNotResolved(String),
    #[error("download client unreachable: {0}")]
    ClientUnreachable(String),
    #[error("no root folder for media type: {media_type:?}")]
    NoRootFolder { media_type: MediaType },
    #[error("source directory not found or inaccessible")]
    SourceInaccessible,
    #[error("import failed: {0}")]
    ImportFailed(String),
    #[error("tag write failed: {0}")]
    TagWriteFailed(String),
    #[error("path collision: {0} already claimed by a different work")]
    PathCollision(String),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

/// How a source file becomes the on-disk file backing a `LibraryItem`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Materialization {
    /// Copy the source into place (`atomic_copy`).
    Copy,
    /// Hard link the source into place; falls back to a size-verified copy
    /// when the link fails (e.g. cross-device).
    HardlinkFirst,
    /// The source already IS the target — used by doors (e.g. library scan)
    /// that discover a file already sitting in a root folder.
    AdoptInPlace,
}

/// Request to bring one file into the library as a `LibraryItem`, shared by
/// every import door (grab import, manual import, Readarr import, scan).
#[derive(Debug)]
pub struct ImportFileRequest {
    pub work_id: WorkId,
    pub root_folder_id: RootFolderId,
    /// Absolute path to the source file. For `AdoptInPlace` this IS the
    /// on-disk file (source == target).
    pub source: std::path::PathBuf,
    /// Relative to the root folder; computed by the calling door.
    pub target_relative: String,
    pub media_type: MediaType,
    pub materialization: Materialization,
    pub import_id: Option<String>,
    pub extract_chapters: bool,
}

/// Why a file was not imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    AlreadyImported,
}

/// Result of `ImportWorkflow::import_file`.
#[derive(Debug)]
pub enum ImportFileOutcome {
    /// A new file was materialized and a `LibraryItem` created.
    Imported {
        item_id: LibraryItemId,
        path: String,
    },
    /// A file already on disk (no prior `LibraryItem`) was adopted.
    Adopted {
        item_id: LibraryItemId,
        path: String,
    },
    /// The target already has a `LibraryItem` for this work at this path.
    Skipped { reason: SkipReason },
}

#[trait_variant::make(Send)]
pub trait ImportWorkflow: Send + Sync {
    async fn import_grab(
        &self,
        user_id: UserId,
        grab_id: GrabId,
    ) -> Result<ImportResult, ImportWorkflowError>;
    async fn import_file(
        &self,
        user_id: UserId,
        req: ImportFileRequest,
    ) -> Result<ImportFileOutcome, ImportWorkflowError>;
}

/// Fire-and-forget bibliography fetch trigger for newly created authors.
/// Trait lives in domain; impl in livrarr-server (spawns background task).
#[trait_variant::make(Send)]
pub trait BibliographyTrigger: Send + Sync {
    fn trigger(&self, author_id: i64, user_id: UserId);
}
