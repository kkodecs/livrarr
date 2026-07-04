//! Startup/first-read recovery for the cover write gate's crash-safe protocol
//! (S2). Scans `covers/{user_id}/` for `*.candidate.tmp` /
//! `*.candidate.meta.json` pairs left behind by a crash between the gate's
//! commit steps, and converges each to the exhaustive observable-state rule:
//!
//! - meta + tmp present, row == meta -> committed, rename lost -> complete
//!   the rename, delete meta.
//! - meta + tmp present, row != meta -> uncommitted (undecided or rejected
//!   cleanup lost) -> delete both; a later pass redoes the work.
//! - meta present, tmp gone -> rename done -> heal the row from meta if they
//!   disagree, delete meta.
//! - no meta -> nothing pending (out of this module's scope).
//!
//! A row is never left describing a missing file, and provenance always
//! converges to the bytes actually on disk (AC-11).

use std::path::Path;

use livrarr_db::WorkDb;
use livrarr_domain::{UserId, WorkId};

use crate::cover_write_gate::{
    candidate_meta_path, candidate_tmp_path, final_cover_path, CandidateMeta,
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CoverRecoveryReport {
    /// Committed, rename completed by recovery.
    pub completed: u32,
    /// Uncommitted candidate discarded (tmp+meta deleted).
    pub discarded: u32,
    /// Row healed from the meta sidecar (rename had completed, DB update lost).
    pub healed: u32,
    /// Candidate belonged to a work that no longer exists.
    pub orphaned: u32,
    /// Meta sidecar failed to parse — discarded.
    pub corrupt: u32,
    /// Row state could not be read (transient DB error) — candidate files
    /// left untouched for the next pass to retry.
    pub skipped: u32,
}

/// Recover every pending candidate under `covers_root` (the `covers/`
/// directory; each direct child directory whose name parses as a `UserId` is
/// scanned). Directories that are not numeric user ids are left alone —
/// legacy-layout adoption is a separate migration
/// (`cover_layout_migration`), not this pass's concern.
pub async fn recover_pending_cover_writes<D: WorkDb + Sync>(
    db: &D,
    covers_root: &Path,
) -> CoverRecoveryReport {
    let mut report = CoverRecoveryReport::default();

    let mut user_dirs = match tokio::fs::read_dir(covers_root).await {
        Ok(d) => d,
        Err(_) => return report,
    };

    while let Ok(Some(entry)) = user_dirs.next_entry().await {
        let path = entry.path();
        if !entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let user_id: UserId = match path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|s| s.parse().ok())
        {
            Some(id) => id,
            None => continue,
        };

        let mut files = match tokio::fs::read_dir(&path).await {
            Ok(f) => f,
            Err(_) => continue,
        };
        while let Ok(Some(file_entry)) = files.next_entry().await {
            let fname = file_entry.file_name();
            let fname_str = match fname.to_str() {
                Some(s) => s,
                None => continue,
            };
            if let Some((work_id, suffix)) = parse_candidate_meta_filename(fname_str) {
                recover_one(db, &path, user_id, work_id, suffix, &mut report).await;
            }
        }
    }

    if report != CoverRecoveryReport::default() {
        tracing::info!(
            completed = report.completed,
            discarded = report.discarded,
            healed = report.healed,
            orphaned = report.orphaned,
            corrupt = report.corrupt,
            skipped = report.skipped,
            "cover write gate recovery: complete"
        );
    }

    report
}

/// Parse `"{work_id}{suffix}.candidate.meta.json"` -> `(work_id, suffix)`.
/// `suffix` is `""` (ebook) or `"_audio"` (audiobook, the one suffix S1
/// mandates) — never the legacy `"_audiobook"`, which recovery does not
/// recognize (the layout migration renames those files before recovery would
/// ever see a candidate for them).
fn parse_candidate_meta_filename(fname: &str) -> Option<(i64, &'static str)> {
    let stem = fname.strip_suffix(".candidate.meta.json")?;
    if let Some(digits) = stem.strip_suffix("_audio") {
        digits.parse::<i64>().ok().map(|id| (id, "_audio"))
    } else {
        stem.parse::<i64>().ok().map(|id| (id, ""))
    }
}

