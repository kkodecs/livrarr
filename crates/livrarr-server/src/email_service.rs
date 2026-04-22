use std::sync::Arc;

use livrarr_domain::services::{EmailService, EmailServiceError, SettingsService};

use crate::services::settings_service::LiveSettingsService;

#[derive(Clone)]
pub struct LiveEmailService<D: Send + Sync> {
    settings: Arc<LiveSettingsService<D>>,
}

impl<D: Send + Sync> LiveEmailService<D> {
    pub fn new(settings: Arc<LiveSettingsService<D>>) -> Self {
        Self { settings }
    }
}

impl<D> EmailService for LiveEmailService<D>
where
    D: livrarr_db::ConfigDb
        + livrarr_db::DownloadClientDb
        + livrarr_db::IndexerDb
        + livrarr_db::RootFolderDb
        + livrarr_db::RemotePathMappingDb
        + livrarr_db::ProviderRetryStateDb
        + Send
        + Sync,
{
    async fn send_test(&self) -> Result<(), EmailServiceError> {
        let cfg = self
            .settings
            .get_email_config()
            .await
            .map_err(|e| EmailServiceError::Config(e.to_string()))?;
        crate::infra::email::send_test(&cfg)
            .await
            .map_err(EmailServiceError::Send)
    }

    async fn send_file(
        &self,
        file_bytes: Vec<u8>,
        filename: &str,
        extension: &str,
    ) -> Result<(), EmailServiceError> {
        let cfg = self
            .settings
            .get_email_config()
            .await
            .map_err(|e| EmailServiceError::Config(e.to_string()))?;
        crate::infra::email::send_file(&cfg, file_bytes, filename, extension)
            .await
            .map_err(EmailServiceError::Send)
    }
}
