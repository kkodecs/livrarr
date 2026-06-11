use crate::identity::*;
use crate::WorkId;

#[derive(Debug, thiserror::Error)]
pub enum WorkIdentityError {
    #[error("invalid anchor value")]
    InvalidAnchorValue,
    #[error("anchor not found")]
    AnchorNotFound,
    #[error("seed carried no usable identity signal")]
    EmptySeed,
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
    /// overwrites a confirmed anchor (additive convergence).
    async fn merge_missing_anchors(
        &self,
        work_id: WorkId,
        incoming: &CapturedIdentity,
    ) -> Result<(), WorkIdentityError>;

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
}
