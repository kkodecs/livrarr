use crate::{DbError, GrabId, GrabStatus, MediaType, UserId};

use super::list::ScanConfirmation;

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
    #[error("scan not found or expired")]
    ScanExpired,
    #[error("scan belongs to another user")]
    ScanForbidden,
    #[error("import failed: {0}")]
    ImportFailed(String),
    #[error("tag write failed: {0}")]
    TagWriteFailed(String),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

#[trait_variant::make(Send)]
pub trait ImportWorkflow: Send + Sync {
    async fn import_grab(
        &self,
        user_id: UserId,
        grab_id: GrabId,
    ) -> Result<ImportResult, ImportWorkflowError>;
    async fn retry_import(
        &self,
        user_id: UserId,
        grab_id: GrabId,
    ) -> Result<ImportResult, ImportWorkflowError>;
    async fn confirm_scan(
        &self,
        user_id: UserId,
        scan_id: &str,
        selections: Vec<ScanConfirmation>,
    ) -> Result<ImportResult, ImportWorkflowError>;
}

/// Fire-and-forget bibliography fetch trigger for newly created authors.
/// Trait lives in domain; impl in livrarr-server (spawns background task).
#[trait_variant::make(Send)]
pub trait BibliographyTrigger: Send + Sync {
    fn trigger(&self, author_id: i64, user_id: UserId);
}
