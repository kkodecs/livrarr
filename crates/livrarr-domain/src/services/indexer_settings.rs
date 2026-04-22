use crate::{
    settings::{
        CreateIndexerParams, UpdateIndexerConfigParams, UpdateIndexerParams, UpdateProwlarrParams,
    },
    DbError, Indexer, IndexerConfig, IndexerId,
};

#[trait_variant::make(Send)]
pub trait IndexerSettingsService: Send + Sync {
    async fn get_indexer(&self, id: IndexerId) -> Result<Indexer, DbError>;
    async fn list_indexers(&self) -> Result<Vec<Indexer>, DbError>;
    async fn create_indexer(&self, params: CreateIndexerParams) -> Result<Indexer, DbError>;
    async fn update_indexer(
        &self,
        id: IndexerId,
        params: UpdateIndexerParams,
    ) -> Result<Indexer, DbError>;
    async fn delete_indexer(&self, id: IndexerId) -> Result<(), DbError>;
    async fn set_supports_book_search(&self, id: IndexerId, supports: bool) -> Result<(), DbError>;
    async fn get_prowlarr_config(&self) -> Result<crate::settings::ProwlarrConfig, DbError>;
    async fn update_prowlarr_config(
        &self,
        params: UpdateProwlarrParams,
    ) -> Result<crate::settings::ProwlarrConfig, DbError>;
    async fn get_indexer_config(&self) -> Result<IndexerConfig, DbError>;
    async fn update_indexer_config(
        &self,
        params: UpdateIndexerConfigParams,
    ) -> Result<IndexerConfig, DbError>;
}
