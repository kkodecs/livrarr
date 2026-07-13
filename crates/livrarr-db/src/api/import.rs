//! Import tracking data access (Readarr library import): `ImportDb` trait + request type.

use crate::{DbError, Import, LibraryItem, LibraryItemId, UserId};

/// Import tracking data access.
#[trait_variant::make(Send)]
pub trait ImportDb: Send + Sync {
    /// Create an import record.
    async fn create_import(&self, req: CreateImportDbRequest) -> Result<(), DbError>;

    /// Get an import by ID.
    async fn get_import(&self, id: &str) -> Result<Option<Import>, DbError>;

    /// List imports for a user (most recent first).
    async fn list_imports(&self, user_id: UserId) -> Result<Vec<Import>, DbError>;

    /// Update import status.
    async fn update_import_status(&self, id: &str, status: &str) -> Result<(), DbError>;

    /// Update import counters.
    async fn update_import_counts(
        &self,
        id: &str,
        authors: i64,
        works: i64,
        files: i64,
        skipped: i64,
    ) -> Result<(), DbError>;

    /// Mark import as completed (set status + completed_at timestamp).
    async fn set_import_completed(&self, id: &str) -> Result<(), DbError>;

    /// List library items by import_id (for undo).
    async fn list_library_items_by_import(
        &self,
        import_id: &str,
    ) -> Result<Vec<LibraryItem>, DbError>;

    /// Delete a library item by ID (no user scope — for undo).
    async fn delete_library_item_by_id(&self, id: LibraryItemId) -> Result<(), DbError>;

    /// List work IDs by import_id that have zero library items (for cover cleanup before delete).
    async fn list_orphan_work_ids_by_import(&self, import_id: &str) -> Result<Vec<i64>, DbError>;

    /// Delete works by import_id that have zero library items.
    async fn delete_orphan_works_by_import(&self, import_id: &str) -> Result<i64, DbError>;

    /// Delete authors by import_id that have zero works.
    async fn delete_orphan_authors_by_import(&self, import_id: &str) -> Result<i64, DbError>;
}

pub struct CreateImportDbRequest {
    pub id: String,
    pub user_id: UserId,
    pub source: String,
    pub source_url: Option<String>,
    pub target_root_folder_id: Option<i64>,
}
