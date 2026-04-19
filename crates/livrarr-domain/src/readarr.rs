use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Request DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadarrConnectRequest {
    pub url: String,
    pub api_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadarrImportRequest {
    pub url: String,
    pub api_key: String,
    pub readarr_root_folder_id: i64,
    pub livrarr_root_folder_id: i64,
    #[serde(default)]
    pub files_only: bool,
    pub container_path: Option<String>,
    pub host_path: Option<String>,
}

// ---------------------------------------------------------------------------
// Response DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadarrConnectResponse {
    pub root_folders: Vec<ReadarrRootFolderInfo>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadarrRootFolderInfo {
    pub id: i64,
    pub name: Option<String>,
    pub path: String,
    pub accessible: Option<bool>,
    pub free_space: Option<i64>,
    pub total_space: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadarrPreviewResponse {
    pub authors_to_create: i64,
    pub authors_existing: i64,
    pub works_to_create: i64,
    pub works_existing: i64,
    pub files_to_import: i64,
    pub files_to_skip: i64,
    pub skipped_items: Vec<ReadarrSkippedItem>,
    pub import_files: Vec<ReadarrPreviewFileItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadarrSkippedItem {
    pub title: String,
    pub author: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadarrPreviewFileItem {
    pub title: String,
    pub author: String,
    pub path: String,
    pub media_type: String,
    pub work_status: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadarrStartResponse {
    pub import_id: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadarrImportProgress {
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
pub struct ReadarrHistoryResponse {
    pub imports: Vec<ReadarrImportRecord>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadarrImportRecord {
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
pub struct ReadarrUndoResponse {
    pub files_deleted: i64,
    pub files_skipped: i64,
    pub works_deleted: i64,
    pub authors_deleted: i64,
}
