//! Identity-layer-rewrite (F2) service/repository traits and the free
//! deterministic functions. IR v1 domain module functions
//! (ir-v1-identity-layer-rewrite.yaml:1016-1071).
//!
//! `WorkIdentityRepository` is intentionally namespaced separately from the
//! legacy anchor-confirmation repository while authority transitions to v2.

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::conflict::ParkedRouteCandidate;
use super::contributor::WorkContributor;
use super::cover::{CoverPlaceholderState, CoverSlotPresentation, WorkCoverPresentation};
use super::door::{
    IdentityEvidenceBundle, IdentityRoadInteraction, IdentityRoadOrigin, IdentityRoadOutcome,
    IdentityRoadRequest, OwnedFileEvidence, ProviderIdentityEvidence, WorkEditClaim,
};
use super::edition::{Edition, EditionFormat};
use super::matching::{
    DirectionalMatchVerdicts, LostMatchGuardSet, ProbeId, WorkIdentityEvidence, WrongMergeGuardSet,
};
use super::migration::{IdentityAuthorityReadiness, IdentityMigrationReport};
use super::review::ReviewResolutionCommand;
use super::route::WorkRoute;
use super::route::{IdentityProvider, RouteKind, WorkRouteState};
use super::shared::{DefaultEdition, EditionId, ReviewActor, ReviewKind};
use super::status::{CapturedIdentity, WorkIdentityPresentation};
use super::title::{IdentityTitleTuple, MachineSubtitleProjection};

// ---------------------------------------------------------------------------
// Errors — unions of every `errors:` list attached to a function returning
// this named error type across IR v1 (domain trait methods, the metadata
// `IdentityRoadServiceImpl` inherent methods sharing the road's error
// vocabulary, and the `livrarr-db` `SqliteDb` methods implementing these
// repository traits).
// ---------------------------------------------------------------------------

fn continuation_owner(kind: ReviewKind) -> &'static str {
    match kind {
        ReviewKind::PendingRoute | ReviewKind::GroupIdentity => "available continuation",
        ReviewKind::IdentityConflict => "wave B",
        ReviewKind::EditionEvidence => "its post-wave-B continuation feature",
        ReviewKind::ImportIdentity => "the Readarr feature",
        ReviewKind::FieldResolution => "wave B's merge unit",
        ReviewKind::ContributorOrder
        | ReviewKind::MigrationRepair
        | ReviewKind::InvariantRepair => "the features that build their respective producers",
    }
}

/// Combining existing books is not offered: manual merge, the merge answer on
/// a review card, automatic absorption and the startup clean-ups that fold
/// Works all refuse while this is false.
pub const WORK_MERGING_AVAILABLE: bool = false;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
pub enum IdentityRoadError {
    #[error("Merging is currently unavailable.")]
    MergingUnavailable,
    #[error("invalid door evidence")]
    InvalidDoorEvidence,
    #[error("stale generation")]
    StaleGeneration,
    #[error("provider boundary error")]
    ProviderBoundary,
    #[error("database error: {0}")]
    Database(String),
    #[error("review card not found")]
    NotFound,
    #[error("review kind mismatch")]
    ReviewKindMismatch,
    #[error("unauthorized scope")]
    UnauthorizedScope,
    #[error("invalid resolution")]
    InvalidResolution,
    #[error("review proposal invalidated: {0}")]
    ReviewProposalInvalidated(String),
    #[error("cancelled")]
    Cancelled,
    #[error("review required")]
    ReviewRequired,
    #[error("blocked on probe {0:?}")]
    ProbeBlocked(ProbeId),
    #[error(
        "continuation unavailable for {}; handled by {}",
        kind.storage_code(),
        continuation_owner(*kind)
    )]
    ContinuationUnavailable { kind: ReviewKind },
    #[error("Another book already has this title and author.")]
    DuplicateIdentity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
