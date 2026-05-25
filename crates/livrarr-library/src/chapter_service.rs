use livrarr_db::{ChapterDb, LibraryItemDb};
use livrarr_domain::services::{ChapterService, FileServiceError};
use livrarr_domain::{AudiobookChapter, LibraryItemId, UserId};

pub struct ChapterServiceImpl<D> {
    db: D,
}

impl<D> ChapterServiceImpl<D> {
    pub fn new(db: D) -> Self {
        Self { db }
    }
}

impl<D> ChapterService for ChapterServiceImpl<D>
where
    D: ChapterDb + LibraryItemDb + Send + Sync + 'static,
{
    async fn get_chapters(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
    ) -> Result<Vec<AudiobookChapter>, FileServiceError> {
        let _item = self
            .db
            .get_library_item(user_id, library_item_id)
            .await
            .map_err(FileServiceError::Db)?;

        self.db
            .get_chapters(library_item_id)
            .await
            .map_err(FileServiceError::Db)
    }
}
