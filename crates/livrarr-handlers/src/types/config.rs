use livrarr_domain::LlmProvider;
use serde::{Deserialize, Serialize};

use super::api_error::ApiError;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamingConfigResponse {
    pub author_folder_format: String,
    pub book_folder_format: String,
    pub rename_files: bool,
    pub replace_illegal_chars: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaManagementConfigResponse {
    pub cwa_ingest_path: Option<String>,
    pub preferred_ebook_formats: Vec<String>,
    pub preferred_audiobook_formats: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProwlarrConfigResponse {
    pub url: Option<String>,
    pub api_key_set: bool,
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataConfigResponse {
    pub hardcover_enabled: bool,
    pub hardcover_api_token_set: bool,
    pub llm_enabled: bool,
    pub llm_provider: Option<LlmProvider>,
    pub llm_endpoint: Option<String>,
    pub llm_api_key_set: bool,
    pub llm_model: Option<String>,
    pub audnexus_url: String,
    pub languages: Vec<String>,
    #[serde(skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub provider_status: std::collections::HashMap<String, String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestProwlarrRequest {
    pub url: String,
    #[serde(skip_serializing)]
    pub api_key: String,
}

impl std::fmt::Debug for TestProwlarrRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestProwlarrRequest")
            .field("url", &self.url)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProwlarrImportRequest {
    pub url: String,
    #[serde(skip_serializing)]
    pub api_key: String,
}

impl std::fmt::Debug for ProwlarrImportRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProwlarrImportRequest")
            .field("url", &self.url)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProwlarrImportResponse {
    pub imported: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMediaManagementApiRequest {
    pub cwa_ingest_path: Option<String>,
    #[serde(default)]
    pub preferred_ebook_formats: Vec<String>,
    #[serde(default)]
    pub preferred_audiobook_formats: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProwlarrApiRequest {
    pub url: Option<String>,
    #[serde(default, deserialize_with = "crate::deserialize_optional_secret")]
    #[serde(skip_serializing)]
    pub api_key: Option<Option<String>>,
    pub enabled: Option<bool>,
}

impl std::fmt::Debug for UpdateProwlarrApiRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateProwlarrApiRequest")
            .field("url", &self.url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("enabled", &self.enabled)
            .finish()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailConfigResponse {
    pub enabled: bool,
    pub smtp_host: String,
    pub smtp_port: i32,
    pub encryption: String,
    pub username: Option<String>,
    pub password_set: bool,
    pub from_address: Option<String>,
    pub recipient_email: Option<String>,
    pub send_on_import: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateEmailApiRequest {
    pub enabled: Option<bool>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i32>,
    pub encryption: Option<String>,
    pub username: Option<String>,
    #[serde(default, deserialize_with = "crate::deserialize_optional_secret")]
    #[serde(skip_serializing)]
    pub password: Option<Option<String>>,
    pub from_address: Option<String>,
    pub recipient_email: Option<String>,
    pub send_on_import: Option<bool>,
}

impl std::fmt::Debug for UpdateEmailApiRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateEmailApiRequest")
            .field("enabled", &self.enabled)
            .field("smtp_host", &self.smtp_host)
            .field("smtp_port", &self.smtp_port)
            .field("encryption", &self.encryption)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("from_address", &self.from_address)
            .field("recipient_email", &self.recipient_email)
            .field("send_on_import", &self.send_on_import)
            .finish()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendEmailRequest {
    pub library_item_id: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMetadataApiRequest {
    pub hardcover_enabled: Option<bool>,
    #[serde(default, deserialize_with = "crate::deserialize_optional_secret")]
    #[serde(skip_serializing)]
    pub hardcover_api_token: Option<Option<String>>,
    pub llm_enabled: Option<bool>,
    pub llm_provider: Option<LlmProvider>,
    pub llm_endpoint: Option<String>,
    #[serde(default, deserialize_with = "crate::deserialize_optional_secret")]
    #[serde(skip_serializing)]
    pub llm_api_key: Option<Option<String>>,
    pub llm_model: Option<String>,
    pub audnexus_url: Option<String>,
    pub languages: Option<Vec<String>>,
}

impl std::fmt::Debug for UpdateMetadataApiRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateMetadataApiRequest")
            .field("hardcover_enabled", &self.hardcover_enabled)
            .field(
                "hardcover_api_token",
                &self.hardcover_api_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("llm_enabled", &self.llm_enabled)
            .field("llm_provider", &self.llm_provider)
            .field("llm_endpoint", &self.llm_endpoint)
            .field(
                "llm_api_key",
                &self.llm_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("llm_model", &self.llm_model)
            .field("audnexus_url", &self.audnexus_url)
            .field("languages", &self.languages)
            .finish()
    }
}

#[trait_variant::make(Send)]
pub trait ConfigApi: Send + Sync {
    async fn get_naming(&self) -> Result<NamingConfigResponse, ApiError>;
    async fn get_media_management(&self) -> Result<MediaManagementConfigResponse, ApiError>;
    async fn update_media_management(
        &self,
        req: UpdateMediaManagementApiRequest,
    ) -> Result<MediaManagementConfigResponse, ApiError>;
    async fn get_prowlarr(&self) -> Result<ProwlarrConfigResponse, ApiError>;
    async fn update_prowlarr(
        &self,
        req: UpdateProwlarrApiRequest,
    ) -> Result<ProwlarrConfigResponse, ApiError>;
    async fn test_prowlarr(&self, req: &TestProwlarrRequest) -> Result<(), ApiError>;
    async fn get_metadata(&self) -> Result<MetadataConfigResponse, ApiError>;
    async fn update_metadata(
        &self,
        req: UpdateMetadataApiRequest,
    ) -> Result<MetadataConfigResponse, ApiError>;
    async fn get_email(&self) -> Result<EmailConfigResponse, ApiError>;
    async fn update_email(
        &self,
        req: UpdateEmailApiRequest,
    ) -> Result<EmailConfigResponse, ApiError>;
}
