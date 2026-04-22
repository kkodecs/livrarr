use crate::{DbError, DownloadClient, DownloadClientId};

#[trait_variant::make(Send)]
pub trait DownloadClientCredentialService: Send + Sync {
    async fn get_download_client_with_credentials(
        &self,
        id: DownloadClientId,
    ) -> Result<DownloadClient, DbError>;
}
