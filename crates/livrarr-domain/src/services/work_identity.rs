use super::identity_edit::{
    ClearedSlot, CollisionInfo, IdentityCompletion, IdentityCompletionOutcome, IdentityEditBasis,
};
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
    /// A first-statement `identity_generation` claim found zero rows: the
    /// resource was current at the door read, but a different identity
    /// mutation won the claim. Doors map this to their dedicated 409 code.
    #[error("identity changed since it was read")]
    StaleIdentity,
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

    // ── identity-edit surface (design identity-edit r4) ──────────────────
    //
    // The methods below carry stub default bodies (written in desugared form
    // because `trait_variant` cannot expand a provided `async fn`): a test
    // double that never exercises identity editing compiles without
    // implementing them, and calling one on such a double is a typed error,
    // never silent success. `SqliteDb` overrides every one.

    /// Apply a certified identity edit in one transaction whose FIRST
    /// statement is the conditional `identity_generation` claim (zero rows →
    /// [`crate::identity_edit::IdentityEditError::StalePreview`]). Plain data
    /// only — no preview-cache or service struct crosses this boundary.
    fn apply_identity_edit(
        &self,
        work_id: WorkId,
        user_id: crate::UserId,
        slot: AnchorType,
        new_value: &str,
        expected_generation: i64,
        drop_slots: &[AnchorType],
    ) -> impl std::future::Future<Output = Result<(), crate::identity_edit::IdentityEditError>> + Send
    {
        let _ = (
            work_id,
            user_id,
            slot,
            new_value,
            expected_generation,
            drop_slots,
        );
        async move {
            Err(crate::identity_edit::IdentityEditError::Db(
                "identity edit not supported by this repository".into(),
            ))
        }
    }

    /// Clear one identity slot in one transaction whose first statement is a
    /// user-scoped generation bump (claiming the then-current slot).
    /// [`crate::identity_edit::IdentityEditError::EmptySlot`] when the slot
    /// holds no confirmed row, no nonempty column, and no pending row.
    fn apply_identity_clear(
        &self,
        work_id: WorkId,
        user_id: crate::UserId,
        slot: AnchorType,
    ) -> impl std::future::Future<
        Output = Result<ClearedSlot, crate::identity_edit::IdentityEditError>,
    > + Send {
        let _ = (work_id, user_id, slot);
        async move {
            Err(crate::identity_edit::IdentityEditError::Db(
                "identity clear not supported by this repository".into(),
            ))
        }
    }

    /// One coherent user-scoped snapshot of generation + validated
    /// ledger∪column slots + open conflicts + badge — the preview basis and
    /// the commit-time no-op authority.
    fn read_identity_edit_basis(
        &self,
        user_id: crate::UserId,
        work_id: WorkId,
    ) -> impl std::future::Future<Output = Result<IdentityEditBasis, WorkIdentityError>> + Send
    {
        let _ = (user_id, work_id);
        async move {
            Err(WorkIdentityError::Db(
                "identity edit basis not supported by this repository".into(),
            ))
        }
    }

    /// Same-user owner of `(anchor_type, value)` over the validated
    /// ledger∪column union, excluding `exclude_work_id`. Another user's
    /// id/title is never returned.
    fn find_anchor_owner(
        &self,
        user_id: crate::UserId,
        anchor_type: &AnchorType,
        value: &str,
        exclude_work_id: WorkId,
    ) -> impl std::future::Future<Output = Result<Option<CollisionInfo>, WorkIdentityError>> + Send
    {
        let _ = (user_id, anchor_type, value, exclude_work_id);
        async move {
            Err(WorkIdentityError::Db(
                "anchor owner lookup not supported by this repository".into(),
            ))
        }
    }

    /// Apply one delayed resolver completion under a first-statement
    /// conditional generation claim: zero rows →
    /// [`IdentityCompletionOutcome::Superseded`] with zero writes (no anchor,
    /// pending row, review state, conflict, badge, or dead-end mutation).
    fn complete_anchors(
        &self,
        work_id: WorkId,
        expected_generation: i64,
        completion: IdentityCompletion,
    ) -> impl std::future::Future<Output = Result<IdentityCompletionOutcome, WorkIdentityError>> + Send
    {
        let _ = (work_id, expected_generation, completion);
        async move {
            Err(WorkIdentityError::Db(
                "claimed completion not supported by this repository".into(),
            ))
        }
    }

    /// `(Work, identity_generation)` from ONE repository read — the coherent
    /// basis every resolver road obtains immediately before its provider
    /// await. A separate work read followed by a generation read is
    /// forbidden: an edit between them could pair stale anchors with a fresh
    /// generation.
    fn get_work_with_identity_generation(
        &self,
        user_id: crate::UserId,
        work_id: WorkId,
    ) -> impl std::future::Future<Output = Result<(Work, i64), WorkIdentityError>> + Send {
        let _ = (user_id, work_id);
        async move {
            Err(WorkIdentityError::Db(
                "generation read not supported by this repository".into(),
            ))
        }
    }

    /// Generation + anchor rows read together in one transaction — the
    /// coherent basis for the pending-affirm door's claim.
    fn read_anchors_with_generation(
        &self,
        work_id: WorkId,
    ) -> impl std::future::Future<Output = Result<(i64, Vec<WorkIdentityAnchor>), WorkIdentityError>>
           + Send {
        let _ = work_id;
        async move {
            Err(WorkIdentityError::Db(
                "generation read not supported by this repository".into(),
            ))
        }
    }

    /// Generation + review candidates read together in one transaction — the
    /// coherent basis for the review apply/dismiss doors' claims.
    fn read_review_candidates_with_generation(
        &self,
        work_id: WorkId,
    ) -> impl std::future::Future<Output = Result<(i64, Option<Vec<Candidate>>), WorkIdentityError>> + Send
    {
        let _ = work_id;
        async move {
            Err(WorkIdentityError::Db(
                "generation read not supported by this repository".into(),
            ))
        }
    }

    /// [`Self::apply_review_candidate`] behind a first-statement conditional
    /// generation claim: zero rows → [`WorkIdentityError::StaleIdentity`]
    /// before the existing parked-state claim runs.
    fn apply_review_candidate_claimed(
        &self,
        work_id: WorkId,
        candidate: &Candidate,
        setter: AnchorSetter,
        expected_generation: i64,
    ) -> impl std::future::Future<Output = Result<(), WorkIdentityError>> + Send {
        let _ = (work_id, candidate, setter, expected_generation);
        async move {
            Err(WorkIdentityError::Db(
                "claimed review apply not supported by this repository".into(),
            ))
        }
    }

    /// [`Self::dismiss_review`] behind a first-statement conditional
    /// generation claim: zero rows → [`WorkIdentityError::StaleIdentity`]
    /// before the existing parked-state claim runs.
    fn dismiss_review_claimed(
        &self,
        work_id: WorkId,
        expected_generation: i64,
    ) -> impl std::future::Future<Output = Result<(), WorkIdentityError>> + Send {
        let _ = (work_id, expected_generation);
        async move {
            Err(WorkIdentityError::Db(
                "claimed review dismiss not supported by this repository".into(),
            ))
        }
    }
}
