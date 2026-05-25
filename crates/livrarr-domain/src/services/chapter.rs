use crate::services::file::FileServiceError;
use crate::{AudiobookChapter, LibraryItemId, UserId};

#[trait_variant::make(Send)]
pub trait ChapterService: Send + Sync {
    async fn get_chapters(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
    ) -> Result<Vec<AudiobookChapter>, FileServiceError>;
}
