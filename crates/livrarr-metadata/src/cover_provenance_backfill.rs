//! One-time startup migration: derive `cover_source`/`audiobook_cover_source`
//! for existing rows from the stored cover URL's host, using the slot-aware
//! host->provider classifier (`provider_for_cover_host_for_slot` — the shared
//! amazon CDN family stamps `goodreads` for the ebook slot and `audible` for
//! the audiobook slot). Idempotent; manually selected ebook rows are never
//! touched; a host
//! the classifier doesn't recognize is left as-is.
//!
//! Interpretation note (flagged — not silently assumed): the brief's "never
//! overwrite a non-NULL source" is read here as "never overwrite a real,
//! previously-stamped provider/origin label" — NOT the literal placeholder
//! string `"add"` that phase-1 create stamps when no source is known
//! (`work_service.rs`'s `.unwrap_or("add")`). `"add"` is eligible for this
//! backfill because it is the exact bug S3 diagnoses (rows whose file/URL
//! demonstrably came from a real provider but whose `cover_source` column
//! still reads the create-time placeholder). Any OTHER non-NULL value
//! (a real provider name, or a deliberate non-provider label such as
//! `"epub"`/`"isbn_ol"`/`"isbn_amazon"`/`"user_upload"` from a user pick) is
//! left untouched, matching the literal rule.

use std::collections::HashMap;

use livrarr_db::WorkDb;
use livrarr_domain::{CoverMediaType, UserId, WorkId};

use crate::cover_rank::provider_for_cover_host_for_slot;

const PLACEHOLDER_SOURCE: &str = "add";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CoverProvenanceBackfillReport {
    pub ebook_stamped: u32,
    pub audiobook_stamped: u32,
}

fn eligible_for_backfill(source: Option<&str>) -> bool {
    matches!(source, None | Some(PLACEHOLDER_SOURCE))
}

pub async fn run_cover_provenance_backfill<D: WorkDb + Sync>(
    db: &D,
) -> CoverProvenanceBackfillReport {
    let mut report = CoverProvenanceBackfillReport::default();

    let owners: HashMap<WorkId, UserId> = match db.list_work_owners_all_users().await {
        Ok(pairs) => pairs.into_iter().collect(),
        Err(e) => {
            tracing::error!(error = %e, "cover provenance backfill: failed to list work owners");
            return report;
        }
    };

    for (work_id, user_id) in owners {
        let work = match db.get_work(user_id, work_id).await {
            Ok(w) => w,
            Err(_) => continue,
        };

        if !work.cover_manual && eligible_for_backfill(work.cover_source.as_deref()) {
            if let Some(url) = work.cover_url.as_deref() {
                if let Some(provider) = provider_for_cover_host_for_slot(url, CoverMediaType::Ebook)
                {
                    let source = format!("{provider:?}").to_lowercase();
                    if db
                        .update_cover_metadata(
                            user_id,
                            work_id,
                            Some(url),
                            &source,
                            work.cover_manual,
                            work.cover_width,
                            work.cover_height,
                        )
                        .await
                        .is_ok()
                    {
                        report.ebook_stamped += 1;
                    }
                }
            }
        }

        if eligible_for_backfill(work.audiobook_cover_source.as_deref()) {
            if let Some(url) = work.audiobook_cover_url.as_deref() {
                if let Some(provider) =
                    provider_for_cover_host_for_slot(url, CoverMediaType::Audiobook)
                {
                    let source = format!("{provider:?}").to_lowercase();
                    let manual = match db.get_audiobook_cover_manual(user_id, work_id).await {
                        Ok(manual) => manual,
                        Err(_) => continue,
                    };
                    if db
                        .update_audiobook_cover_metadata(
                            user_id,
                            work_id,
                            Some(url),
                            &source,
                            manual,
                            work.audiobook_cover_width,
                            work.audiobook_cover_height,
                        )
                        .await
                        .is_ok()
                    {
                        report.audiobook_stamped += 1;
                    }
                }
            }
        }
    }

    if report.ebook_stamped > 0 || report.audiobook_stamped > 0 {
        tracing::info!(
            ebook_stamped = report.ebook_stamped,
            audiobook_stamped = report.audiobook_stamped,
            "cover provenance backfill: complete"
        );
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligible_for_null_and_placeholder_only() {
        assert!(eligible_for_backfill(None));
        assert!(eligible_for_backfill(Some("add")));
        assert!(!eligible_for_backfill(Some("goodreads")));
        assert!(!eligible_for_backfill(Some("epub")));
        assert!(!eligible_for_backfill(Some("isbn_ol")));
        assert!(!eligible_for_backfill(Some("user_upload")));
    }
}
