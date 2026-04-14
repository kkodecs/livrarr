//! Enrichment service — wrapper around the enrichment handler logic.
//!
//! Extracts the call to `enrich_work` from `AppState` so the caller
//! holds the service rather than threading `&AppState` everywhere.

use livrarr_db::sqlite::SqliteDb;
use livrarr_db::UpdateWorkEnrichmentDbRequest;
use livrarr_domain::Work;

use crate::handlers::enrichment::EnrichmentOutcome;
use crate::state::AppState;

/// Drives metadata enrichment for a work.
///
/// Wraps [`crate::handlers::enrichment::enrich_work`] behind a struct boundary
/// so tests can construct it with mock-friendly dependencies.
#[derive(Clone)]
pub struct EnrichmentService {
    /// Held as `AppState` because the enrichment pipeline accesses many fields
    /// (http_client, http_client_safe, goodreads_rate_limiter, data_dir, db).
    /// Individual fields are extracted as this service evolves.
    state: AppState,
}

impl EnrichmentService {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    /// Run enrichment for a work. Never fails — errors are captured in the outcome.
    pub async fn enrich(&self, work: &Work) -> EnrichmentOutcome {
        crate::handlers::enrichment::enrich_work(&self.state, work).await
    }

    /// Apply the enrichment outcome to the database.
    pub async fn apply_outcome(
        &self,
        user_id: livrarr_domain::UserId,
        work_id: livrarr_domain::WorkId,
        outcome: UpdateWorkEnrichmentDbRequest,
    ) -> Result<Work, livrarr_domain::DbError> {
        use livrarr_db::WorkDb;
        self.state
            .db
            .update_work_enrichment(user_id, work_id, outcome)
            .await
    }

    /// Expose the inner DB handle for callers that need raw access.
    pub fn db(&self) -> &SqliteDb {
        &self.state.db
    }
}
