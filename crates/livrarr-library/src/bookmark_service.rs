use livrarr_db::{BookmarkDb, LibraryItemDb};
use livrarr_domain::services::{BookmarkService, FileServiceError};
use livrarr_domain::{Bookmark, LibraryItemId, UserId};

pub struct BookmarkServiceImpl<D> {
    db: D,
}

impl<D> BookmarkServiceImpl<D> {
    pub fn new(db: D) -> Self {
        Self { db }
    }
}

impl<D> BookmarkService for BookmarkServiceImpl<D>
where
    D: BookmarkDb + LibraryItemDb + Send + Sync + 'static,
{
    async fn list_bookmarks(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
    ) -> Result<Vec<Bookmark>, FileServiceError> {
        let _item = self
            .db
            .get_library_item(user_id, library_item_id)
            .await
            .map_err(FileServiceError::Db)?;

        self.db
            .list_bookmarks(user_id, library_item_id)
            .await
            .map_err(FileServiceError::Db)
    }

    async fn create_bookmark(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
        position: &str,
        sort_key: f64,
        name: &str,
        chapter_title: Option<&str>,
    ) -> Result<Bookmark, FileServiceError> {
        let item = self
            .db
            .get_library_item(user_id, library_item_id)
            .await
            .map_err(FileServiceError::Db)?;

        let bookmark = Bookmark {
            id: 0,
            user_id,
            work_id: item.work_id,
            library_item_id,
            media_type: item.media_type,
            position: position.to_string(),
            sort_key,
            name: name.to_string(),
            chapter_title: chapter_title.map(|s| s.to_string()),
            paired_bookmark_id: None,
            created_at: Default::default(),
        };

        self.db
            .create_bookmark(&bookmark)
            .await
            .map_err(FileServiceError::Db)
    }

    async fn rename_bookmark(
        &self,
        user_id: UserId,
        bookmark_id: i64,
        name: &str,
    ) -> Result<(), FileServiceError> {
        self.db
            .rename_bookmark(user_id, bookmark_id, name)
            .await
            .map_err(FileServiceError::Db)
    }

    async fn delete_bookmark(
        &self,
        user_id: UserId,
        bookmark_id: i64,
    ) -> Result<(), FileServiceError> {
        self.db
            .delete_bookmark(user_id, bookmark_id)
            .await
            .map_err(FileServiceError::Db)
    }
}
