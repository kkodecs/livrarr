//! Amended `IdentityStatus`/`CapturedIdentity` (IR v1 amendments 2026-08-05).
//!
//! These namespaced v2 shapes coexist with the legacy scalar-anchor types until
//! the installation-wide authority marker activates the new read/write path.

use serde::{Deserialize, Serialize};

use super::route::{RouteKind, RouteProvenance, WorkRoute, WorkRouteState};
use super::title::IdentityTitleTuple;

/// Exactly three variants (IR v1 amendment, FP-028). Conflicts/reviews are
/// orthogonal records and never overload this projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentityStatus {
    UserConfirmed,
    Connected,
    NotConnected,
}

/// The one-way identity-to-enrichment contract (IR v1 amendment, replacing
/// the five-scalar-anchor legacy shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapturedIdentity {
    pub user_id: crate::UserId,
    pub own_work_id: crate::WorkId,
    pub identity_title: IdentityTitleTuple,
    pub primary_author_id: crate::AuthorId,
    /// Audited text-distinction key for the broad main-title/Author cohort.
    /// `common` is the only value eligible for automatic same-text folding.
    pub text_distinction: String,
    pub active_routes: Vec<WorkRoute>,
    pub status: IdentityStatus,
    pub identity_generation: i64,
}

/// Route-authoritative compatibility projection for the identifier fields on
/// the public Work DTO. Frozen columns on `works` are deliberately not an
/// input to this shape.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkIdentifierProjection {
    pub ol_key: Option<String>,
    pub hc_key: Option<String>,
    pub gr_key: Option<String>,
    pub isbn_13: Option<String>,
    pub asin: Option<String>,
}

/// Batch presentation data required by Work-bearing response surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkIdentityPresentation {
    pub work_id: crate::WorkId,
    pub status: IdentityStatus,
    pub identifiers: WorkIdentifierProjection,
}

/// Select identifier values from active routes only.
///
/// Multiple eligible edition routes are resolved deterministically: an
/// explicitly user-confirmed route wins, followed by evidence provenance
/// (`UserChoice`, `OwnedFile`, provider, merge, migration), newest observation,
/// lexical provider-scoped id, then route id. Goodreads work routes outrank the
/// migration-only Goodreads edition route used to preserve pre-cutover `gr_key`
/// meaning without consulting the frozen column.
pub fn project_work_identifiers(routes: &[WorkRoute]) -> WorkIdentifierProjection {
    fn provenance_rank(route: &WorkRoute) -> u8 {
        match route.provenance {
            RouteProvenance::UserChoice => 5,
            RouteProvenance::OwnedFile { .. } => 4,
            RouteProvenance::Provider(_)
            | RouteProvenance::SearchFallback { .. }
            | RouteProvenance::TextDecisiveSearchFallback { .. } => 3,
            RouteProvenance::MergeCoalesced => 2,
            RouteProvenance::Migrated { .. } => 1,
        }
    }

    fn better(candidate: &WorkRoute, current: &WorkRoute) -> bool {
        (candidate.user_confirmed && !current.user_confirmed)
            || (candidate.user_confirmed == current.user_confirmed
                && (provenance_rank(candidate) > provenance_rank(current)
                    || (provenance_rank(candidate) == provenance_rank(current)
                        && (candidate.observed_at > current.observed_at
                            || (candidate.observed_at == current.observed_at
                                && (candidate.provider_scoped_id < current.provider_scoped_id
                                    || (candidate.provider_scoped_id
                                        == current.provider_scoped_id
                                        && candidate.id < current.id)))))))
    }

    fn select<'a>(routes: impl Iterator<Item = &'a WorkRoute>) -> Option<String> {
        routes
            .filter(|route| matches!(route.state, WorkRouteState::Active))
            .fold(None, |selected: Option<&WorkRoute>, route| match selected {
                Some(current) if !better(route, current) => Some(current),
                _ => Some(route),
            })
            .map(|route| route.provider_scoped_id.clone())
    }

    let gr_work = select(
        routes
            .iter()
            .filter(|route| matches!(route.kind, RouteKind::GoodreadsWork)),
    );
    let migrated_gr_edition = select(routes.iter().filter(|route| {
        matches!(route.kind, RouteKind::GoodreadsBookEdition)
            && matches!(
                &route.provenance,
                RouteProvenance::Migrated { legacy_field } if legacy_field == "gr_key"
            )
    }));

    WorkIdentifierProjection {
        ol_key: select(
            routes
                .iter()
                .filter(|route| matches!(route.kind, RouteKind::OpenLibraryWork)),
        ),
        hc_key: select(
            routes
                .iter()
                .filter(|route| matches!(route.kind, RouteKind::HardcoverWork)),
        ),
        gr_key: gr_work.or(migrated_gr_edition),
        isbn_13: select(
            routes
                .iter()
                .filter(|route| matches!(route.kind, RouteKind::Isbn13Edition)),
        ),
        asin: select(
            routes
                .iter()
                .filter(|route| matches!(route.kind, RouteKind::AsinEdition)),
        ),
    }
}
