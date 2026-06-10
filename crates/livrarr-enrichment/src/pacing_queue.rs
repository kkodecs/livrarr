//! The pacing gate (REQ-010): the single seam all provider network calls pass
//! through — per-provider rate, per-provider daily budget (GB, ST-004), and
//! foreground-before-background lane priority. DB-stateful (status + budget).
//! Complements the scatter-gather `ProviderQueue` with lanes + budget + status.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

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
    /// In-memory membership of works with a queued/running provider call — the
    /// source of the derived "in progress" indicator (REQ-011). Held in memory
    /// (not the DB) so it self-heals if the process dies mid-run. `submit`
    /// populates it as calls enter/leave the gate.
    in_flight: Arc<Mutex<HashSet<WorkId>>>,
}

impl<DB> LivePacingQueue<DB> {
    pub fn new(db: Arc<DB>) -> Self {
        Self {
            db,
            in_flight: Arc::new(Mutex::new(HashSet::new())),
        }
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
        self.in_flight
            .lock()
            .expect("pacing in_flight lock poisoned")
            .contains(&work_id)
    }
}
