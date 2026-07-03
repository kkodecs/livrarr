use crate::{GrabStatus, LibraryItem, MediaType, Work};

use super::common::ServiceError;
use super::work::TagSyncItemResult;

#[derive(Debug, Clone)]
pub struct ImportGrabResult {
    pub final_status: GrabStatus,
    pub imported_count: usize,
    pub failed_count: usize,
    pub skipped_count: usize,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct ImportSingleFileRequest {
    pub source: std::path::PathBuf,
    pub target_path: String,
    pub root_folder_path: String,
    pub root_folder_id: i64,
    pub media_type: MediaType,
    pub user_id: i64,
    pub work_id: i64,
    pub author_name: String,
    pub title: String,
    pub import_id: Option<String>,
}

#[derive(Debug)]
pub enum ImportFileResult {
    Ok,
    Warning(String),
    Failed(String),
}

#[trait_variant::make(Send)]
pub trait ImportService: Send + Sync {
    async fn import_grab(
        &self,
        user_id: i64,
        grab_id: i64,
    ) -> Result<ImportGrabResult, ServiceError>;

    async fn import_single_file(&self, req: ImportSingleFileRequest) -> ImportFileResult;

    /// Best-effort physical reorganization of a work's already-imported
    /// files onto the standard naming/layout path (the merge "files
    /// re-organize under the survivor" step, REQ-015 c). For each library
    /// item, recomputes its target path via [`Self::build_target_path`]; if
    /// the target differs from the item's current path, moves the file and
    /// updates the stored path. A destination collision (something else
    /// already occupies the computed path) is never overwritten and never
    /// deletes anything — the item is left at its current path and a
    /// warning is returned instead. Retags moved items when the work has
    /// enrichment data, mirroring the retag-on-import behavior.
    ///
    /// Returns one warning string per item that could not be relocated;
    /// an empty vec means every item now lives at its canonical path (or
    /// already did). Never returns an `Err` for per-item problems — only
    /// for a failure to read the work/items themselves.
    async fn reorganize_work_files(
        &self,
        user_id: i64,
        work_id: i64,
    ) -> Result<Vec<String>, ServiceError>;

    #[allow(clippy::too_many_arguments)]
    fn build_target_path(
        &self,
        root_folder_path: &str,
        user_id: i64,
        author: &str,
        title: &str,
        media_type: MediaType,
        source: &std::path::Path,
        source_root: &std::path::Path,
    ) -> String;
}

#[trait_variant::make(Send)]
pub trait TagService: Send + Sync {
    async fn retag_library_items(
        &self,
        work: &Work,
        items: &[LibraryItem],
    ) -> Vec<TagSyncItemResult>;
}

#[trait_variant::make(Send)]
pub trait CoverIoService: Send + Sync {
    async fn read_cover_bytes(&self, user_id: i64, work_id: i64) -> Option<Vec<u8>>;
}
