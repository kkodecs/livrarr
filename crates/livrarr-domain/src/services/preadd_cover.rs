use serde::Serialize;

use crate::UserId;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreaddCoverCandidate {
    pub proxy_url: String,
    pub source: String,
    pub title: String,
    pub author_name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PreaddCoverError {
    #[error("provider error: {0}")]
    Provider(String),
    #[error("timeout")]
    Timeout,
}

#[trait_variant::make(Send)]
pub trait PreaddCoverService: Sync + 'static {
    async fn fetch_cover_alternatives(
        &self,
        user_id: UserId,
        title: &str,
        author: &str,
        lang: &str,
        isbn_13: Option<&str>,
    ) -> Result<Vec<PreaddCoverCandidate>, PreaddCoverError>;
}
