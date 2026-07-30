use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::DbError;
use livrarr_domain::{
    AgreedAuthorRouteEvidence, AuthorCompatibilityProjection, AuthorEvidenceFingerprint, AuthorId,
    AuthorKeyAttempt, AuthorKeyAttemptOutcome, AuthorLinkCandidate, AuthorLinkCursor,
    AuthorLinkProgressUpdate, AuthorLinkReview, AuthorLinkTrigger, AuthorNameSource,
    AuthorProvider, AuthorRoadInput, AuthorRoute, AuthorRouteKey, AuthorSweepProgress,
    ProviderAuthorNameObservation, RejectedAuthorRouteEvidence, RequestPriority, RouteWriteOutcome,
    SettledWorkProviderKey, UserId, WorkId,
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

    async fn ensure_missing_progress_rows(&self, limit: u32) -> Result<u32, DbError>;

    async fn claim_due(
        &self,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<AuthorLinkClaim>, DbError>;

    async fn load_road_input(&self, claim: AuthorLinkClaim) -> Result<AuthorRoadInput, DbError>;

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

    async fn complete_key_attempt(
        &self,
        claim: AuthorLinkClaim,
        key_attempt_id: i64,
        outcome: AuthorKeyAttemptOutcome,
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
}
