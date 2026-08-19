//! Background identity/enrichment convergence sweep.

use tokio_util::sync::CancellationToken;

use crate::state::AppState;

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
    let metadata_handoff = crate::identity_layer::run_identity_convergence_tick(state, cancel)
        .await
        .map_err(|error| error.to_string())?;
    tracing::debug!(
        visited = metadata_handoff.visited_work_count,
        captured = metadata_handoff.captured_route_count,
        "identity convergence metadata handoff"
    );
    Ok(())
}
