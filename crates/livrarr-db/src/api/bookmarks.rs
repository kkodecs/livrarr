//! Bookmark data access: `BookmarkDb` trait.

use crate::{Bookmark, DbError, LibraryItemId, UserId};

/// Bookmark data access.
#[trait_variant::make(Send)]
pub trait BookmarkDb: Send + Sync {
    async fn list_bookmarks(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
    ) -> Result<Vec<Bookmark>, DbError>;

    async fn create_bookmark(&self, bookmark: &Bookmark) -> Result<Bookmark, DbError>;

    async fn rename_bookmark(
        &self,
        user_id: UserId,
        bookmark_id: i64,
        name: &str,
    ) -> Result<(), DbError>;

    async fn delete_bookmark(&self, user_id: UserId, bookmark_id: i64) -> Result<(), DbError>;
}
