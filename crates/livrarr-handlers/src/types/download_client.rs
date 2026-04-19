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

#[derive(Debug, Serialize, Deserialize)]
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
    pub password: Option<String>,
    pub category: String,
    pub enabled: bool,
    pub api_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
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
    pub password: Option<Option<String>>,
    pub category: Option<String>,
    pub enabled: Option<bool>,
    #[serde(default, deserialize_with = "crate::deserialize_optional_secret")]
    pub api_key: Option<Option<String>>,
    pub is_default_for_protocol: Option<bool>,
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
