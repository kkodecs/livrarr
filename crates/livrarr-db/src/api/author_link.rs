use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::DbError;
use livrarr_domain::{
    AgreedAuthorRouteEvidence, Author, AuthorCompatibilityProjection, AuthorEvidenceFingerprint,
    AuthorId, AuthorKeyAttempt, AuthorKeyAttemptOutcome, AuthorLinkCandidate, AuthorLinkCursor,
    AuthorLinkProgress, AuthorLinkProgressUpdate, AuthorLinkReview, AuthorLinkTrigger,
    AuthorNameSource, AuthorNameVariant, AuthorProvider, AuthorRoadInput, AuthorRoute,
    AuthorRouteKey, AuthorSweepProgress, OutstandingKeyRetry, ProviderAuthorNameObservation,
    RejectedAuthorRouteEvidence, RequestPriority, RouteWriteOutcome, SettledWorkProviderKey,
    UserId, WorkId,
};

#[derive(Debug, Clone)]
pub struct AuthorLinkClaim {
    pub author_id: AuthorId,
    pub user_id: UserId,
    pub claim_token: Uuid,
    pub lease_expires_at: DateTime<Utc>,
    pub cursor: Option<AuthorLinkCursor>,
    pub display_name_generation: i64,
}

#[derive(Debug, Clone)]
pub struct GuardedRouteWrite {
    pub claim_token: Option<Uuid>,
    pub author_id: AuthorId,
    pub evidence: AgreedAuthorRouteEvidence,
}

#[derive(Debug, Clone)]
pub struct AuthorRouteBackfillReport {
    pub legacy_values: u64,
    pub canonical_routes: u64,
    pub missing_routes: u64,
    pub invalid_values: u64,
    pub missing_progress_rows: u64,
}

#[derive(Debug, Clone)]
pub struct AuthorProviderCall {
    pub provider: AuthorProvider,
    pub work_route: String,
    pub priority: RequestPriority,
}

/// Input to the shared author create/adopt gate.
///
/// It deliberately carries no `ol_key`/`gr_key`/`hc_key`: an author is created
/// or adopted on identity alone, and an explicitly selected provider route is a
/// separate user-sovereign step taken after the gate commits.
#[derive(Debug, Clone)]
pub struct CreateAuthorGateRequest {
    pub user_id: UserId,
    pub name: String,
    pub sort_name: Option<String>,
    pub import_id: Option<String>,
    pub initial_name_source: AuthorNameSource,
    pub trigger: AuthorLinkTrigger,
}

#[derive(Debug, Clone)]
pub struct RenameAuthorDbRequest {
    pub user_id: UserId,
    pub author_id: AuthorId,
    pub display_name: String,
    pub variant_id: i64,
}

#[trait_variant::make(Send)]
pub trait AuthorLinkDb: Send + Sync {
    async fn ensure_enqueued(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        trigger: AuthorLinkTrigger,
    ) -> Result<(), DbError>;

    /// The shared author create/adopt gate: one transaction that converges a
    /// creation race onto a single author row and leaves that winner with its
    /// initial name variant and a due author-link progress row.
    ///
    /// Every add door enters here, so an author can never be committed in a
    /// state the sweep cannot see. Returns the converged author and whether this
    /// caller is the one that created it.
    async fn create_or_adopt_author(
        &self,
        request: CreateAuthorGateRequest,
    ) -> Result<(Author, bool), DbError>;

    async fn ensure_missing_progress_rows(&self, limit: u32) -> Result<u32, DbError>;

