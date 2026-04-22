use crate::{GrabStatus, LibraryItem, MediaType, Work};

use super::common::ServiceError;

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
    async fn retag_library_items(&self, work: &Work, items: &[LibraryItem]) -> Vec<String>;
}

#[trait_variant::make(Send)]
pub trait CoverIoService: Send + Sync {
    async fn read_cover_bytes(&self, user_id: i64, work_id: i64) -> Option<Vec<u8>>;
}
