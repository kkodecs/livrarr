//! Background convergence logic extracted from `work_service.rs` (M-005 / Phase 2 dedup).
//!
//! `converge_work` and `retry_all_incomplete` bodies live here as free functions.
//! The `WorkService` trait methods remain in `work_service.rs` as thin delegation
//! wrappers so the public contract is unchanged.

use livrarr_db::{
    AuthorDb, ConfigDb, EnrichmentRetryDb, GrabDb, LibraryItemDb, ProvenanceDb, WorkDb,
    WorkDbCreate,
};
use livrarr_domain::services::{
    ConvergeOutcome, ConvergencePass, EnrichmentMode, EnrichmentWorkflow, HttpFetcher,
    LedgerPassAccounting, RefreshSurface, RetrySummary, SourceProviderData, WorkService,
    WorkServiceError,
};
use livrarr_domain::{EnrichmentStatus, UserId, Work, WorkId};

use crate::work_service::WorkServiceImpl;

/// Background convergence pass for one work: read the captured identity and
/// run either the route-search leg or background enrichment, as routes and
/// enrichment status dictate.
///
/// Called exclusively by the `WorkService::converge_work` thin wrapper.
pub(crate) async fn converge_work<D, E, H>(
    svc: &WorkServiceImpl<D, E, H>,
    user_id: UserId,
    work_id: WorkId,
    _threshold: u32,
    source_provider_data: Option<SourceProviderData>,
) -> Result<ConvergencePass, WorkServiceError>
where
    D: WorkDb
        + WorkDbCreate
        + AuthorDb
        + LibraryItemDb
        + GrabDb
        + ProvenanceDb
        + livrarr_db::SourceReferenceDb
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
    let captured = livrarr_domain::identity_layer::WorkIdentityRepository::read_captured_identity(
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
    let has_route = |kind: livrarr_domain::identity_layer::RouteKind| {
        captured
            .active_routes
            .iter()
            .any(|route| route.kind == kind)
    };
    let normalized_language = work
        .language
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let english_applicability =
        matches!(normalized_language.as_str(), "" | "en" | "eng" | "english")
            || normalized_language.starts_with("en-")
            || normalized_language.starts_with("en_");
    // REQ-027 v11 search-only visit: another provider's Work route no
    // longer makes a connected/enriched Work a convergence no-op. The
    // queue remains the authority for anchor standing and configured
    // search availability; this gate merely ensures it is invoked.
    let provider_search_chaseable =
        !has_route(livrarr_domain::identity_layer::RouteKind::GoodreadsWork)
            || (english_applicability
                && (!has_route(livrarr_domain::identity_layer::RouteKind::OpenLibraryWork)
                    || !has_route(livrarr_domain::identity_layer::RouteKind::HardcoverWork)));
    let enrichment_incomplete = matches!(
        work.enrichment_status,
        EnrichmentStatus::Unenriched | EnrichmentStatus::Failed
    );
    let search_only = !enrichment_incomplete
        && !bridge_chaseable
        && provider_search_chaseable
        && source_provider_data.is_none();
    let mut enrichment_outcome = None;
    let mut search_outcome = None;
    if search_only {
        search_outcome = Some(
            svc.enrichment
                .search_work_routes(user_id, work_id, livrarr_domain::RequestPriority::Low)
                .await
                .map_err(|error| WorkServiceError::Enrichment(error.to_string()))?,
        );
    } else if enrichment_incomplete || bridge_chaseable || source_provider_data.is_some() {
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
    let mut route_handoff = enrichment_outcome
        .as_mut()
        .and_then(|outcome| outcome.route_handoff.take());
    if let Some(search) = search_outcome.as_mut() {
        let fresh = std::mem::take(&mut search.captured_provider_identity)
            .into_iter()
            .filter(|evidence| {
                !captured.active_routes.iter().any(|route| {
                    route.provider == evidence.route.provider
                        && route.kind == evidence.route.kind
                        && route.provider_scoped_id == evidence.route.value
                })
            })
            .collect::<Vec<_>>();
        let route_proposals = std::mem::take(&mut search.captured_route_proposals);
        if !fresh.is_empty() || !route_proposals.is_empty() {
            route_handoff = Some(livrarr_domain::identity_layer::CapturedRouteHandoff {
                metadata_generation: captured.identity_generation,
                provider_identity: fresh,
                route_proposals,
            });
        }
    }
    let search_leg_fired = search_outcome
        .as_ref()
        .is_some_and(|outcome| outcome.search_leg_fired)
        || enrichment_outcome
            .as_ref()
            .is_some_and(|outcome| outcome.search_leg_fired);
    let outcome = if search_only {
        if search_leg_fired {
            ConvergeOutcome::StillIncomplete
        } else if matches!(
            after.enrichment_status,
            EnrichmentStatus::Enriched | EnrichmentStatus::Thin
        ) {
            ConvergeOutcome::Completed
        } else {
            ConvergeOutcome::StillIncomplete
        }
    } else if bridge_chaseable || provider_search_chaseable || route_handoff.is_some() {
        ConvergeOutcome::StillIncomplete
    } else if matches!(
        after.enrichment_status,
        EnrichmentStatus::Enriched | EnrichmentStatus::Thin
    ) {
        ConvergeOutcome::Completed
    } else {
        ConvergeOutcome::StillIncomplete
    };
    Ok(ConvergencePass {
        outcome,
        route_handoff,
        observed_identity_generation: Some(captured.identity_generation),
        provider_chase_attempted: enrichment_outcome
            .as_ref()
            .is_some_and(|outcome| outcome.provider_chase_attempted)
            || search_outcome
                .as_ref()
                .is_some_and(|outcome| outcome.provider_chase_attempted),
        search_leg_fired,
        // REQ-027: the pass-level ledger fold. `combine` keeps one
        // sub-outcome's failed leg visible at the burn site — a burnable
        // sibling can never OR it away.
        ledger_accounting: enrichment_outcome
            .as_ref()
            .map_or(LedgerPassAccounting::Idle, |outcome| {
                outcome.ledger_accounting
            })
            .combine(
                search_outcome
                    .as_ref()
                    .map_or(LedgerPassAccounting::Idle, |outcome| {
                        outcome.ledger_accounting
                    }),
            ),
    })
}

/// Single-pass sweep over every incomplete work for the user.
///
/// "Incomplete" = `Failed` or `Unenriched`. Each re-enriches through the one
/// road (`refresh` → `run_unified` → materialize).
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
        + livrarr_db::SourceReferenceDb
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
    // Single pass over every "incomplete" work — Failed or Unenriched —
    // filtered in memory (like refresh_all). This REPLACES
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
            )
        })
        .collect();

    let total = incomplete.len();
    let mut recovered = 0usize;
    let mut route_handoffs = Vec::new();

    for work in &incomplete {
        // Re-enrich through the one road (refresh -> run_unified ->
        // materialize). A refresh error never blocks the rest of the sweep.
        // Low: unattended retry-all-incomplete sweep (B4 table).
        if let Ok(refresh) = svc.refresh(user_id, work.id, RefreshSurface::Bulk).await {
            if let Some(handoff) = refresh.route_handoff {
                // OAI-U5-201: this Work's fate is undetermined until the
                // registered Retry-All caller resolves the ConvergenceVisit
                // handoff. Carry its prior incomplete status so the caller
                // can restore it if the handoff does not settle for this
                // same Work; do not declare recovery from this pre-handoff
                // snapshot (spec-v11 AC-005 line 664).
                route_handoffs.push((work.id, handoff, work.enrichment_status));
                continue;
            }
            if let Ok(after) = svc.db.get_work(user_id, work.id).await {
                let still_incomplete = matches!(
                    after.enrichment_status,
                    EnrichmentStatus::Failed | EnrichmentStatus::Unenriched
                );
                if !still_incomplete {
                    recovered += 1;
                }
            }
        }
    }

    // A refresh() error or a post-refresh get_work() error leaves a work
    // counted in neither `recovered` nor `route_handoffs`, so it still lands
    // in `still_incomplete` below via subtraction — unchanged from before
    // OAI-U5-201. Handoff-bearing works are excluded from `recovered` here
    // and from this subtraction (via route_handoffs.len()); the registered
    // Retry-All caller finalizes their recovered/still_incomplete split only
    // after each handoff resolves (spec-v11 AC-005 line 664).
    Ok(RetrySummary {
        total,
        recovered,
        still_incomplete: total - recovered - route_handoffs.len(),
        route_handoffs,
    })
}
