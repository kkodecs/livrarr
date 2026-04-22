use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Author Monitor Tick (JOBS-AUTHOR-001)
// ---------------------------------------------------------------------------

pub async fn author_monitor_tick(state: AppState, cancel: CancellationToken) -> Result<(), String> {
    use livrarr_db::UserDb;
    use livrarr_domain::services::{AuthorMonitorWorkflow, MonitorError, MonitorReport};

    let users = state
        .db
        .list_users()
        .await
        .map_err(|e| format!("author monitor: failed to list users: {e}"))?;

    let mut total_report = MonitorReport {
        authors_checked: 0,
        new_works_found: 0,
        works_added: 0,
        notifications_created: 0,
    };

    for user in &users {
        if cancel.is_cancelled() {
            break;
        }
        match state
            .author_monitor_workflow
            .run_monitor(user.id, cancel.clone())
            .await
        {
            Ok(report) => {
                total_report.authors_checked += report.authors_checked;
                total_report.new_works_found += report.new_works_found;
                total_report.works_added += report.works_added;
                total_report.notifications_created += report.notifications_created;
            }
            Err(MonitorError::AlreadyRunning) => return Ok(()),
            Err(e) => {
                tracing::warn!(user_id = user.id, "author monitor failed for user: {e}");
                // Continue to next user — don't let one user's failure starve others
            }
        }
    }

    info!(
        users = users.len(),
        authors_checked = total_report.authors_checked,
        new_works = total_report.new_works_found,
        added = total_report.works_added,
        notifications = total_report.notifications_created,
        "author monitor complete"
    );

    Ok(())
}
