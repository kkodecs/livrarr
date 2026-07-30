use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use livrarr_domain::{AuthorId, AuthorSweepProgress};

use crate::context::{HasAuthorLinkService, HasAuthorService};
use crate::{
    ApiError, AuthContext, AuthorLinkReviewResponse, AuthorResponse, AuthorRouteResponse,
    RenameAuthorRequest, SelectAuthorNameRequest,
};

pub async fn list_author_link_review<S: HasAuthorLinkService>(
    State(state): State<S>,
    auth: AuthContext,
) -> Result<Json<Vec<AuthorLinkReviewResponse>>, ApiError> {
    todo!()
}

pub async fn pick_author_link_candidate<S: HasAuthorLinkService>(
    State(state): State<S>,
    auth: AuthContext,
    Path(candidate_id): Path<i64>,
) -> Result<Json<AuthorRouteResponse>, ApiError> {
    todo!()
}

pub async fn dismiss_author_link_candidate<S: HasAuthorLinkService>(
    State(state): State<S>,
    auth: AuthContext,
    Path(candidate_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    todo!()
}

pub async fn remove_author_route<S: HasAuthorLinkService>(
    State(state): State<S>,
    auth: AuthContext,
    Path((author_id, route_id)): Path<(AuthorId, i64)>,
) -> Result<StatusCode, ApiError> {
    todo!()
}

pub async fn re_resolve_author<S: HasAuthorLinkService>(
    State(state): State<S>,
    auth: AuthContext,
    Path(author_id): Path<AuthorId>,
) -> Result<StatusCode, ApiError> {
    todo!()
}

pub async fn rename_author<S: HasAuthorService>(
    State(state): State<S>,
    auth: AuthContext,
    Path(author_id): Path<AuthorId>,
    Json(request): Json<RenameAuthorRequest>,
) -> Result<Json<AuthorResponse>, ApiError> {
    todo!()
}

pub async fn select_author_name<S: HasAuthorService>(
    State(state): State<S>,
    auth: AuthContext,
    Path(author_id): Path<AuthorId>,
    Json(request): Json<SelectAuthorNameRequest>,
) -> Result<Json<AuthorResponse>, ApiError> {
    todo!()
}

pub async fn author_link_sweep_progress<S: HasAuthorLinkService>(
    State(state): State<S>,
    auth: AuthContext,
) -> Result<Json<AuthorSweepProgress>, ApiError> {
    todo!()
}
