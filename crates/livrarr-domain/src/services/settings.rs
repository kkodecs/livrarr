use crate::{
    settings::{
        CreateDownloadClientParams, CreateIndexerParams, UpdateDownloadClientParams,
        UpdateEmailParams, UpdateIndexerConfigParams, UpdateIndexerParams,
        UpdateMediaManagementParams, UpdateMetadataParams, UpdateProwlarrParams,
    },
    DbError, DownloadClient, DownloadClientId, Indexer, IndexerConfig, IndexerId, MediaType,
    RemotePathMapping, RemotePathMappingId, RootFolder, RootFolderId,
};

#[trait_variant::make(Send)]
pub trait SettingsService: Send + Sync {
    // --- Config ---
    async fn get_naming_config(&self) -> Result<crate::settings::NamingConfig, DbError>;
    async fn get_media_management_config(
        &self,
    ) -> Result<crate::settings::MediaManagementConfig, DbError>;
    async fn update_media_management_config(
        &self,
        params: UpdateMediaManagementParams,
    ) -> Result<crate::settings::MediaManagementConfig, DbError>;
    async fn get_metadata_config(&self) -> Result<crate::settings::MetadataConfig, DbError>;
    async fn update_metadata_config(
        &self,
        params: UpdateMetadataParams,
    ) -> Result<crate::settings::MetadataConfig, DbError>;
    async fn get_prowlarr_config(&self) -> Result<crate::settings::ProwlarrConfig, DbError>;
    async fn update_prowlarr_config(
        &self,
        params: UpdateProwlarrParams,
    ) -> Result<crate::settings::ProwlarrConfig, DbError>;
    async fn get_email_config(&self) -> Result<crate::settings::EmailConfig, DbError>;
    async fn update_email_config(
        &self,
        params: UpdateEmailParams,
    ) -> Result<crate::settings::EmailConfig, DbError>;
    async fn validate_metadata_languages(
        &self,
        languages: &[String],
        llm_enabled: Option<bool>,
        llm_endpoint: Option<&str>,
        llm_api_key: Option<&str>,
        llm_model: Option<&str>,
    ) -> Result<Vec<String>, String>;

    async fn get_indexer_config(&self) -> Result<IndexerConfig, DbError>;
    async fn update_indexer_config(
        &self,
        params: UpdateIndexerConfigParams,
    ) -> Result<IndexerConfig, DbError>;

    // --- Download clients ---
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

    // --- Indexers ---
    async fn get_indexer(&self, id: IndexerId) -> Result<Indexer, DbError>;
    async fn get_indexer_with_credentials(&self, id: IndexerId) -> Result<Indexer, DbError>;
    async fn list_indexers(&self) -> Result<Vec<Indexer>, DbError>;
    async fn create_indexer(&self, params: CreateIndexerParams) -> Result<Indexer, DbError>;
    async fn update_indexer(
        &self,
        id: IndexerId,
        params: UpdateIndexerParams,
    ) -> Result<Indexer, DbError>;
    async fn delete_indexer(&self, id: IndexerId) -> Result<(), DbError>;
    async fn set_supports_book_search(&self, id: IndexerId, supports: bool) -> Result<(), DbError>;

    // --- Root folders ---
    async fn get_root_folder(&self, id: RootFolderId) -> Result<RootFolder, DbError>;
    async fn list_root_folders(&self) -> Result<Vec<RootFolder>, DbError>;
    async fn create_root_folder(
        &self,
        path: &str,
        media_type: MediaType,
    ) -> Result<RootFolder, DbError>;
    async fn delete_root_folder(&self, id: RootFolderId) -> Result<(), DbError>;

    // --- Remote path mappings ---
    async fn get_remote_path_mapping(
        &self,
        id: RemotePathMappingId,
    ) -> Result<RemotePathMapping, DbError>;
    async fn list_remote_path_mappings(&self) -> Result<Vec<RemotePathMapping>, DbError>;
    async fn create_remote_path_mapping(
        &self,
        host: &str,
        remote_path: &str,
        local_path: &str,
    ) -> Result<RemotePathMapping, DbError>;
    async fn update_remote_path_mapping(
        &self,
        id: RemotePathMappingId,
        host: &str,
        remote_path: &str,
        local_path: &str,
    ) -> Result<RemotePathMapping, DbError>;
    async fn delete_remote_path_mapping(&self, id: RemotePathMappingId) -> Result<(), DbError>;
}
