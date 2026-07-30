use livrarr_db::{AuthorDb, AuthorLinkClaim, AuthorLinkDb, WorkDb};
use livrarr_domain::services::{
    AuthorLinkService, AuthorLinkWorkflow, AuthorProviderGateway, AuthorServiceError,
};
use livrarr_domain::{
    AgreedAuthorRouteEvidence, Author, AuthorCompatibilityProjection, AuthorId,
    AuthorLinkCandidate, AuthorLinkError, AuthorLinkProgress, AuthorLinkProgressUpdate,
    AuthorLinkReview, AuthorLinkState, AuthorLinkTrigger, AuthorNameSource, AuthorNameVariant,
    AuthorRoute, AuthorRouteKey, AuthorSweepProgress, AuthorSweepTickSummary,
    RejectedAuthorRouteEvidence, RouteWriteOutcome, UserId,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorNameRankModel {
    EnglishOrUndetermined,
    ForeignDominant,
}

pub fn author_name_rank_table(model: AuthorNameRankModel) -> &'static [AuthorNameSource] {
    todo!()
}

pub fn choose_author_display_name<'a>(
    variants: &[AuthorNameVariant],
    work_languages: impl Iterator<Item = Option<&'a str>>,
) -> Option<AuthorNameVariant> {
    todo!()
}

pub struct AuthorLinkingServiceImpl<D, G>
where
    D: AuthorLinkDb + AuthorDb + WorkDb,
    G: AuthorProviderGateway,
{
    pub db: D,
    pub gateway: G,
}

impl<D, G> AuthorLinkingServiceImpl<D, G>
where
    D: AuthorLinkDb + AuthorDb + WorkDb,
    G: AuthorProviderGateway,
{
    pub async fn run_author(
        &self,
        claim: AuthorLinkClaim,
    ) -> Result<AuthorLinkProgressUpdate, AuthorLinkError> {
        todo!()
    }
}

impl<D, G> AuthorLinkService for AuthorLinkingServiceImpl<D, G>
where
    D: AuthorLinkDb + AuthorDb + WorkDb + Send + Sync,
    G: AuthorProviderGateway + Send + Sync,
{
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

impl<D, G> AuthorLinkWorkflow for AuthorLinkingServiceImpl<D, G>
where
    D: AuthorLinkDb + AuthorDb + WorkDb + Send + Sync,
    G: AuthorProviderGateway + Send + Sync,
{
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

pub struct AuthorResponseAssembler;

impl AuthorResponseAssembler {
    pub async fn route_view(
        &self,
        user_id: UserId,
        author: &Author,
    ) -> Result<
        (
            Vec<AuthorRoute>,
            AuthorLinkState,
            bool,
            AuthorCompatibilityProjection,
        ),
        AuthorServiceError,
    > {
        todo!()
    }
}
