use livrarr_domain::{AuthorId, UserId};
use serde::{Deserialize, Serialize};

use super::api_error::ApiError;
use super::work::WorkDetailResponse;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorSearchResult {
    pub ol_key: String,
    pub name: String,
    pub sort_name: Option<String>,
}

#[trait_variant::make(Send)]
pub trait AuthorApi: Send + Sync {
    async fn lookup(
        &self,
        user_id: UserId,
        term: &str,
    ) -> Result<Vec<AuthorSearchResult>, ApiError>;
    async fn add(
        &self,
        user_id: UserId,
        req: AddAuthorApiRequest,
    ) -> Result<AuthorResponse, ApiError>;
    async fn list(&self, user_id: UserId) -> Result<Vec<AuthorResponse>, ApiError>;
    async fn get(&self, user_id: UserId, id: AuthorId) -> Result<AuthorDetailResponse, ApiError>;
    async fn update(
        &self,
        user_id: UserId,
        id: AuthorId,
        req: UpdateAuthorApiRequest,
    ) -> Result<AuthorResponse, ApiError>;
    async fn delete(&self, user_id: UserId, id: AuthorId) -> Result<(), ApiError>;
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddAuthorApiRequest {
    pub name: String,
    pub sort_name: Option<String>,
    pub ol_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAuthorApiRequest {
    #[serde(default, deserialize_with = "super::double_option::deserialize")]
    pub monitored: Option<Option<bool>>,
    #[serde(default, deserialize_with = "super::double_option::deserialize")]
    pub monitor_new_items: Option<Option<bool>>,
    #[serde(default, deserialize_with = "super::double_option::deserialize")]
    pub gr_key: Option<Option<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorResponse {
    pub id: AuthorId,
    pub name: String,
    pub sort_name: Option<String>,
    pub ol_key: Option<String>,
    pub gr_key: Option<String>,
    pub monitored: bool,
    pub monitor_new_items: bool,
    pub added_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorDetailResponse {
    pub author: AuthorResponse,
    pub works: Vec<WorkDetailResponse>,
}
