use crate::UserId;

use super::work::{EagerQuery, LookupRequest, LookupResponse, LookupResult, WorkServiceError};

#[trait_variant::make(Send)]
pub trait DiscoveryService: Send + Sync {
    async fn lookup(&self, req: LookupRequest) -> Result<Vec<LookupResult>, WorkServiceError>;
    async fn lookup_filtered(
        &self,
        user_id: UserId,
        req: LookupRequest,
        raw: bool,
    ) -> Result<LookupResponse, WorkServiceError>;
    /// Eager, bulk best-guess discovery for manual import (#97). Groups
    /// `queries` by author and issues one author-scoped query per provider
    /// (Google Books `inauthor:`, OpenLibrary `author:`) instead of one search
    /// per title — imports cluster heavily by author, so this collapses N
    /// title searches into ~one call per author per provider. Each query's
    /// title is then matched locally against the author's returned corpus.
    ///
    /// Suggestion-only: no resolver call, so the returned `LookupResult` carries
    /// `candidate_id: None`. Identity is locked later at create time by
    /// `add`'s resolve-at-pick. Queries with no confident corpus match are
    /// omitted from the result; each present entry pairs the query `id` with
    /// its best match.
    async fn eager_match_by_author(
        &self,
        user_id: UserId,
        queries: Vec<EagerQuery>,
    ) -> Result<Vec<(usize, LookupResult)>, WorkServiceError>;
}
