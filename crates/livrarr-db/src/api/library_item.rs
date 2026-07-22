//! Library item data access: `LibraryItemDb` trait + request type.

use crate::{
    DbError, LibraryItem, LibraryItemId, MediaType, RootFolderId, TagStatus, UserId, WorkId,
};

/// Library item data access.
///
/// Satisfies: IMPORT-015
#[trait_variant::make(Send)]
pub trait LibraryItemDb: Send + Sync {
    /// Get library item by ID for a user.
    async fn get_library_item(
        &self,
        user_id: UserId,
        id: LibraryItemId,
    ) -> Result<LibraryItem, DbError>;

    /// List library items for a user (unbounded — for internal use).
    async fn list_library_items(&self, user_id: UserId) -> Result<Vec<LibraryItem>, DbError>;

    /// List library items for a user, paginated.
    async fn list_library_items_paginated(
        &self,
        user_id: UserId,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<LibraryItem>, i64), DbError>;

    /// List library items for a set of work IDs (batch enrichment for paginated work lists).
    async fn list_library_items_by_work_ids(
        &self,
        user_id: UserId,
        work_ids: &[WorkId],
    ) -> Result<Vec<LibraryItem>, DbError>;

    /// List library items for a specific work.
    async fn list_library_items_by_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<LibraryItem>, DbError>;

    /// Create library item. Enforces UNIQUE(user_id, root_folder_id, path).
    ///
    /// Satisfies: IMPORT-015
    /// Precondition: File has been copied to the target path.
    /// Postcondition: Record created. On path conflict for same work -> returns existing (idempotent).
    ///                On path conflict for different work -> returns Constraint error.
    async fn create_library_item(
        &self,
        req: CreateLibraryItemDbRequest,
    ) -> Result<LibraryItem, DbError>;

    /// Delete library item.
    async fn delete_library_item(
        &self,
        user_id: UserId,
        id: LibraryItemId,
    ) -> Result<LibraryItem, DbError>;

    /// Check if any library items exist for a root folder (for root folder delete guard).
    ///
    /// Satisfies: IMPORT-004
    async fn library_items_exist_for_root(
        &self,
        root_folder_id: RootFolderId,
    ) -> Result<bool, DbError>;

    /// List library items for a work in supported tag-write formats (for re-enrichment tag rewrite).
    ///
    /// Satisfies: TAG-007
    async fn list_taggable_items_by_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<LibraryItem>, DbError>;

    /// Update library item file_size (after tag writing changes file size).
    ///
    /// Satisfies: TAG-V21-004
    async fn update_library_item_size(
        &self,
        user_id: UserId,
        id: LibraryItemId,
        file_size: i64,
    ) -> Result<(), DbError>;

    /// Update library item path (after the merge reorganize step physically
    /// relocates the file, REQ-015 c). `new_path` is relative to the item's
    /// root folder, matching the convention every other path column uses.
    async fn update_library_item_path(
        &self,
        user_id: UserId,
        id: LibraryItemId,
        new_path: &str,
    ) -> Result<(), DbError>;

    /// Check if user has a library item for this work with the given media type.
    ///
    /// Satisfies: RSS-FILTER-002
    async fn work_has_library_item(
        &self,
        user_id: UserId,
        work_id: WorkId,
        media_type: MediaType,
    ) -> Result<bool, DbError>;

    /// List library items needing tag sync: pending for enriched works,
    /// or synced/failed whose `tagged_at_generation` is older than the work's
    /// current `merge_generation`. The tag convergence sweep (Phase 7) calls this.
    async fn list_library_items_needing_tag_sync(
        &self,
        limit: u32,
    ) -> Result<Vec<LibraryItem>, DbError>;

    /// Update tag sync status and generation for a library item.
    async fn update_library_item_tag_status(
        &self,
        id: LibraryItemId,
        tag_status: TagStatus,
        tagged_at_generation: i64,
    ) -> Result<(), DbError>;

    /// Look up the item (if any) at a (user_id, root_folder_id, path) key,
    /// regardless of which work owns it. Read-only pre-check (Unit D2):
    /// mirrors the collision check `create_library_item` already runs
    /// internally, exposed here so an import path can detect a different
    /// work's row at the same target BEFORE any staging file I/O begins —
    /// not just at the (now post-rename) finalize step.
    async fn find_library_item_by_path(
        &self,
        user_id: UserId,
        root_folder_id: RootFolderId,
        path: &str,
    ) -> Result<Option<LibraryItem>, DbError>;
}

pub struct CreateLibraryItemDbRequest {
    pub user_id: UserId,
    pub work_id: WorkId,
    pub root_folder_id: RootFolderId,
    pub path: String,
    pub media_type: MediaType,
    pub file_size: i64,
    pub import_id: Option<String>,
    /// Initial tag sync state. Set by import pipeline:
    /// - `Synced` if inline tag write succeeded against an Enriched work
    /// - `Failed` if inline tag write failed
    /// - `Pending` for Unenriched/Failed/Conflict works (convergence sweep handles)
    pub tag_status: TagStatus,
    /// Generation snapshot at write time; convergence detects stale items.
    pub tagged_at_generation: i64,
}
