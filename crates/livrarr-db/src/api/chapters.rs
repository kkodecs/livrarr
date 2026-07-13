//! Chapter data access: `ChapterDb` trait.

use crate::{AudiobookChapter, DbError, LibraryItemId};

/// Chapter data access.
#[trait_variant::make(Send)]
pub trait ChapterDb: Send + Sync {
    async fn get_chapters(
        &self,
        library_item_id: LibraryItemId,
    ) -> Result<Vec<AudiobookChapter>, DbError>;

    async fn replace_chapters(
        &self,
        library_item_id: LibraryItemId,
        chapters: &[AudiobookChapter],
    ) -> Result<(), DbError>;

    async fn has_chapters(&self, library_item_id: LibraryItemId) -> Result<bool, DbError>;

    async fn list_unscanned_audiobook_items(&self)
        -> Result<Vec<(LibraryItemId, String)>, DbError>;

    async fn update_chapter_scan_result(
        &self,
        library_item_id: LibraryItemId,
        chapter_scan_status: &str,
        duration_seconds: Option<f64>,
    ) -> Result<(), DbError>;
}
