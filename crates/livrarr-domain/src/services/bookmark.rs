use crate::services::file::FileServiceError;
use crate::{Bookmark, LibraryItemId, UserId};

#[trait_variant::make(Send)]
pub trait BookmarkService: Send + Sync {
    async fn list_bookmarks(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
    ) -> Result<Vec<Bookmark>, FileServiceError>;

    async fn create_bookmark(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
        position: &str,
        sort_key: f64,
        name: &str,
        chapter_title: Option<&str>,
    ) -> Result<Bookmark, FileServiceError>;

    async fn rename_bookmark(
        &self,
        user_id: UserId,
        bookmark_id: i64,
        name: &str,
    ) -> Result<(), FileServiceError>;

    async fn delete_bookmark(
        &self,
        user_id: UserId,
        bookmark_id: i64,
    ) -> Result<(), FileServiceError>;
}
