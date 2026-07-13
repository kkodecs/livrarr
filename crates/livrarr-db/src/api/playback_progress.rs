//! Playback progress data access: `PlaybackProgressDb` trait.

use crate::{DbError, LibraryItemId, PlaybackProgress, ProgressKind, UserId};

/// Playback progress data access.
#[trait_variant::make(Send)]
pub trait PlaybackProgressDb: Send + Sync {
    /// Get playback progress for a user + library item.
    async fn get_progress(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
    ) -> Result<Option<PlaybackProgress>, DbError>;

    /// Insert or update playback progress with finished_at lifecycle.
    ///
    /// When `kind` is `Progress`, the item belongs to a kash link, and
    /// `cross_format_ts` is a finite timestamp, the link's per-user
    /// `cross_format_state.furthest_ts` advances monotonically
    /// (`MAX(furthest_ts, ts)`) IN THE SAME SQLite transaction as the
    /// progress write. `Seek` or a missing/non-finite ts never touches
    /// cross-format state (REQ-003/REQ-016).
    async fn upsert_progress(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
        position: &str,
        progress_pct: f64,
        kind: ProgressKind,
        cross_format_ts: Option<f64>,
    ) -> Result<(), DbError>;

    /// Insert or update progress without touching finished_at.
    async fn upsert_progress_no_lifecycle(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
        position: &str,
        progress_pct: f64,
    ) -> Result<(), DbError>;

    /// Batch fetch progress for multiple library items.
    async fn get_progress_for_items(
        &self,
        user_id: UserId,
        library_item_ids: &[LibraryItemId],
    ) -> Result<Vec<PlaybackProgress>, DbError>;
}
