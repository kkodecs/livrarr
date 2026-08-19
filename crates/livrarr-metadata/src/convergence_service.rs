//! Background convergence logic extracted from `work_service.rs` (M-005 / Phase 2 dedup).
//!
//! `converge_work` and `retry_all_incomplete` bodies live here as free functions.
//! The `WorkService` trait methods remain in `work_service.rs` as thin delegation
//! wrappers so the public contract is unchanged.

use livrarr_db::{
    AuthorDb, ConfigDb, EnrichmentRetryDb, GrabDb, LibraryItemDb, ProvenanceDb, WorkDb,
    WorkDbCreate,
};
use livrarr_domain::identity::{AnchorConfidence, AnchorType, ConflictSource, IdentityMode};
use livrarr_domain::services::{
    ConvergeOutcome, ConvergencePass, EnrichmentMode, EnrichmentWorkflow, HttpFetcher,
    RefreshSurface, RetrySummary, SourceProviderData, WorkService, WorkServiceError,
};
use livrarr_domain::{EnrichmentStatus, IdentityStatus, UserId, Work, WorkId};

use crate::work_service::{chaseable_anchor_types, WorkServiceImpl};

/// Background convergence pass for one work: settle a chaseable identity anchor
/// (or terminalize an exhausted Pending work), run background enrichment when
/// identity permits, and account dead-end retry counters.
///
/// Called exclusively by the `WorkService::converge_work` thin wrapper.
pub(crate) async fn converge_work<D, E, H>(
    svc: &WorkServiceImpl<D, E, H>,
    user_id: UserId,
    work_id: WorkId,
    threshold: u32,
    source_provider_data: Option<SourceProviderData>,
) -> Result<ConvergencePass, WorkServiceError>
where
    D: WorkDb
        + WorkDbCreate
        + AuthorDb
        + LibraryItemDb
        + GrabDb
        + ProvenanceDb
        + EnrichmentRetryDb
        + livrarr_db::ProviderRetryStateDb
        + ConfigDb
        + livrarr_db::SeriesDb
        + livrarr_db::HistoryDb
        + livrarr_db::AuthorLinkDb
        + livrarr_domain::services::WorkIdentityRepository
        + livrarr_domain::identity_layer::WorkIdentityRepository
        + Send
        + Sync,
    E: EnrichmentWorkflow + Send + Sync,
    H: HttpFetcher + Clone + Send + Sync + 'static,
{
    // Fresh row (R-10): the job hands us an id; re-read so we settle on truth.
    let work = svc.get(user_id, work_id).await?;
    if svc.identity_routes_authoritative {
        let captured =
            livrarr_domain::identity_layer::WorkIdentityRepository::read_captured_identity(
                &svc.db, user_id, work_id,
            )
            .await
            .map_err(|error| WorkServiceError::Validation(error.to_string()))?;
        let bridge_chaseable = !captured.active_routes.iter().any(|route| {
            matches!(
                route.kind,
                livrarr_domain::identity_layer::RouteKind::OpenLibraryWork
                    | livrarr_domain::identity_layer::RouteKind::GoodreadsWork
                    | livrarr_domain::identity_layer::RouteKind::HardcoverWork
            )
        });
        let enrichment_incomplete = matches!(
            work.enrichment_status,
            EnrichmentStatus::Unenriched | EnrichmentStatus::Failed
        );
        let mut enrichment_outcome = None;
        if enrichment_incomplete || bridge_chaseable || source_provider_data.is_some() {
            // Normal convergence preserves provider retry standing. In
            // particular, spec v10 needs a prior terminal `not_found` anchor
            // to become eligible for the route-search leg on this visit.
            // Manual refresh owns the unconditional reset. Settlement and
            // identity-edit transactions invalidate standing only when their
            // active route graph changes, restoring anchor-first dispatch for
            // a newly derived key without weakening same-anchor dead ends.
            enrichment_outcome = Some(
                svc.run_unified_enrichment(
                    user_id,
                    &work,
                    source_provider_data,
                    EnrichmentMode::Background,
                    None,
                    livrarr_domain::RequestPriority::Low,
                    livrarr_domain::Freshness::PreferCache,
                )
                .await,
            );
        }
        let after = svc.get(user_id, work_id).await?;
        let outcome = if bridge_chaseable
            || enrichment_outcome
                .as_ref()
                .is_some_and(|o| o.route_handoff.is_some())
        {
            ConvergeOutcome::StillIncomplete
        } else if matches!(
            after.enrichment_status,
            EnrichmentStatus::Enriched | EnrichmentStatus::Thin
        ) {
            ConvergeOutcome::Completed
        } else {
            ConvergeOutcome::StillIncomplete
        };
        return Ok(ConvergencePass {
            outcome,
            route_handoff: enrichment_outcome
                .as_mut()
                .and_then(|outcome| outcome.route_handoff.take()),
            provider_chase_attempted: enrichment_outcome
                .as_ref()
                .is_some_and(|outcome| outcome.provider_chase_attempted),
        });
    }
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
        return Ok(ConvergencePass {
            outcome: ConvergeOutcome::Terminal,
            route_handoff: None,
            provider_chase_attempted: false,
        });
    }

    // Step 1 — identity / ID-chasing leg via the one identity road. Settle ONLY when
    // a chaseable missing anchor remains (R-5): a fully-anchored or fully-dead-ended
    // Confirmed work is never fanned out; a Pending work that reached here still
    // holds a chaseable bridge. Background keeps Audnexus eligible; Convergence
    // attributes any raised conflict.
    let mut work = work;
    let mut settle_superseded = false;
    if !chaseable.is_empty() {
        if let Some(resolver) = svc.resolver.as_ref() {
            match crate::async_resolver::settle_identity(
                resolver.as_ref(),
                &svc.db,
                user_id,
                &work,
                IdentityMode::Background,
                ConflictSource::Convergence,
            )
            .await
            {
                Ok(report) => settle_superseded = report.superseded,
                Err(e) => {
                    tracing::warn!(work_id, "convergence identity settle failed: {e}");
                }
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
                livrarr_domain::Freshness::PreferCache,
            )
            .await;
        work = svc.get(user_id, work_id).await?;
    }

    // Step 3 — dead-end accounting (R-1/R-2). A harvested anchor clears its counter;
    // a chaseable anchor still missing and unguessed gets +1 (an at-threshold anchor
    // is already excluded from `chaseable`, so it is never re-bumped). A Superseded
    // settle skips this entirely: the `before_missing` snapshot predates the winning
    // identity edit, so applying it would account against the wrong identity.
    let anchors_after = svc.db.list_anchors(work_id).await.unwrap_or_default();
    if !settle_superseded {
        let still_missing = missing_of(&work);
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
    }

    // Step 4 — outcome for the job's pacing. has_chaseable is recomputed AFTER
    // Step 3's dead-end bumps so an anchor that just hit the threshold no
    // longer holds the work open.
    let dead_ends_after = svc
        .db
        .list_anchor_dead_ends(work_id)
        .await
        .unwrap_or_default();
    let chaseable_after =
        chaseable_anchor_types(&work, &anchors_after, &dead_ends_after, threshold);
    let outcome = converge_outcome(
        work.identity_status,
        work.enrichment_status,
        !chaseable_after.is_empty(),
    );
    Ok(ConvergencePass {
        outcome,
        route_handoff: None,
        provider_chase_attempted: false,
    })
}

