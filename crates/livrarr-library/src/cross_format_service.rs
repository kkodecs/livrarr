use std::path::Path;

use sha2::{Digest, Sha256};

use livrarr_db::{CrossFormatStateDb, DbError, KashLinkDb, LibraryItemDb, RootFolderDb};
use livrarr_domain::kash::{
    anchor_at_or_before, chapter_label, parse_kash, resolve_target, AlignmentEntry,
    DURATION_TOLERANCE_SECS,
};
use livrarr_domain::services::{CrossFormatError, CrossFormatService, FileService, ResumePrompt};
use livrarr_domain::{KashLink, LibraryItem, LibraryItemId, MediaType, UserId};

/// Cross-format resume service: link validation, prompt computation, anchor
/// serving, decline, sync-to-here. Generic over the DB (link/state/item
/// access) and a [`FileService`] for absolute-path resolution (root-folder
/// join + traversal checks live there — not reimplemented here).
pub struct CrossFormatServiceImpl<D, F> {
    db: D,
    files: F,
}

impl<D, F> CrossFormatServiceImpl<D, F> {
    pub fn new(db: D, files: F) -> Self {
        Self { db, files }
    }
}

// ---------------------------------------------------------------------------
// Internal helper types
// ---------------------------------------------------------------------------

/// Fully validated link context returned by `load_validated`.
struct Validated {
    link: KashLink,
    kash: livrarr_domain::kash::Kash,
    opened: LibraryItem,
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Format an audio timestamp as H:MM:SS (e.g. "0:00:20" for 20 seconds).
fn format_audio_label(ts: f64) -> String {
    let total_secs = ts.round() as u64;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    format!("{hours}:{minutes:02}:{seconds:02}")
}

/// Map any `DbError` to `CrossFormatError::Db`.
fn map_db_err(e: DbError) -> CrossFormatError {
    CrossFormatError::Db(e.to_string())
}

// ---------------------------------------------------------------------------
// Service implementation
// ---------------------------------------------------------------------------

impl<D, F> CrossFormatServiceImpl<D, F>
where
    D: KashLinkDb + CrossFormatStateDb + LibraryItemDb + RootFolderDb + Send + Sync + 'static,
    F: FileService + Send + Sync + 'static,
{
    /// Load and validate all link artefacts required by prompt/anchors/sync.
    ///
    /// Validation gates: item ownership, link existence, audio duration match,
    /// kash readability, and epub hash match (REQ-007/REQ-008/REQ-014).
    async fn load_validated(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
    ) -> Result<Validated, CrossFormatError> {
        // Step 1: opened item — scopes to this user; NotFound covers foreign probes.
        let opened = self
            .db
            .get_library_item(user_id, library_item_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => CrossFormatError::NotLinked,
                other => map_db_err(other),
            })?;

        // Step 2: link must exist for either side.
        let link = self
            .db
            .link_for_item(library_item_id)
            .await
            .map_err(map_db_err)?
            .ok_or(CrossFormatError::NotLinked)?;

        // Step 3: both sides must still belong to this user.
        let audio = self
            .db
            .get_library_item(user_id, link.audio_item_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => CrossFormatError::LinkStale,
                other => map_db_err(other),
            })?;
        let ebook = self
            .db
            .get_library_item(user_id, link.ebook_item_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => CrossFormatError::LinkStale,
                other => map_db_err(other),
            })?;

        // Step 4: build the .kash path from the audio item's root folder + relative path,
        // swapping the extension. The m4b itself need not exist here (AC-010).
        let audio_root = self
            .db
            .get_root_folder(audio.root_folder_id)
            .await
            .map_err(|_| CrossFormatError::LinkStale)?;
        let kash_path = Path::new(&audio_root.path)
            .join(&audio.path)
            .with_extension("kash");

        // Step 5: read and parse the kash sidecar.
        let bytes = tokio::fs::read(&kash_path)
            .await
            .map_err(|_| CrossFormatError::KashUnreadable)?;
        let kash = parse_kash(&bytes).map_err(|_| CrossFormatError::KashUnreadable)?;

        // Step 6: audio identity — duration within tolerance.
        let audio_duration = audio.duration_seconds.ok_or(CrossFormatError::LinkStale)?;
        if (audio_duration - kash.duration_seconds).abs() > DURATION_TOLERANCE_SECS {
            return Err(CrossFormatError::LinkStale);
        }

        // Step 7: ebook identity — epub hash must match.
        let epub_path = self
            .files
            .resolve_path(user_id, ebook.id)
            .await
            .map_err(|_| CrossFormatError::LinkStale)?;
        let expected_hash = kash.epub_hash.clone();
        let actual_hash = tokio::task::spawn_blocking(move || -> Result<String, ()> {
            let file_bytes = std::fs::read(&epub_path).map_err(|_| ())?;
            let digest = Sha256::digest(&file_bytes);
            Ok(format!("{:x}", digest))
        })
        .await
        .map_err(|_| CrossFormatError::LinkStale)?
        .map_err(|_| CrossFormatError::LinkStale)?;

        if actual_hash != expected_hash {
            return Err(CrossFormatError::LinkStale);
        }

        Ok(Validated { link, kash, opened })
    }
}

