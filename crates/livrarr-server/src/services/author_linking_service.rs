use livrarr_db::sqlite::SqliteDb;
use livrarr_domain::services::{AuthorLinkService, AuthorLinkWorkflow};
use livrarr_domain::{
    AgreedAuthorRouteEvidence, AuthorId, AuthorLinkCandidate, AuthorLinkError, AuthorLinkProgress,
    AuthorLinkReview, AuthorLinkTrigger, AuthorRoute, AuthorRouteKey, AuthorSweepProgress,
    AuthorSweepTickSummary, RejectedAuthorRouteEvidence, RouteWriteOutcome, UserId,
};
use tokio_util::sync::CancellationToken;

use crate::state::AppState;

/// The author-linking road as the live application composes it: the production
/// repository and the production provider gateway.
pub type LiveAuthorLinkRoad = livrarr_metadata::author_linking::AuthorLinkingServiceImpl<
    SqliteDb,
    livrarr_external_data::AuthorProviderGatewayImpl<livrarr_http::fetcher::HttpFetcherImpl>,
>;

/// The type `AppState.author_link_service` holds.
///
/// It carries no state because the live author-link service is [`AppState`]
/// itself: composing the road needs the repository *and* the shared provider
/// transports, and the composition root is the one place both are already in
/// hand. Route handlers reach the road through
/// `HasAuthorLinkService`, which resolves to that impl.
pub struct LiveAuthorLinkingService;

impl AppState {
    /// Build the author-linking road from this state's own shared handles.
    ///
    /// Pure struct construction: the repository handle and the three provider
    /// clients are clones of the process-wide transports (one shared
    /// `HttpFetcherImpl`, one shared `HttpClient`, one live config snapshot), so
    /// no connection pool, socket, or queue is created here.
    fn author_link_road(&self) -> LiveAuthorLinkRoad {
        let gateway = livrarr_external_data::AuthorProviderGatewayImpl::new(
            livrarr_external_data::OpenLibraryClient::new(self.http_fetcher.clone()),
            livrarr_external_data::GoodreadsClient::new(
                self.http_fetcher.clone(),
                self.http_client.clone(),
                livrarr_external_data::goodreads::GOODREADS_BASE_URL,
            ),
            livrarr_external_data::HardcoverClient::new(
                self.http_fetcher.clone(),
                self.live_metadata_config.clone(),
            ),
        );
        LiveAuthorLinkRoad {
            db: self.db.clone(),
            gateway,
        }
    }
}

impl AuthorLinkService for AppState {
    async fn list_review(&self, user_id: UserId) -> Result<Vec<AuthorLinkReview>, AuthorLinkError> {
        self.author_link_road().list_review(user_id).await
    }

    async fn pick_candidate(
        &self,
        user_id: UserId,
        candidate_id: i64,
    ) -> Result<AuthorRoute, AuthorLinkError> {
        self.author_link_road()
            .pick_candidate(user_id, candidate_id)
            .await
    }

    async fn attach_selected_route(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        key: AuthorRouteKey,
    ) -> Result<AuthorRoute, AuthorLinkError> {
        self.author_link_road()
            .attach_selected_route(user_id, author_id, key)
            .await
    }

    async fn dismiss_candidate(
        &self,
        user_id: UserId,
        candidate_id: i64,
    ) -> Result<(), AuthorLinkError> {
        self.author_link_road()
            .dismiss_candidate(user_id, candidate_id)
            .await
    }

    async fn remove_route(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        route_id: i64,
    ) -> Result<(), AuthorLinkError> {
        self.author_link_road()
            .remove_route(user_id, author_id, route_id)
            .await
    }

    async fn re_resolve(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<AuthorLinkProgress, AuthorLinkError> {
        self.author_link_road().re_resolve(user_id, author_id).await
    }

    async fn progress(&self, user_id: UserId) -> Result<AuthorSweepProgress, AuthorLinkError> {
        self.author_link_road().progress(user_id).await
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