    async fn claim_due(
        &self,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<AuthorLinkClaim>, DbError>;

    async fn load_road_input(&self, claim: AuthorLinkClaim) -> Result<AuthorRoadInput, DbError>;

    /// The author's durable progress row.
    ///
    /// The road needs the persisted evidence generation and attempt count that
    /// `load_road_input` deliberately leaves out of its evidence snapshot, and
    /// the user-facing re-resolve door returns this row directly.
    async fn load_progress(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<AuthorLinkProgress, DbError>;

    /// Open a new evidence generation for a claimed author.
    ///
    /// Changed settled evidence retires the previous generation's question:
    /// the stored generation moves forward (never backward), still-pending
    /// candidates from older generations are superseded, and the road cursor is
    /// cleared so no tier resumes at a position the new evidence never
    /// produced. Persisting the generation before any route or candidate write
    /// is what keeps every effect of this run in one reviewable generation.
    async fn begin_evidence_generation(
        &self,
        claim: AuthorLinkClaim,
        evidence_generation: i64,
    ) -> Result<(), DbError>;

    async fn compute_evidence_fingerprint(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<AuthorEvidenceFingerprint, DbError>;

    async fn prepare_key_attempts(
        &self,
        claim: AuthorLinkClaim,
        evidence_generation: i64,
        keys: Vec<SettledWorkProviderKey>,
    ) -> Result<Vec<AuthorKeyAttempt>, DbError>;

    /// Record one key attempt's transition and, in the same statement, how many
    /// authorial-slot observations it made.
    ///
    /// The count travels with the transition rather than in a second write, so
    /// an attempt can never reach a completed state without its observation. A
    /// failure arm passes 0 — a key that failed observed nothing.
    async fn complete_key_attempt(
        &self,
        claim: AuthorLinkClaim,
        key_attempt_id: i64,
        outcome: AuthorKeyAttemptOutcome,
        authorial_credits_seen: u32,
    ) -> Result<(), DbError>;

    /// How many authorial-slot observations every key attempt in one evidence
    /// generation has durably recorded.
    ///
    /// The Tier-2 gate reads this rather than a per-pass in-memory tally: a
    /// terminal attempt is never returned by `prepare_key_attempts` again, so
    /// what an earlier pass learned is otherwise absent from the next pass's
    /// count and Tier 2 opens on a question that was already answered.
    async fn generation_authorial_credit_count(
        &self,
        claim: AuthorLinkClaim,
        evidence_generation: i64,
    ) -> Result<u64, DbError>;

    /// Every key attempt in one evidence generation that is still owed a run.
    ///
    /// A retry scheduled for later is deliberately withheld by
    /// `prepare_key_attempts`, so a pass that runs no key has an empty tally and
    /// would otherwise conclude the author has nothing outstanding — retiring a
    /// live retry's state and pulling its deadline forward to a parked day.
    async fn generation_outstanding_retries(
        &self,
        claim: AuthorLinkClaim,
        evidence_generation: i64,
    ) -> Result<Vec<OutstandingKeyRetry>, DbError>;

    /// How many questions of one evidence generation are still waiting on the
    /// user.
    ///
    /// The same reason: a question an earlier pass parked is durable and still
    /// on the review page, so a later pass that writes no candidate must not
    /// report the author as holding no evidence.
    async fn generation_pending_candidate_count(
        &self,
        claim: AuthorLinkClaim,
        evidence_generation: i64,
    ) -> Result<u32, DbError>;

    /// Un-suppress every question this author's dismissals are silencing, and
    /// make the author replay — in one transaction.
    ///
    /// All three effects land together or not at all: every still-active
    /// dismissal is stamped `revoked_at` (never deleted, so the user's decision
    /// stays on the record), the progress row is queued and made immediately due
    /// with any live claim voided, and `evaluated_fingerprint` is cleared so the
    /// next pass opens a new generation with fresh, non-terminal key attempts.
    /// A revocation without the replay leaves the question unanswerable; a
    /// replay without the revocation re-suppresses immediately.
    async fn revoke_dismissals_and_replay(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<(), DbError>;

    async fn apply_guarded_route(
        &self,
        write: GuardedRouteWrite,
    ) -> Result<RouteWriteOutcome, DbError>;

    async fn record_candidates(
        &self,
        claim: AuthorLinkClaim,
        candidates: Vec<AuthorLinkCandidate>,
    ) -> Result<(), DbError>;

    async fn record_readarr_rejection(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        rejected: RejectedAuthorRouteEvidence,
    ) -> Result<AuthorLinkCandidate, DbError>;

    async fn advance_progress(
        &self,
        claim: AuthorLinkClaim,
        update: AuthorLinkProgressUpdate,
    ) -> Result<(), DbError>;

    async fn pick_candidate_as_user(
        &self,
        user_id: UserId,
        candidate_id: i64,
    ) -> Result<AuthorRoute, DbError>;

    async fn attach_route_as_user(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        key: AuthorRouteKey,
    ) -> Result<AuthorRoute, DbError>;

    async fn dismiss_candidate_as_user(
        &self,
        user_id: UserId,
        candidate_id: i64,
    ) -> Result<(), DbError>;

    async fn list_review(&self, user_id: UserId) -> Result<Vec<AuthorLinkReview>, DbError>;

    async fn sweep_progress(&self, user_id: UserId) -> Result<AuthorSweepProgress, DbError>;

    async fn remove_route_as_user(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        route_id: i64,
    ) -> Result<(), DbError>;

    async fn list_active_routes(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        provider: Option<AuthorProvider>,
    ) -> Result<Vec<AuthorRoute>, DbError>;

    /// Every route row the author-detail panel shows: active rows first, then the
    /// removal history.
    ///
    /// Removed rows are provenance — "you took this away" — never linkage, so a
    /// caller must derive link state and monitorability from the active rows
    /// alone (`AuthorRouteView::from_route_history` is the one place that does).
    async fn list_routes_for_view(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<AuthorRoute>, DbError>;

    async fn has_active_route(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        provider: AuthorProvider,
    ) -> Result<bool, DbError>;

    async fn compatibility_projection(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<AuthorCompatibilityProjection, DbError>;

    async fn ingest_legacy_routes(&self) -> Result<AuthorRouteBackfillReport, DbError>;

    async fn verify_cutover_ready(&self) -> Result<AuthorRouteBackfillReport, DbError>;
}

#[trait_variant::make(Send)]
pub trait AuthorNameVariantDb: Send + Sync {
    async fn record_observed_names(
        &self,
        user_id: UserId,
        work_id: WorkId,
        observations: &[ProviderAuthorNameObservation],
    ) -> Result<u32, DbError>;

    /// The same observation write for a caller that already holds the author.
    ///
    /// The work-scoped form resolves the author *from a work*; an import that is
    /// processing an author list has no work in hand, and a name it drops there
    /// is a name the display picker can never offer again (FP-035).
    async fn record_author_observed_names(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        observations: &[ProviderAuthorNameObservation],
    ) -> Result<u32, DbError>;

    /// Every spelling recorded for the author, oldest row first.
    ///
    /// The display-name picker cannot offer choices it cannot list, and the
    /// name guard needs the author's full associated-name snapshot rather than
    /// just `authors.name`.
    async fn list_name_variants(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<AuthorNameVariant>, DbError>;
}
