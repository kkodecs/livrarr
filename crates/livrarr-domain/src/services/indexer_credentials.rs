use crate::{DbError, Indexer, IndexerId};

#[trait_variant::make(Send)]
pub trait IndexerCredentialService: Send + Sync {
    async fn get_indexer_with_credentials(&self, id: IndexerId) -> Result<Indexer, DbError>;
}
