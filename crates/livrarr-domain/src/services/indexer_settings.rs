use crate::{
    settings::{CreateIndexerParams, UpdateIndexerParams},
    DbError, Indexer, IndexerId,
};

#[trait_variant::make(Send)]
pub trait IndexerSettingsService: Send + Sync {
    async fn get_indexer(&self, id: IndexerId) -> Result<Indexer, DbError>;
    async fn get_indexer_with_credentials(&self, id: IndexerId) -> Result<Indexer, DbError>;
    async fn list_indexers(&self) -> Result<Vec<Indexer>, DbError>;
    async fn create_indexer(&self, params: CreateIndexerParams) -> Result<Indexer, DbError>;
    async fn update_indexer(
        &self,
        id: IndexerId,
        params: UpdateIndexerParams,
    ) -> Result<Indexer, DbError>;
    async fn delete_indexer(&self, id: IndexerId) -> Result<(), DbError>;
    async fn set_supports_book_search(&self, id: IndexerId, supports: bool) -> Result<(), DbError>;
}
