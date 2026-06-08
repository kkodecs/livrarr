//! The pacing gate (REQ-010): the single seam all provider network calls pass
//! through — per-provider rate, per-provider daily budget (GB, ST-004), and
//! foreground-before-background lane priority. DB-stateful (status + budget).
//! Complements the scatter-gather `ProviderQueue` with lanes + budget + status.

use std::sync::Arc;

use livrarr_domain::services::{PacingLane, ProviderCallOutcome};
use livrarr_domain::{MetadataProvider, WorkId};

/// The single pacing gate (REQ-010). One instance in AppState; the
/// provider-gateway routes every provider fetch through it.
#[trait_variant::make(Send)]
pub trait PacingQueue: Send + Sync {
    /// Pass one provider call through the gate on `lane`: enforce the per-provider
    /// rate limit + daily budget, granting foreground ahead of background. Returns
    /// the gate decision recorded to the provider status page (REQ-010). The
    /// caller performs the actual fetch only on `Ok`.
    async fn submit(&self, provider: MetadataProvider, lane: PacingLane) -> ProviderCallOutcome;

    /// Whether a queued/running call exists for this work — the source of the
    /// derived "in progress" indicator (REQ-011), never a stored status. Self-heals
    /// if a process dies mid-run.
    fn has_pending_or_running(&self, work_id: WorkId) -> bool;
}

/// DB-stateful `PacingQueue` impl (REQ-010): two lane worker pools (foreground
/// drains first), per-provider daily budget with backoff-to-reset, and provider
/// status rows. One instance in AppState.
pub struct LivePacingQueue<DB> {
    db: Arc<DB>,
}

impl<DB> LivePacingQueue<DB> {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }
}

impl<DB> PacingQueue for LivePacingQueue<DB>
where
    DB: Send + Sync + 'static,
{
    async fn submit(&self, provider: MetadataProvider, lane: PacingLane) -> ProviderCallOutcome {
        let _ = (&self.db, provider, lane);
        todo!()
    }

    fn has_pending_or_running(&self, work_id: WorkId) -> bool {
        let _ = (&self.db, work_id);
        todo!()
    }
}
