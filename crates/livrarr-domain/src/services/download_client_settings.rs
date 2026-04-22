use crate::{
    settings::{CreateDownloadClientParams, UpdateDownloadClientParams},
    DbError, DownloadClient, DownloadClientId,
};

#[trait_variant::make(Send)]
pub trait DownloadClientSettingsService: Send + Sync {
    async fn get_download_client(&self, id: DownloadClientId) -> Result<DownloadClient, DbError>;
    async fn get_download_client_with_credentials(
        &self,
        id: DownloadClientId,
    ) -> Result<DownloadClient, DbError>;
    async fn list_download_clients(&self) -> Result<Vec<DownloadClient>, DbError>;
    async fn create_download_client(
        &self,
        params: CreateDownloadClientParams,
    ) -> Result<DownloadClient, DbError>;
    async fn update_download_client(
        &self,
        id: DownloadClientId,
        params: UpdateDownloadClientParams,
    ) -> Result<DownloadClient, DbError>;
    async fn delete_download_client(&self, id: DownloadClientId) -> Result<(), DbError>;
}
