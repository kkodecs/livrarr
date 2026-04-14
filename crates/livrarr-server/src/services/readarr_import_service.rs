//! Readarr import service — wraps the Readarr library import pipeline.

use crate::state::AppState;

/// Drives the Readarr library import (connect, preview, start, progress, undo).
#[derive(Clone)]
pub struct ReadarrImportService {
    /// Held as `AppState` because the Readarr import pipeline accesses db,
    /// readarr_import_progress, http_client, and data_dir.
    state: AppState,
}

impl ReadarrImportService {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    /// Expose the inner state for handlers that need it.
    pub fn state(&self) -> &AppState {
        &self.state
    }
}
