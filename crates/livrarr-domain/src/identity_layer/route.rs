//! Route vocabulary — the works-side generalization of `crate::author_link`'s
//! `AuthorRoute`. IR v1 domain module (ir-v1-identity-layer-rewrite.yaml:704-761).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::shared::FileRevision;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IdentityProvider {
    OpenLibrary,
    Goodreads,
    Hardcover,
    IsbnRegistry,
    Amazon,
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RouteScope {
    Work,
    Edition,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RouteKind {
    OpenLibraryWork,
    GoodreadsWork,
    HardcoverWork,
    Isbn13Edition,
    AsinEdition,
    GoodreadsBookEdition,
    Undeclared {
        provider_kind: String,
        scope: RouteScope,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RouteOwner {
    Work(crate::WorkId),
    Edition(super::shared::EditionId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkRouteState {
    Active,
    Retired { audit_id: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RouteProvenance {
    UserChoice,
    OwnedFile {
        library_item_id: Option<crate::LibraryItemId>,
        file_revision: FileRevision,
    },
    Provider(IdentityProvider),
    SearchFallback {
        provider: IdentityProvider,
        corroborating_kind: RouteKind,
    },
    TextDecisiveSearchFallback {
        provider: IdentityProvider,
    },
    Migrated {
        legacy_field: String,
    },
    MergeCoalesced,
}

/// A user-scoped provider route whose owner is exactly one Work or Edition.
/// Amends the pre-F2 "Anchors are monotonic" invariant per IR v1 amendments
/// (2026-08-05): active routes change owner/state only through audited
/// resolution, audited merge, or user edit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkRoute {
    pub id: i64,
    pub user_id: crate::UserId,
    pub owner: RouteOwner,
    pub resolved_work_id: crate::WorkId,
    pub provider: IdentityProvider,
    pub kind: RouteKind,
    pub provider_scoped_id: String,
    pub state: WorkRouteState,
    pub provenance: RouteProvenance,
    pub user_confirmed: bool,
    pub observed_at: DateTime<Utc>,
}
