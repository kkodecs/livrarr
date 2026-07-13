//! Persistent provider-response cache data access: `ProviderResponseCacheDb` trait.

use chrono::{DateTime, Utc};

use crate::{DbError, MetadataProvider};

/// One cached provider detail payload, keyed by (provider, anchor_type, anchor).
/// Global, not per-user: provider payloads are user-independent public
/// metadata (REQ-009).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCacheEntry {
    pub provider: MetadataProvider,
    pub anchor_type: String,
    pub anchor: String,
    pub payload_json: String,
    pub fetched_at: DateTime<Utc>,
}

/// Persistent provider-response cache storage (REQ-009). Storage only —
/// freshness policy (TTL comparison, success-only writes, the `Freshness`
/// knob) lives at the provider-queue dispatch seam, never here.
#[trait_variant::make(Send)]
pub trait ProviderResponseCacheDb: Send + Sync {
    async fn get_provider_cache_entry(
        &self,
        provider: MetadataProvider,
        anchor_type: &str,
        anchor: &str,
    ) -> Result<Option<ProviderCacheEntry>, DbError>;

    /// Upsert the entry at its (provider, anchor_type, anchor) key
    /// (`ON CONFLICT ... DO UPDATE` semantics).
    async fn upsert_provider_cache_entry(&self, entry: ProviderCacheEntry) -> Result<(), DbError>;

    /// Evict oldest-`fetched_at`-first until at most `max_rows` remain.
    /// Returns the number of rows evicted.
    async fn evict_provider_cache_to_cap(&self, max_rows: i64) -> Result<u64, DbError>;

    async fn count_provider_cache_entries(&self) -> Result<i64, DbError>;
}
