use livrarr_db::sqlite::SqliteDb;
use livrarr_db::WorkDb;
use livrarr_metadata::series_link::{reconcile_work_series, SeriesLinkOrigin};

/// Startup back-fill (REQ-002): give every work carrying an orphan
/// `series_name` (non-empty string, NULL `series_id`, known author) a series
/// row — an unmonitored stub when none exists — and the FK link. Idempotent:
/// reconciled works drop out of the orphan listing, so subsequent restarts
/// are no-ops. Works with a NULL author are not in the listing (they heal
/// here once an author exists).
pub async fn run_series_backfill(db: SqliteDb) {
    let works = match db.list_orphan_series_works_all_users().await {
        Ok(w) => w,
        Err(e) => {
            tracing::error!(error = %e, "series backfill: failed to list orphan works");
            return;
        }
    };

    if works.is_empty() {
        tracing::info!("series backfill: no orphan series works");
        return;
    }

    tracing::info!(count = works.len(), "series backfill: starting");
    let mut linked: u32 = 0;
    let mut skipped: u32 = 0;
    let mut errors: u32 = 0;

    for work in &works {
        match reconcile_work_series(&db, work, SeriesLinkOrigin::System).await {
            Ok(Some(_)) => linked += 1,
            Ok(None) => skipped += 1,
            Err(e) => {
                tracing::warn!(work_id = work.id, error = %e, "series backfill: reconcile failed");
                errors += 1;
            }
        }
    }

    tracing::info!(linked, skipped, errors, "series backfill: complete");
}
