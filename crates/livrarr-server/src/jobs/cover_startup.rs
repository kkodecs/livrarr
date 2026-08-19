//! Startup wrapper for the covers startup sequence. The three passes and
//! their strict ordering live in `livrarr_metadata::cover_startup`; this
//! module just wires the sequence into the server's startup job spawn.

use std::path::PathBuf;
use std::sync::Arc;

use livrarr_db::sqlite::SqliteDb;

pub async fn run_cover_startup_passes(
    db: SqliteDb,
    enrichment: Arc<crate::state::LiveEnrichmentService>,
    http: livrarr_http::fetcher::HttpFetcherImpl,
    covers_root: PathBuf,
) {
    livrarr_metadata::cover_startup::run_cover_startup_passes(&db, &covers_root).await;
    match livrarr_metadata::cover_startup::run_identity_round15_gr_cover_reselect(
        &db,
        enrichment.as_ref(),
        &http,
        &covers_root,
    )
    .await
    {
        Ok(report)
            if report.works_failed != 0
                || report.queued_works_remaining != 0
                || report.automatic_target_works_remaining != 0 =>
        {
            tracing::warn!(
                ebook_slots = report.ebook_slots,
                ebook_reselected = report.ebook_slots_reselected,
                ebook_placeholder = report.ebook_slots_placeholder,
                audiobook_slots = report.audiobook_slots,
                audiobook_reselected = report.audiobook_slots_reselected,
                audiobook_placeholder = report.audiobook_slots_placeholder,
                manual_ebook_preserved = report.manual_ebook_slots_preserved,
                manual_audiobook_preserved = report.manual_audiobook_slots_preserved,
                works_materialized = report.works_materialized,
                works_failed = report.works_failed,
                queued_works_remaining = report.queued_works_remaining,
                automatic_target_works_remaining = report.automatic_target_works_remaining,
                "identity round-15 Goodreads cover reselect partial; serving continues and the worklist will retry on next startup"
            )
        }
        Ok(report) => tracing::info!(
            ebook_slots = report.ebook_slots,
            ebook_reselected = report.ebook_slots_reselected,
            ebook_placeholder = report.ebook_slots_placeholder,
            audiobook_slots = report.audiobook_slots,
            audiobook_reselected = report.audiobook_slots_reselected,
            audiobook_placeholder = report.audiobook_slots_placeholder,
            manual_ebook_preserved = report.manual_ebook_slots_preserved,
            manual_audiobook_preserved = report.manual_audiobook_slots_preserved,
            works_materialized = report.works_materialized,
            works_failed = report.works_failed,
            queued_works_remaining = report.queued_works_remaining,
            automatic_target_works_remaining = report.automatic_target_works_remaining,
            "identity round-15 Goodreads cover reselect complete"
        ),
        Err(error) => panic!("identity round-15 Goodreads cover reselect failed: {error}"),
    }
}