/// Step-4 outcome mapping for one [`converge_work`] pass.
///
/// `Completed` means no selection branch will re-pick the work: identity has
/// settled (`Confirmed`/`Provisional`), enrichment has settled
/// (`Enriched`/`Thin`), and no anchor remains chaseable. A terminal identity
/// (`NeedsReview`/`Conflict`/`NotFound`) always maps to `Terminal`, regardless
/// of enrichment or chaseable state — that check runs first. Everything else,
/// including a settled identity/enrichment pair that still has a chaseable
/// anchor, is `StillIncomplete`: the work remains selectable via
/// `list_convergence_due`'s chaseable-anchor branch, so the outcome says so
/// and lets the job back it off one cadence instead of clearing the clock.
fn converge_outcome(
    identity: IdentityStatus,
    enrichment: EnrichmentStatus,
    has_chaseable: bool,
) -> ConvergeOutcome {
    if matches!(
        identity,
        IdentityStatus::NeedsReview | IdentityStatus::Conflict | IdentityStatus::NotFound
    ) {
        ConvergeOutcome::Terminal
    } else if matches!(
        identity,
        IdentityStatus::Confirmed | IdentityStatus::Provisional
    ) && matches!(
        enrichment,
        EnrichmentStatus::Enriched | EnrichmentStatus::Thin
    ) && !has_chaseable
    {
        ConvergeOutcome::Completed
    } else {
        ConvergeOutcome::StillIncomplete
    }
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
pub(crate) async fn retry_all_incomplete<D, E, H>(
    svc: &WorkServiceImpl<D, E, H>,
    user_id: UserId,
) -> Result<RetrySummary, WorkServiceError>
where
    D: WorkDb
        + WorkDbCreate
        + AuthorDb
        + LibraryItemDb
        + GrabDb
        + ProvenanceDb
        + EnrichmentRetryDb
        + livrarr_db::ProviderRetryStateDb
        + ConfigDb
        + livrarr_db::SeriesDb
        + livrarr_db::HistoryDb
        + livrarr_db::AuthorLinkDb
        + livrarr_domain::services::WorkIdentityRepository
        + livrarr_domain::identity_layer::WorkIdentityRepository
        + Send
        + Sync,
    E: EnrichmentWorkflow + Send + Sync,
    H: HttpFetcher + Clone + Send + Sync + 'static,
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
            ) || (!svc.identity_routes_authoritative
                && w.identity_status == IdentityStatus::Pending)
        })
        .collect();

    let total = incomplete.len();
    let mut recovered = 0usize;
    let mut route_handoffs = Vec::new();

    for work in &incomplete {
        // A Pending work re-resolves identity first via the one identity road
        // (settle_identity) — Background mode so Audnexus stays eligible
        // (REQ-001). The promoted anchor survives the refresh below
        // (reset_enrichment_for_refresh touches only enrichment).
        if !svc.identity_routes_authoritative && work.identity_status == IdentityStatus::Pending {
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
        // Low: unattended retry-all-incomplete sweep (B4 table).
        if let Ok(refresh) = svc.refresh(user_id, work.id, RefreshSurface::Bulk).await {
            if let Some(handoff) = refresh.route_handoff {
                route_handoffs.push((work.id, handoff));
            }
            if let Ok(after) = svc.db.get_work(user_id, work.id).await {
                let still_incomplete = matches!(
                    after.enrichment_status,
                    EnrichmentStatus::Failed | EnrichmentStatus::Unenriched
                ) || (!svc.identity_routes_authoritative
                    && after.identity_status == IdentityStatus::Pending);
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
        route_handoffs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TERMINAL_IDENTITIES: [IdentityStatus; 3] = [
        IdentityStatus::NeedsReview,
        IdentityStatus::Conflict,
        IdentityStatus::NotFound,
    ];
    const SETTLED_IDENTITIES: [IdentityStatus; 2] =
        [IdentityStatus::Confirmed, IdentityStatus::Provisional];
    const SETTLED_ENRICHMENTS: [EnrichmentStatus; 2] =
        [EnrichmentStatus::Enriched, EnrichmentStatus::Thin];
    const ALL_ENRICHMENTS: [EnrichmentStatus; 4] = [
        EnrichmentStatus::Unenriched,
        EnrichmentStatus::Enriched,
        EnrichmentStatus::Thin,
        EnrichmentStatus::Failed,
    ];

    #[test]
    fn terminal_identities_always_map_to_terminal() {
        for identity in TERMINAL_IDENTITIES {
            for enrichment in ALL_ENRICHMENTS {
                for has_chaseable in [false, true] {
                    assert_eq!(
                        converge_outcome(identity, enrichment, has_chaseable),
                        ConvergeOutcome::Terminal,
                        "identity={identity:?} enrichment={enrichment:?} \
                         has_chaseable={has_chaseable}"
                    );
                }
            }
        }
    }

    #[test]
    fn settled_identity_and_enrichment_with_no_chaseable_anchor_completes() {
        for identity in SETTLED_IDENTITIES {
            for enrichment in SETTLED_ENRICHMENTS {
                assert_eq!(
                    converge_outcome(identity, enrichment, false),
                    ConvergeOutcome::Completed,
                    "identity={identity:?} enrichment={enrichment:?}"
                );
            }
        }
    }

    #[test]
    fn settled_identity_and_enrichment_with_chaseable_anchor_stays_incomplete() {
        for identity in SETTLED_IDENTITIES {
            for enrichment in SETTLED_ENRICHMENTS {
                assert_eq!(
                    converge_outcome(identity, enrichment, true),
                    ConvergeOutcome::StillIncomplete,
                    "identity={identity:?} enrichment={enrichment:?}"
                );
            }
        }
    }

    #[test]
    fn pending_identity_never_completes_or_terminalizes() {
        for enrichment in ALL_ENRICHMENTS {
            for has_chaseable in [false, true] {
                assert_eq!(
                    converge_outcome(IdentityStatus::Pending, enrichment, has_chaseable),
                    ConvergeOutcome::StillIncomplete,
                    "enrichment={enrichment:?} has_chaseable={has_chaseable}"
                );
            }
        }
    }

    #[test]
    fn confirmed_identity_with_unsettled_enrichment_stays_incomplete() {
        for enrichment in [EnrichmentStatus::Unenriched, EnrichmentStatus::Failed] {
            for has_chaseable in [false, true] {
                assert_eq!(
                    converge_outcome(IdentityStatus::Confirmed, enrichment, has_chaseable),
                    ConvergeOutcome::StillIncomplete,
                    "enrichment={enrichment:?} has_chaseable={has_chaseable}"
                );
            }
        }
    }
}
