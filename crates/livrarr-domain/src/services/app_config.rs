use crate::{
    settings::{UpdateEmailParams, UpdateMediaManagementParams, UpdateMetadataParams},
    DbError,
};

#[trait_variant::make(Send)]
pub trait AppConfigService: Send + Sync {
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
}
