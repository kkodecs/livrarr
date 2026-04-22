use crate::{
    DbError, LibraryItem, LibraryItemId, MediaType, RootFolder, RootFolderId, UserId, Work, WorkId,
};

#[derive(Debug, thiserror::Error)]
pub enum ManualImportServiceError {
    #[error("not found")]
    NotFound,
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

#[trait_variant::make(Send)]
pub trait ManualImportService: Send + Sync {
    async fn list_works(&self, user_id: UserId) -> Result<Vec<Work>, ManualImportServiceError>;

    async fn list_root_folders(&self) -> Result<Vec<RootFolder>, ManualImportServiceError>;

    async fn list_library_items_by_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<LibraryItem>, ManualImportServiceError>;

    async fn list_library_items_by_work_ids(
        &self,
        user_id: UserId,
        work_ids: &[WorkId],
    ) -> Result<Vec<LibraryItem>, ManualImportServiceError>;

    async fn get_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Work, ManualImportServiceError>;

    async fn delete_library_item(
        &self,
        user_id: UserId,
        item_id: LibraryItemId,
    ) -> Result<LibraryItem, ManualImportServiceError>;

    async fn create_library_item(
        &self,
        user_id: UserId,
        work_id: WorkId,
        root_folder_id: RootFolderId,
        path: String,
        media_type: MediaType,
        file_size: i64,
    ) -> Result<LibraryItem, ManualImportServiceError>;

    async fn create_history_event(
        &self,
        user_id: UserId,
        work_id: Option<WorkId>,
        event_type: crate::EventType,
        data: serde_json::Value,
    ) -> Result<(), ManualImportServiceError>;
}
