//! Root folder data access: `RootFolderDb` trait.

use crate::{DbError, MediaType, RootFolder, RootFolderId};

/// Root folder data access.
/// Shared infrastructure: admin-managed, visible to all users.
///
/// Satisfies: IMPORT-001, IMPORT-002, IMPORT-004, AUTH-004
#[trait_variant::make(Send)]
pub trait RootFolderDb: Send + Sync {
    async fn get_root_folder(&self, id: RootFolderId) -> Result<RootFolder, DbError>;
    async fn list_root_folders(&self) -> Result<Vec<RootFolder>, DbError>;
    async fn create_root_folder(
        &self,
        path: &str,
        media_type: MediaType,
    ) -> Result<RootFolder, DbError>;
    async fn delete_root_folder(&self, id: RootFolderId) -> Result<(), DbError>;

    /// Get root folder by media type (at most one per type).
    async fn get_root_folder_by_media_type(
        &self,
        media_type: MediaType,
    ) -> Result<Option<RootFolder>, DbError>;
}
