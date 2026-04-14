//! Manual import service — wraps the manual import scan and import pipeline.
//!
//! The N+1 fix for `has_existing_media_type` lives in the `scan` handler directly
//! (batch `list_library_items_by_work_ids` before the per-file loop).

use crate::state::AppState;

/// Drives manual import scanning and import operations.
#[derive(Clone)]
pub struct ManualImportService {
    /// Held as `AppState` because the manual import pipeline accesses db,
    /// manual_import_scans, ol_rate_limiter, http_client, and import_semaphore.
    state: AppState,
}

impl ManualImportService {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    /// Expose the inner state for handlers that need it.
    pub fn state(&self) -> &AppState {
        &self.state
    }
}
