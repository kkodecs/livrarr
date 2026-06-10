use serde::{Deserialize, Serialize};

use crate::kash::AlignmentEntry;
use crate::{LibraryItemId, MediaType, UserId};

/// A cross-format resume offer for the OPENED format: jump to where the
/// linked other format got. `position` is in the opened format's own
/// coordinate (CFI string for ebook, seconds-as-string for audio — matching
/// the existing `playback_progress.position` encoding); `label` is the
/// human-readable target (never a raw CFI — REQ-004).
#[derive(Debug, Clone)]
pub struct ResumePrompt {
    pub format: MediaType,
    pub position: String,
    pub label: String,
}

/// How a progress save was produced. Only genuine consumption may advance
/// the cross-format furthest mark (REQ-003); a manual seek/scrub never does.
/// Serde default is `Seek` so a stale client that omits the field can never
/// poison the furthest mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProgressKind {
    Progress,
    #[default]
    Seek,
}

#[derive(Debug, thiserror::Error)]
pub enum CrossFormatError {
    #[error("library item is not part of a kash link")]
    NotLinked,
    #[error("kash link no longer matches the files on disk")]
    LinkStale,
    #[error("kash sidecar unreadable")]
    KashUnreadable,
    #[error("database error: {0}")]
    Db(String),
}

/// Cross-format resume operations (Whispersync model): monotonic per-(user,
/// link) furthest mark in audio-timestamp space, prompt-to-jump when the
/// opened format is behind, polite decline, manual sync-to-here override.
#[trait_variant::make(Send)]
pub trait CrossFormatService: Send + Sync {
    /// Compute the resume offer for an opened item, or `None` when: the item
    /// is unlinked, the link fails validation (silent fallback — REQ-007/
    /// REQ-008), nothing is recorded yet, the prompt is decline-suppressed
    /// (REQ-017), or the target is not strictly ahead (REQ-015).
    async fn resume_prompt(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
        current_ts: f64,
    ) -> Result<Option<ResumePrompt>, CrossFormatError>;

    /// The `.kash` alignment for the ebook reader's CFI→ts resolution.
    /// Errors (NotLinked/LinkStale/KashUnreadable) tell the reader to skip
    /// all cross-format reporting — serving anchors for a stale link would
    /// let garbage CFIs poison the furthest mark.
    async fn anchors_for_item(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
    ) -> Result<Vec<AlignmentEntry>, CrossFormatError>;

    /// Record a decline for the opened item's format: the prompt stays
    /// suppressed until the furthest mark advances beyond its value at
    /// decline time (REQ-017).
    async fn decline_resume(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
    ) -> Result<(), CrossFormatError>;

    /// Explicit override: set the link's furthest mark to the nearest anchor
    /// at or before `current_ts` (may DECREASE — REQ-018) and re-arm
    /// prompting (clears both decline thresholds).
    async fn sync_to_here(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
        current_ts: f64,
    ) -> Result<(), CrossFormatError>;
}
