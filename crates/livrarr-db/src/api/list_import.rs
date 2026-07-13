//! CSV list-import (Goodreads / Hardcover preview+confirm) data access: `ListImportDb` trait.

use crate::{DbError, UserId, WorkId};

/// A single row from the list_import_previews table, returned by fetch operations.
pub struct ListImportPreviewRow {
    pub title: String,
    pub author: String,
    pub isbn_13: Option<String>,
    pub isbn_10: Option<String>,
    pub goodreads_book_id: Option<String>,
    pub year: Option<i32>,
}

/// A row from the imports table for the list-import listing endpoint.
pub struct ListImportSummaryRow {
    pub id: String,
    pub source: String,
    pub status: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub works_created: i64,
}

/// Data needed to validate ownership of a running import record.
pub struct ListImportRecord {
    pub user_id: i64,
    pub status: String,
}

/// Database operations for the CSV list-import flow (Goodreads / Hardcover previews).
#[trait_variant::make(Send)]
pub trait ListImportDb: Send + Sync {
    /// Insert a single row into the list_import_previews table.
    #[allow(clippy::too_many_arguments)]
    async fn insert_list_import_preview_row(
        &self,
        preview_id: &str,
        user_id: UserId,
        row_index: i64,
        title: &str,
        author: &str,
        isbn_13: Option<&str>,
        isbn_10: Option<&str>,
        goodreads_book_id: Option<&str>,
        year: Option<i32>,
        source_status: Option<&str>,
        source_rating: Option<f32>,
        preview_status: &str,
        source: &str,
        created_at: &str,
    ) -> Result<(), DbError>;

    /// Count preview rows for the given (preview_id, user_id) pair.
    async fn count_list_import_previews(
        &self,
        preview_id: &str,
        user_id: UserId,
    ) -> Result<i64, DbError>;

    /// Get the source field from any preview row for the given (preview_id, user_id).
    async fn get_list_import_source(
        &self,
        preview_id: &str,
        user_id: UserId,
    ) -> Result<String, DbError>;

    /// Create a new list-import record with status = 'running'.
    async fn create_list_import_record(
        &self,
        id: &str,
        user_id: UserId,
        source: &str,
        started_at: &str,
    ) -> Result<(), DbError>;

    /// Get ownership + status for a list-import record.
    async fn get_list_import_record(&self, id: &str) -> Result<Option<ListImportRecord>, DbError>;

    /// Get a single preview row by (preview_id, user_id, row_index).
    async fn get_list_import_preview_row(
        &self,
        preview_id: &str,
        user_id: UserId,
        row_index: i64,
    ) -> Result<Option<ListImportPreviewRow>, DbError>;

    /// Tag the most-recently-created work for a user with the given import_id.
    async fn tag_last_work_with_import(
        &self,
        import_id: &str,
        user_id: UserId,
    ) -> Result<(), DbError>;

    /// Increment works_created counter on a list-import record.
    async fn increment_list_import_works_created(
        &self,
        import_id: &str,
        delta: i64,
    ) -> Result<(), DbError>;

    /// Complete a list-import (status = 'completed', completed_at = now).
    /// Scoped to user_id + status = 'running'. Returns rows_affected.
    async fn complete_list_import(
        &self,
        import_id: &str,
        user_id: UserId,
        completed_at: &str,
    ) -> Result<u64, DbError>;

    /// Validate + get status of a list-import for a specific user.
    async fn get_list_import_status_for_user(
        &self,
        import_id: &str,
        user_id: UserId,
    ) -> Result<Option<String>, DbError>;

    /// Delete works tagged with the given import_id for the given user.
    async fn delete_works_by_list_import(
        &self,
        import_id: &str,
        user_id: UserId,
    ) -> Result<i64, DbError>;

    /// Mark a list-import as 'undone'.
    async fn mark_list_import_undone(&self, import_id: &str) -> Result<(), DbError>;

    /// List list-imports (goodreads/hardcover sources) for a user, newest first, limit 50.
    async fn list_list_imports(
        &self,
        user_id: UserId,
    ) -> Result<Vec<ListImportSummaryRow>, DbError>;

    /// Check if a work with the given ISBN-13 exists for a user (via works table or external_ids).
    async fn work_exists_by_isbn_13(&self, user_id: UserId, isbn: &str) -> Result<bool, DbError>;

    /// Check if a work with the given ISBN-10 exists for a user (via external_ids).
    async fn work_exists_by_isbn_10(&self, user_id: UserId, isbn: &str) -> Result<bool, DbError>;

    /// Delete list_import_previews rows older than the given cutoff (RFC3339 string).
    async fn delete_stale_list_import_previews(&self, cutoff: &str) -> Result<u64, DbError>;

    /// Tag a specific work with an import_id. Explicit, race-free (unlike tag_last_work).
    async fn tag_work_with_import(
        &self,
        user_id: UserId,
        work_id: WorkId,
        import_id: &str,
    ) -> Result<(), DbError>;

    /// List work IDs tagged with the given import_id for a user.
    async fn list_works_by_import(
        &self,
        import_id: &str,
        user_id: UserId,
    ) -> Result<Vec<WorkId>, DbError>;
}
