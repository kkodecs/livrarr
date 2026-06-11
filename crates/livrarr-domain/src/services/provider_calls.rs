use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::ServiceError;
use crate::WorkId;

/// One provider fetch attempt — network, cache-served, or skipped (REQ-001).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderCallRecord {
    /// Canonical provider key (the enrichment `provider_name()` vocabulary,
    /// e.g. "google_books"), never a display name.
    pub provider: String,
    pub operation: CallOperation,
    pub work_id: Option<WorkId>,
    pub started_at: DateTime<Utc>,
    pub duration_ms: i64,
    pub outcome: CallOutcomeClass,
    /// Short outcome detail (an error class at most); never payload bodies,
    /// filenames, or credentials.
    pub detail: Option<String>,
}

/// Which surface issued the fetch (REQ-001's operation vocabulary).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallOperation {
    Lookup,
    Identity,
    Enrich,
    Cover,
}

/// Reporting outcome class for a provider fetch attempt (REQ-001). This is the
/// reporting vocabulary; `ProviderOutcome` stays the control-flow vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallOutcomeClass {
    Success,
    NotFound,
    RateLimited,
    Timeout,
    Error,
    SkippedNoAnchor,
    SkippedPolicy,
    LlmRejected,
    Cached,
}

/// Fire-and-forget instrumentation sink (REQ-001). Deliberately sync and
/// dyn-safe (`Arc<dyn ProviderCallSink>`) so any crate can record without a db
/// edge or a generics explosion. Implementations enqueue for asynchronous
/// persistence and return immediately; on a full queue they drop the record
/// and count the drop — never block the caller or propagate errors into the
/// instrumented call path.
pub trait ProviderCallSink: Send + Sync {
    fn record(&self, rec: ProviderCallRecord);
}

/// No-op sink for compositions that don't persist call records (tests, CLI).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopCallSink;

impl ProviderCallSink for NoopCallSink {
    fn record(&self, _rec: ProviderCallRecord) {}
}

/// Per-provider rolling-24h aggregates for the status-page panel (REQ-002).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderStats {
    pub provider: String,
    pub calls_24h: i64,
    pub success_rate: f64,
    /// Median over network-outcome rows only (skips/cached excluded).
    pub median_latency_ms: i64,
    pub last_error: Option<(String, DateTime<Utc>)>,
    pub last_success: Option<DateTime<Utc>>,
}

/// Record-fed provider panel query (REQ-002, replaces the ok/error-only view).
/// Impl lives in livrarr-server over livrarr-db; handlers bind via a `Has*`
/// capability trait.
#[trait_variant::make(Send)]
pub trait ProviderStatsService: Send + Sync {
    async fn provider_stats_24h(&self) -> Result<Vec<ProviderStats>, ServiceError>;
}
