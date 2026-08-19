//! The covers startup sequence: three one-shot passes over the covers
//! directory whose correctness depends on strict order, so they run
//! sequentially from one caller — never as parallel tasks.
//!
//! 1. Layout migration first — it moves legacy root-level files into
//!    per-user directories and renames the legacy `_audiobook` suffix; the
//!    recovery pass and every serving path only understand the per-user
//!    layout, so nothing else may observe the tree until it is settled.
//! 2. Gate-write recovery second — it converges rows and pending candidate
//!    files to a consistent state, taking the same per-slot locks live
//!    writers use.
//! 3. Provenance backfill last — it derives missing cover sources from the
//!    now-converged rows; running it against rows recovery is still healing
//!    would race the heal and could stamp a source derived from a URL the
//!    heal is about to replace.

use std::path::Path;
use std::sync::Arc;

use livrarr_db::{LibraryItemDb, WorkDb};
use livrarr_domain::services::{HttpFetcher, MaterializeService};
use livrarr_domain::CoverMediaType;
use livrarr_enrichment::EnrichmentService;

pub async fn run_cover_startup_passes<D: WorkDb + Sync>(db: &D, covers_root: &Path) {
    crate::cover_layout_migration::run_cover_layout_migration(db, covers_root).await;
    crate::cover_write_gate_recovery::recover_pending_cover_writes(db, covers_root).await;
    crate::cover_provenance_backfill::run_cover_provenance_backfill(db).await;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdentityRound15GrCoverReselectReport {
    pub ebook_slots: usize,
    pub ebook_slots_reselected: usize,
    pub ebook_slots_placeholder: usize,
    pub audiobook_slots: usize,
    pub audiobook_slots_reselected: usize,
    pub audiobook_slots_placeholder: usize,
    pub manual_ebook_slots_preserved: usize,
    pub manual_audiobook_slots_preserved: usize,
    pub works_materialized: usize,
    pub works_failed: usize,
    pub queued_works_remaining: usize,
    pub automatic_target_works_remaining: usize,
}

/// Marker-gated repair for the wrong-book Goodreads covers written before v9.
///
/// The database first snapshots targets into a durable queue. For each queued
/// work this function computes the replacement by replaying the production
/// merge engine over normalized payloads already on disk (zero provider
/// dispatch), clears only still-machine-owned Goodreads slots, runs the normal
/// cover write gate, and forces the change-gated materializer with changed=true.
/// The queue row is removed only after materialization, so an interrupted
/// startup resumes a cleared or already-reselected slot safely. Each target is
/// isolated: a failed target is warned and left queued while its siblings run.
pub async fn run_identity_round15_gr_cover_reselect<E, H>(
    db: &livrarr_db::sqlite::SqliteDb,
    enrichment: &E,
    http: &H,
    covers_root: &Path,
) -> Result<IdentityRound15GrCoverReselectReport, String>
where
    E: EnrichmentService + Sync,
    H: HttpFetcher + Clone + Send + Sync + 'static,
{
    let plan =
        livrarr_db::identity_layer::plan_identity_round15_gr_cover_reselect(db.pool()).await?;
    let mut report = IdentityRound15GrCoverReselectReport {
        ebook_slots: plan.ebook_slots,
        audiobook_slots: plan.audiobook_slots,
        manual_ebook_slots_preserved: plan.manual_ebook_slots_preserved,
        manual_audiobook_slots_preserved: plan.manual_audiobook_slots_preserved,
        ..Default::default()
    };

    for target in plan.targets {
        let target_result: Result<(), String> = async {
            let replacement = enrichment
                .reselect_covers_from_persisted_payloads(target.user_id, target.work_id)
                .await
                .map_err(|error| {
                    format!(
                        "reselect persisted covers for work {}: {error}",
                        target.work_id
                    )
                })?;
            let slots = livrarr_db::identity_layer::clear_identity_round15_gr_cover_slots(
                db.pool(),
                target,
            )
            .await?;

            if target.ebook && !slots.ebook {
                report.manual_ebook_slots_preserved += 1;
            }
            if target.audiobook && !slots.audiobook {
                report.manual_audiobook_slots_preserved += 1;
            }

            let covers_dir = covers_root.join(target.user_id.to_string());
            let after_clear = match db.get_work(target.user_id, target.work_id).await {
                Ok(work) => work,
                Err(livrarr_domain::DbError::NotFound { .. }) => {
                    livrarr_db::identity_layer::finish_identity_round15_gr_cover_target(
                        db.pool(),
                        target,
                    )
                    .await?;
                    return Ok(());
                }
                Err(error) => {
                    return Err(format!(
                        "read work {} during round-15 cover repair: {error}",
                        target.work_id
                    ));
                }
            };

            let mut ebook_prefetched = None;
            if slots.ebook {
                let already_reselected = after_clear.cover_url.is_some()
                    && after_clear
                        .cover_source
                        .as_deref()
                        .is_some_and(|source| !source.eq_ignore_ascii_case("goodreads"));
                if !already_reselected {
                    let _ = tokio::fs::remove_file(crate::cover_write_gate::final_cover_path(
                        &covers_dir,
                        target.work_id,
                        "",
                    ))
                    .await;
                    let _ = tokio::fs::remove_file(crate::cover_write_gate::final_cover_path(
                        &covers_dir,
                        target.work_id,
                        "_thumb",
                    ))
                    .await;
                    if let Some(resolution) = replacement.ebook {
                        let outcome = crate::cover_write_gate::run_cover_write_gate(
                            db,
                            http,
                            target.user_id,
                            crate::cover_write_gate::CoverWriteGateInput {
                                covers_dir: covers_dir.clone(),
                                work_id: target.work_id,
                                media_type: CoverMediaType::Ebook,
                                resolution,
                            },
                        )
                        .await;
                        if let crate::cover_write_gate::GateOutcome::Accepted { bytes, .. } =
                            outcome
                        {
                            ebook_prefetched = Some(bytes);
                        }
                    }
                }
            }

            if slots.audiobook {
                let already_reselected = after_clear.audiobook_cover_url.is_some()
                    && after_clear
                        .audiobook_cover_source
                        .as_deref()
                        .is_some_and(|source| !source.eq_ignore_ascii_case("goodreads"));
                if !already_reselected {
                    let _ = tokio::fs::remove_file(crate::cover_write_gate::final_cover_path(
                        &covers_dir,
                        target.work_id,
                        "_audio",
                    ))
                    .await;
                    let _ = tokio::fs::remove_file(crate::cover_write_gate::final_cover_path(
                        &covers_dir,
                        target.work_id,
                        "_audio_thumb",
                    ))
                    .await;
                    if let Some(resolution) = replacement.audiobook {
                        let _ = crate::cover_write_gate::run_cover_write_gate(
                            db,
                            http,
                            target.user_id,
                            crate::cover_write_gate::CoverWriteGateInput {
                                covers_dir: covers_dir.clone(),
                                work_id: target.work_id,
                                media_type: CoverMediaType::Audiobook,
                                resolution,
                            },
                        )
                        .await;
                    }
                }
            }

            let repaired = db
                .get_work(target.user_id, target.work_id)
                .await
                .map_err(|error| {
                    format!(
                        "read repaired work {} during round-15 cover repair: {error}",
                        target.work_id
                    )
                })?;
            if slots.ebook {
                if repaired.cover_url.is_some()
                    && repaired
                        .cover_source
                        .as_deref()
                        .is_some_and(|source| !source.eq_ignore_ascii_case("goodreads"))
                {
                    report.ebook_slots_reselected += 1;
                } else {
                    report.ebook_slots_placeholder += 1;
                }
            }
            if slots.audiobook {
                if repaired.audiobook_cover_url.is_some()
                    && repaired
                        .audiobook_cover_source
                        .as_deref()
                        .is_some_and(|source| !source.eq_ignore_ascii_case("goodreads"))
                {
                    report.audiobook_slots_reselected += 1;
                } else {
                    report.audiobook_slots_placeholder += 1;
                }
            }

            if slots.ebook || slots.audiobook {
                let items = db
                    .list_taggable_items_by_work(target.user_id, target.work_id)
                    .await
                    .map_err(|error| {
                        format!(
                            "list taggable items for round-15 work {}: {error}",
                            target.work_id
                        )
                    })?;
                let audiobook_manual = db
                    .get_audiobook_cover_manual(target.user_id, target.work_id)
                    .await
                    .map_err(|error| {
                        format!(
                            "read audiobook cover ownership for round-15 work {}: {error}",
                            target.work_id
                        )
                    })?;
                let materializer =
                    livrarr_materialize::LiveMaterializeService::new(Arc::new(http.clone()));
                let outcome = materializer
                    .materialize(livrarr_domain::services::MaterializeRequest {
                        work_id: target.work_id,
                        changed: true,
                        tag_fields_changed: true,
                        ebook_cover: livrarr_domain::services::CoverSlotState {
                            chosen_new_url: None,
                            current_url: repaired.cover_url.clone(),
                            current_path: None,
                            user_locked: repaired.cover_manual,
                            prefetched_bytes: ebook_prefetched,
                        },
                        audiobook_cover: livrarr_domain::services::CoverSlotState {
                            chosen_new_url: None,
                            current_url: repaired.audiobook_cover_url.clone(),
                            current_path: None,
                            user_locked: audiobook_manual,
                            prefetched_bytes: None,
                        },
                        file_paths: items
                            .iter()
                            .map(|item| std::path::PathBuf::from(&item.path))
                            .collect(),
                        tags: livrarr_domain::services::MaterializeTags {
                            title: repaired.title.clone(),
                            subtitle: repaired.subtitle.clone(),
                            author: repaired.author_name.clone(),
                            narrator: repaired.narrator.clone(),
                            year: repaired.year,
                            genre: repaired.genres.clone(),
                            description: repaired.description.clone(),
                            publisher: repaired.publisher.clone(),
                            isbn: repaired.isbn_13.clone(),
                            language: repaired.language.clone(),
                            series_name: repaired.series_name.clone(),
                            series_position: repaired.series_position,
                        },
                        covers_dir,
                    })
                    .await
                    .map_err(|error| {
                        format!(
                            "force materialize round-15 work {}: {error}",
                            target.work_id
                        )
                    })?;
                if outcome.skipped_unchanged {
                    return Err(format!(
                        "round-15 work {} was incorrectly skipped as unchanged",
                        target.work_id
                    ));
                }
                report.works_materialized += 1;
            }

            livrarr_db::identity_layer::finish_identity_round15_gr_cover_target(db.pool(), target)
                .await?;
            Ok(())
        }
        .await;
        if let Err(error) = target_result {
            report.works_failed += 1;
            tracing::warn!(
                user_id = target.user_id,
                work_id = target.work_id,
                error = %error,
                "identity round-15 Goodreads cover reselect target failed; leaving it queued"
            );
        }
    }

    let completion =
        livrarr_db::identity_layer::complete_identity_round15_gr_cover_reselect(db.pool()).await?;
    report.queued_works_remaining = completion.queued_works;
    report.automatic_target_works_remaining = completion.automatic_target_works;
    Ok(report)
}
