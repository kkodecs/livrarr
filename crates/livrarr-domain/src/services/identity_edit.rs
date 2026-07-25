//! Identity-edit service/repository contract types (design identity-edit r4).
//!
//! The domain leaf never names the enrichment-layer provider payload: a
//! preview fetch surfaces here as [`IdentityPreviewRecord`], mapped from the
//! provider payload by the metadata-crate workflow adapter.

use crate::identity::{
    AnchorSetter, AnchorType, Candidate, CapturedIdentity, IdentityConflictKind,
    NewIdentityConflict,
};
use crate::{IdentityStatus, Work, WorkId};

/// The certified record a preview fetch resolved — what the user is asked to
/// certify. Domain-owned; carries display fields plus the canonical identity
/// fields the assessment verdicts run on.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IdentityPreviewRecord {
    pub title: Option<String>,
    pub author: Option<String>,
    pub year: Option<i32>,
    pub language: Option<String>,
    pub cover_url: Option<String>,
    pub ol_key: Option<String>,
    pub gr_key: Option<String>,
    pub hc_key: Option<String>,
    pub isbn_13: Option<String>,
    pub asin: Option<String>,
}

/// Domain-level outcome of one preview fetch leg.
#[derive(Debug, Clone)]
pub enum IdentityPreviewOutcome {
    Resolved(Box<IdentityPreviewRecord>),
    NotFound,
    NotConfigured,
    /// Retryable outage or permanent fetch failure — nothing certifiable.
    Unavailable,
}

/// keep / drop verdict for one sibling work-key slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiblingAction {
    Keep,
    Drop,
}

/// Assessment of one sibling work-key slot against the certified record.
#[derive(Debug, Clone)]
pub struct SiblingAssessment {
    pub slot: AnchorType,
    pub action: SiblingAction,
    /// Drop cause: `disagrees` / `unproven` / `unverifiable`; `None` for keeps.
    pub cause: Option<String>,
}

/// Informational bridge warning — bridges are never auto-dropped.
#[derive(Debug, Clone)]
pub struct BridgeWarning {
    pub slot: AnchorType,
    pub message: String,
}

/// Same-user owner of the pasted identifier — Confirm is disabled; the UI
/// offers Merge works.
#[derive(Debug, Clone)]
pub struct CollisionInfo {
    pub owning_work_id: WorkId,
    pub owning_work_title: String,
}

/// Result of `WorkService::preview_identity_edit`.
#[derive(Debug, Clone, Default)]
pub struct IdentityEditPreview {
    pub resolved: Option<IdentityPreviewRecord>,
    pub slot: Option<AnchorType>,
    pub canonical_value: Option<String>,
    /// Present only when certifiable (resolved, no collision).
    pub preview_id: Option<String>,
    pub siblings: Vec<SiblingAssessment>,
    pub bridge_warnings: Vec<BridgeWarning>,
    pub collision: Option<CollisionInfo>,
    /// Open identity conflicts exist — enrichment stays paused until reviewed.
    pub conflict_warning: bool,
    /// Why `resolved` is absent (provider outage / not found).
    pub failure_reason: Option<String>,
}

/// Result of `WorkService::commit_identity_edit`.
#[derive(Debug)]
pub struct IdentityEditCommit {
    pub work: Work,
    /// True no-op: the token was consumed but nothing was written — no
    /// history, no refresh.
    pub no_op: bool,
    pub old_value: Option<String>,
    pub new_value: String,
}

/// Result of `WorkService::clear_identity_slot`.
#[derive(Debug)]
pub struct IdentityEditClear {
    pub work: Work,
    /// Display form of what was cleared (confirmed/column value, or the
    /// latent pending guess for a pending-only slot).
    pub old_value: String,
    /// Open conflicts keep re-matching paused; the caller skips the refresh
    /// spawn and the UI says so.
    pub parked_by_conflicts: bool,
}

/// What the repository-level clear transaction removed.
#[derive(Debug)]
pub struct ClearedSlot {
    pub old_value: String,
    pub parked_by_conflicts: bool,
}

/// Per-slot half of [`IdentityEditBasis`]: the validated ledger∪column view.
#[derive(Debug, Clone, Default)]
pub struct IdentitySlotBasis {
    /// Confirmed ledger row, if any.
    pub confirmed: Option<(String, AnchorSetter)>,
    /// Raw works column value, if nonempty.
    pub column: Option<String>,
    /// Whether `column` passes the slot's canonical normalizer (a quarantined
    /// invalid value stays visible/clearable but earns no badge or collision).
    pub column_valid: bool,
    /// Pending (unaffirmed) guesses for the slot.
    pub pending: Vec<String>,
    pub dead_end: bool,
}

impl IdentitySlotBasis {
    /// The slot's effective identity value: confirmed ledger row, else the
    /// validated column (ground truth 6b — column-only legacy works are real).
    pub fn effective(&self) -> Option<&str> {
        self.confirmed
            .as_ref()
            .map(|(v, _)| v.as_str())
            .or_else(|| {
                self.column_valid
                    .then_some(self.column.as_deref())
                    .flatten()
            })
    }
}

/// One coherent user-scoped snapshot of a work's identity state — the basis
/// for preview assessment and the commit-time no-op check. Read in a single
/// repository transaction; `generation` is the staleness authority.
#[derive(Debug, Clone)]
pub struct IdentityEditBasis {
    pub generation: i64,
    pub ol_work: IdentitySlotBasis,
    pub gr_work: IdentitySlotBasis,
    pub hc_work: IdentitySlotBasis,
    pub isbn_13: IdentitySlotBasis,
    pub asin: IdentitySlotBasis,
    pub open_conflict_kinds: Vec<IdentityConflictKind>,
    pub stored_badge: IdentityStatus,
    /// Badge the shared ledger∪column derivation computes right now.
    pub derived_badge: IdentityStatus,
}

impl IdentityEditBasis {
    pub fn slot(&self, anchor_type: &AnchorType) -> &IdentitySlotBasis {
        match anchor_type.as_str() {
            AnchorType::OL_WORK => &self.ol_work,
            AnchorType::GR_WORK => &self.gr_work,
            AnchorType::HC_WORK => &self.hc_work,
            AnchorType::ISBN_13 => &self.isbn_13,
            _ => &self.asin,
        }
    }
}

/// One delayed-completion write set, applied under a single first-statement
/// generation claim (design §Claims and delayed completion). Every field is
/// optional; a completion applies exactly what it carries.
#[derive(Debug, Default)]
pub struct IdentityCompletion {
    /// Fill anchor types the work lacks (never overwrites confirmed rows).
    pub merge_anchors: Option<CapturedIdentity>,
    /// Monotonic badge raise decided by the identity engine, if any.
    pub target_badge: Option<IdentityStatus>,
    /// Fuzzy guesses held for user affirmation (ledger-only, no column sync).
    pub pending_guesses: Vec<(AnchorType, String)>,
    /// Park the work `NeedsReview` with these ranked candidates.
    pub review_candidates: Option<Vec<Candidate>>,
    /// Conflicts to raise (dedup + badge handled in the same transaction).
    pub conflicts: Vec<NewIdentityConflict>,
}

/// Outcome of a claimed completion: `Superseded` means the generation claim
/// found zero rows — a newer identity mutation won; nothing was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityCompletionOutcome {
    Applied { anchors_merged: Vec<AnchorType> },
    Superseded,
}
