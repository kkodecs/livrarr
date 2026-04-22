use serde::{Deserialize, Serialize};

use crate::{DbError, MediaType, UserId, WorkId};

/// Kept for backward compatibility with existing behavioral tests.
/// New code should use the redesigned preview(bytes) API.
#[derive(Debug)]
pub struct ListPreviewRequest {
    pub source: ListSource,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListSource {
    GoodreadsCsv,
    OpenLibrary,
    Hardcover,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListPreviewResponse {
    pub preview_id: String,
    pub source: String,
    pub total_rows: usize,
    pub rows: Vec<ListPreviewRow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListPreviewRow {
    pub row_index: usize,
    pub title: String,
    pub author: String,
    pub isbn_13: Option<String>,
    pub isbn_10: Option<String>,
    pub year: Option<i32>,
    pub source_status: Option<String>,
    pub source_rating: Option<f32>,
    pub preview_status: String,
}

/// Legacy match status for backward-compat behavioral tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListMatchStatus {
    Matched,
    NotFound,
    AlreadyExists,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListConfirmResponse {
    pub import_id: String,
    pub results: Vec<ListConfirmRowResult>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListConfirmRowResult {
    pub row_index: usize,
    pub status: String,
    pub message: Option<String>,
}

/// Legacy response shape for backward-compat behavioral tests.
#[derive(Debug)]
pub struct ListConfirmLegacyResponse {
    pub added: usize,
    pub skipped: usize,
    pub failed: Vec<ListFailedRow>,
}

#[derive(Debug)]
pub struct ListFailedRow {
    pub title: String,
    pub error: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListUndoResponse {
    pub works_removed: usize,
    pub works_skipped: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListImportSummary {
    pub id: String,
    pub source: String,
    pub status: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub works_created: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum ListServiceError {
    #[error("import not found")]
    NotFound,
    #[error("parse error: {0}")]
    Parse(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

#[derive(Debug)]
pub struct ScanConfirmation {
    pub relative_path: String,
    pub work_id: WorkId,
    pub media_type: MediaType,
}

#[trait_variant::make(Send)]
pub trait ListService: Send + Sync {
    async fn preview(
        &self,
        user_id: UserId,
        bytes: Vec<u8>,
    ) -> Result<ListPreviewResponse, ListServiceError>;

    async fn confirm(
        &self,
        user_id: UserId,
        preview_id: &str,
        import_id: Option<&str>,
        row_indices: &[usize],
    ) -> Result<ListConfirmResponse, ListServiceError>;

    async fn complete(&self, user_id: UserId, import_id: &str) -> Result<(), ListServiceError>;

    async fn undo(
        &self,
        user_id: UserId,
        import_id: &str,
    ) -> Result<ListUndoResponse, ListServiceError>;

    async fn list_imports(
        &self,
        user_id: UserId,
    ) -> Result<Vec<ListImportSummary>, ListServiceError>;
}
