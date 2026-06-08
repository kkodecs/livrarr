//! Pacing lane + per-call outcome (REQ-010). The `PacingQueue` *trait* itself
//! lives in livrarr-enrichment — its `submit` gates a provider-fetch closure whose
//! output is a provider payload (`NormalizedWorkDetail`), which this leaf crate
//! cannot name. Only the lane + the outcome value type are domain contracts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::EnrichmentMode;

/// Lane within the shared pacing queue (REQ-010). Foreground (a user is waiting)
/// drains before Background (bulk / queue-drain).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PacingLane {
    Foreground,
    Background,
}

impl From<EnrichmentMode> for PacingLane {
    /// Derive the lane from the invocation mode (DD-005): an interactive
    /// Manual/HardRefresh call is Foreground; Background is Background.
    fn from(mode: EnrichmentMode) -> Self {
        match mode {
            EnrichmentMode::Manual | EnrichmentMode::HardRefresh => PacingLane::Foreground,
            EnrichmentMode::Background => PacingLane::Background,
        }
    }
}

/// Outcome of one provider call through the pacing gate, recorded to the
/// provider status page (REQ-010).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCallOutcome {
    Ok,
    RateLimited,
    QuotaExhaustedUntil(DateTime<Utc>),
    Blocked,
}
