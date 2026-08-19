//! Title-tuple and subtitle-projection vocabulary. IR v1 domain module
//! (ir-v1-identity-layer-rewrite.yaml:784-808) and `subtitle_projection_policy`.

use serde::{Deserialize, Serialize};

use super::shared::EditionId;
use super::shared::EvidenceProvenance;

/// Captured once from the P5-winning evidence. `main` is the identity key's
/// main component; `subtitle`/`volume` participate in the exact-tuple group
/// key. Never mutated by machine subtitle projection, defaults, monitoring,
/// or provider refresh (FP-015).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityTitleTuple {
    pub main: String,
    pub subtitle: Option<String>,
    pub volume: Option<String>,
    pub normalized_main: String,
    pub normalized_subtitle: String,
    pub normalized_volume: String,
    pub provenance: EvidenceProvenance,
}

/// Display-only subtitle override state; independent of `IdentityTitleTuple`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubtitleOverride {
    Automatic,
    UserValue(String),
    UserAbsent,
}

/// Recomputed totally from preserved editions/defaults/source order after
/// every trigger transaction (subtitle_projection_policy.recompute_triggers).
/// Never merged, never a second identity key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MachineSubtitleProjection {
    pub user_id: crate::UserId,
    pub work_id: crate::WorkId,
    /// Selected-edition absence remains absence.
    pub value: Option<String>,
    pub edition_id: Option<EditionId>,
    pub provenance: Option<EvidenceProvenance>,
    pub computed_at_generation: i64,
}
