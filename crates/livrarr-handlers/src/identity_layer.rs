//! Identity-layer-rewrite (F2) additive handlers. IR v1 `livrarr-handlers`
//! module (ir-v1-identity-layer-rewrite.yaml:1332-1351).
//!
//! DEVIATION FROM IR v1's LITERAL MODULE PATH: IR v1 names this function
//! `identity_review::resolve`, but `crate::identity_review::resolve` already
//! exists (the old anchor-confirmation review handler, `HasWorkIdentityRepository`-
//! bound). Additive-only forbids adding a second same-named function to that
//! module, so the new one lives here instead — the function NAME is
//! verbatim, only the module prefix differs. Flagged loudly per the stub
//! packet's deviation-reporting rule; see STUBS-REPORT.md.
//!
//! The registered handler validates the typed card command against the path,
//! constructs the authenticated actor, and delegates the continuation to the
//! injected identity road. It owns no identity persistence.

use axum::extract::{Path, State};
use axum::Json;
use serde_json::json;

use crate::context::{HasIdentityLayerRepository, HasIdentityRoadService};
use crate::types::identity_layer::ReviewResolutionRequest;
use crate::{ApiError, AuthContext};
use livrarr_domain::identity_layer::{
    IdentityRepositoryError, IdentityRoadError, IdentityRoadOutcome, IdentityRoadService,
    PendingReviewCard, ReviewActor, WorkIdentityRepository,
};

pub async fn list<S: HasIdentityLayerRepository>(
    State(state): State<S>,
    ctx: AuthContext,
) -> Result<Json<Vec<PendingReviewCard>>, ApiError> {
    let cards = state
        .identity_layer_repository()
        .list_pending_reviews(ReviewActor::AuthenticatedUser {
            user_id: ctx.user.id,
        })
        .await
        .map_err(map_identity_repository_error)?;
    Ok(Json(cards))
}

pub async fn dismiss<S: HasIdentityLayerRepository>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(card_id): Path<i64>,
) -> Result<axum::http::StatusCode, ApiError> {
    state
        .identity_layer_repository()
        .dismiss_pending_review(
            ReviewActor::AuthenticatedUser {
                user_id: ctx.user.id,
            },
            card_id,
        )
        .await
        .map_err(map_identity_repository_error)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub async fn resolve<S: HasIdentityRoadService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(card_id): Path<i64>,
    Json(request): Json<ReviewResolutionRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if request.command.card_id() != card_id {
        return Err(ApiError::BadRequest(
            "route card id does not match resolution command".to_string(),
        ));
    }
    let outcome = state
        .identity_road_service()
        .resolve_review(
            ReviewActor::AuthenticatedUser {
                user_id: ctx.user.id,
            },
            request.command,
        )
        .await
        .map_err(map_identity_road_error)?;
    Ok(Json(match outcome {
        IdentityRoadOutcome::Settled {
            work_id,
            created,
            routes,
            status,
            library_items_moved,
            grabs_moved,
        } => json!({
            "workId": work_id,
            "created": created,
            "routes": routes,
            "status": status,
            "libraryItemsMoved": library_items_moved,
            "grabsMoved": grabs_moved,
            "provenance": "User"
        }),
        IdentityRoadOutcome::ReviewPending {
            review_id,
            kind,
            unattached,
            expected_generation,
            provenance,
        } => json!({
            "cardId": review_id,
            "kind": kind,
            "unattached": unattached,
            "expectedGeneration": expected_generation,
            "provenance": provenance,
        }),
        IdentityRoadOutcome::Deferred { reason } => json!({"deferred": reason}),
        IdentityRoadOutcome::Rejected { reason } => json!({"rejected": reason}),
    }))
}

fn map_identity_road_error(error: IdentityRoadError) -> ApiError {
    match error {
        IdentityRoadError::NotFound => ApiError::NotFound,
        IdentityRoadError::StaleGeneration => ApiError::Conflict {
            reason: "stale identity generation".to_string(),
        },
        IdentityRoadError::ReviewProposalInvalidated(reason) => ApiError::Conflict {
            reason: format!("review proposal invalidated: {reason}"),
        },
        IdentityRoadError::ReviewKindMismatch | IdentityRoadError::InvalidResolution => {
            ApiError::BadRequest(error.to_string())
        }
        IdentityRoadError::UnauthorizedScope => ApiError::Forbidden,
        IdentityRoadError::InvalidDoorEvidence => ApiError::Unprocessable(error.to_string()),
        IdentityRoadError::ProviderBoundary => ApiError::BadGateway(error.to_string()),
        IdentityRoadError::Cancelled => ApiError::ServiceUnavailable,
        IdentityRoadError::ReviewRequired | IdentityRoadError::ProbeBlocked(_) => {
            ApiError::Conflict {
                reason: error.to_string(),
            }
        }
        IdentityRoadError::Database(message) => ApiError::Internal(message),
    }
}

fn map_identity_repository_error(error: IdentityRepositoryError) -> ApiError {
    match error {
        IdentityRepositoryError::NotFound => ApiError::NotFound,
        IdentityRepositoryError::UnauthorizedScope => ApiError::Forbidden,
        IdentityRepositoryError::StaleGeneration
        | IdentityRepositoryError::RouteOwnershipCollision
        | IdentityRepositoryError::KeyCollision
        | IdentityRepositoryError::StillAmbiguous => ApiError::Conflict {
            reason: error.to_string(),
        },
        IdentityRepositoryError::ReviewProposalInvalidated(reason) => ApiError::Conflict {
            reason: format!("review proposal invalidated: {reason}"),
        },
        IdentityRepositoryError::ReviewKindMismatch
        | IdentityRepositoryError::InvalidResolution => ApiError::BadRequest(error.to_string()),
        IdentityRepositoryError::Cancelled => ApiError::ServiceUnavailable,
        IdentityRepositoryError::Database(message) => ApiError::Internal(message),
        IdentityRepositoryError::AtomicRollback => ApiError::Internal(error.to_string()),
    }
}
