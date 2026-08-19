//! Door evidence contract — the typed input to `IdentityRoadService::settle`.
//! IR v1 domain module (ir-v1-identity-layer-rewrite.yaml:848-936,
//! `door_evidence_contract`, `minimal_work_contract`).

use serde::{Deserialize, Serialize};

use super::route::WorkRoute;
use super::services::IdentityRoadError;
use super::shared::{FileRevision, RouteKey};
use super::status::IdentityStatus;
use super::title::IdentityTitleTuple;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DoorKind {
    DirectAdd,
    ManualImport,
    ListImport,
    AuthorMonitor,
    SeriesMonitor,
    ReadarrImport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentityRoadOrigin {
    CreationDoor(DoorKind),
    EnrichmentPass,
    ManualRefresh,
    ConvergenceVisit,
    /// A user requested a new canonical title and/or primary Author.
    WorkUpdateRekey,
    /// A user requested that `loser_work_id` fold into the selected Work.
    ManualWorkMerge {
        loser_work_id: crate::WorkId,
        choices: Vec<crate::services::MergeFieldChoiceEntry>,
    },
    /// A user affirmed one previously parked provider route.
    AffirmPendingRoute,
}

/// Required input per `minimal_work_contract`: "A valid parsed main title and
/// at least one resolved Author record."
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinimumWorkEvidence {
    pub title: String,
    pub authors: Vec<crate::AuthorId>,
}

/// IR v1 names `OwnedFileEvidence` without a field list; shaped to match
/// `RouteProvenance::OwnedFile`'s fields (same evidence tier, same document).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedFileEvidence {
    pub library_item_id: crate::LibraryItemId,
    pub file_revision: FileRevision,
}

/// The normalized Work identity carried by provider-only creation doors.
/// Existing-work capture handoffs do not need to repeat this core because
/// their coherent [`super::status::CapturedIdentity`] supplies it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderWorkIdentityCore {
    pub identity_title: IdentityTitleTuple,
    pub primary_author_id: crate::AuthorId,
}

/// How a provider route reached the machine-observation handoff. Ordinary
/// anchor payloads keep their established provider provenance; REQ-027 search
/// fallback carries the edition-id kind that corroborated the selected work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ProviderIdentityEvidenceProvenance {
    #[default]
    AnchorPayload,
    SearchFallback {
        corroborating_kind: super::route::RouteKind,
    },
    /// REQ-027(b): Same-title + author-Agree with no distinct provider-work
    /// tie. Unlike `SearchFallback`, no edition id corroborated this route.
    TextDecisiveSearchFallback,
}

/// Provider evidence carries both its typed route and, when it is the only
/// creation evidence, the normalized Work core needed to build the decision
/// request. A genuinely coreless provider-only creation still fails closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderIdentityEvidence {
    pub provider: super::route::IdentityProvider,
    pub route: RouteKey,
    pub work_core: Option<ProviderWorkIdentityCore>,
    pub provenance: ProviderIdentityEvidenceProvenance,
}

/// IR v1 names `UserIdentityChoice` without a field list. DirectAdd's
/// `outcome_policy` text ("The user's candidate pick or explicit title+author
/// creation") gives the two-arm shape directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserIdentityChoice {
    ExistingWork(crate::WorkId),
    ExplicitCreate(MinimumWorkEvidence),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityEvidenceBundle {
    pub user_choice: Option<UserIdentityChoice>,
    pub owned_files: Vec<OwnedFileEvidence>,
    pub provider_identity: Vec<ProviderIdentityEvidence>,
    pub minimum: Option<MinimumWorkEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentityRoadInteraction {
    HumanWatching,
    MachineAlone,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityRoadRequest {
    pub user_id: crate::UserId,
    pub origin: IdentityRoadOrigin,
    pub evidence: IdentityEvidenceBundle,
    pub interaction: IdentityRoadInteraction,
    pub existing_work_id: Option<crate::WorkId>,
}

/// Stable machine-readable reason for a deferred identity decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeferReason(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IdentityRoadOutcome {
    Settled {
        work_id: crate::WorkId,
        created: bool,
        routes: Vec<WorkRoute>,
        status: IdentityStatus,
        library_items_moved: usize,
        grabs_moved: usize,
    },
    ReviewPending {
        review_id: i64,
        kind: super::shared::ReviewKind,
        unattached: bool,
        expected_generation: i64,
        provenance: super::shared::EvidenceProvenance,
    },
    Deferred {
        reason: DeferReason,
    },
    Rejected {
        reason: IdentityRoadError,
    },
}
