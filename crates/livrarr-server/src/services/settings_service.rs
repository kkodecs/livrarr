use livrarr_db::{
    ConfigDb, DownloadClientDb, DownloadClientId, IndexerDb, IndexerId, ProviderRetryStateDb,
    RemotePathMappingDb, RemotePathMappingId, RootFolderDb, RootFolderId,
};
use livrarr_domain::settings::{
    CreateDownloadClientParams, CreateIndexerParams, EmailConfig, MediaManagementConfig,
    MetadataConfig, NamingConfig, ProwlarrConfig, UpdateDownloadClientParams, UpdateEmailParams,
    UpdateIndexerConfigParams, UpdateIndexerParams, UpdateMediaManagementParams,
    UpdateMetadataParams, UpdateProwlarrParams,
};
use livrarr_domain::{
    DbError, DownloadClient, Indexer, IndexerConfig, MediaType, RemotePathMapping, RootFolder,
};

pub use livrarr_domain::services::SettingsService;

// =============================================================================
// LiveSettingsService
// =============================================================================

pub struct LiveSettingsService<DB> {
    db: DB,
}

impl<DB> LiveSettingsService<DB> {
    pub fn new(db: DB) -> Self {
        Self { db }
    }
}

impl<DB> SettingsService for LiveSettingsService<DB>
where
    DB: ConfigDb
        + DownloadClientDb
        + IndexerDb
        + RootFolderDb
        + RemotePathMappingDb
        + ProviderRetryStateDb,
{
    async fn get_naming_config(&self) -> Result<NamingConfig, DbError> {
        self.db.get_naming_config().await
    }

    async fn get_media_management_config(&self) -> Result<MediaManagementConfig, DbError> {
        self.db.get_media_management_config().await
    }

    async fn update_media_management_config(
        &self,
        params: UpdateMediaManagementParams,
    ) -> Result<MediaManagementConfig, DbError> {
        self.db.update_media_management_config(params.into()).await
    }

    async fn get_metadata_config(&self) -> Result<MetadataConfig, DbError> {
        self.db.get_metadata_config().await
    }

    async fn update_metadata_config(
        &self,
        params: UpdateMetadataParams,
    ) -> Result<MetadataConfig, DbError> {
        let cfg = self.db.update_metadata_config(params.into()).await?;

        // Reset NotConfigured retry states for providers whose config may have changed.
        // Cheap no-op DELETE if no rows match.
        use livrarr_domain::MetadataProvider;
        if let Err(e) = self
            .db
            .reset_not_configured_outcomes(MetadataProvider::Hardcover)
            .await
        {
            tracing::warn!("failed to reset NotConfigured outcomes for Hardcover: {e}");
        }

        Ok(cfg)
    }

    async fn validate_metadata_languages(
        &self,
        languages: &[String],
        llm_enabled: Option<bool>,
        llm_endpoint: Option<&str>,
        llm_api_key: Option<&str>,
        llm_model: Option<&str>,
    ) -> Result<Vec<String>, String> {
        let existing = self
            .db
            .get_metadata_config()
            .await
            .map_err(|e| format!("failed to read existing config: {e}"))?;
        let effective_llm_enabled = llm_enabled.unwrap_or(existing.llm_enabled);
        let effective_endpoint = llm_endpoint.or(existing.llm_endpoint.as_deref());
        let effective_key = llm_api_key
            .map(|s| s.to_string())
            .or(existing.llm_api_key.clone());
        let effective_model = llm_model.or(existing.llm_model.as_deref());
        let llm_configured = livrarr_metadata::language::is_llm_configured(
            effective_llm_enabled,
            effective_endpoint,
            effective_key.as_deref(),
            effective_model,
        );
        livrarr_metadata::language::validate_languages(languages, llm_configured)
    }

    async fn get_prowlarr_config(&self) -> Result<ProwlarrConfig, DbError> {
        self.db.get_prowlarr_config().await
    }

    async fn update_prowlarr_config(
        &self,
        params: UpdateProwlarrParams,
    ) -> Result<ProwlarrConfig, DbError> {
        self.db.update_prowlarr_config(params.into()).await
    }

    async fn get_email_config(&self) -> Result<EmailConfig, DbError> {
        self.db.get_email_config().await
    }

    async fn update_email_config(&self, params: UpdateEmailParams) -> Result<EmailConfig, DbError> {
        self.db.update_email_config(params.into()).await
    }

    async fn get_indexer_config(&self) -> Result<IndexerConfig, DbError> {
        self.db.get_indexer_config().await
    }

    async fn update_indexer_config(
        &self,
        params: UpdateIndexerConfigParams,
    ) -> Result<IndexerConfig, DbError> {
        self.db.update_indexer_config(params.into()).await
    }

    async fn get_download_client(&self, id: DownloadClientId) -> Result<DownloadClient, DbError> {
        self.db.get_download_client(id).await
    }

    async fn get_download_client_with_credentials(
        &self,
        id: DownloadClientId,
    ) -> Result<DownloadClient, DbError> {
        self.db.get_download_client_with_credentials(id).await
    }

    async fn list_download_clients(&self) -> Result<Vec<DownloadClient>, DbError> {
        self.db.list_download_clients().await
    }

    async fn create_download_client(
        &self,
        params: CreateDownloadClientParams,
    ) -> Result<DownloadClient, DbError> {
        self.db.create_download_client(params.into()).await
    }

    async fn update_download_client(
        &self,
        id: DownloadClientId,
        params: UpdateDownloadClientParams,
    ) -> Result<DownloadClient, DbError> {
        self.db.update_download_client(id, params.into()).await
    }

    async fn delete_download_client(&self, id: DownloadClientId) -> Result<(), DbError> {
        self.db.delete_download_client(id).await
    }

    async fn get_indexer(&self, id: IndexerId) -> Result<Indexer, DbError> {
        self.db.get_indexer(id).await
    }

    async fn get_indexer_with_credentials(&self, id: IndexerId) -> Result<Indexer, DbError> {
        self.db.get_indexer_with_credentials(id).await
    }

    async fn list_indexers(&self) -> Result<Vec<Indexer>, DbError> {
        self.db.list_indexers().await
    }

    async fn create_indexer(&self, params: CreateIndexerParams) -> Result<Indexer, DbError> {
        self.db.create_indexer(params.into()).await
    }

    async fn update_indexer(
        &self,
        id: IndexerId,
        params: UpdateIndexerParams,
    ) -> Result<Indexer, DbError> {
        self.db.update_indexer(id, params.into()).await
    }

    async fn delete_indexer(&self, id: IndexerId) -> Result<(), DbError> {
        self.db.delete_indexer(id).await
    }

    async fn set_supports_book_search(&self, id: IndexerId, supports: bool) -> Result<(), DbError> {
        self.db.set_supports_book_search(id, supports).await
    }

    async fn get_root_folder(&self, id: RootFolderId) -> Result<RootFolder, DbError> {
        self.db.get_root_folder(id).await
    }

    async fn list_root_folders(&self) -> Result<Vec<RootFolder>, DbError> {
        self.db.list_root_folders().await
    }

    async fn create_root_folder(
        &self,
        path: &str,
        media_type: MediaType,
    ) -> Result<RootFolder, DbError> {
        self.db.create_root_folder(path, media_type).await
    }

    async fn delete_root_folder(&self, id: RootFolderId) -> Result<(), DbError> {
        self.db.delete_root_folder(id).await
    }

    async fn get_remote_path_mapping(
        &self,
        id: RemotePathMappingId,
    ) -> Result<RemotePathMapping, DbError> {
        self.db.get_remote_path_mapping(id).await
    }

    async fn list_remote_path_mappings(&self) -> Result<Vec<RemotePathMapping>, DbError> {
        self.db.list_remote_path_mappings().await
    }

    async fn create_remote_path_mapping(
        &self,
        host: &str,
        remote_path: &str,
        local_path: &str,
    ) -> Result<RemotePathMapping, DbError> {
        self.db
            .create_remote_path_mapping(host, remote_path, local_path)
            .await
    }

    async fn update_remote_path_mapping(
        &self,
        id: RemotePathMappingId,
        host: &str,
        remote_path: &str,
        local_path: &str,
    ) -> Result<RemotePathMapping, DbError> {
        self.db
            .update_remote_path_mapping(id, host, remote_path, local_path)
            .await
    }

    async fn delete_remote_path_mapping(&self, id: RemotePathMappingId) -> Result<(), DbError> {
        self.db.delete_remote_path_mapping(id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use livrarr_db::{ProviderRetryStateDb, UserDb, WorkDb};
    use livrarr_domain::{MetadataProvider, OutcomeClass};

    #[tokio::test]
    async fn update_metadata_config_resets_not_configured_outcomes() {
        let db = livrarr_db::create_test_db().await;

        let user = db
            .create_user(livrarr_db::CreateUserDbRequest {
                username: "test".into(),
                password_hash: "hash".into(),
                api_key_hash: "keyhash".into(),
                role: livrarr_domain::UserRole::Admin,
            })
            .await
            .unwrap();
        let work = db
            .create_work(livrarr_db::CreateWorkDbRequest {
                user_id: user.id,
                title: "Test Work".into(),
                author_name: "Test Author".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        // Record NotConfigured for Hardcover.
        db.record_terminal_outcome(
            user.id,
            work.id,
            MetadataProvider::Hardcover,
            OutcomeClass::NotConfigured,
            None,
        )
        .await
        .unwrap();

        assert!(db
            .get_retry_state(user.id, work.id, MetadataProvider::Hardcover)
            .await
            .unwrap()
            .is_some());

        // Call update_metadata_config through LiveSettingsService — all None = no-op
        // config change, but the reset side-effect should still fire.
        let svc = LiveSettingsService::new(db.clone());
        svc.update_metadata_config(UpdateMetadataParams {
            hardcover_enabled: None,
            hardcover_api_token: None,
            llm_enabled: None,
            llm_provider: None,
            llm_endpoint: None,
            llm_api_key: None,
            llm_model: None,
            audnexus_url: None,
            languages: None,
        })
        .await
        .unwrap();

        // NotConfigured row should be gone.
        assert!(
            db.get_retry_state(user.id, work.id, MetadataProvider::Hardcover)
                .await
                .unwrap()
                .is_none(),
            "NotConfigured row should be deleted after config save"
        );
    }
}
