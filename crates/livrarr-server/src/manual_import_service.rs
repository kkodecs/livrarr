use livrarr_db::{
    record_history, ConfigDb, CreateLibraryItemDbRequest, HistoryDb, LibraryItemDb, RootFolderDb,
    WorkDb,
};
use livrarr_domain::history_events;
use livrarr_domain::services::{ManualImportService, ManualImportServiceError};
use livrarr_domain::{
    LibraryItem, LibraryItemId, MediaType, RootFolder, RootFolderId, UserId, Work, WorkId,
};

pub struct ManualImportServiceImpl<D> {
    db: D,
}

impl<D> ManualImportServiceImpl<D> {
    pub fn new(db: D) -> Self {
        Self { db }
    }
}

fn map_db_err(e: livrarr_domain::DbError) -> ManualImportServiceError {
    match e {
        livrarr_domain::DbError::NotFound { .. } => ManualImportServiceError::NotFound,
        other => ManualImportServiceError::Db(other),
    }
}

impl<D> ManualImportService for ManualImportServiceImpl<D>
where
    D: WorkDb + RootFolderDb + LibraryItemDb + ConfigDb + HistoryDb + Send + Sync + 'static,
{
    async fn list_works(&self, user_id: UserId) -> Result<Vec<Work>, ManualImportServiceError> {
        self.db.list_works(user_id).await.map_err(map_db_err)
    }

    async fn list_root_folders(&self) -> Result<Vec<RootFolder>, ManualImportServiceError> {
        self.db.list_root_folders().await.map_err(map_db_err)
    }

    async fn list_library_items_by_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<LibraryItem>, ManualImportServiceError> {
        self.db
            .list_library_items_by_work(user_id, work_id)
            .await
            .map_err(map_db_err)
    }

    async fn list_library_items_by_work_ids(
        &self,
        user_id: UserId,
        work_ids: &[WorkId],
    ) -> Result<Vec<LibraryItem>, ManualImportServiceError> {
        self.db
            .list_library_items_by_work_ids(user_id, work_ids)
            .await
            .map_err(map_db_err)
    }

    async fn get_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Work, ManualImportServiceError> {
        self.db.get_work(user_id, work_id).await.map_err(map_db_err)
    }

    async fn delete_library_item(
        &self,
        user_id: UserId,
        item_id: LibraryItemId,
    ) -> Result<LibraryItem, ManualImportServiceError> {
        let item = self
            .db
            .delete_library_item(user_id, item_id)
            .await
            .map_err(map_db_err)?;
        let work_title = self
            .db
            .get_work(user_id, item.work_id)
            .await
            .map(|w| w.title)
            .unwrap_or_default();
        record_history(
            &self.db,
            user_id,
            history_events::file_deleted(
                item.work_id,
                &work_title,
                &item.path,
                item.media_type.as_str(),
                false,
            ),
        )
        .await;
        Ok(item)
    }

    async fn create_library_item(
        &self,
        user_id: UserId,
        work_id: WorkId,
        root_folder_id: RootFolderId,
        path: String,
        media_type: MediaType,
        file_size: i64,
    ) -> Result<LibraryItem, ManualImportServiceError> {
        self.db
            .create_library_item(CreateLibraryItemDbRequest {
                user_id,
                work_id,
                root_folder_id,
                path,
                media_type,
                file_size,
                import_id: None,
                tag_status: livrarr_db::TagStatus::Pending,
                tagged_at_generation: 0,
            })
            .await
            .map_err(map_db_err)
    }
}
