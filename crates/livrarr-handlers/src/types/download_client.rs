use livrarr_domain::{DownloadClientId, DownloadClientImplementation};
use serde::{Deserialize, Serialize};

use super::api_error::ApiError;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadClientResponse {
    pub id: DownloadClientId,
    pub name: String,
    pub implementation: DownloadClientImplementation,
    pub host: String,
    pub port: u16,
    pub use_ssl: bool,
    pub skip_ssl_validation: bool,
    pub url_base: Option<String>,
    pub username: Option<String>,
    pub category: String,
    pub enabled: bool,
    pub client_type: String,
    pub api_key_set: bool,
    pub is_default_for_protocol: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDownloadClientApiRequest {
    pub name: String,
    pub implementation: DownloadClientImplementation,
    pub host: String,
    pub port: u16,
    pub use_ssl: bool,
    pub skip_ssl_validation: bool,
    pub url_base: Option<String>,
    pub username: Option<String>,
    #[serde(skip_serializing)]
    pub password: Option<String>,
    pub category: String,
    pub enabled: bool,
    #[serde(skip_serializing)]
    pub api_key: Option<String>,
}

impl std::fmt::Debug for CreateDownloadClientApiRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateDownloadClientApiRequest")
            .field("name", &self.name)
            .field("implementation", &self.implementation)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("use_ssl", &self.use_ssl)
            .field("skip_ssl_validation", &self.skip_ssl_validation)
            .field("url_base", &self.url_base)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("category", &self.category)
            .field("enabled", &self.enabled)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDownloadClientApiRequest {
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub use_ssl: Option<bool>,
    pub skip_ssl_validation: Option<bool>,
    pub url_base: Option<String>,
    pub username: Option<String>,
    #[serde(default, deserialize_with = "crate::deserialize_optional_secret")]
    #[serde(skip_serializing)]
    pub password: Option<Option<String>>,
    pub category: Option<String>,
    pub enabled: Option<bool>,
    #[serde(default, deserialize_with = "crate::deserialize_optional_secret")]
    #[serde(skip_serializing)]
    pub api_key: Option<Option<String>>,
    pub is_default_for_protocol: Option<bool>,
}

impl std::fmt::Debug for UpdateDownloadClientApiRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateDownloadClientApiRequest")
            .field("name", &self.name)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("use_ssl", &self.use_ssl)
            .field("skip_ssl_validation", &self.skip_ssl_validation)
            .field("url_base", &self.url_base)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("category", &self.category)
            .field("enabled", &self.enabled)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("is_default_for_protocol", &self.is_default_for_protocol)
            .finish()
    }
}

#[trait_variant::make(Send)]
pub trait DownloadClientApi: Send + Sync {
    async fn list(&self) -> Result<Vec<DownloadClientResponse>, ApiError>;
    async fn create(
        &self,
        req: CreateDownloadClientApiRequest,
    ) -> Result<DownloadClientResponse, ApiError>;
    async fn get(&self, id: DownloadClientId) -> Result<DownloadClientResponse, ApiError>;
    async fn update(
        &self,
        id: DownloadClientId,
        req: UpdateDownloadClientApiRequest,
    ) -> Result<DownloadClientResponse, ApiError>;
    async fn delete(&self, id: DownloadClientId) -> Result<(), ApiError>;
    async fn test(&self, req: CreateDownloadClientApiRequest) -> Result<(), ApiError>;
}