pub enum IdentityRepositoryError {
    #[error("Merging is currently unavailable.")]
    MergingUnavailable,
    #[error("Another book already has this title and author.")]
    DuplicateIdentity,
    #[error("not found")]
    NotFound,
    #[error("database error: {0}")]
    Database(String),
    #[error("stale generation")]
    StaleGeneration,
    #[error("route ownership collision")]
    RouteOwnershipCollision,
    #[error("key collision")]
    KeyCollision,
    #[error("still ambiguous")]
    StillAmbiguous,
    #[error("atomic rollback")]
    AtomicRollback,
    #[error("review kind mismatch")]
    ReviewKindMismatch,
    #[error("unauthorized scope")]
    UnauthorizedScope,
    #[error("invalid resolution")]
    InvalidResolution,
    #[error("review proposal invalidated: {0}")]
    ReviewProposalInvalidated(String),
    #[error("cancelled")]
    Cancelled,
    #[error("standing dismissal")]
    StandingDismissal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
pub enum EditionRepositoryError {
    #[error("contradictory evidence parked")]
    ContradictoryEvidenceParked,
    #[error("route ownership collision")]
    RouteOwnershipCollision,
    #[error("database error: {0}")]
    Database(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
pub enum IdentityMigrationError {
    #[error("invalid legacy fixture")]
    InvalidFixture,
    #[error("not a snapshot database")]
    NotSnapshot,
    #[error("schema mismatch")]
    SchemaMismatch,
    #[error("cancelled")]
    Cancelled,
    #[error("database error: {0}")]
    Database(String),
    #[error("rehearsal mismatch")]
    RehearsalMismatch,
    #[error("collision")]
    Collision,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
pub enum TitleParseError {
    #[error("invalid main title")]
    InvalidMainTitle,
}

// ---------------------------------------------------------------------------
// Commands and outcomes crossing the domain/persistence boundary.
// ---------------------------------------------------------------------------

/// Everything committed by the sole identity-settlement transaction.
#[derive(Debug, Clone)]
pub struct SettlementCommit {
    pub user_id: crate::UserId,
    /// `None` for a create; `Some` for a re-key of an existing Work.
    pub existing_work_id: Option<crate::WorkId>,
    /// Door-stamped birth source. Production creation roads always carry one;
    /// existing-work settlements and low-level repository fixtures carry none.
    pub add_source: Option<crate::history_events::WorkAddSource>,
    pub identity_title: IdentityTitleTuple,
    /// Explicit proof that two same-main/Author Works are textually distinct.
    /// Absence permits normal broad-group reconciliation.
    pub text_distinction: Option<String>,
    pub contributors: Vec<WorkContributor>,
    pub routes: Vec<WorkRoute>,
    /// Existing Works absorbed by this settlement into the selected/created
    /// winner. Persistence moves every dependent row atomically.
    pub absorbed_work_ids: Vec<crate::WorkId>,
    /// The coherent pre-decision generation claimed by this transaction.
    pub expected_generation: i64,
    /// Typed review-card drafts minted only if this settlement commits.
    pub review_cards: Vec<SettlementReviewCard>,
    /// Facts a creation door supplied for the new Work. Written only by the
    /// created branch, inside the same transaction as the Work row and its
    /// birth event; an existing-Work settlement ignores them.
    pub creation_facts: Option<crate::CreationFacts>,
}

/// A typed card draft carried into the sole settlement transaction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SettlementReviewCard {
    IdentityConflict {
        conflict_id: i64,
        work_id: crate::WorkId,
    },
    PendingRoute {
        work_id: crate::WorkId,
        candidate: ParkedRouteCandidate,
    },
    GroupIdentity {
        work_ids: Vec<crate::WorkId>,
        proposed_identity: Option<WorkIdentityEvidence>,
        merge_choices: Vec<crate::services::MergeFieldChoiceEntry>,
    },
    FieldResolution {
        work_id: crate::WorkId,
        evidence_ids: Vec<i64>,
    },
    ContributorOrder {
        work_id: crate::WorkId,
        contributors: Vec<WorkContributor>,
    },
    EditionEvidence {
        edition_id: EditionId,
        evidence_ids: Vec<i64>,
    },
    ImportIdentity {
        work_id: Option<crate::WorkId>,
        evidence: IdentityEvidenceBundle,
    },
    MigrationRepair {
        legacy_key: String,
        reason: String,
    },
    InvariantRepair {
        work_id: Option<crate::WorkId>,
        invariant: String,
    },
}

impl SettlementReviewCard {
    pub fn kind(&self) -> ReviewKind {
        match self {
            Self::IdentityConflict { .. } => ReviewKind::IdentityConflict,
            Self::PendingRoute { .. } => ReviewKind::PendingRoute,
            Self::GroupIdentity { .. } => ReviewKind::GroupIdentity,
            Self::FieldResolution { .. } => ReviewKind::FieldResolution,
            Self::ContributorOrder { .. } => ReviewKind::ContributorOrder,
            Self::EditionEvidence { .. } => ReviewKind::EditionEvidence,
            Self::ImportIdentity { .. } => ReviewKind::ImportIdentity,
            Self::MigrationRepair { .. } => ReviewKind::MigrationRepair,
            Self::InvariantRepair { .. } => ReviewKind::InvariantRepair,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MintedReviewCard {
    pub id: i64,
    pub kind: ReviewKind,
    pub generation: i64,
}

/// Mirrors the fully-specified `IdentityRoadOutcome::Settled` shape and
/// returns any cards atomically minted by the transaction.
#[derive(Debug, Clone)]
pub struct SettlementCommitOutcome {
    pub identity: CapturedIdentity,
    pub created: bool,
    pub audit_id: i64,
    pub review_cards: Vec<MintedReviewCard>,
}

/// A recoverable, user-scoped pending card loaded from the TEXT payload as
/// typed JSON. `card_id` is deliberately independent from any conflict id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingReviewCard {
    pub id: i64,
    pub user_id: crate::UserId,
    pub work_id: Option<crate::WorkId>,
    /// Current user-facing Work presentation. These fields are joined at read
    /// time so cards minted before the presentation contract remain useful.
    pub work_title: Option<String>,
    pub work_author: Option<String>,
    pub kind: ReviewKind,
    pub generation: i64,
    pub payload: SettlementReviewCard,
}

/// Result of the repository's single claimed review continuation.
#[derive(Debug, Clone)]
pub struct ReviewContinuationOutcome {
    pub card_id: i64,
    pub kind: ReviewKind,
    pub generation: i64,
    pub audit_id: i64,
    pub identity: Option<CapturedIdentity>,
    pub library_items_moved: usize,
    pub grabs_moved: usize,
}

/// Direct format/language evidence for one user-scoped Edition.
#[derive(Debug, Clone)]
pub struct EditionEvidenceCommand {
    pub user_id: crate::UserId,
    pub edition_id: EditionId,
    pub format: Option<super::edition::EditionFormat>,
    pub language: Option<String>,
    pub provenance: super::shared::EvidenceProvenance,
}

/// Door-facing edition evidence names the owning Work; persistence selects or
/// creates the Edition and applies the evidence in one transaction.
#[derive(Debug, Clone)]
pub struct EditionWorkEvidenceCommand {
    pub user_id: crate::UserId,
    pub work_id: crate::WorkId,
    pub format: super::edition::EditionFormat,
    pub language: Option<String>,
    pub provenance: super::shared::EvidenceProvenance,
}

#[derive(Debug, Clone)]
pub struct EditionEvidenceOutcome {
    pub edition: Edition,
}

/// A copied real-library SQLite file, per `migration_plan.rehearsal`
/// ("Copy the PO's real SQLite library...") and the CLI's
/// `identity-cutover rehearse --snapshot <copied-db>`.
#[derive(Debug, Clone)]
pub struct SnapshotDatabase {
    pub path: std::path::PathBuf,
}

/// Internal (non-wire) command for a minimum-only ManualImport settlement.
/// Carries no path and no provider-wire fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualImportMinimumCommand {
    pub user_id: crate::UserId,
    pub title: String,
    pub author: String,
    pub owned_file: OwnedFileEvidence,
    pub author_route: Option<crate::AuthorRouteKey>,
    pub request_start_work_hint: Option<crate::WorkId>,
}

/// Fresh provider identities produced by one metadata pass against a coherent
/// identity generation. Persisted routes are never copied into this value:
/// the road receives only observations that were absent from the snapshot the
/// pass planned from.
#[derive(Debug, Clone)]
pub struct CapturedRouteHandoff {
    pub metadata_generation: i64,
    pub provider_identity: Vec<ProviderIdentityEvidence>,
    /// Decisive REQ-027 picks without corroborating edition evidence. The road
    /// alone turns these into generation-neutral PendingRoute cards.
    pub route_proposals: Vec<super::shared::RouteKey>,
}

// ---------------------------------------------------------------------------
// Traits — `trait_variant::make(Send)`, generic/enum dispatch only (FP-029).
// ---------------------------------------------------------------------------

/// The one production road entry for all six Work-creation doors and every
/// re-key continuation (`architecture_decisions.identity_road_chokepoint`).
#[trait_variant::make(Send)]
pub trait IdentityRoadService: Send + Sync {
    async fn settle(
        &self,
        request: IdentityRoadRequest,
    ) -> Result<IdentityRoadOutcome, IdentityRoadError>;

    /// [`Self::settle`] for a creation door that also supplies the facts the
    /// selected result carried. Production roads persist them in the created
    /// branch of the same settlement transaction. A road without that
    /// persistence refuses rather than silently dropping the facts.
    fn settle_creation(
        &self,
        request: IdentityRoadRequest,
        facts: crate::CreationFacts,
    ) -> impl std::future::Future<Output = Result<IdentityRoadOutcome, IdentityRoadError>> + Send
    {
        let _ = (request, facts);
        async move {
            Err(IdentityRoadError::Database(
                "this identity road cannot persist creation facts".to_string(),
            ))
        }
    }

    /// [`Self::settle`] for a user's title/author edit of an existing Work,
    /// carrying what the edit observed. Production roads claim the observed
    /// generation and keep the stored title when the user left it unchanged.
    /// A road without that support refuses rather than settling unclaimed.
    fn settle_work_edit(
        &self,
        request: IdentityRoadRequest,
        claim: WorkEditClaim,
    ) -> impl std::future::Future<Output = Result<IdentityRoadOutcome, IdentityRoadError>> + Send
    {
        let _ = (request, claim);
        async move {
            Err(IdentityRoadError::Database(
                "this identity road cannot settle a Work edit".to_string(),
            ))
        }
    }

    async fn resolve_review(
        &self,
        actor: ReviewActor,
        command: ReviewResolutionCommand,
    ) -> Result<IdentityRoadOutcome, IdentityRoadError>;

    async fn settle_manual_import_minimum(
        &self,
        command: ManualImportMinimumCommand,
    ) -> Result<IdentityRoadOutcome, IdentityRoadError>;

    /// The single machine-observation handoff authority. Concrete production
    /// roads override this to validate `metadata_generation` against their
    /// repository before settlement. The default keeps test/decorator roads
    /// source-compatible while preserving the fresh-only/no-empty-call shape.
    fn apply_captured_route_handoff(
        &self,
        user_id: crate::UserId,
        work_id: crate::WorkId,
        trigger: IdentityRoadOrigin,
        handoff: CapturedRouteHandoff,
    ) -> impl std::future::Future<Output = Result<Option<IdentityRoadOutcome>, IdentityRoadError>> + Send
    {
        async move {
            if !matches!(
                trigger,
                IdentityRoadOrigin::EnrichmentPass
                    | IdentityRoadOrigin::ManualRefresh
                    | IdentityRoadOrigin::ConvergenceVisit
            ) || user_id <= 0
                || work_id <= 0
            {
                return Err(IdentityRoadError::InvalidDoorEvidence);
            }
            if handoff.provider_identity.is_empty() && handoff.route_proposals.is_empty() {
                return Ok(None);
            }
            if !handoff.route_proposals.is_empty() {
                return Err(IdentityRoadError::InvalidDoorEvidence);
            }
            self.settle(IdentityRoadRequest {
                user_id,
                origin: trigger,
                evidence: IdentityEvidenceBundle {
                    user_choice: None,
                    owned_files: Vec::new(),
                    provider_identity: handoff.provider_identity,
                    minimum: None,
                },
                interaction: IdentityRoadInteraction::MachineAlone,
                existing_work_id: Some(work_id),
            })
            .await
            .map(Some)
        }
    }
}

/// NEW shadow trait — see module header. Not the existing 400+ line
/// anchor-confirmation `WorkIdentityRepository`.
#[trait_variant::make(Send)]
pub trait WorkIdentityRepository: Send + Sync {
    async fn read_captured_identity(
        &self,
        user_id: crate::UserId,
        work_id: crate::WorkId,
    ) -> Result<CapturedIdentity, IdentityRepositoryError>;

    /// Batch route-authoritative projection used by every Work-bearing
    /// presentation surface.
    async fn read_identity_presentations(
        &self,
        user_id: crate::UserId,
        work_ids: &[crate::WorkId],
    ) -> Result<Vec<WorkIdentityPresentation>, IdentityRepositoryError>;

    /// Coherent identities in one broad normalized-main + primary-author
    /// reconciliation group. The road needs this read to evaluate every pair
    /// before choosing one settlement.
    async fn list_captured_identities_in_group(
        &self,
        user_id: crate::UserId,
        normalized_main: String,
        primary_author_id: crate::AuthorId,
    ) -> Result<Vec<CapturedIdentity>, IdentityRepositoryError>;

    /// Current display name plus retained variants used by the F1 author-route
    /// name guard. Names remain persistence data; the road receives no SQL
    /// capability.
    async fn read_primary_author_names(
        &self,
        user_id: crate::UserId,
        author_id: crate::AuthorId,
    ) -> Result<Vec<String>, IdentityRepositoryError>;

    async fn commit_settlement(
        &self,
        command: SettlementCommit,
    ) -> Result<SettlementCommitOutcome, IdentityRepositoryError>;

    /// Compatibility `commit_settlement` plus the road's exact origin and the
    /// already-validated explicit-choice fact. Live road settlement must use
    /// this; low-level callers may keep `commit_settlement`.
    fn commit_settlement_with_review_context(
        &self,
        command: SettlementCommit,
        origin: IdentityRoadOrigin,
        validated_explicit_choice: bool,
    ) -> impl std::future::Future<Output = Result<SettlementCommitOutcome, IdentityRepositoryError>> + Send
    {
        async move {
            let _ = (origin, validated_explicit_choice);
            self.commit_settlement(command).await
        }
    }

    /// One `BEGIN IMMEDIATE` transaction owning hint revalidation, Author
    /// resolution, complete-group reads, the one decision, and commit/rollback.
    fn settle_manual_import_minimum(
        &self,
        _command: ManualImportMinimumCommand,
    ) -> impl std::future::Future<Output = Result<IdentityRoadOutcome, IdentityRepositoryError>> + Send
    {
        async move {
            Err(IdentityRepositoryError::Database(
                "manual-import minimum settlement is not implemented".to_string(),
            ))
        }
    }

    /// Generation-checked, identity-neutral origination for a machine search
    /// proposal. Equivalent pending routes reuse the oldest durable card.
    fn commit_pending_route_review(
        &self,
        _user_id: crate::UserId,
        _work_id: crate::WorkId,
        _expected_generation: i64,
        _candidate: ParkedRouteCandidate,
    ) -> impl std::future::Future<Output = Result<MintedReviewCard, IdentityRepositoryError>> + Send
    {
        async move {
            Err(IdentityRepositoryError::Database(
                "pending-route review persistence is not implemented".to_string(),
            ))
        }
    }

    /// Compatibility `commit_pending_route_review` plus the road's exact
    /// origin and already-validated explicit-choice fact. Live captured-route
    /// persistence must use this; low-level callers may keep the unadorned
    /// method.
    fn commit_pending_route_review_with_review_context(
        &self,
        user_id: crate::UserId,
        work_id: crate::WorkId,
        expected_generation: i64,
        candidate: ParkedRouteCandidate,
        origin: IdentityRoadOrigin,
        validated_explicit_choice: bool,
    ) -> impl std::future::Future<Output = Result<MintedReviewCard, IdentityRepositoryError>> + Send
    {
        async move {
            let _ = (origin, validated_explicit_choice);
            self.commit_pending_route_review(user_id, work_id, expected_generation, candidate)
                .await
        }
    }

    /// Load one pending typed card in the actor's admitted scope.
    async fn load_pending_review(
        &self,
        actor: ReviewActor,
        card_id: i64,
    ) -> Result<PendingReviewCard, IdentityRepositoryError>;

    /// List every pending typed card admitted by the actor's scope. This is
    /// the durable presentation seam for interactive 202 responses.
    async fn list_pending_reviews(
        &self,
        actor: ReviewActor,
    ) -> Result<Vec<PendingReviewCard>, IdentityRepositoryError>;

    /// Dismiss one pending card with an actor audit in the same transaction.
    /// Dismissal deliberately does not mutate Work identity generation.
    async fn dismiss_pending_review(
        &self,
        actor: ReviewActor,
        card_id: i64,
    ) -> Result<(), IdentityRepositoryError>;

    /// Resolve a legacy conflict id to its distinct pending card. A conflict
    /// id is never accepted as though it were the review-card id.
    async fn load_pending_conflict_review(
        &self,
        actor: ReviewActor,
        conflict_id: i64,
    ) -> Result<PendingReviewCard, IdentityRepositoryError>;

    /// Revalidate and commit one review action, actor audit, card resolution,
    /// and generation claim atomically. Cancellation leaves the card pending.
    async fn commit_review_continuation(
        &self,
        actor: ReviewActor,
        command: ReviewResolutionCommand,
        cancel: CancellationToken,
    ) -> Result<ReviewContinuationOutcome, IdentityRepositoryError>;
}

#[trait_variant::make(Send)]
pub trait EditionRepository: Send + Sync {
    async fn apply_evidence(
        &self,
        command: EditionEvidenceCommand,
    ) -> Result<EditionEvidenceOutcome, EditionRepositoryError>;

    async fn apply_work_evidence(
        &self,
        command: EditionWorkEvidenceCommand,
    ) -> Result<EditionEvidenceOutcome, EditionRepositoryError>;
}

#[trait_variant::make(Send)]
pub trait IdentityCutoverService: Send + Sync {
    async fn rehearse(
        &self,
        snapshot: SnapshotDatabase,
        cancel: CancellationToken,
    ) -> Result<IdentityMigrationReport, IdentityMigrationError>;

    async fn apply(
        &self,
        approved_report: IdentityMigrationReport,
        cancel: CancellationToken,
    ) -> Result<IdentityMigrationReport, IdentityMigrationError>;

    async fn ensure_authority_ready(
        &self,
        cancel: CancellationToken,
    ) -> Result<IdentityAuthorityReadiness, IdentityMigrationError>;
}

// ---------------------------------------------------------------------------
// Free deterministic functions.
// ---------------------------------------------------------------------------

/// Exhaustive continuation availability: PendingRoute and GroupIdentity have
/// continuations; the other kinds are refused by name.
pub fn require_continuation(kind: ReviewKind) -> Result<(), IdentityRoadError> {
    match kind {
        ReviewKind::PendingRoute | ReviewKind::GroupIdentity => Ok(()),
        ReviewKind::IdentityConflict
        | ReviewKind::FieldResolution
        | ReviewKind::ContributorOrder
        | ReviewKind::EditionEvidence
        | ReviewKind::ImportIdentity
        | ReviewKind::MigrationRepair
        | ReviewKind::InvariantRepair => Err(IdentityRoadError::ContinuationUnavailable { kind }),
    }
}

/// The merge answer on a GroupIdentity card combines existing books, which is
/// not offered while `WORK_MERGING_AVAILABLE` is false. Every other answer
/// proceeds to its continuation. Checked before any generation or write.
pub fn require_resolution_offered(
    command: &ReviewResolutionCommand,
) -> Result<(), IdentityRoadError> {
    let merge_answer = matches!(
        command,
        ReviewResolutionCommand::GroupIdentity {
            action: super::review::GroupIdentityAction::AttachOrMerge { .. },
            ..
        }
    );
    if merge_answer && !WORK_MERGING_AVAILABLE {
        return Err(IdentityRoadError::MergingUnavailable);
    }
    Ok(())
}

/// Split provider display text into the immutable identity tuple.
pub fn title_parts_from_provider(
    provider_title: String,
    structured_subtitle: Option<String>,
) -> Result<IdentityTitleTuple, TitleParseError> {
    let parsed = crate::identity_matching::parse_title(&provider_title);
    if parsed.main.is_empty() {
        return Err(TitleParseError::InvalidMainTitle);
    }

    let normalized_input = crate::title_cleanup::collapse_whitespace(&provider_title);
    // ParsedTitle exposes canonical text, so display casing is preserved from
    // the collapsed provider value up to its first recognized subtitle tail.
    let provider_main = normalized_input
        .split_once(':')
        .map_or(normalized_input.as_str(), |(main, _)| main)
        .trim();
    let display_main = strip_extracted_trailing_parentheticals(provider_main).to_string();
    let subtitle = structured_subtitle
        .map(|value| crate::title_cleanup::collapse_whitespace(&value))
        .filter(|value| !value.is_empty())
        .or_else(|| {
            normalized_input
                .split_once(':')
                .map(|(_, tail)| crate::title_cleanup::collapse_whitespace(tail))
                .filter(|value| !value.is_empty())
        })
        .or_else(|| parsed.subtitle.clone());
    let volume = parsed
        .series_markers
        .first()
        .map(|marker| marker.number.to_string());
    let normalized_main =
        crate::identity_matching::strip_leading_identity_article(&display_main.to_lowercase())
            .to_string();

    Ok(IdentityTitleTuple {
        main: display_main,
        subtitle: subtitle.clone(),
        volume: volume.clone(),
        normalized_main,
        normalized_subtitle: subtitle.as_deref().unwrap_or_default().to_lowercase(),
        normalized_volume: volume.unwrap_or_default(),
        provenance: super::shared::EvidenceProvenance::Provider(IdentityProvider::Other(
            "provider".to_string(),
        )),
    })
}

/// Keep display main aligned with `identity_matching::parse_title`: every
/// trailing parenthetical that parser extracts into series/volume, subtitle,
/// or junk belongs outside the immutable main. This deliberately does not
/// remove interior parentheses that the parser did not extract.
fn strip_extracted_trailing_parentheticals(mut provider_main: &str) -> &str {
    while let Some(captures) = crate::title_cleanup::RE_TRAILING_PAREN.captures(provider_main) {
        let Some(extracted) = captures.get(0) else {
            break;
        };
        provider_main = provider_main[..extracted.start()].trim_end();
    }
    provider_main
}

/// Parse a stored identity tuple through the shared stored-title parser, so
/// subtitle agreement is judged by the same policy as a provider title
/// "Main: Subtitle". The stored volume stays a separate positional input to
/// the verdict.
fn parse_identity_title(title: &IdentityTitleTuple) -> crate::identity_matching::ParsedTitle {
    crate::identity_matching::parse_stored_title(&title.main, title.subtitle.as_deref())
}

pub fn evaluate_match(
    left: WorkIdentityEvidence,
    right: WorkIdentityEvidence,
    lost_match: LostMatchGuardSet,
    wrong_merge: WrongMergeGuardSet,
) -> DirectionalMatchVerdicts {
    use crate::identity_matching::{
        AuthorVerdict, GreyCause, IdVerdict, LanguageVerdict, TitleVerdict,
    };

    let left_title = parse_identity_title(&left.title);
    let right_title = parse_identity_title(&right.title);
    let mut title = crate::identity_matching::title_verdict_with_positions(
        &left_title,
        left.title
            .volume
            .as_deref()
            .and_then(|value| value.parse().ok()),
        &right_title,
        right
            .title
            .volume
            .as_deref()
            .and_then(|value| value.parse().ok()),
    );
    if !wrong_merge.main_title_guard.0 && matches!(title, TitleVerdict::Different) {
        title = TitleVerdict::Grey {
            score: 0.0,
            cause: GreyCause::NearMain,
        };
    }
    if !wrong_merge.volume_conflict_guard && matches!(title, TitleVerdict::VetoVolume) {
        title = TitleVerdict::Grey {
            score: 1.0,
            cause: GreyCause::VolumeAsymmetry,
        };
    }
    if lost_match.one_sided_subtitle_recovery
        && matches!(
            title,
            TitleVerdict::Grey {
                cause: GreyCause::OneSidedSubtitle,
                ..
            }
        )
    {
        title = TitleVerdict::Same;
    }

    let author = if left.primary_author_id == right.primary_author_id {
        AuthorVerdict::Agree
    } else if wrong_merge.author_disagreement_guard {
        AuthorVerdict::Disagree
    } else {
        AuthorVerdict::Grey
    };

    let left_active: Vec<_> = left
        .routes
        .iter()
        .filter(|route| matches!(route.state, WorkRouteState::Active))
        .collect();
    let right_active: Vec<_> = right
        .routes
        .iter()
        .filter(|route| matches!(route.state, WorkRouteState::Active))
        .collect();
    let same_work_route = left_active.iter().any(|left_route| {
        is_work_kind(&left_route.kind)
            && right_active.iter().any(|right_route| {
                left_route.provider == right_route.provider
                    && left_route.kind == right_route.kind
                    && left_route.provider_scoped_id == right_route.provider_scoped_id
            })
    });
    let contradictory_work_route = left_active.iter().any(|left_route| {
        is_work_kind(&left_route.kind)
            && right_active.iter().any(|right_route| {
                left_route.provider == right_route.provider
                    && left_route.kind == right_route.kind
                    && left_route.provider_scoped_id != right_route.provider_scoped_id
            })
    });
    let shared_edition_route = left_active.iter().any(|left_route| {
        is_edition_kind(&left_route.kind)
            && right_active.iter().any(|right_route| {
                left_route.provider == right_route.provider
                    && left_route.kind == right_route.kind
                    && left_route.provider_scoped_id == right_route.provider_scoped_id
            })
    });
    // A same-provider Work-id disagreement is judged before any shared id:
    // one matching catalog must not hide another catalog's contradiction.
    let id = if contradictory_work_route && wrong_merge.work_key_contradiction_guard {
        IdVerdict::WorkKeyContradiction
    } else if same_work_route {
        IdVerdict::WorkKeyEqual
    } else if shared_edition_route && lost_match.shared_edition_id_confirmation {
        IdVerdict::EditionBridge
    } else {
        IdVerdict::NoEvidence
    };

    DirectionalMatchVerdicts {
        title,
        author,
        language: LanguageVerdict::Neutral,
        id,
    }
}

pub fn select_machine_subtitle(
    user_id: crate::UserId,
    work_id: crate::WorkId,
    mut editions: Vec<Edition>,
    defaults: Vec<DefaultEdition>,
) -> MachineSubtitleProjection {
    editions.retain(|edition| {
        edition.user_id == user_id
            && edition.work_id == work_id
            && !matches!(edition.format, EditionFormat::Unknown)
            && matches!(edition.state, super::edition::EditionState::Active)
    });
    editions.sort_by(|left, right| {
        let left_default = defaults.iter().any(|default| {
            default.user_id == user_id
                && default.work_id == work_id
                && default.edition_id == left.id
                && default.format == left.format
        });
        let right_default = defaults.iter().any(|default| {
            default.user_id == user_id
                && default.work_id == work_id
                && default.edition_id == right.id
                && default.format == right.format
        });
        (
            !left_default,
            edition_format_rank(&left.format),
            provider_rank(left),
        )
            .cmp(&(
                !right_default,
                edition_format_rank(&right.format),
                provider_rank(right),
            ))
            .then_with(|| left.provider_edition_id.cmp(&right.provider_edition_id))
            .then_with(|| left.id.cmp(&right.id))
    });
    let selected = editions.first();
    MachineSubtitleProjection {
        user_id,
        work_id,
        value: selected
            .and_then(|edition| edition.subtitle.as_ref().map(|value| value.value.clone())),
        edition_id: selected.map(|edition| edition.id),
        provenance: selected.and_then(|edition| {
            edition
                .subtitle
                .as_ref()
                .map(|value| value.provenance.clone())
        }),
        // Persistence replaces this neutral value with the claimed generation.
        computed_at_generation: 0,
    }
}

pub fn select_covers_and_placeholders(
    identity: CapturedIdentity,
    _editions: Vec<Edition>,
    candidates: Vec<crate::CoverCandidate>,
) -> WorkCoverPresentation {
    let mut ebook: Vec<_> = candidates
        .iter()
        .filter(|candidate| candidate.media_type == crate::CoverMediaType::Ebook)
        .cloned()
        .collect();
    let mut audiobook: Vec<_> = candidates
        .iter()
        .filter(|candidate| candidate.media_type == crate::CoverMediaType::Audiobook)
        .cloned()
        .collect();
    ebook.sort_by_key(cover_rank);
    audiobook.sort_by_key(cover_rank);

    let ebook_selected = ebook.first().cloned();
    let audiobook_selected = audiobook.first().cloned();
    let has_usable_route = identity.active_routes.iter().any(|route| {
        matches!(route.state, WorkRouteState::Active)
            && !matches!(route.kind, RouteKind::Undeclared { .. })
    });
    let placeholder = || {
        Some(if has_usable_route {
            CoverPlaceholderState::Searching
        } else {
            CoverPlaceholderState::NowhereToLook
        })
    };
    let uncovered = ebook_selected.is_none() || audiobook_selected.is_none();

    WorkCoverPresentation {
        format_needed: uncovered.then(|| CoverPlaceholderState::FormatNeeded {
            candidates: candidates.clone(),
        }),
        ebook: CoverSlotPresentation {
            placeholder: ebook_selected.is_none().then(placeholder).flatten(),
            selected: ebook_selected,
        },
        audiobook: CoverSlotPresentation {
            placeholder: audiobook_selected.is_none().then(placeholder).flatten(),
            selected: audiobook_selected,
        },
    }
}

fn is_work_kind(kind: &RouteKind) -> bool {
    matches!(
        kind,
        RouteKind::OpenLibraryWork | RouteKind::GoodreadsWork | RouteKind::HardcoverWork
    )
}

fn is_edition_kind(kind: &RouteKind) -> bool {
    matches!(
        kind,
        RouteKind::Isbn13Edition | RouteKind::AsinEdition | RouteKind::GoodreadsBookEdition
    )
}

fn edition_format_rank(format: &EditionFormat) -> (u8, &str) {
    match format {
        EditionFormat::Ebook => (0, ""),
        EditionFormat::Audiobook => (1, ""),
        EditionFormat::Physical => (2, ""),
        EditionFormat::Other(name) => (3, name),
        EditionFormat::Unknown => (4, ""),
    }
}

fn provider_rank(edition: &Edition) -> (u8, String) {
    let rank = match edition.source_provider.as_ref() {
        None => 8,
        Some(IdentityProvider::Hardcover) => 1,
        Some(IdentityProvider::OpenLibrary) => 2,
        Some(IdentityProvider::Goodreads) => 3,
        Some(IdentityProvider::Other(name)) if name == "google_books" => 4,
        Some(IdentityProvider::Other(name)) if name == "audible" => 5,
        Some(IdentityProvider::Other(name)) if name == "audnexus" => 6,
        Some(IdentityProvider::Other(_)) => 7,
        Some(IdentityProvider::IsbnRegistry | IdentityProvider::Amazon) => 8,
    };
    let name = edition
        .source_provider
        .as_ref()
        .map(|provider| format!("{provider:?}"))
        .unwrap_or_default();
    (rank, name)
}

fn cover_rank(candidate: &crate::CoverCandidate) -> (u8, String) {
    let source = candidate.source.to_ascii_lowercase();
    let rank = match source.as_str() {
        "user" | "yours" => 0,
        "owned_file" | "your_file" | "your-file" => 1,
        _ => 3,
    };
    (rank, candidate.candidate_id.clone())
}