impl<D, F> CrossFormatService for CrossFormatServiceImpl<D, F>
where
    D: KashLinkDb + CrossFormatStateDb + LibraryItemDb + RootFolderDb + Send + Sync + 'static,
    F: FileService + Send + Sync + 'static,
{
    async fn resume_prompt(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
        current_ts: f64,
    ) -> Result<Option<ResumePrompt>, CrossFormatError> {
        // Silent fallback for all validation errors (REQ-007/REQ-008).
        let v = match self.load_validated(user_id, library_item_id).await {
            Ok(v) => v,
            Err(CrossFormatError::NotLinked)
            | Err(CrossFormatError::LinkStale)
            | Err(CrossFormatError::KashUnreadable) => return Ok(None),
            Err(e) => return Err(e),
        };

        let state = self
            .db
            .get_or_default(user_id, v.link.id)
            .await
            .map_err(map_db_err)?;

        // No recorded progress yet.
        if state.furthest_ts <= 0.0 {
            return Ok(None);
        }

        // Decline suppression (REQ-017): re-arms only when furthest advances beyond the threshold.
        let declined = match v.opened.media_type {
            MediaType::Ebook => state.ebook_declined_at_ts,
            MediaType::Audiobook => state.audio_declined_at_ts,
        };
        if let Some(d) = declined {
            if state.furthest_ts <= d {
                return Ok(None);
            }
        }

        // Never-backward: suppress if target is not strictly ahead (REQ-015/AC-007).
        let target = match resolve_target(&v.kash, state.furthest_ts, current_ts) {
            Some(t) => t,
            None => return Ok(None),
        };

        // Build the prompt for the opened format.
        let (position, label) = match v.opened.media_type {
            MediaType::Audiobook => {
                let pos = (target.ts.round() as u64).to_string();
                let lbl = format_audio_label(target.ts);
                (pos, lbl)
            }
            MediaType::Ebook => {
                let pos = target.cfi.clone();
                let lbl = chapter_label(&v.kash, target.ts);
                (pos, lbl)
            }
        };

        Ok(Some(ResumePrompt {
            format: v.opened.media_type,
            position,
            label,
        }))
    }

    async fn anchors_for_item(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
    ) -> Result<Vec<AlignmentEntry>, CrossFormatError> {
        // Errors propagate — the ebook reader uses them to skip cross-format reporting.
        let v = self.load_validated(user_id, library_item_id).await?;
        Ok(v.kash.alignment)
    }

    async fn decline_resume(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
    ) -> Result<(), CrossFormatError> {
        // No file validation — declining must work even if files drifted.
        let opened = self
            .db
            .get_library_item(user_id, library_item_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => CrossFormatError::NotLinked,
                other => map_db_err(other),
            })?;
        let link = self
            .db
            .link_for_item(library_item_id)
            .await
            .map_err(map_db_err)?
            .ok_or(CrossFormatError::NotLinked)?;
        let state = self
            .db
            .get_or_default(user_id, link.id)
            .await
            .map_err(map_db_err)?;
        self.db
            .set_decline(user_id, link.id, opened.media_type, state.furthest_ts)
            .await
            .map_err(map_db_err)
    }

    async fn sync_to_here(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
        current_ts: f64,
    ) -> Result<(), CrossFormatError> {
        // Errors propagate — sync needs a valid alignment to resolve the anchor.
        let v = self.load_validated(user_id, library_item_id).await?;
        let ts = anchor_at_or_before(&v.kash, current_ts)
            .map(|a| a.ts)
            .unwrap_or(0.0);
        self.db
            .sync_to(user_id, v.link.id, ts)
            .await
            .map_err(map_db_err)
    }
}
