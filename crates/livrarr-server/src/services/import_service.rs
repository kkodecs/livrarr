//! Import service — wraps the grab import pipeline.
//!
//! Exposes `import_grab` and the supporting single-file / batch helpers
//! through a struct so callers (retry handler, download poller) hold a
//! service rather than threading `&AppState` directly.

use crate::handlers::import::ImportGrabResult;
use crate::state::AppState;
use crate::ApiError;

/// Runs the file import pipeline for a completed grab.
#[derive(Clone)]
pub struct ImportService {
    /// Held as `AppState` because the import pipeline accesses db, data_dir,
    /// import_semaphore, import_locks, and HTTP clients.
    /// Individual fields are extracted incrementally as the service evolves.
    state: AppState,
}

impl ImportService {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    /// Import all files from a completed grab.
    ///
    /// Precondition: grab status already set to `importing` by caller.
    pub async fn import_grab(
        &self,
        user_id: i64,
        grab_id: i64,
    ) -> Result<ImportGrabResult, ApiError> {
        crate::handlers::import::import_grab(&self.state, user_id, grab_id).await
    }
}
