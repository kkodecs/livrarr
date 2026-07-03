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
    /// The default language for newly added works: applied wherever a
    /// creation door has no explicit language for the book.
    async fn get_default_language(&self) -> Result<String, DbError>;
    async fn update_default_language(&self, language: &str) -> Result<String, DbError>;
    /// Validate and normalize a default-language code against the supported
    /// set. Returns the normalized code; `Err` carries the reason.
    async fn validate_default_language(&self, language: &str) -> Result<String, String>;
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
        google_books_api_key: Option<&str>,
    ) -> Result<Vec<String>, String>;
}
