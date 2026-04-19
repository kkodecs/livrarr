use livrarr_db::{
    DownloadClientDb, GrabDb, LibraryItemDb, RemotePathMappingDb, RootFolderDb, WorkDb,
};
use livrarr_domain::services::{ImportIoService, ImportIoServiceError};
use livrarr_domain::{
    DownloadClient, DownloadClientId, Grab, GrabId, LibraryItem, LibraryItemId, RemotePathMapping,
    RootFolder, RootFolderId, UserId, Work, WorkId,
};

pub struct ImportIoServiceImpl<D> {
    db: D,
}

impl<D> ImportIoServiceImpl<D> {
    pub fn new(db: D) -> Self {
        Self { db }
    }
}

fn map_db_err(e: livrarr_domain::DbError) -> ImportIoServiceError {
    match e {
        livrarr_domain::DbError::NotFound { .. } => ImportIoServiceError::NotFound,
        other => ImportIoServiceError::Db(other),
    }
}

impl<D> ImportIoService for ImportIoServiceImpl<D>
where
    D: GrabDb
        + DownloadClientDb
        + WorkDb
        + LibraryItemDb
        + RootFolderDb
        + RemotePathMappingDb
        + Send
        + Sync
        + 'static,
{
    async fn get_grab(
        &self,
        user_id: UserId,
        grab_id: GrabId,
    ) -> Result<Grab, ImportIoServiceError> {
        self.db.get_grab(user_id, grab_id).await.map_err(map_db_err)
    }

    async fn get_download_client(
        &self,
        client_id: DownloadClientId,
    ) -> Result<DownloadClient, ImportIoServiceError> {
        self.db
            .get_download_client(client_id)
            .await
            .map_err(map_db_err)
    }

    async fn set_grab_content_path(
        &self,
        user_id: UserId,
        grab_id: GrabId,
        content_path: &str,
    ) -> Result<(), ImportIoServiceError> {
        self.db
            .set_grab_content_path(user_id, grab_id, content_path)
            .await
            .map_err(map_db_err)
    }

    async fn get_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Work, ImportIoServiceError> {
        self.db.get_work(user_id, work_id).await.map_err(map_db_err)
    }

    async fn list_library_items_by_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<LibraryItem>, ImportIoServiceError> {
        self.db
            .list_library_items_by_work(user_id, work_id)
            .await
            .map_err(map_db_err)
    }

    async fn get_root_folder(
        &self,
        root_folder_id: RootFolderId,
    ) -> Result<RootFolder, ImportIoServiceError> {
        self.db
            .get_root_folder(root_folder_id)
            .await
            .map_err(map_db_err)
    }

    async fn list_root_folders(&self) -> Result<Vec<RootFolder>, ImportIoServiceError> {
        self.db.list_root_folders().await.map_err(map_db_err)
    }

    async fn list_remote_path_mappings(
        &self,
    ) -> Result<Vec<RemotePathMapping>, ImportIoServiceError> {
        self.db
            .list_remote_path_mappings()
            .await
            .map_err(map_db_err)
    }

    async fn update_library_item_size(
        &self,
        user_id: UserId,
        item_id: LibraryItemId,
        new_size: i64,
    ) -> Result<(), ImportIoServiceError> {
        self.db
            .update_library_item_size(user_id, item_id, new_size)
            .await
            .map_err(map_db_err)
    }

    async fn create_library_item(
        &self,
        req: livrarr_domain::services::CreateLibraryItemRequest,
    ) -> Result<LibraryItem, ImportIoServiceError> {
        self.db
            .create_library_item(livrarr_db::CreateLibraryItemDbRequest {
                user_id: req.user_id,
                work_id: req.work_id,
                root_folder_id: req.root_folder_id,
                path: req.path,
                media_type: req.media_type,
                file_size: req.file_size,
                import_id: req.import_id,
            })
            .await
            .map_err(map_db_err)
    }
}
