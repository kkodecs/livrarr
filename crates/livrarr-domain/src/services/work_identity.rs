use crate::identity::*;
use crate::{Work, WorkId};

#[derive(Debug, thiserror::Error)]
pub enum WorkIdentityError {
    #[error("invalid anchor value")]
    InvalidAnchorValue,
    #[error("anchor not found")]
    AnchorNotFound,
    #[error("seed carried no usable identity signal")]
    EmptySeed,
    #[error("work is not parked for review")]
    NotParked,
    #[error("database error: {0}")]
    Db(String),
}

/// Outcome of refresh-time identity anchor completion (REQ-008): what the
/// identity track newly established, and which providers were skipped and why.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnchorCompletionReport {
    /// (anchor_type, value) newly established via the identity track.
    pub resolved: Vec<(String, String)>,
    /// (provider, reason) — reason is one of "suppressed", "not_found",
    /// "unresolvable".
    pub skipped: Vec<(String, String)>,
}

#[trait_variant::make(Send)]
pub trait WorkIdentityRepository: Send + Sync {
    async fn confirm_anchor(
        &self,
        work_id: WorkId,
        anchor_type: AnchorType,
        value: &str,
        setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError>;

    /// Confirm a pending anchor, then atomically derive and persist the
    /// `identity_status` badge from the updated anchor set (M-020). The badge
    /// is written in the same transaction as the anchor promotion, so the UI
    /// reflects the correct status immediately without waiting for background
    /// refresh. Called exclusively from the user-affirm path.
    async fn confirm_anchor_and_recompute_badge(
        &self,
        work_id: WorkId,
        anchor_type: AnchorType,
        value: &str,
        setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError>;

    async fn supersede_anchor(
        &self,
        work_id: WorkId,
        anchor_type: AnchorType,
        old_value: &str,
        new_value: &str,
        setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError>;

    async fn set_identity_pending(
        &self,
        work_id: WorkId,
        reason: PendingReason,
        setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError>;

    /// Surface a Tier-B identity-pending work the resolver cannot deterministically
    /// match as needs-review — never an indefinite background-retry loop (REQ-026).
    async fn set_needs_review(&self, work_id: WorkId) -> Result<(), WorkIdentityError>;

    /// Persist the ranked candidates behind a `NeedsReview` park (REQ-010),
    /// replacing any prior set for the same work. Queryable per work by the
    /// review surface; never touches the identity badge itself.
    async fn record_review_candidates(
        &self,
        work_id: WorkId,
        candidates: &[Candidate],
    ) -> Result<(), WorkIdentityError>;

    /// The candidates recorded behind a work's current park, if any.
    /// `None` when the work was never parked with a candidate set to show.
    async fn get_review_candidates(
        &self,
        work_id: WorkId,
    ) -> Result<Option<Vec<Candidate>>, WorkIdentityError>;

    /// List every work parked `NeedsReview` for a user (AC-013 review surface).
    /// Ordered by id; a work with no recorded candidate set (should not occur
    /// in practice) is still listed — the caller pairs each with
    /// [`Self::get_review_candidates`].
    async fn list_needs_review_works(
        &self,
        user_id: crate::UserId,
    ) -> Result<Vec<Work>, WorkIdentityError>;

    /// Apply a user-picked candidate from a `NeedsReview` park (AC-013): confirm
    /// every anchor the candidate carries (setter attributes who chose it),
    /// atomically recompute and write the resulting `identity_status` badge from
    /// the updated anchor set, and clear the park's recorded candidate row.
    /// Mirrors [`Self::confirm_anchor_and_recompute_badge`]'s one-transaction
    /// contract, generalized to a candidate's full anchor set rather than one
    /// (type, value) pair.
    ///
    /// Guarded: the transaction's first statement atomically verifies the work
    /// is currently `NeedsReview` and claims it. A settled work — even one
    /// holding a stale candidates row — returns [`WorkIdentityError::NotParked`]
    /// with zero writes; a concurrent resolve/dismiss loses the claim the same
    /// way, so the read-candidates-then-apply window cannot double-apply.
    async fn apply_review_candidate(
        &self,
        work_id: WorkId,
        candidate: &Candidate,
        setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError>;

    /// Dismiss a `NeedsReview` park without adopting any candidate: the work
    /// reverts to `Pending` (no merge, no anchor writes) and its recorded
    /// candidate row is cleared. A duplicate one click away from the
    /// merge-two-works action, per AC-013's dismiss semantics.
    ///
    /// Guarded like [`Self::apply_review_candidate`]: only a work currently
    /// `NeedsReview` can be dismissed — a settled work (Confirmed/Provisional/
    /// Conflict) is never downgraded to Pending by this call; it returns
    /// [`WorkIdentityError::NotParked`] untouched.
    async fn dismiss_review(&self, work_id: WorkId) -> Result<(), WorkIdentityError>;

    /// Raise the badge to `Confirmed` (REQ-003/008) — a work anchor (OL/GR/HC)
    /// fixed the identity. Writes only the `identity_status` column for the one
    /// work; the engine calls it solely on a monotonic raise.
    async fn set_identity_confirmed(&self, work_id: WorkId) -> Result<(), WorkIdentityError>;

    /// Raise the badge to `Provisional` (REQ-003/008) — an ISBN/ASIN bridge with
    /// no work anchor yet. Writes only the `identity_status` column for the one work.
    async fn set_identity_provisional(&self, work_id: WorkId) -> Result<(), WorkIdentityError>;

    async fn verify_anchor_cache_consistency(
        &self,
    ) -> Result<Vec<ConsistencyDivergence>, WorkIdentityError>;

    async fn find_work_by_anchor(
        &self,
        user_id: crate::UserId,
        anchor_type: &AnchorType,
        anchor_value: &str,
    ) -> Result<Option<WorkId>, WorkIdentityError>;

    async fn list_anchors(
        &self,
        work_id: WorkId,
    ) -> Result<Vec<WorkIdentityAnchor>, WorkIdentityError>;

    /// Fill anchor types the existing work lacks from `incoming`; never
    /// overwrites a confirmed anchor (additive convergence). Returns the anchor
    /// types it actually confirmed — the single source of truth for the
    /// `anchors_merged` audit list (REQ-008).
    async fn merge_missing_anchors(
        &self,
        work_id: WorkId,
        incoming: &CapturedIdentity,
    ) -> Result<Vec<AnchorType>, WorkIdentityError>;

    /// For each work-anchor type the existing work holds with a differing
    /// incoming value, produce a conflict attributed to `source` (the creation
    /// path raising it, so a Readarr/list/series-monitor conflict is not
    /// mislabelled as a manual add). A type the work lacks is a gap for
    /// `merge_missing_anchors`, not a conflict.
    async fn detect_conflicting_anchors(
        &self,
        existing_work_id: WorkId,
        incoming: &CapturedIdentity,
        source: ConflictSource,
    ) -> Result<Vec<NewIdentityConflict>, WorkIdentityError>;

    /// Persist an observable open conflict row for any federated anchor kind
    /// (REQ-020). Idempotent: an existing open conflict of the same kind on the
    /// same work is returned rather than duplicated. Returns the conflict id.
    async fn raise_identity_conflict(
        &self,
        conflict: NewIdentityConflict,
    ) -> Result<i64, WorkIdentityError>;

    /// One-shot startup backfill (ordered after schema migrations, like the
    /// FK-check harness): rewrite drifted slug-form Goodreads keys in the works
    /// column to bare-numeric via the domain normalizer, then backfill the
    /// goodreads_work anchor rows. Idempotent. SQL cannot call the normalizer,
    /// so this is a Rust step, not a .sql migration (REQ-002; ir-v2 D-014).
    async fn backfill_gr_numeric(&self) -> Result<(), WorkIdentityError>;

    /// Persist a fuzzy (title/author-matched) anchor guess to the ledger only —
    /// `confidence='pending'`, `setter='auto_search'`. MUST NOT write any
    /// `works.*` column: the pending-guess firewall (REQ-004) keeps an
    /// unverified guess out of the synced identity until a user affirms it.
    /// Never downgrades an already-confirmed anchor of the same value
    /// (monotonic, REQ-008).
    async fn record_pending_anchor(
        &self,
        work_id: WorkId,
        anchor_type: AnchorType,
        value: &str,
    ) -> Result<(), WorkIdentityError>;

    /// Increment the durable per-(work, anchor) convergence attempt counter,
    /// creating the row at 1 on first failure. Read back via
    /// [`Self::list_anchor_dead_ends`] to gate further chasing (REQ-009).
    async fn bump_anchor_attempt(
        &self,
        work_id: WorkId,
        anchor_type: AnchorType,
    ) -> Result<(), WorkIdentityError>;

    /// List a work's durable dead-end attempt counters — one row per anchor
    /// type that has failed at least once.
    async fn list_anchor_dead_ends(
        &self,
        work_id: WorkId,
    ) -> Result<Vec<AnchorDeadEnd>, WorkIdentityError>;

    /// Clear one anchor type's dead-end counter — called the moment that anchor
    /// is successfully harvested, so a later loss of it is chased afresh
    /// (REQ-009).
    async fn clear_anchor_dead_end(
        &self,
        work_id: WorkId,
        anchor_type: AnchorType,
    ) -> Result<(), WorkIdentityError>;

    /// Clear ALL of a work's dead-end counters — called only by the single-work
    /// manual "try again" refresh, never by a routine background tick (REQ-009).
    async fn clear_anchor_dead_ends(&self, work_id: WorkId) -> Result<(), WorkIdentityError>;
}
