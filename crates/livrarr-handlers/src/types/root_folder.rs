use livrarr_domain::{MediaType, RootFolderId};
use serde::{Deserialize, Serialize};

use super::api_error::ApiError;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootFolderResponse {
    pub id: RootFolderId,
    pub path: String,
    pub media_type: MediaType,
    pub free_space: Option<i64>,
    pub total_space: Option<i64>,
}

#[trait_variant::make(Send)]
pub trait RootFolderApi: Send + Sync {
    async fn list(&self) -> Result<Vec<RootFolderResponse>, ApiError>;
    async fn create(
        &self,
        path: &str,
        media_type: MediaType,
    ) -> Result<RootFolderResponse, ApiError>;
    async fn get(&self, id: RootFolderId) -> Result<RootFolderResponse, ApiError>;
    async fn delete(&self, id: RootFolderId) -> Result<(), ApiError>;
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRootFolderRequest {
    pub path: String,
    pub media_type: MediaType,
}
