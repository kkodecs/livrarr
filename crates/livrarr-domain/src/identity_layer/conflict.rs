//! Amended `IdentityConflict` (IR v1 amendments 2026-08-05) and
//! `identity_conflict_policy`. This namespaced type coexists with the legacy
//! scalar-anchor conflict until v2 authority activation.

use serde::{Deserialize, Serialize};

use super::route::RouteOwner;
use super::shared::{EditionId, RouteKey};

/// Exactly three classes (IR v1 amendment, replacing veto/enforcement
/// semantics with non-blocking user-resolved route disputes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentityConflictClass {
    SameProviderWorkIdDisagreement,
    CrossProviderWorkKeyDisagreement,
    RouteOwnedByDifferentWork,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentityConflictResolution {
    Accept {
        surviving_routes: Vec<RouteKey>,
        target_edition: Option<EditionId>,
    },
    Reject {
        surviving_routes: Vec<RouteKey>,
    },
    DifferentWork {
        winning_work_id: crate::WorkId,
        surviving_routes: Vec<RouteKey>,
        target_edition: Option<EditionId>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentityConflictRowStatus {
    Pending,
    Resolved,
}

/// A contested typed route retained with its proposed owner until resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParkedRouteCandidate {
    pub route: RouteKey,
    pub proposed_owner: RouteOwner,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdentityConflict {
    pub id: i64,
    pub user_id: crate::UserId,
    pub class: IdentityConflictClass,
    pub current_work_id: crate::WorkId,
    pub candidate: ParkedRouteCandidate,
    pub status: IdentityConflictRowStatus,
    pub resolution: Option<IdentityConflictResolution>,
    pub audit_id: Option<i64>,
}
