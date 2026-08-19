//! Shared types used across multiple identity-layer-rewrite (F2) crates.
//! IR v1 `shared_types` (ir-v1-identity-layer-rewrite.yaml:1430-1454).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::route::{IdentityProvider, RouteKind};

/// Stable numeric identifier for a first-class [`super::edition::Edition`].
/// IR v1 `kind: type-alias`; matches the existing `WorkId`/`UserId`/`AuthorId`
/// convention in `crate::entities` (all plain `i64` aliases).
pub type EditionId = i64;

/// The typed, provider-scoped key that uniquely names one [`super::route::WorkRoute`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RouteKey {
    pub provider: IdentityProvider,
    pub kind: RouteKind,
    pub value: String,
}

/// The exact-revision fingerprint of one owned file, used to key
/// [`super::edition::EmbeddedCoverInspectionRecord`] and route/edition file evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileRevision {
    pub size_bytes: u64,
    pub modified_ns: i128,
    pub sha256: [u8; 32],
}

/// The closed set of review-card kinds reachable through
/// `IdentityRoadService::resolve_review` and the pre-activation cutover CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewKind {
    IdentityConflict,
    PendingRoute,
    GroupIdentity,
    FieldResolution,
    ContributorOrder,
    EditionEvidence,
    ImportIdentity,
    MigrationRepair,
    InvariantRepair,
}

impl ReviewKind {
    /// Canonical TEXT vocabulary for `identity_review_cards.kind`.
    ///
    /// The payload column is JSON, but the kind discriminator is a plain TEXT
    /// code. Keeping both directions here prevents SQL writers and CLI readers
    /// from drifting between raw enum names and JSON string literals.
    pub const fn storage_code(self) -> &'static str {
        match self {
            Self::IdentityConflict => "IdentityConflict",
            Self::PendingRoute => "PendingRoute",
            Self::GroupIdentity => "GroupIdentity",
            Self::FieldResolution => "FieldResolution",
            Self::ContributorOrder => "ContributorOrder",
            Self::EditionEvidence => "EditionEvidence",
            Self::ImportIdentity => "ImportIdentity",
            Self::MigrationRepair => "MigrationRepair",
            Self::InvariantRepair => "InvariantRepair",
        }
    }

    pub fn from_storage_code(value: &str) -> Option<Self> {
        match value {
            "IdentityConflict" => Some(Self::IdentityConflict),
            "PendingRoute" => Some(Self::PendingRoute),
            "GroupIdentity" => Some(Self::GroupIdentity),
            "FieldResolution" => Some(Self::FieldResolution),
            "ContributorOrder" => Some(Self::ContributorOrder),
            "EditionEvidence" => Some(Self::EditionEvidence),
            "ImportIdentity" => Some(Self::ImportIdentity),
            "MigrationRepair" => Some(Self::MigrationRepair),
            "InvariantRepair" => Some(Self::InvariantRepair),
            _ => None,
        }
    }
}

/// The actor resolving a review card: an authenticated post-activation user,
/// or the pre-activation exclusive one-shot cutover operator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewActor {
    AuthenticatedUser {
        user_id: crate::UserId,
    },
    CutoverOperator {
        installation_id: String,
        invocation_id: String,
    },
}

/// One explicit default Edition for a Work's declared format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DefaultEdition {
    pub user_id: crate::UserId,
    pub work_id: crate::WorkId,
    pub format: super::edition::EditionFormat,
    pub edition_id: EditionId,
    pub provenance: EvidenceProvenance,
}

/// A value paired with where it came from and when it was observed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourcedValue<T> {
    pub value: T,
    pub provenance: EvidenceProvenance,
    pub observed_at: DateTime<Utc>,
}

/// Where an identity-layer fact came from, for P5 precedence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceProvenance {
    User,
    OwnedFile,
    Provider(IdentityProvider),
    Migrated,
}
