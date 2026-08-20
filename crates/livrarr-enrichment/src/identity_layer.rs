//! Identity-layer-rewrite (F2) route-driven enrichment surface. IR v1
//! `livrarr-enrichment` module (ir-v1-identity-layer-rewrite.yaml:1204-1236).
//! Sibling seam (FP-032): never calls `livrarr-identity`, never writes
//! `identity_routes`/`IdentityStatus`; returns evidence to its
//! `livrarr-metadata` caller instead.
//!
//! DELIBERATE SHADOW NAMES (found late — flagged loudly here and in
//! STUBS-REPORT.md rather than only in the domain-crate shadow notes):
//! - `crate::EnrichmentService` (crate root, `lib.rs`) already exists — a
//!   pre-F2 trait (`enrich_work`/`reset_for_manual_refresh`/
//!   `inject_source_data`/...) implemented by the existing
//!   `EnrichmentServiceImpl<DB, Q, ME>`.
//! - `crate::EnrichmentError` (crate root, `lib.rs`) already exists as the
//!   error type for that pre-F2 trait.
//!
//! Both this module's `EnrichmentService` and `EnrichmentError` are
//! *different* items at a different path
//! (`livrarr_enrichment::identity_layer::*`); neither is re-exported at the
//! crate root, so both compile with no ambiguity.

use std::collections::HashSet;
use std::sync::Arc;

use livrarr_domain::identity_layer::{
    CapturedIdentity, CoverPlaceholderState, CoverSlotPresentation, IdentityProvider,
    MachineSubtitleProjection, RouteKind, WorkCoverPresentation, WorkIdentityRepository, WorkRoute,
    WorkRouteState,
};
use livrarr_domain::{RequestPriority, WorkId};
use livrarr_external_data::identity_layer::{
    NormalizedWorkIdentityEvidence, ProviderRouteEvidence,
};

/// IR v1 names `RouteProviderCall` without a field list. The planned
/// outbound call for one usable route — the route plus the priority
/// `IdentityProviderGateway::fetch_by_route` also takes.
#[derive(Debug, Clone)]
pub struct RouteProviderCall {
    pub route: WorkRoute,
    pub priority: RequestPriority,
}

#[derive(Debug, Clone)]
pub struct RouteEnrichmentPlan {
    pub usable_routes: Vec<WorkRoute>,
    pub provider_calls: Vec<RouteProviderCall>,
    pub manual_search_only: bool,
}

#[derive(Debug, Clone)]
pub struct WorkPresentationProjection {
    pub subtitle: MachineSubtitleProjection,
    pub covers: WorkCoverPresentation,
}

#[derive(Debug, Clone)]
pub struct EnrichmentApplyOutcome {
    /// Includes readable Goodreads work id from the already-fetched book page.
    pub metadata_generation: i64,
    pub captured_route_evidence: Vec<ProviderRouteEvidence>,
    pub presentation: WorkPresentationProjection,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum EnrichmentError {
    #[error("stale generation")]
    StaleGeneration,
    #[error("provider drift")]
    ProviderDrift,
    #[error("database error: {0}")]
    Database(String),
}

#[trait_variant::make(Send)]
pub trait EnrichmentService: Send + Sync {
    /// Pure/deterministic planning over an already-read identity; IR v1
    /// gives this a bare `RouteEnrichmentPlan` output (no `Result`), so it
    /// is not part of the trait's fallible async surface either — kept
    /// synchronous.
    fn plan_from_routes(&self, identity: CapturedIdentity) -> RouteEnrichmentPlan;

