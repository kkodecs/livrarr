use crate::{
    DbError, DownloadClient, DownloadClientId, Grab, GrabId, LibraryItem, LibraryItemId, MediaType,
    RemotePathMapping, RootFolder, RootFolderId, UserId, Work, WorkId,
};

pub struct CreateLibraryItemRequest {
    pub user_id: UserId,
    pub work_id: WorkId,
    pub root_folder_id: RootFolderId,
    pub path: String,
    pub media_type: MediaType,
    pub file_size: i64,
    pub import_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportIoServiceError {
    #[error("not found")]
    NotFound,
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

#[trait_variant::make(Send)]
pub trait ImportIoService: Send + Sync {
    async fn get_grab(
        &self,
        user_id: UserId,
        grab_id: GrabId,
    ) -> Result<Grab, ImportIoServiceError>;

    async fn get_download_client(
        &self,
        client_id: DownloadClientId,
    ) -> Result<DownloadClient, ImportIoServiceError>;

    async fn set_grab_content_path(
        &self,
        user_id: UserId,
        grab_id: GrabId,
        content_path: &str,
    ) -> Result<(), ImportIoServiceError>;

    async fn get_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Work, ImportIoServiceError>;

    async fn list_library_items_by_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<LibraryItem>, ImportIoServiceError>;

    async fn get_root_folder(
        &self,
        root_folder_id: RootFolderId,
    ) -> Result<RootFolder, ImportIoServiceError>;

    async fn list_root_folders(&self) -> Result<Vec<RootFolder>, ImportIoServiceError>;

    async fn list_remote_path_mappings(
        &self,
    ) -> Result<Vec<RemotePathMapping>, ImportIoServiceError>;

    async fn update_library_item_size(
        &self,
        user_id: UserId,
        item_id: LibraryItemId,
        new_size: i64,
    ) -> Result<(), ImportIoServiceError>;

    async fn create_library_item(
        &self,
        req: CreateLibraryItemRequest,
    ) -> Result<LibraryItem, ImportIoServiceError>;
}