async fn recover_one<D: WorkDb + Sync>(
    db: &D,
    user_dir: &Path,
    user_id: UserId,
    work_id: WorkId,
    suffix: &'static str,
    report: &mut CoverRecoveryReport,
) {
    let media_type = if suffix == "_audio" {
        livrarr_domain::CoverMediaType::Audiobook
    } else {
        livrarr_domain::CoverMediaType::Ebook
    };
    // Same lock the write gate holds for its whole protocol — a live gate
    // run and this recovery pass must never interleave on one slot's
    // candidate files.
    let _slot_guard = crate::cover_write_gate::lock_slot(user_id, work_id, media_type).await;

    let meta_path = candidate_meta_path(user_dir, work_id, suffix);
    let tmp_path = candidate_tmp_path(user_dir, work_id, suffix);
    let final_path = final_cover_path(user_dir, work_id, suffix);

    let meta_bytes = match tokio::fs::read(&meta_path).await {
        Ok(b) => b,
        Err(_) => return, // vanished between listing and read — another pass handled it
    };
    let meta: CandidateMeta = match serde_json::from_slice(&meta_bytes) {
        Ok(m) => m,
        Err(_) => {
            let _ = tokio::fs::remove_file(&meta_path).await;
            let _ = tokio::fs::remove_file(&tmp_path).await;
            report.corrupt += 1;
            return;
        }
    };

    let work = match db.get_work(user_id, work_id).await {
        Ok(w) => w,
        Err(livrarr_domain::DbError::NotFound { .. }) => {
            // Definitive: the work row is gone — the candidate has no owner.
            let _ = tokio::fs::remove_file(&meta_path).await;
            let _ = tokio::fs::remove_file(&tmp_path).await;
            report.orphaned += 1;
            return;
        }
        Err(e) => {
            // Transient (pool busy, I/O hiccup): deleting here would discard
            // a possibly-committed candidate and permanently desync the row
            // from disk. Leave both files; the next pass retries.
            tracing::warn!(
                work_id,
                error = %e,
                "cover recovery: row state unreadable — candidate left for a later pass"
            );
            report.skipped += 1;
            return;
        }
    };

    let (row_url, row_source, row_trust, row_w, row_h) = if suffix == "_audio" {
        (
            work.audiobook_cover_url.clone(),
            work.audiobook_cover_source.clone(),
            work.audiobook_cover_trust,
            work.audiobook_cover_width,
            work.audiobook_cover_height,
        )
    } else {
        (
            work.cover_url.clone(),
            work.cover_source.clone(),
            work.cover_trust,
            work.cover_width,
            work.cover_height,
        )
    };

    let row_matches_meta = row_url.as_deref() == Some(meta.url.as_str())
        && row_source.as_deref() == Some(meta.source.as_str())
        && row_trust == meta.trust
        && row_w == meta.width
        && row_h == meta.height;

    let tmp_exists = tokio::fs::try_exists(&tmp_path).await.unwrap_or(false);

    let sibling_cover_url = work.audiobook_cover_url.clone();

    if tmp_exists {
        if row_matches_meta {
            if tokio::fs::rename(&tmp_path, &final_path).await.is_ok() {
                let _ = tokio::fs::remove_file(&meta_path).await;
                // The cover file just changed; the crashed writer never
                // reached its own thumbnail invalidation.
                crate::cover_write_gate::invalidate_thumbnails(
                    user_dir,
                    work_id,
                    suffix,
                    media_type,
                    sibling_cover_url.as_deref(),
                )
                .await;
                report.completed += 1;
            }
        } else {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            let _ = tokio::fs::remove_file(&meta_path).await;
            report.discarded += 1;
        }
        return;
    }

    // tmp gone: the rename already ran. Heal the row from meta if it
    // disagrees — provenance must converge to the bytes on disk, not dims
    // alone.
    if !row_matches_meta {
        let heal_result = if suffix == "_audio" {
            db.update_audiobook_cover_metadata(
                user_id,
                work_id,
                Some(&meta.url),
                &meta.source,
                meta.trust,
                meta.width,
                meta.height,
            )
            .await
        } else {
            db.update_cover_metadata(
                user_id,
                work_id,
                Some(&meta.url),
                &meta.source,
                meta.trust,
                meta.width,
                meta.height,
            )
            .await
        };
        if heal_result.is_ok() {
            report.healed += 1;
        }
    }
    let _ = tokio::fs::remove_file(&meta_path).await;
    // The rename that already ran replaced the cover file, and the crashed
    // writer died before invalidating thumbnails — replay that step here
    // for every tmp-gone case (healed or already-consistent alike; worst
    // case a still-valid thumbnail regenerates once on next view).
    crate::cover_write_gate::invalidate_thumbnails(
        user_dir,
        work_id,
        suffix,
        media_type,
        sibling_cover_url.as_deref(),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ebook_candidate_meta_filename() {
        assert_eq!(
            parse_candidate_meta_filename("42.candidate.meta.json"),
            Some((42, ""))
        );
    }

    #[test]
    fn parse_audiobook_candidate_meta_filename() {
        assert_eq!(
            parse_candidate_meta_filename("42_audio.candidate.meta.json"),
            Some((42, "_audio"))
        );
    }

    #[test]
    fn parse_rejects_non_candidate_filenames() {
        assert_eq!(parse_candidate_meta_filename("42.jpg"), None);
        assert_eq!(parse_candidate_meta_filename("42.candidate.tmp"), None);
        assert_eq!(
            parse_candidate_meta_filename("42_audiobook.candidate.meta.json"),
            None,
            "legacy _audiobook suffix is not a recognized candidate — the \
             layout migration renames it before recovery runs"
        );
        assert_eq!(
            parse_candidate_meta_filename("not-a-number.candidate.meta.json"),
            None
        );
    }
}