    async fn apply_provider_evidence(
        &self,
        work_id: WorkId,
        evidence: NormalizedWorkIdentityEvidence,
    ) -> Result<EnrichmentApplyOutcome, EnrichmentError>;
}

/// Production F2 enrichment seam for one coherent captured-identity snapshot.
/// The caller constructs one service per pass, so the snapshot generation is
/// the CAS claim used by `apply_provider_evidence` without adding an untyped
/// generation parameter to normalized provider evidence.
pub struct RouteDrivenEnrichmentService<R>
where
    R: WorkIdentityRepository + Send + Sync + 'static,
{
    repository: Arc<R>,
    snapshot: CapturedIdentity,
    priority: RequestPriority,
}

impl<R> RouteDrivenEnrichmentService<R>
where
    R: WorkIdentityRepository + Send + Sync + 'static,
{
    pub fn new(repository: R, snapshot: CapturedIdentity, priority: RequestPriority) -> Self {
        Self {
            repository: Arc::new(repository),
            snapshot,
            priority,
        }
    }

    pub fn from_arc(
        repository: Arc<R>,
        snapshot: CapturedIdentity,
        priority: RequestPriority,
    ) -> Self {
        Self {
            repository,
            snapshot,
            priority,
        }
    }
}

impl<R> EnrichmentService for RouteDrivenEnrichmentService<R>
where
    R: WorkIdentityRepository + Send + Sync + 'static,
{
    fn plan_from_routes(&self, identity: CapturedIdentity) -> RouteEnrichmentPlan {
        let mut seen = HashSet::new();
        let mut usable_routes = Vec::new();
        let mut manual_search_only = false;
        for route in identity
            .active_routes
            .into_iter()
            .filter(|route| matches!(route.state, WorkRouteState::Active))
        {
            if !is_declared_fetch_route(&route.provider, &route.kind) {
                manual_search_only = true;
                continue;
            }
            let key = (
                route.provider.clone(),
                route.kind.clone(),
                route.provider_scoped_id.clone(),
            );
            if seen.insert(key) {
                usable_routes.push(route);
            }
        }
        let provider_calls = usable_routes
            .iter()
            .cloned()
            .map(|route| RouteProviderCall {
                route,
                priority: self.priority,
            })
            .collect();
        RouteEnrichmentPlan {
            usable_routes,
            provider_calls,
            manual_search_only,
        }
    }

    async fn apply_provider_evidence(
        &self,
        work_id: WorkId,
        evidence: NormalizedWorkIdentityEvidence,
    ) -> Result<EnrichmentApplyOutcome, EnrichmentError> {
        if work_id != self.snapshot.own_work_id {
            return Err(EnrichmentError::ProviderDrift);
        }
        let live = self
            .repository
            .read_captured_identity(self.snapshot.user_id, work_id)
            .await
            .map_err(|error| EnrichmentError::Database(error.to_string()))?;
        if live.identity_generation != self.snapshot.identity_generation {
            return Err(EnrichmentError::StaleGeneration);
        }
        let source_was_planned = self.snapshot.active_routes.iter().any(|route| {
            matches!(route.state, WorkRouteState::Active)
                && route.provider == evidence.provider
                && is_declared_fetch_route(&route.provider, &route.kind)
        });
        let evidence_is_scoped = evidence
            .work_routes
            .iter()
            .chain(
                evidence
                    .editions
                    .iter()
                    .flat_map(|edition| edition.routes.iter()),
            )
            .all(|route| {
                route.provider == evidence.provider
                    && is_declared_fetch_route(&route.provider, &route.kind)
                    && !route.provider_scoped_id.trim().is_empty()
            });
        if !source_was_planned || !evidence_is_scoped {
            return Err(EnrichmentError::ProviderDrift);
        }

        let mut seen = HashSet::new();
        let captured_route_evidence = evidence
            .work_routes
            .into_iter()
            .filter(|route| {
                seen.insert((
                    route.provider.clone(),
                    route.kind.clone(),
                    route.provider_scoped_id.clone(),
                ))
            })
            .collect();
        let subtitle = evidence
            .editions
            .iter()
            .find_map(|edition| edition.subtitle.clone());
        Ok(EnrichmentApplyOutcome {
            metadata_generation: live.identity_generation,
            captured_route_evidence,
            presentation: WorkPresentationProjection {
                subtitle: MachineSubtitleProjection {
                    user_id: live.user_id,
                    work_id: live.own_work_id,
                    value: subtitle,
                    edition_id: None,
                    provenance: None,
                    computed_at_generation: live.identity_generation,
                },
                covers: empty_cover_presentation(),
            },
        })
    }
}

fn is_declared_fetch_route(provider: &IdentityProvider, kind: &RouteKind) -> bool {
    matches!(
        (provider, kind),
        (IdentityProvider::OpenLibrary, RouteKind::OpenLibraryWork)
            | (IdentityProvider::Goodreads, RouteKind::GoodreadsBookEdition)
            | (IdentityProvider::Hardcover, RouteKind::HardcoverWork)
            | (IdentityProvider::IsbnRegistry, RouteKind::Isbn13Edition)
            | (IdentityProvider::Amazon, RouteKind::AsinEdition)
    )
}

fn empty_cover_presentation() -> WorkCoverPresentation {
    WorkCoverPresentation {
        format_needed: None,
        ebook: CoverSlotPresentation {
            selected: None,
            placeholder: Some(CoverPlaceholderState::NowhereToLook),
        },
        audiobook: CoverSlotPresentation {
            selected: None,
            placeholder: Some(CoverPlaceholderState::NowhereToLook),
        },
    }
}
