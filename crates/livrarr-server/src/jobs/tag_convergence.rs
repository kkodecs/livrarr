use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::state::AppState;
use crate::tag_service::{build_tag_metadata, read_cover_bytes, tag_sync_single_item};
use livrarr_db::{LibraryItemDb, WorkDb};
use livrarr_domain::services::ImportIoService;
use livrarr_domain::TagStatus;

// ---------------------------------------------------------------------------
// Tag Convergence Tick (JOBS-TAG-CONV-001)
// ---------------------------------------------------------------------------

/// Tag convergence sweep. Runs on 60-second interval.
///
/// Recovers files stuck in pending/stale state due to crashes or race conditions.
/// Not a full retag worker — only handles items that the primary tag sync path missed.
/// In normal operation (no crashes, no races), finds nothing to do.
///
/// Precondition: tag_status and tagged_at_generation columns exist on library_items.
/// Postcondition: pending/stale items tagged, or marked failed if tag write fails.
pub async fn tag_convergence_tick(
    state: AppState,
    cancel: CancellationToken,
) -> Result<(), String> {
    loop {
        if cancel.is_cancelled() {
            break;
        }

        let batch = match state.db.list_library_items_needing_tag_sync(50).await {
            Ok(b) => b,
            Err(e) => {
                return Err(format!(
                    "tag_convergence: list_library_items_needing_tag_sync failed: {e}"
                ));
            }
        };

        if batch.is_empty() {
            break;
        }

        for item in &batch {
            if cancel.is_cancelled() {
                break;
            }

            // I/O backpressure — reuse existing import semaphore.
            let _permit = state.import_semaphore.acquire().await;

            let work = match state.db.get_work(item.user_id, item.work_id).await {
                Ok(w) => w,
                Err(e) => {
                    warn!(
                        item_id = item.id,
                        work_id = item.work_id,
                        "tag_convergence: get_work failed: {e}"
                    );
                    continue;
                }
            };

            if work.enrichment_status != livrarr_domain::EnrichmentStatus::Enriched {
                // Work not ready yet — skip, will be picked up on next tick.
                continue;
            }

            let tag_metadata = build_tag_metadata(&work);
            let cover_data = read_cover_bytes(&state.data_dir, item.user_id, work.id).await;

            // Look up root folder path for this item.
            let root_folder_path = match state
                .import_io_service
                .get_root_folder(item.root_folder_id)
                .await
            {
                Ok(rf) => rf.path,
                Err(e) => {
                    warn!(
                        item_id = item.id,
                        root_folder_id = item.root_folder_id,
                        "tag_convergence: get_root_folder failed: {e}"
                    );
                    continue;
                }
            };

            let merge_generation = state
                .db
                .get_merge_generation(item.user_id, work.id)
                .await
                .unwrap_or(0);

            let tag_result = tag_sync_single_item(
                item,
                &root_folder_path,
                &tag_metadata,
                cover_data.as_deref(),
            )
            .await;

            let new_status = match tag_result {
                Ok(()) => TagStatus::Synced,
                Err(ref e) => {
                    warn!(item_id = item.id, "tag_convergence: tag write failed: {e}");
                    TagStatus::Failed
                }
            };

            if let Err(e) = state
                .db
                .update_library_item_tag_status(item.id, new_status, merge_generation)
                .await
            {
                warn!(
                    item_id = item.id,
                    "tag_convergence: update_library_item_tag_status failed: {e}"
                );
            }
        }
    }

    Ok(())
}
