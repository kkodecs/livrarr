use livrarr_domain::RemotePathMappingId;
use serde::{Deserialize, Serialize};

use super::api_error::ApiError;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePathMappingResponse {
    pub id: RemotePathMappingId,
    pub host: String,
    pub remote_path: String,
    pub local_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRemotePathMappingRequest {
    pub host: Option<String>,
    pub remote_path: Option<String>,
    pub local_path: Option<String>,
}

#[trait_variant::make(Send)]
pub trait RemotePathMappingApi: Send + Sync {
    async fn list(&self) -> Result<Vec<RemotePathMappingResponse>, ApiError>;
    async fn create(
        &self,
        host: &str,
        remote_path: &str,
        local_path: &str,
    ) -> Result<RemotePathMappingResponse, ApiError>;
    async fn get(&self, id: RemotePathMappingId) -> Result<RemotePathMappingResponse, ApiError>;
    async fn update(
        &self,
        id: RemotePathMappingId,
        req: UpdateRemotePathMappingRequest,
    ) -> Result<RemotePathMappingResponse, ApiError>;
    async fn delete(&self, id: RemotePathMappingId) -> Result<(), ApiError>;
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRemotePathMappingApiRequest {
    pub host: String,
    pub remote_path: String,
    pub local_path: String,
}
