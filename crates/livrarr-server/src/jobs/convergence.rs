//! Background identity/enrichment convergence sweep.

use chrono::Duration as ChronoDuration;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::state::AppState;
use livrarr_db::{UserDb, WorkDb};
use livrarr_domain::services::{ConvergeOutcome, WorkService};

/// Convergence sweep — drives incomplete works toward full identity + enrichment.
///
/// Enabled by default; `[convergence] enabled = false` opts out — the lever
/// for libraries where worst-case provider volume is a concern.
/// When enabled, each tick walks every user, selects the works due for a
/// convergence pass, runs one `converge_work` pass over each, and paces the work's
/// next attempt via `next_convergence_at`.
///
/// The job is intentionally thin: all orchestration — settle a chaseable anchor,
/// run background enrichment, account dead-end retry counters — lives in
/// `WorkService::converge_work`. A background job cannot reach the private metadata
/// orchestration helpers (compile wall), so it goes through this one public entry.
///
/// Both stages are bounded: selection (`list_convergence_due`) drops anchors that
/// reached the dead-end `attempt_threshold`, and `converge_work` terminalizes an
/// exhausted pending work to needs-review — never an indefinite retry loop.
pub async fn convergence_tick(state: AppState, cancel: CancellationToken) -> Result<(), String> {
    let cfg = &state.config.convergence;
    if !cfg.enabled {
        return Ok(());
    }

    let threshold = cfg.attempt_threshold;
    let batch = cfg.batch_size;
    let cadence = ChronoDuration::seconds(cfg.interval_secs as i64);

    let users = state
        .db
        .list_users()
        .await
        .map_err(|e| format!("convergence: list_users failed: {e}"))?;

    for user in &users {
        if cancel.is_cancelled() {
            break;
        }

        let due = match state
            .db
            .list_convergence_due(user.id, chrono::Utc::now(), threshold, batch)
            .await
        {
            Ok(d) => d,
            Err(e) => {
                // A per-user failure must not stop the sweep for other users.
                warn!(
                    user_id = user.id,
                    "convergence: list_convergence_due failed: {e}"
                );
                continue;
            }
        };

        for work_id in due {
            if cancel.is_cancelled() {
                break;
            }

            let outcome = match state
                .work_service
                .converge_work(user.id, work_id, threshold)
                .await
            {
                Ok(o) => o,
                Err(e) => {
                    warn!(
                        user_id = user.id,
                        work_id, "convergence: converge_work failed: {e}"
                    );
                    // Best-effort backoff so a persistently-failing work waits
                    // one cadence instead of re-selecting every tick.
                    if let Err(e) = state
                        .db
                        .set_next_convergence_at(
                            user.id,
                            work_id,
                            Some(chrono::Utc::now() + cadence),
                        )
                        .await
                    {
                        warn!(
                            user_id = user.id,
                            work_id, "convergence: set_next_convergence_at failed: {e}"
                        );
                    }
                    continue;
                }
            };

            // Completed/Terminal works stop being selected (clear the clock) —
            // Completed means no chaseable anchors remain, so no selection
            // branch re-picks the work; a still-incomplete work backs off one
            // cadence before re-selection.
            let next = match outcome {
                ConvergeOutcome::Completed | ConvergeOutcome::Terminal => None,
                ConvergeOutcome::StillIncomplete => Some(chrono::Utc::now() + cadence),
            };

            if let Err(e) = state
                .db
                .set_next_convergence_at(user.id, work_id, next)
                .await
            {
                warn!(
                    user_id = user.id,
                    work_id, "convergence: set_next_convergence_at failed: {e}"
                );
            }
        }
    }

    Ok(())
}
