//! Indexer (Torznab) data access: `IndexerDb` trait + request types.

use livrarr_domain::settings::{CreateIndexerParams, UpdateIndexerParams};

use crate::{DbError, Indexer, IndexerId, IndexerRssState};

// ---------------------------------------------------------------------------
// v2.1 — Indexer DB (Torznab)
// ---------------------------------------------------------------------------

/// Indexer data access. Not user-scoped — indexers are global.
///
/// Satisfies: IDX-001, IDX-002, IDX-004, IDX-009, IDX-010, RSS-FETCH-002, RSS-GAP-001
#[trait_variant::make(Send)]
pub trait IndexerDb: Send + Sync {
    async fn get_indexer(&self, id: IndexerId) -> Result<Indexer, DbError>;

    /// Get indexer with credentials (api_key populated).
    /// Use for outbound connections (test, search). Default get_indexer is equivalent
    /// but callers that make outbound calls should use this variant to signal intent.
    async fn get_indexer_with_credentials(&self, id: IndexerId) -> Result<Indexer, DbError>;
    async fn list_indexers(&self) -> Result<Vec<Indexer>, DbError>;
    async fn list_enabled_interactive_indexers(&self) -> Result<Vec<Indexer>, DbError>;
    async fn create_indexer(&self, req: CreateIndexerDbRequest) -> Result<Indexer, DbError>;
    async fn update_indexer(
        &self,
        id: IndexerId,
        req: UpdateIndexerDbRequest,
    ) -> Result<Indexer, DbError>;
    async fn delete_indexer(&self, id: IndexerId) -> Result<(), DbError>;
    async fn set_supports_book_search(&self, id: IndexerId, supports: bool) -> Result<(), DbError>;

    /// List indexers with enabled=1 AND enable_rss=1.
    ///
    /// Satisfies: RSS-FETCH-002
    async fn list_enabled_rss_indexers(&self) -> Result<Vec<Indexer>, DbError>;

    /// Get RSS state for an indexer. Returns None if no state row exists (first sync).
    ///
    /// Satisfies: RSS-GAP-001, RSS-JOB-001
    async fn get_rss_state(
        &self,
        indexer_id: IndexerId,
    ) -> Result<Option<IndexerRssState>, DbError>;

    /// Insert or update RSS state for an indexer.
    ///
    /// Satisfies: RSS-GAP-001
    async fn upsert_rss_state(
        &self,
        indexer_id: IndexerId,
        last_publish_date: Option<&str>,
        last_guid: &str,
    ) -> Result<(), DbError>;
}

pub struct CreateIndexerDbRequest {
    pub name: String,
    pub protocol: String,
    pub url: String,
    pub api_path: String,
    pub api_key: Option<String>,
    pub categories: Vec<i32>,
    pub priority: i32,
    pub enable_automatic_search: bool,
    pub enable_interactive_search: bool,
    pub enable_rss: bool,
    pub enabled: bool,
}

pub struct UpdateIndexerDbRequest {
    pub name: Option<String>,
    pub url: Option<String>,
    pub api_path: Option<String>,
    /// Tri-state: None = keep existing, Some(None) = clear, Some(Some(v)) = set.
    pub api_key: Option<Option<String>>,
    pub categories: Option<Vec<i32>>,
    pub priority: Option<i32>,
    pub enable_automatic_search: Option<bool>,
    pub enable_interactive_search: Option<bool>,
    pub enable_rss: Option<bool>,
    pub enabled: Option<bool>,
}

impl From<CreateIndexerParams> for CreateIndexerDbRequest {
    fn from(p: CreateIndexerParams) -> Self {
        Self {
            name: p.name,
            protocol: p.protocol,
            url: p.url,
            api_path: p.api_path,
            api_key: p.api_key,
            categories: p.categories,
            priority: p.priority,
            enable_automatic_search: p.enable_automatic_search,
            enable_interactive_search: p.enable_interactive_search,
            enable_rss: p.enable_rss,
            enabled: p.enabled,
        }
    }
}

impl From<UpdateIndexerParams> for UpdateIndexerDbRequest {
    fn from(p: UpdateIndexerParams) -> Self {
        Self {
            name: p.name,
            url: p.url,
            api_path: p.api_path,
            api_key: p.api_key,
            categories: p.categories,
            priority: p.priority,
            enable_automatic_search: p.enable_automatic_search,
            enable_interactive_search: p.enable_interactive_search,
            enable_rss: p.enable_rss,
            enabled: p.enabled,
        }
    }
}
