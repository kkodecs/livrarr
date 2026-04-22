use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Author Monitor Tick (JOBS-AUTHOR-001)
// ---------------------------------------------------------------------------

pub async fn author_monitor_tick(state: AppState, cancel: CancellationToken) -> Result<(), String> {
    use livrarr_domain::services::AuthorMonitorWorkflow;

    let report = state
        .author_monitor_workflow
        .run_monitor(cancel)
        .await
        .map_err(|e| format!("author monitor: {e}"))?;

    info!(
        authors_checked = report.authors_checked,
        new_works = report.new_works_found,
        added = report.works_added,
        notifications = report.notifications_created,
        "author monitor complete"
    );

    Ok(())
}
