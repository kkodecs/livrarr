//! Provider call record data access: `ProviderCallRecordDb` trait + retention policy.

use crate::{DbError, ProviderCallRecord, ProviderStats};

// ---------------------------------------------------------------------------
// Provider Call Records (REQ-001) + Field Dissents (REQ-014)
// ---------------------------------------------------------------------------

/// Retention bounds for the provider call-record store (REQ-001): records are
/// evicted oldest-first once either bound is exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub max_age_days: u32,
    pub max_records: u64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_age_days: 30,
            max_records: 100_000,
        }
    }
}

/// Persisted per-provider call records (REQ-001). System-scoped — provider
/// telemetry carries no user data, so these queries take no user_id (deliberate
/// exception to the tenant rule).
#[trait_variant::make(Send)]
pub trait ProviderCallRecordDb: Send + Sync {
    /// Append a batch in one transaction (append-only log, no conflict target).
    async fn record_provider_calls(&self, batch: Vec<ProviderCallRecord>) -> Result<(), DbError>;

    /// Rolling-24h aggregates per provider for the status panel (REQ-002).
    /// Median latency covers network outcomes only; skipped/cached rows are
    /// excluded from the latency denominator.
    async fn query_provider_stats_24h(&self) -> Result<Vec<ProviderStats>, DbError>;

    /// Evict to the retention bounds, oldest first; returns rows deleted.
    async fn evict_call_records(&self, policy: RetentionPolicy) -> Result<u64, DbError>;
}
