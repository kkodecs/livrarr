//! Server-side cache of harvested per-provider payloads, keyed on
//! (user_id, candidate_id). Opaque, consume-once, short-TTL, all tiers — the
//! add path looks up by the echoed candidate_id and feeds the merge engine
//! in-process (no re-query). Mirrors the established `lookup_cache` idiom
//! (Arc<Mutex<HashMap>> + a `created_at` instant for TTL).
//! See ir-v2 metadata-transport-cache (REQ-007/014/015; D-005/D-015).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use livrarr_domain::identity::CandidateId;
use livrarr_domain::{MetadataProvider, UserId};

use crate::NormalizedWorkDetail;

/// Per-resolution per-provider payloads (D-015 preserves per-provider provenance).
type ProviderPayloads = HashMap<MetadataProvider, NormalizedWorkDetail>;
/// One cached resolution: the per-provider payloads plus when they were stored (TTL).
type CachedEntry = (ProviderPayloads, Instant);
/// User-scoped, consume-once store keyed by (user_id, candidate_id).
type CacheMap = HashMap<(UserId, CandidateId), CachedEntry>;

/// Consume-once, user-scoped cache of the per-provider payloads fetched during
/// one resolution.
#[derive(Clone)]
pub struct TransportCache {
    inner: Arc<Mutex<CacheMap>>,
    ttl: Duration,
}

impl TransportCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            ttl,
        }
    }

    /// Store the payloads under (user_id, id); consume-once. Keyed by user_id so
    /// it is never cross-user readable (REQ-014, Principle 4).
    pub fn cache_put(&self, user_id: UserId, id: CandidateId, payloads: ProviderPayloads) {
        let mut cache = self.inner.lock().unwrap();
        cache.insert((user_id, id), (payloads, Instant::now()));
    }

    /// Remove and return the payloads for (user_id, id) if present and unexpired;
    /// `None` signals the caller to fall back to network enrichment (REQ-015).
    pub fn cache_take(&self, user_id: UserId, id: CandidateId) -> Option<ProviderPayloads> {
        let mut cache = self.inner.lock().unwrap();
        let (payloads, created_at) = cache.remove(&(user_id, id))?;
        if created_at.elapsed() <= self.ttl {
            Some(payloads)
        } else {
            None
        }
    }
}
