use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use livrarr_domain::services::{
    AuthorLinkService, AuthorRouteView, AuthorService, AuthorViewService,
};
use livrarr_domain::{AuthorId, AuthorLinkReview, AuthorSweepProgress};

use crate::context::{HasAuthorLinkService, HasAuthorService, HasAuthorViewService};
use crate::types::api_error::FieldError;
use crate::{
    ApiError, AuthContext, AuthorLinkReviewResponse, AuthorResponse, AuthorRouteResponse,
    RenameAuthorRequest, SelectAuthorNameRequest,
};

/// The authors whose provider linking is waiting on the user.
///
/// The review row already carries the author's active routes and its
/// current-generation candidates, so the response is assembled from what the one
/// repository read returned rather than by asking again per row.
pub async fn list_author_link_review<S: HasAuthorLinkService>(
    State(state): State<S>,
    auth: AuthContext,
) -> Result<Json<Vec<AuthorLinkReviewResponse>>, ApiError> {
    let reviews = state
        .author_link_service()
        .list_review(auth.user.id)
        .await?;
    Ok(Json(reviews.into_iter().map(review_to_response).collect()))
}

/// Attach the provider route the user picked from the review surface.
pub async fn pick_author_link_candidate<S: HasAuthorLinkService>(
    State(state): State<S>,
    auth: AuthContext,
    Path(candidate_id): Path<i64>,
) -> Result<Json<AuthorRouteResponse>, ApiError> {
    let route = state
        .author_link_service()
        .pick_candidate(auth.user.id, candidate_id)
        .await?;
    Ok(Json(AuthorRouteResponse::from_route(&route)))
}

/// Retire a candidate the user does not want. Routes and tombstones are
/// untouched — dismissal answers a question, it does not change a link.
pub async fn dismiss_author_link_candidate<S: HasAuthorLinkService>(
    State(state): State<S>,
    auth: AuthContext,
    Path(candidate_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state
        .author_link_service()
        .dismiss_candidate(auth.user.id, candidate_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Remove one of an author's provider routes.
pub async fn remove_author_route<S: HasAuthorLinkService>(
    State(state): State<S>,
    auth: AuthContext,
    Path((author_id, route_id)): Path<(AuthorId, i64)>,
) -> Result<StatusCode, ApiError> {
    state
        .author_link_service()
        .remove_route(auth.user.id, author_id, route_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Ask for an author to be looked at again.
///
/// The work is durable and unattended: this returns as soon as the task is due
/// and never waits on a provider.
pub async fn re_resolve_author<S: HasAuthorLinkService>(
    State(state): State<S>,
    auth: AuthContext,
    Path(author_id): Path<AuthorId>,
) -> Result<StatusCode, ApiError> {
    state
        .author_link_service()
        .re_resolve(auth.user.id, author_id)
        .await?;
    Ok(StatusCode::ACCEPTED)
}

/// Rename an author to a name the user typed.
pub async fn rename_author<S: HasAuthorService + HasAuthorViewService>(
    State(state): State<S>,
    auth: AuthContext,
    Path(author_id): Path<AuthorId>,
    Json(request): Json<RenameAuthorRequest>,
) -> Result<Json<AuthorResponse>, ApiError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::Validation {
            errors: vec![FieldError {
                field: "name".into(),
                message: "cannot be empty".into(),
            }],
        });
    }
    let author = state
        .author_service()
        .rename(auth.user.id, author_id, name)
        .await?;
    let view = state
        .author_view_service()
        .route_view(auth.user.id, &author)
        .await?;
    Ok(Json(AuthorResponse::from_author_and_view(&author, view)))
}

/// Promote one of the author's already-observed names to the displayed one.
pub async fn select_author_name<S: HasAuthorService + HasAuthorViewService>(
    State(state): State<S>,
    auth: AuthContext,
    Path(author_id): Path<AuthorId>,
    Json(request): Json<SelectAuthorNameRequest>,
) -> Result<Json<AuthorResponse>, ApiError> {
    let author = state
        .author_service()
        .select_name_variant(auth.user.id, author_id, request.variant_id)
        .await?;
    let view = state
        .author_view_service()
        .route_view(auth.user.id, &author)
        .await?;
    Ok(Json(AuthorResponse::from_author_and_view(&author, view)))
}

/// How much of the user's library the linking sweep still has to look at.
pub async fn author_link_sweep_progress<S: HasAuthorLinkService>(
    State(state): State<S>,
    auth: AuthContext,
) -> Result<Json<AuthorSweepProgress>, ApiError> {
    let progress = state.author_link_service().progress(auth.user.id).await?;
    Ok(Json(progress))
}

fn review_to_response(review: AuthorLinkReview) -> AuthorLinkReviewResponse {
    let under_review = !review.candidates.is_empty();
    let view = AuthorRouteView::from_active_routes(review.routes, under_review);
    AuthorLinkReviewResponse {
        author: AuthorResponse::from_author_and_view(&review.author, view),
        candidates: review.candidates,
    }
}
