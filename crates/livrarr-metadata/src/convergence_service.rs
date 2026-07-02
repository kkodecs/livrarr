//! Background convergence logic extracted from `work_service.rs` (M-005 / Phase 2 dedup).
//!
//! `converge_work` and `retry_all_incomplete` bodies live here as free functions.
//! The `WorkService` trait methods remain in `work_service.rs` as thin delegation
//! wrappers so the public contract is unchanged.

use livrarr_db::{
    AuthorDb, ConfigDb, EnrichmentRetryDb, LibraryItemDb, ProvenanceDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::identity::{AnchorConfidence, AnchorType, ConflictSource, IdentityMode};
use livrarr_domain::services::{
    ConvergeOutcome, EnrichmentMode, EnrichmentWorkflow, HttpFetcher, LlmCaller, RetrySummary,
    WorkService, WorkServiceError,
};
use livrarr_domain::{EnrichmentStatus, IdentityStatus, UserId, Work, WorkId};

use crate::work_service::{chaseable_anchor_types, WorkServiceImpl};

/// Background convergence pass for one work: settle a chaseable identity anchor
/// (or terminalize an exhausted Pending work), run background enrichment when
/// identity permits, and account dead-end retry counters.
///
/// Called exclusively by the `WorkService::converge_work` thin wrapper.
pub(crate) async fn converge_work<D, E, H, L, M, T>(
    svc: &WorkServiceImpl<D, E, H, L, M, T>,
    user_id: UserId,
    work_id: WorkId,
    threshold: u32,
) -> Result<ConvergeOutcome, WorkServiceError>
where
    D: WorkDb
        + WorkDbCreate
        + AuthorDb
        + LibraryItemDb
        + ProvenanceDb
        + EnrichmentRetryDb
        + livrarr_db::ProviderRetryStateDb
        + ConfigDb
        + livrarr_db::SeriesDb
        + livrarr_domain::services::WorkIdentityRepository
        + Send
        + Sync,
    E: EnrichmentWorkflow + Send + Sync,
    H: HttpFetcher + Clone + Send + Sync + 'static,
    L: LlmCaller + Send + Sync,
    M: crate::MergeEngine + Send + Sync,
    T: livrarr_domain::services::TagService + Send + Sync,
{
    // Fresh row (R-10): the job hands us an id; re-read so we settle on truth.
    let work = svc.get(user_id, work_id).await?;
    let was_pending = work.identity_status == IdentityStatus::Pending;

    // The anchor slots that are currently NULL on works.*.
    let missing_of = |w: &Work| -> Vec<String> {
        [
            (AnchorType::OL_WORK, w.ol_key.is_none()),
            (AnchorType::GR_WORK, w.gr_key.is_none()),
            (AnchorType::HC_WORK, w.hc_key.is_none()),
            (AnchorType::ISBN_13, w.isbn_13.is_none()),
            (AnchorType::ASIN, w.asin.is_none()),
        ]
        .into_iter()
        .filter(|(_, missing)| *missing)
        .map(|(t, _)| t.to_string())
        .collect()
    };
    let before_missing = missing_of(&work);
    let holds_anchor = before_missing.len() < 5;

    let anchors = svc.db.list_anchors(work_id).await.unwrap_or_default();
    let dead_ends = svc
        .db
        .list_anchor_dead_ends(work_id)
        .await
        .unwrap_or_default();
    let chaseable = chaseable_anchor_types(&work, &anchors, &dead_ends, threshold);

    // Step 0 — Pending dead-end (M9 / the convergence trap). settle_identity treats
    // a NoCandidates Unresolved as TRANSIENT (ST-002) and keeps the work Pending, so
    // re-settling a hopeless Pending work would fan out to providers every cadence
    // forever. Terminalize to NeedsReview when a Pending work has no identity path:
    // it holds NO hard anchor to resolve from (an anchorless, title-only work is not
    // chased in the background), OR every still-missing anchor is already
    // pending-guessed / at the dead-end threshold (chaseable empty).
    //
    // DIVERGENCE from ir-v2 convergence-orchestration step 0: the IR's
    // `chaseable.is_empty()` (missing-based) does NOT catch an anchorless Pending
    // work (all 5 missing -> chaseable non-empty). The behavioral contract
    // (test_id_completeness converge_work_terminal, "Converge Pending No Chase")
    // requires it to terminalize on the first pass — hence the `!holds_anchor`
    // clause. [Flagged for cross-family review; Codex authored that test.]
    if was_pending && (!holds_anchor || chaseable.is_empty()) {
        svc.db.set_needs_review(work_id).await.map_err(|e| {
            WorkServiceError::Validation(format!("convergence set_needs_review failed: {e}"))
        })?;
        return Ok(ConvergeOutcome::Terminal);
    }

    // Step 1 — identity / ID-chasing leg via the one identity road. Settle ONLY when
    // a chaseable missing anchor remains (R-5): a fully-anchored or fully-dead-ended
    // Confirmed work is never fanned out; a Pending work that reached here still
    // holds a chaseable bridge. Background keeps Audnexus eligible; Convergence
    // attributes any raised conflict.
    let mut work = work;
    if !chaseable.is_empty() {
        if let Some(resolver) = svc.resolver.as_ref() {
            if let Err(e) = crate::async_resolver::settle_identity(
                resolver.as_ref(),
                &svc.db,
                user_id,
                &work,
                IdentityMode::Background,
                ConflictSource::Convergence,
            )
            .await
            {
                tracing::warn!(work_id, "convergence identity settle failed: {e}");
            }
            work = svc.get(user_id, work_id).await?;
        }
    }

    // Step 2 — enrichment leg (Background path — NEVER refresh, RE-005). Runs when
    // identity permits (settled) and enrichment is still incomplete.
    let identity_permits = !matches!(
        work.identity_status,
        IdentityStatus::Pending | IdentityStatus::Conflict | IdentityStatus::NeedsReview
    );
    let enrichment_incomplete = matches!(
        work.enrichment_status,
        EnrichmentStatus::Unenriched | EnrichmentStatus::Failed
    );
    if identity_permits && enrichment_incomplete {
        // Low: background convergence (B4 table).
        let _ = svc
            .run_unified_enrichment(
                user_id,
                &work,
                None,
                EnrichmentMode::Background,
                None,
                livrarr_domain::RequestPriority::Low,
            )
            .await;
        work = svc.get(user_id, work_id).await?;
    }

    // Step 3 — dead-end accounting (R-1/R-2). A harvested anchor clears its counter;
    // a chaseable anchor still missing and unguessed gets +1 (an at-threshold anchor
    // is already excluded from `chaseable`, so it is never re-bumped).
    let still_missing = missing_of(&work);
    let anchors_after = svc.db.list_anchors(work_id).await.unwrap_or_default();
    let pending_after: Vec<String> = anchors_after
        .iter()
        .filter(|a| a.confidence == AnchorConfidence::Pending)
        .map(|a| a.anchor_type.as_str().to_string())
        .collect();
    for t in &before_missing {
        if !still_missing.contains(t) {
            let _ = svc
                .db
                .clear_anchor_dead_end(work_id, AnchorType::new(t))
                .await;
        }
    }
    for at in &chaseable {
        let key = at.as_str().to_string();
        if still_missing.contains(&key) && !pending_after.contains(&key) {
            let _ = svc.db.bump_anchor_attempt(work_id, at.clone()).await;
        }
    }

    // Step 4 — outcome for the job's pacing.
    let outcome = if matches!(
        work.identity_status,
        IdentityStatus::NeedsReview | IdentityStatus::Conflict | IdentityStatus::NotFound
    ) {
        ConvergeOutcome::Terminal
    } else if matches!(
        work.identity_status,
        IdentityStatus::Confirmed | IdentityStatus::Provisional
    ) && matches!(
        work.enrichment_status,
        EnrichmentStatus::Enriched | EnrichmentStatus::Thin
    ) {
        ConvergeOutcome::Completed
    } else {
        ConvergeOutcome::StillIncomplete
    };
    Ok(outcome)
}

/// Single-pass sweep over every incomplete work for the user.
///
/// "Incomplete" = `Failed`, `Unenriched`, or identity-`Pending`. For each:
/// - Pending works re-resolve identity first via `settle_identity` (Background
///   mode so Audnexus stays eligible).
/// - All works re-enrich through the one road (`refresh` → `run_unified` →
///   materialize).
///
/// Called exclusively by the `WorkService::retry_all_incomplete` thin wrapper.
pub(crate) async fn retry_all_incomplete<D, E, H, L, M, T>(
    svc: &WorkServiceImpl<D, E, H, L, M, T>,
    user_id: UserId,
) -> Result<RetrySummary, WorkServiceError>
where
    D: WorkDb
        + WorkDbCreate
        + AuthorDb
        + LibraryItemDb
        + ProvenanceDb
        + EnrichmentRetryDb
        + livrarr_db::ProviderRetryStateDb
        + ConfigDb
        + livrarr_db::SeriesDb
        + livrarr_domain::services::WorkIdentityRepository
        + Send
        + Sync,
    E: EnrichmentWorkflow + Send + Sync,
    H: HttpFetcher + Clone + Send + Sync + 'static,
    L: LlmCaller + Send + Sync,
    M: crate::MergeEngine + Send + Sync,
    T: livrarr_domain::services::TagService + Send + Sync,
{
    // Single pass over every "incomplete" work — Failed, Unenriched, or
    // identity-Pending — filtered in memory (like refresh_all). This REPLACES
    // the deleted background retry job: user-triggered, one pass, no recurring
    // loop (REQ-011 / PO §7).
    let works = svc
        .db
        .list_works(user_id)
        .await
        .map_err(WorkServiceError::Db)?;
    let incomplete: Vec<Work> = works
        .into_iter()
        .filter(|w| {
            matches!(
                w.enrichment_status,
                EnrichmentStatus::Failed | EnrichmentStatus::Unenriched
            ) || w.identity_status == IdentityStatus::Pending
        })
        .collect();

    let total = incomplete.len();
    let mut recovered = 0usize;

    for work in &incomplete {
        // A Pending work re-resolves identity first via the one identity road
        // (settle_identity) — Background mode so Audnexus stays eligible
        // (REQ-001). The promoted anchor survives the refresh below
        // (reset_enrichment_for_refresh touches only enrichment).
        if work.identity_status == IdentityStatus::Pending {
            if let Some(resolver) = svc.resolver.as_ref() {
                if let Err(e) = crate::async_resolver::settle_identity(
                    resolver.as_ref(),
                    &svc.db,
                    user_id,
                    work,
                    IdentityMode::Background,
                    ConflictSource::ManualRetry,
                )
                .await
                {
                    tracing::warn!(
                        work_id = work.id,
                        "retry-incomplete identity settle failed: {e}"
                    );
                }
            }
        }

        // Re-enrich through the one road (refresh -> run_unified ->
        // materialize). A refresh error never blocks the rest of the sweep.
        if svc.refresh(user_id, work.id).await.is_ok() {
            if let Ok(after) = svc.db.get_work(user_id, work.id).await {
                let still_incomplete = matches!(
                    after.enrichment_status,
                    EnrichmentStatus::Failed | EnrichmentStatus::Unenriched
                ) || after.identity_status == IdentityStatus::Pending;
                if !still_incomplete {
                    recovered += 1;
                }
            }
        }
    }

    Ok(RetrySummary {
        total,
        recovered,
        still_incomplete: total - recovered,
    })
}
