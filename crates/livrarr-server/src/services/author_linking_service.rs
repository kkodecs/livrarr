use livrarr_domain::services::{AuthorLinkService, AuthorLinkWorkflow};
use livrarr_domain::{
    AgreedAuthorRouteEvidence, AuthorId, AuthorLinkCandidate, AuthorLinkError, AuthorLinkProgress,
    AuthorLinkReview, AuthorLinkTrigger, AuthorRoute, AuthorRouteKey, AuthorSweepProgress,
    AuthorSweepTickSummary, RejectedAuthorRouteEvidence, RouteWriteOutcome, UserId,
};
use tokio_util::sync::CancellationToken;

pub struct LiveAuthorLinkingService;

impl AuthorLinkService for LiveAuthorLinkingService {
    async fn list_review(&self, user_id: UserId) -> Result<Vec<AuthorLinkReview>, AuthorLinkError> {
        todo!()
    }

    async fn pick_candidate(
        &self,
        user_id: UserId,
        candidate_id: i64,
    ) -> Result<AuthorRoute, AuthorLinkError> {
        todo!()
    }

    async fn attach_selected_route(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        key: AuthorRouteKey,
    ) -> Result<AuthorRoute, AuthorLinkError> {
        todo!()
    }

    async fn dismiss_candidate(
        &self,
        user_id: UserId,
        candidate_id: i64,
    ) -> Result<(), AuthorLinkError> {
        todo!()
    }

    async fn remove_route(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        route_id: i64,
    ) -> Result<(), AuthorLinkError> {
        todo!()
    }

    async fn re_resolve(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<AuthorLinkProgress, AuthorLinkError> {
        todo!()
    }

    async fn progress(&self, user_id: UserId) -> Result<AuthorSweepProgress, AuthorLinkError> {
        todo!()
    }
}

impl AuthorLinkWorkflow for LiveAuthorLinkingService {
    async fn enqueue(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        trigger: AuthorLinkTrigger,
    ) -> Result<(), AuthorLinkError> {
        todo!()
    }

    async fn submit_evidence(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        evidence: AgreedAuthorRouteEvidence,
    ) -> Result<RouteWriteOutcome, AuthorLinkError> {
        todo!()
    }

    async fn record_readarr_rejection(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        rejected: RejectedAuthorRouteEvidence,
    ) -> Result<AuthorLinkCandidate, AuthorLinkError> {
        todo!()
    }

    async fn run_due(
        &self,
        batch_size: u32,
        cancel: CancellationToken,
    ) -> Result<AuthorSweepTickSummary, AuthorLinkError> {
        todo!()
    }
}
