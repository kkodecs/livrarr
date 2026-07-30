use tokio_util::sync::CancellationToken;

use crate::{
    AgreedAuthorRouteEvidence, AuthorId, AuthorLinkCandidate, AuthorLinkError, AuthorLinkProgress,
    AuthorLinkReview, AuthorLinkTrigger, AuthorProvider, AuthorProviderError, AuthorRoute,
    AuthorRouteKey, AuthorSweepProgress, AuthorSweepTickSummary, OpenLibraryAuthorCandidate,
    OpenLibraryAuthorKey, OpenLibraryCatalogPage, ProviderAuthorRef, RejectedAuthorRouteEvidence,
    RequestPriority, RouteWriteOutcome, UserId,
};

#[trait_variant::make(Send)]
pub trait AuthorLinkService: Send + Sync {
    async fn list_review(&self, user_id: UserId) -> Result<Vec<AuthorLinkReview>, AuthorLinkError>;

    async fn pick_candidate(
        &self,
        user_id: UserId,
        candidate_id: i64,
    ) -> Result<AuthorRoute, AuthorLinkError>;

    async fn attach_selected_route(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        key: AuthorRouteKey,
    ) -> Result<AuthorRoute, AuthorLinkError>;

    async fn dismiss_candidate(
        &self,
        user_id: UserId,
        candidate_id: i64,
    ) -> Result<(), AuthorLinkError>;

    async fn remove_route(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        route_id: i64,
    ) -> Result<(), AuthorLinkError>;

    async fn re_resolve(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<AuthorLinkProgress, AuthorLinkError>;

    async fn progress(&self, user_id: UserId) -> Result<AuthorSweepProgress, AuthorLinkError>;
}

#[trait_variant::make(Send)]
pub trait AuthorLinkWorkflow: Send + Sync {
    async fn enqueue(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        trigger: AuthorLinkTrigger,
    ) -> Result<(), AuthorLinkError>;

    async fn submit_evidence(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        evidence: AgreedAuthorRouteEvidence,
    ) -> Result<RouteWriteOutcome, AuthorLinkError>;

    async fn record_readarr_rejection(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        rejected: RejectedAuthorRouteEvidence,
    ) -> Result<AuthorLinkCandidate, AuthorLinkError>;

    async fn run_due(
        &self,
        batch_size: u32,
        cancel: CancellationToken,
    ) -> Result<AuthorSweepTickSummary, AuthorLinkError>;
}

#[trait_variant::make(Send)]
pub trait AuthorProviderGateway: Send + Sync {
    async fn fetch_work_authors(
        &self,
        provider: AuthorProvider,
        work_route: String,
        priority: RequestPriority,
    ) -> Result<Vec<ProviderAuthorRef>, AuthorProviderError>;

    async fn search_open_library_authors(
        &self,
        query: String,
        limit: u32,
        priority: RequestPriority,
    ) -> Result<Vec<OpenLibraryAuthorCandidate>, AuthorProviderError>;

    async fn fetch_open_library_catalog_page(
        &self,
        author_route: OpenLibraryAuthorKey,
        cursor: Option<String>,
        priority: RequestPriority,
    ) -> Result<OpenLibraryCatalogPage, AuthorProviderError>;
}
