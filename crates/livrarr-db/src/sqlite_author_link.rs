use chrono::{DateTime, Utc};
use livrarr_domain::{
    AuthorCompatibilityProjection, AuthorEvidenceFingerprint, AuthorId, AuthorKeyAttempt,
    AuthorKeyAttemptOutcome, AuthorLinkCandidate, AuthorLinkProgressUpdate, AuthorLinkReview,
    AuthorLinkTrigger, AuthorProvider, AuthorRoadInput, AuthorRoute, AuthorRouteKey,
    AuthorSweepProgress, ProviderAuthorNameObservation, RejectedAuthorRouteEvidence,
    RouteWriteOutcome, SettledWorkProviderKey, UserId, WorkId,
};

use crate::sqlite::SqliteDb;
use crate::{
    AuthorLinkClaim, AuthorLinkDb, AuthorNameVariantDb, AuthorRouteBackfillReport, DbError,
    GuardedRouteWrite,
};

impl AuthorLinkDb for SqliteDb {
    async fn ensure_enqueued(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        trigger: AuthorLinkTrigger,
    ) -> Result<(), DbError> {
        todo!()
    }

    async fn ensure_missing_progress_rows(&self, limit: u32) -> Result<u32, DbError> {
        todo!()
    }

    async fn claim_due(
        &self,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<AuthorLinkClaim>, DbError> {
        todo!()
    }

    async fn load_road_input(&self, claim: AuthorLinkClaim) -> Result<AuthorRoadInput, DbError> {
        todo!()
    }

    async fn compute_evidence_fingerprint(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<AuthorEvidenceFingerprint, DbError> {
        todo!()
    }

    async fn prepare_key_attempts(
        &self,
        claim: AuthorLinkClaim,
        evidence_generation: i64,
        keys: Vec<SettledWorkProviderKey>,
    ) -> Result<Vec<AuthorKeyAttempt>, DbError> {
        todo!()
    }

    async fn complete_key_attempt(
        &self,
        claim: AuthorLinkClaim,
        key_attempt_id: i64,
        outcome: AuthorKeyAttemptOutcome,
    ) -> Result<(), DbError> {
        todo!()
    }

    async fn apply_guarded_route(
        &self,
        write: GuardedRouteWrite,
    ) -> Result<RouteWriteOutcome, DbError> {
        todo!()
    }

    async fn record_candidates(
        &self,
        claim: AuthorLinkClaim,
        candidates: Vec<AuthorLinkCandidate>,
    ) -> Result<(), DbError> {
        todo!()
    }

    async fn record_readarr_rejection(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        rejected: RejectedAuthorRouteEvidence,
    ) -> Result<AuthorLinkCandidate, DbError> {
        todo!()
    }

    async fn advance_progress(
        &self,
        claim: AuthorLinkClaim,
        update: AuthorLinkProgressUpdate,
    ) -> Result<(), DbError> {
        todo!()
    }

    async fn pick_candidate_as_user(
        &self,
        user_id: UserId,
        candidate_id: i64,
    ) -> Result<AuthorRoute, DbError> {
        todo!()
    }

    async fn attach_route_as_user(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        key: AuthorRouteKey,
    ) -> Result<AuthorRoute, DbError> {
        todo!()
    }

    async fn dismiss_candidate_as_user(
        &self,
        user_id: UserId,
        candidate_id: i64,
    ) -> Result<(), DbError> {
        todo!()
    }

    async fn list_review(&self, user_id: UserId) -> Result<Vec<AuthorLinkReview>, DbError> {
        todo!()
    }

    async fn sweep_progress(&self, user_id: UserId) -> Result<AuthorSweepProgress, DbError> {
        todo!()
    }

    async fn remove_route_as_user(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        route_id: i64,
    ) -> Result<(), DbError> {
        todo!()
    }

    async fn list_active_routes(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        provider: Option<AuthorProvider>,
    ) -> Result<Vec<AuthorRoute>, DbError> {
        todo!()
    }

    async fn has_active_route(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        provider: AuthorProvider,
    ) -> Result<bool, DbError> {
        todo!()
    }

    async fn compatibility_projection(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<AuthorCompatibilityProjection, DbError> {
        todo!()
    }

    async fn ingest_legacy_routes(&self) -> Result<AuthorRouteBackfillReport, DbError> {
        todo!()
    }

    async fn verify_cutover_ready(&self) -> Result<AuthorRouteBackfillReport, DbError> {
        todo!()
    }
}

impl AuthorNameVariantDb for SqliteDb {
    async fn record_observed_names(
        &self,
        user_id: UserId,
        work_id: WorkId,
        observations: &[ProviderAuthorNameObservation],
    ) -> Result<u32, DbError> {
        todo!()
    }
}
