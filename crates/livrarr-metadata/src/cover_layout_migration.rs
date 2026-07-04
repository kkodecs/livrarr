//! S4 startup migration: adopt legacy root-level cover files
//! (`covers/{work_id}{suffix}.jpg`) into the owning user's directory
//! (`covers/{user_id}/{work_id}{suffix}.jpg`, keyed by `works.user_id`) and
//! rename the legacy `_audiobook` suffix to the one suffix every road now
//! uses, `_audio` (S1). Idempotent and restart-safe: a file already adopted
//! is simply absent from the root on the next run. Orphan files (no matching
//! work row) are LOGGED and left in place — never silently deleted.

use std::collections::HashMap;
use std::path::Path;

use livrarr_db::WorkDb;
use livrarr_domain::{UserId, WorkId};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CoverLayoutMigrationReport {
    pub adopted: u32,
    /// Of `adopted`, how many also had their suffix renamed from the legacy
    /// `_audiobook` to `_audio`.
    pub legacy_audiobook_suffix_renamed: u32,
    pub orphaned: u32,
    pub errors: u32,
}

/// Parse a legacy root-level cover filename into `(work_id, canonical
/// destination filename)`. Recognizes the full/thumb variants for both
/// slots; the audiobook variants normalize to the `_audio` suffix (S1)
/// regardless of whether the source used the legacy `_audiobook` spelling.
/// Returns `None` for anything else (including `.candidate.*` sidecar/temp
/// files — never this migration's concern; those live per-user already by
/// construction and are handled by `cover_write_gate_recovery`).
fn canonical_legacy_filename(fname: &str) -> Option<(WorkId, String)> {
    if fname.contains(".candidate.") {
        return None;
    }
    let stem = fname.strip_suffix(".jpg")?;

    let (id_str, canonical_suffix) = if let Some(id) = stem.strip_suffix("_audiobook_thumb") {
        (id, "_audio_thumb")
    } else if let Some(id) = stem.strip_suffix("_audio_thumb") {
        (id, "_audio_thumb")
    } else if let Some(id) = stem.strip_suffix("_audiobook") {
        (id, "_audio")
    } else if let Some(id) = stem.strip_suffix("_audio") {
        (id, "_audio")
    } else if let Some(id) = stem.strip_suffix("_thumb") {
        (id, "_thumb")
    } else {
        (stem, "")
    };

    let work_id: WorkId = id_str.parse().ok()?;
    Some((work_id, format!("{work_id}{canonical_suffix}.jpg")))
}

pub async fn run_cover_layout_migration<D: WorkDb + Sync>(
    db: &D,
    covers_root: &Path,
) -> CoverLayoutMigrationReport {
    let mut report = CoverLayoutMigrationReport::default();

    let owners: HashMap<WorkId, UserId> = match db.list_work_owners_all_users().await {
        Ok(pairs) => pairs.into_iter().collect(),
        Err(e) => {
            tracing::error!(error = %e, "cover layout migration: failed to list work owners");
            return report;
        }
    };

    let mut entries = match tokio::fs::read_dir(covers_root).await {
        Ok(e) => e,
        Err(_) => return report, // no covers directory yet — nothing to migrate
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry
            .file_type()
            .await
            .map(|t| !t.is_file())
            .unwrap_or(true)
        {
            continue; // skip user subdirectories and anything not a plain file
        }
        let fname = entry.file_name();
        let Some(fname_str) = fname.to_str() else {
            continue;
        };
        let Some((work_id, canonical_name)) = canonical_legacy_filename(fname_str) else {
            continue;
        };

        let Some(&user_id) = owners.get(&work_id) else {
            tracing::warn!(
                file = fname_str,
                work_id,
                "cover layout migration: orphan file (no matching work) — left in place"
            );
            report.orphaned += 1;
            continue;
        };

        let user_dir = covers_root.join(user_id.to_string());
        if let Err(e) = tokio::fs::create_dir_all(&user_dir).await {
            tracing::warn!(file = fname_str, error = %e, "cover layout migration: create_dir_all failed");
            report.errors += 1;
            continue;
        }

        let dest = user_dir.join(&canonical_name);
        if tokio::fs::try_exists(&dest).await.unwrap_or(false) {
            // Never overwrite — look-before-delete applies to writes too.
            tracing::warn!(
                file = fname_str,
                dest = %dest.display(),
                "cover layout migration: destination already exists — skipped"
            );
            continue;
        }

        let source = covers_root.join(fname_str);
        match tokio::fs::rename(&source, &dest).await {
            Ok(()) => {
                report.adopted += 1;
                if fname_str.contains("_audiobook") {
                    report.legacy_audiobook_suffix_renamed += 1;
                }
            }
            Err(e) => {
                tracing::warn!(file = fname_str, error = %e, "cover layout migration: rename failed");
                report.errors += 1;
            }
        }
    }

    if report.adopted > 0 || report.orphaned > 0 || report.errors > 0 {
        tracing::info!(
            adopted = report.adopted,
            legacy_audiobook_suffix_renamed = report.legacy_audiobook_suffix_renamed,
            orphaned = report.orphaned,
            errors = report.errors,
            "cover layout migration: complete"
        );
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ebook_full_and_thumb() {
        assert_eq!(
            canonical_legacy_filename("42.jpg"),
            Some((42, "42.jpg".to_string()))
        );
        assert_eq!(
            canonical_legacy_filename("42_thumb.jpg"),
            Some((42, "42_thumb.jpg".to_string()))
        );
    }

    #[test]
    fn legacy_audiobook_suffix_renames_to_audio() {
        assert_eq!(
            canonical_legacy_filename("42_audiobook.jpg"),
            Some((42, "42_audio.jpg".to_string()))
        );
        assert_eq!(
            canonical_legacy_filename("42_audiobook_thumb.jpg"),
            Some((42, "42_audio_thumb.jpg".to_string()))
        );
    }

    #[test]
    fn already_canonical_audio_suffix_is_unchanged() {
        assert_eq!(
            canonical_legacy_filename("42_audio.jpg"),
            Some((42, "42_audio.jpg".to_string()))
        );
        assert_eq!(
            canonical_legacy_filename("42_audio_thumb.jpg"),
            Some((42, "42_audio_thumb.jpg".to_string()))
        );
    }

    #[test]
    fn candidate_files_are_ignored() {
        assert_eq!(canonical_legacy_filename("42.candidate.tmp"), None);
        assert_eq!(canonical_legacy_filename("42.candidate.meta.json"), None);
        assert_eq!(canonical_legacy_filename("42_audio.candidate.tmp"), None);
    }

    #[test]
    fn non_numeric_and_non_jpg_are_ignored() {
        assert_eq!(canonical_legacy_filename("not-a-number.jpg"), None);
        assert_eq!(canonical_legacy_filename("42.png"), None);
    }
}
