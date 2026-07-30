use chrono::{DateTime, Utc};
use livrarr_domain::{
    AuthorId, AuthorLinkCandidate, AuthorLinkState, AuthorNameSource, AuthorProvider,
    AuthorRouteProvenance, AuthorRouteState, UserId,
};
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
    #[serde(default, deserialize_with = "super::double_option::deserialize")]
    pub monitor_language: Option<Option<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorResponse {
    pub id: AuthorId,
    pub name: String,
    pub sort_name: Option<String>,
    pub ol_key: Option<String>,
    pub gr_key: Option<String>,
    pub hc_key: Option<String>,
    pub routes: Vec<AuthorRouteResponse>,
    pub name_variants: Vec<AuthorNameVariantResponse>,
    pub link_state: AuthorLinkState,
    pub monitorable: bool,
    pub monitored: bool,
    pub monitor_new_items: bool,
    /// The persisted monitor language choice (REQ-003).
    pub monitor_language: Option<String>,
    pub added_at: String,
}

impl AuthorResponse {
    /// The author plus its assembled route panel.
    ///
    /// The scalar `*_key` fields are the panel's compatibility projection, never
    /// the frozen `authors.*_key` columns: one authority answers what an author
    /// is linked to, and it is the route ledger.
    pub fn from_author_and_view(
        author: &livrarr_domain::Author,
        view: livrarr_domain::services::AuthorRouteView,
    ) -> Self {
        Self {
            id: author.id,
            name: author.name.clone(),
            sort_name: author.sort_name.clone(),
            ol_key: view.compatibility.ol_key,
            gr_key: view.compatibility.gr_key,
            hc_key: view.compatibility.hc_key,
            routes: view
                .routes
                .iter()
                .map(AuthorRouteResponse::from_route)
                .collect(),
            name_variants: view
                .name_variants
                .iter()
                .map(AuthorNameVariantResponse::from_variant)
                .collect(),
            link_state: view.link_state,
            monitorable: view.monitorable,
            monitored: author.monitored,
            monitor_new_items: author.monitor_new_items,
            monitor_language: author.monitor_language.clone(),
            added_at: author.added_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorRouteResponse {
    pub id: i64,
    pub provider: AuthorProvider,
    pub value: String,
    pub state: AuthorRouteState,
    pub provenance: AuthorRouteProvenance,
    pub removed_at: Option<DateTime<Utc>>,
}

impl AuthorRouteResponse {
    pub fn from_route(route: &livrarr_domain::AuthorRoute) -> Self {
        Self {
            id: route.id,
            provider: route.key.provider(),
            value: route.key.value(),
            state: route.state,
            provenance: route.provenance,
            removed_at: route.removed_at,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorNameVariantResponse {
    pub id: i64,
    pub name: String,
    pub source: AuthorNameSource,
    pub selected: bool,
}

impl AuthorNameVariantResponse {
    pub fn from_variant(variant: &livrarr_domain::AuthorNameVariant) -> Self {
        Self {
            id: variant.id,
            name: variant.name.clone(),
            source: variant.source,
            // "Selected" is the user's own choice, not whatever ranking happens
            // to be showing right now.
            selected: variant.user_selected_at.is_some(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorLinkReviewResponse {
    pub author: AuthorResponse,
    pub candidates: Vec<AuthorLinkCandidate>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickAuthorRouteRequest {
    pub candidate_id: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameAuthorRequest {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectAuthorNameRequest {
    pub variant_id: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorDetailResponse {
    pub author: AuthorResponse,
    pub works: Vec<WorkDetailResponse>,
}
