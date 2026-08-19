//! `ReviewResolutionCommand` — the one typed continuation surface reached by
//! both the authenticated post-activation HTTP handler and the pre-activation
//! exclusive cutover CLI. IR v1 domain module
//! (ir-v1-identity-layer-rewrite.yaml:887-898) and `pre_activation_review_cli`.

use serde::{Deserialize, Serialize};

use super::conflict::IdentityConflictResolution;
use super::contributor::{AuthorRef, ContributorPartition, WorkReference};
use super::door::{IdentityEvidenceBundle, MinimumWorkEvidence};
use super::route::RouteOwner;
use super::shared::RouteKey;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingRouteAction {
    Affirm { surviving_routes: Vec<RouteKey> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupIdentityAction {
    AttachOrMerge { anchor: crate::WorkId },
    DifferentFromAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldResolutionAction {
    ChoosePreservedValue { evidence_id: i64 },
    ExplicitAbsence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditionEvidenceAction {
    ChooseDirectEvidence { evidence_id: i64 },
    RetainUnknownOrAbsent,
    ArchiveEmptyShell,
}

/// IR v1 names `ImportIdentityChoice` without a field list; shaped after
/// `readarr_identity_precedence.nonconflict_exits`'
/// `explicit_import_review_identity_or_minimum_creation_choice` and mirrors
/// `UserIdentityChoice`'s two-arm shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportIdentityChoice {
    UseIdentity(crate::WorkId),
    CreateMinimum(MinimumWorkEvidence),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportIdentityAction {
    CorrectedMetadataRetry { evidence: IdentityEvidenceBundle },
    ResolvingIdentifier { route: RouteKey },
    ExplicitIdentityOrMinimum { choice: ImportIdentityChoice },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MigrationRepairAction {
    CorrectTypedValue { route: RouteKey },
    AssignScopeAndOwner { route: RouteKey, owner: RouteOwner },
    DiscardProvenNonIdentity { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvariantRepairAction {
    Recompute,
    SupplyMissingReference { reference: WorkReference },
    DiscardMalformedDerivedRow { reason: String },
}

/// Every mutation re-enters through this one continuation
/// (`IdentityRoadService::resolve_review`); every variant carries
/// `card_id` + `expected_generation` for the optimistic-concurrency claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewResolutionCommand {
    IdentityConflict {
        card_id: i64,
        expected_generation: i64,
        action: IdentityConflictResolution,
    },
    PendingRoute {
        card_id: i64,
        expected_generation: i64,
        action: PendingRouteAction,
    },
    GroupIdentity {
        card_id: i64,
        expected_generation: i64,
        action: GroupIdentityAction,
    },
    FieldResolution {
        card_id: i64,
        expected_generation: i64,
        action: FieldResolutionAction,
    },
    ContributorOrder {
        card_id: i64,
        expected_generation: i64,
        partition: Vec<ContributorPartition>,
        order: Vec<AuthorRef>,
        primary: AuthorRef,
    },
    EditionEvidence {
        card_id: i64,
        expected_generation: i64,
        action: EditionEvidenceAction,
    },
    ImportIdentity {
        card_id: i64,
        expected_generation: i64,
        action: ImportIdentityAction,
    },
    MigrationRepair {
        card_id: i64,
        expected_generation: i64,
        action: MigrationRepairAction,
    },
    InvariantRepair {
        card_id: i64,
        expected_generation: i64,
        action: InvariantRepairAction,
    },
}

impl ReviewResolutionCommand {
    pub fn card_id(&self) -> i64 {
        match self {
            Self::IdentityConflict { card_id, .. }
            | Self::PendingRoute { card_id, .. }
            | Self::GroupIdentity { card_id, .. }
            | Self::FieldResolution { card_id, .. }
            | Self::ContributorOrder { card_id, .. }
            | Self::EditionEvidence { card_id, .. }
            | Self::ImportIdentity { card_id, .. }
            | Self::MigrationRepair { card_id, .. }
            | Self::InvariantRepair { card_id, .. } => *card_id,
        }
    }

    pub fn expected_generation(&self) -> i64 {
        match self {
            Self::IdentityConflict {
                expected_generation,
                ..
            }
            | Self::PendingRoute {
                expected_generation,
                ..
            }
            | Self::GroupIdentity {
                expected_generation,
                ..
            }
            | Self::FieldResolution {
                expected_generation,
                ..
            }
            | Self::ContributorOrder {
                expected_generation,
                ..
            }
            | Self::EditionEvidence {
                expected_generation,
                ..
            }
            | Self::ImportIdentity {
                expected_generation,
                ..
            }
            | Self::MigrationRepair {
                expected_generation,
                ..
            }
            | Self::InvariantRepair {
                expected_generation,
                ..
            } => *expected_generation,
        }
    }

    pub fn kind(&self) -> super::shared::ReviewKind {
        match self {
            Self::IdentityConflict { .. } => super::shared::ReviewKind::IdentityConflict,
            Self::PendingRoute { .. } => super::shared::ReviewKind::PendingRoute,
            Self::GroupIdentity { .. } => super::shared::ReviewKind::GroupIdentity,
            Self::FieldResolution { .. } => super::shared::ReviewKind::FieldResolution,
            Self::ContributorOrder { .. } => super::shared::ReviewKind::ContributorOrder,
            Self::EditionEvidence { .. } => super::shared::ReviewKind::EditionEvidence,
            Self::ImportIdentity { .. } => super::shared::ReviewKind::ImportIdentity,
            Self::MigrationRepair { .. } => super::shared::ReviewKind::MigrationRepair,
            Self::InvariantRepair { .. } => super::shared::ReviewKind::InvariantRepair,
        }
    }
}
