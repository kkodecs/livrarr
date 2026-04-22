use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::state::AppState;
use livrarr_domain::services::WorkService;

// ---------------------------------------------------------------------------
// Enrichment Retry Tick (JOBS-ENRICH-001)
// ---------------------------------------------------------------------------

pub async fn enrichment_retry_tick(
    state: AppState,
    cancel: CancellationToken,
) -> Result<(), String> {
    // Queue-aware retry tick. For each user, asks the new
    // ProviderRetryStateDb which (work_id, provider) pairs have
    // next_attempt_at <= now and are in WillRetry or Suppressed state.
    // Dedups by work_id and dispatches one enrich_work call per due work
    // — the queue's restart-safety logic skips providers whose retry-state
    // row is already terminal, so only the actually-due providers run.
    // Circuit breaker, throttling, and merge-engine all apply automatically.
    use livrarr_db::{ProviderRetryStateDb, UserDb};
    use std::collections::HashSet;

    let users = match state.db.list_users().await {
        Ok(u) => u,
        Err(e) => return Err(format!("list_users: {e}")),
    };

    let now = chrono::Utc::now();
    let mut total_due = 0usize;
    let mut total_dispatched = 0usize;

    for user in &users {
        if cancel.is_cancelled() {
            return Ok(());
        }
        let due = match state.db.list_works_due_for_retry(user.id, now).await {
            Ok(d) => d,
            Err(e) => {
                warn!(
                    "enrichment_retry: list_works_due_for_retry({}): {e}",
                    user.id
                );
                continue;
            }
        };
        if due.is_empty() {
            continue;
        }
        total_due += due.len();

        // Dedup by work_id — one dispatch covers all due providers for that
        // work (queue skips already-terminal providers via restart-safety).
        let work_ids: HashSet<livrarr_domain::WorkId> = due.iter().map(|(w, _)| *w).collect();

        for work_id in work_ids {
            if cancel.is_cancelled() {
                return Ok(());
            }
            match tokio::time::timeout(
                Duration::from_secs(30),
                livrarr_domain::services::EnrichmentWorkflow::enrich_work(
                    state.enrichment_workflow.as_ref(),
                    user.id,
                    work_id,
                    livrarr_domain::services::EnrichmentMode::Background,
                ),
            )
            .await
            {
                Ok(Ok(result)) => {
                    total_dispatched += 1;
                    if let Some(ref cover_url) = result.work.cover_url {
                        if let Err(e) = state
                            .work_service
                            .download_cover_from_url(user.id, work_id, cover_url)
                            .await
                        {
                            warn!(work_id, %e, "cover download failed");
                        }
                    }
                }
                Ok(Err(e)) => {
                    warn!(
                        "enrichment_retry: enrich_work({}, {}) failed: {e}",
                        user.id, work_id
                    );
                }
                Err(_) => {
                    warn!(
                        "enrichment_retry: enrich_work({}, {}) timed out",
                        user.id, work_id
                    );
                }
            }
        }
    }

    if total_due > 0 {
        debug!(
            "enrichment_retry: {} due (work,provider) pairs across users; dispatched {} works",
            total_due, total_dispatched,
        );
    }
    Ok(())
}
