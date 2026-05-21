use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::state::AppState;
use livrarr_domain::services::{WorkIdentityRepository, WorkService};

// ---------------------------------------------------------------------------
// Enrichment Retry Tick (JOBS-ENRICH-001)
// ---------------------------------------------------------------------------

/// Enrichment retry job. Runs on 5-minute interval.
///
/// Three query sources:
/// 1. Works with retryable provider states (existing)
/// 2. Stale Unenriched works (crash recovery — add() interrupted)
/// 3. Failed works with no provider retry state rows (failed before any provider queried)
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
    use livrarr_db::{ProviderRetryStateDb, UserDb, WorkDb};
    use std::collections::HashSet;

    let users = match state.db.list_users().await {
        Ok(u) => u,
        Err(e) => return Err(format!("list_users: {e}")),
    };

    let now = chrono::Utc::now();
    let mut total_due = 0usize;
    let mut total_dispatched = 0usize;

    // Source 1: works with retryable provider states (per-user — existing behavior).
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
            dispatch_enrich(&state, user.id, work_id, &mut total_dispatched).await;
        }
    }

    // Source 2: stale unenriched works — global query, crash recovery.
    // Each work carries its own user_id; dispatch against work.user_id directly.
    if cancel.is_cancelled() {
        return Ok(());
    }
    let stale_threshold = now - chrono::Duration::minutes(5);
    let stale = match state.db.list_stale_unenriched_works(stale_threshold).await {
        Ok(s) => s,
        Err(e) => {
            warn!("enrichment_retry: list_stale_unenriched_works: {e}");
            vec![]
        }
    };

    for work in &stale {
        if cancel.is_cancelled() {
            return Ok(());
        }
        info!(
            work_id = work.id,
            user_id = work.user_id,
            "enrichment_retry: recovering unenriched work from interrupted add()"
        );
        dispatch_enrich(&state, work.user_id, work.id, &mut total_dispatched).await;
    }

    // Source 3: orphan failed works — global query, no provider retry state rows.
    // Each work carries its own user_id; dispatch against work.user_id directly.
    if cancel.is_cancelled() {
        return Ok(());
    }
    let orphans = match state.db.list_failed_works_without_retry_state().await {
        Ok(o) => o,
        Err(e) => {
            warn!("enrichment_retry: list_failed_works_without_retry_state: {e}");
            vec![]
        }
    };

    for work in &orphans {
        if cancel.is_cancelled() {
            return Ok(());
        }
        info!(
            work_id = work.id,
            user_id = work.user_id,
            "enrichment_retry: retrying orphan failed work (no provider state)"
        );
        dispatch_enrich(&state, work.user_id, work.id, &mut total_dispatched).await;
    }

    // Source 4: identity-pending works — re-attempt OL resolution.
    if cancel.is_cancelled() {
        return Ok(());
    }
    let pending = match state.db.list_identity_pending_works().await {
        Ok(p) => p,
        Err(e) => {
            warn!("enrichment_retry: list_identity_pending_works: {e}");
            vec![]
        }
    };

    if !pending.is_empty() {
        use livrarr_domain::identity::{AnchorSetter, EnglishSeed, IdentityResolution};
        use livrarr_domain::services::IdentityResolver;

        for work in &pending {
            if cancel.is_cancelled() {
                return Ok(());
            }
            let seed = EnglishSeed {
                title: work.title.clone(),
                author_name: work.author_name.clone(),
                isbn: work.isbn_13.clone(),
                user_confirmed_ol_key: None,
            };
            let resolution = state.identity_resolver.resolve(&seed).await;
            match resolution {
                IdentityResolution::Confirmed {
                    ol_key, method: _, ..
                } => {
                    info!(
                        work_id = work.id,
                        ol_key = %ol_key,
                        "enrichment_retry: identity resolved, promoting anchor"
                    );
                    let _ = state
                        .db
                        .confirm_ol_anchor(work.id, &ol_key, AnchorSetter::AutoSearch)
                        .await;
                    dispatch_enrich(&state, work.user_id, work.id, &mut total_dispatched).await;
                }
                IdentityResolution::Pending { .. } => {
                    debug!(
                        work_id = work.id,
                        "enrichment_retry: identity still pending, skipping"
                    );
                }
                IdentityResolution::Conflict { .. } => {
                    debug!(
                        work_id = work.id,
                        "enrichment_retry: identity conflict detected, skipping"
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

/// Dispatch a single enrich_work call with a 30-second timeout.
async fn dispatch_enrich(
    state: &AppState,
    user_id: livrarr_domain::UserId,
    work_id: livrarr_domain::WorkId,
    total_dispatched: &mut usize,
) {
    match tokio::time::timeout(
        Duration::from_secs(30),
        livrarr_domain::services::EnrichmentWorkflow::enrich_work(
            state.enrichment_workflow.as_ref(),
            user_id,
            work_id,
            livrarr_domain::services::EnrichmentMode::Background,
        ),
    )
    .await
    {
        Ok(Ok(result)) => {
            *total_dispatched += 1;
            if !result.work.cover_manual {
                if let Some(ref cover_url) = result.work.cover_url {
                    if let Err(e) = state
                        .work_service
                        .download_cover_from_url(user_id, work_id, cover_url)
                        .await
                    {
                        warn!(work_id, %e, "cover download failed");
                    }
                }
            }
        }
        Ok(Err(e)) => {
            warn!(
                "enrichment_retry: enrich_work({}, {}) failed: {e}",
                user_id, work_id
            );
        }
        Err(_) => {
            warn!(
                "enrichment_retry: enrich_work({}, {}) timed out",
                user_id, work_id
            );
        }
    }
}
