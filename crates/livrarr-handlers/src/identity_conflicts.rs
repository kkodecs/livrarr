use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::context::{
    HasIdentityConflictService, HasIdentityLayerRepository, HasIdentityRoadService,
};
use crate::{ApiError, AuthContext};
use livrarr_domain::identity::*;
use livrarr_domain::identity_layer::{IdentityRoadService, RouteKey, WorkIdentityRepository as _};
use livrarr_domain::services::IdentityConflictService;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityConflictDto {
    pub id: i64,
    pub existing_work_id: i64,
    pub kind: String,
    pub incoming_title: String,
    pub incoming_author: String,
    pub incoming_ol_key: Option<String>,
    pub raised_at: String,
    pub raised_by: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityConflictDetailDto {
    pub id: i64,
    pub existing_work_id: i64,
    pub kind: IdentityConflictKind,
    pub incoming: IncomingConflictPayload,
    pub raised_at: String,
    pub raised_by: ConflictSource,
    pub raised_source_path: Option<String>,
    pub status: ConflictStatus,
    pub resolved_at: Option<String>,
    pub resolution_action: Option<ConflictResolutionAction>,
    pub resolution_notes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveRequest {
    pub action: ConflictResolutionAction,
    pub notes: Option<String>,
    #[serde(default)]
    pub surviving_routes: Option<Vec<RouteKey>>,
    #[serde(default)]
    pub target_edition: Option<i64>,
    #[serde(default)]
    pub winning_work_id: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveResponse {
    pub status: String,
    pub action: ConflictResolutionAction,
}

pub async fn list_open<S: HasIdentityConflictService>(
    State(ctx): State<S>,
    auth: AuthContext,
) -> Result<Json<Vec<IdentityConflictDto>>, ApiError> {
    let conflicts = ctx
        .identity_conflict_service()
        .list_open(auth.user.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let dtos: Vec<IdentityConflictDto> = conflicts
        .into_iter()
        .map(|c| IdentityConflictDto {
            id: c.id,
            existing_work_id: c.existing_work_id,
            kind: serde_json::to_value(c.kind)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default(),
            incoming_title: c.incoming.title.clone(),
            incoming_author: c.incoming.author_name.clone(),
            incoming_ol_key: c.incoming.ol_key.clone(),
            raised_at: c.raised_at.to_rfc3339(),
            raised_by: serde_json::to_value(c.raised_by)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default(),
            status: serde_json::to_value(c.status)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default(),
        })
        .collect();

    Ok(Json(dtos))
}

pub async fn get_detail<S: HasIdentityConflictService>(
    State(ctx): State<S>,
    Path(id): Path<i64>,
    auth: AuthContext,
) -> Result<Json<IdentityConflictDetailDto>, ApiError> {
    let conflict = ctx
        .identity_conflict_service()
        .get(id, auth.user.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound)?;

    Ok(Json(IdentityConflictDetailDto {
        id: conflict.id,
        existing_work_id: conflict.existing_work_id,
        kind: conflict.kind,
        incoming: conflict.incoming,
        raised_at: conflict.raised_at.to_rfc3339(),
        raised_by: conflict.raised_by,
        raised_source_path: conflict.raised_source_path,
        status: conflict.status,
        resolved_at: conflict.resolved_at.map(|dt| dt.to_rfc3339()),
        resolution_action: conflict.resolution_action,
        resolution_notes: conflict.resolution_notes,
    }))
}

pub async fn resolve<S: HasIdentityRoadService + HasIdentityLayerRepository>(
    State(ctx): State<S>,
    Path(id): Path<i64>,
    auth: AuthContext,
    Json(body): Json<ResolveRequest>,
) -> Result<Json<ResolveResponse>, ApiError> {
    let actor = livrarr_domain::identity_layer::ReviewActor::AuthenticatedUser {
        user_id: auth.user.id,
    };
    let pending = ctx
        .identity_layer_repository()
        .load_pending_conflict_review(actor.clone(), id)
        .await
        .map_err(map_repository_error)?;
    let current_routes = match pending.work_id {
        Some(work_id) => ctx
            .identity_layer_repository()
            .read_captured_identity(auth.user.id, work_id)
            .await
            .map_err(map_repository_error)?
            .active_routes
            .into_iter()
            .map(|route| RouteKey {
                provider: route.provider,
                kind: route.kind,
                value: route.provider_scoped_id,
            })
            .collect(),
        None => Vec::new(),
    };
    let surviving_routes = body.surviving_routes.unwrap_or(current_routes);
    let action = match body.action {
        ConflictResolutionAction::KeepExisting => {
            livrarr_domain::identity_layer::IdentityConflictResolution::Reject { surviving_routes }
        }
        ConflictResolutionAction::AcceptSeparate => {
            let winning_work_id = body.winning_work_id.ok_or_else(|| ApiError::Conflict {
                reason: "accept-separate requires winningWorkId".to_string(),
            })?;
            livrarr_domain::identity_layer::IdentityConflictResolution::DifferentWork {
                winning_work_id,
                surviving_routes,
                target_edition: body.target_edition,
            }
        }
        ConflictResolutionAction::ReplaceAnchor | ConflictResolutionAction::Merge => {
            livrarr_domain::identity_layer::IdentityConflictResolution::Accept {
                surviving_routes,
                target_edition: body.target_edition,
            }
        }
    };
    ctx.identity_road_service()
        .resolve_review(
            actor,
            livrarr_domain::identity_layer::ReviewResolutionCommand::IdentityConflict {
                card_id: pending.id,
                expected_generation: pending.generation,
                action,
            },
        )
        .await
        .map_err(map_road_error)?;

    Ok(Json(ResolveResponse {
        status: "resolved".to_string(),
        action: body.action,
    }))
}

pub async fn dismiss<S: HasIdentityRoadService + HasIdentityLayerRepository>(
    State(ctx): State<S>,
    Path(id): Path<i64>,
    auth: AuthContext,
) -> Result<StatusCode, ApiError> {
    let actor = livrarr_domain::identity_layer::ReviewActor::AuthenticatedUser {
        user_id: auth.user.id,
    };
    let pending = ctx
        .identity_layer_repository()
        .load_pending_conflict_review(actor.clone(), id)
        .await
        .map_err(map_repository_error)?;
    let surviving_routes = match pending.work_id {
        Some(work_id) => ctx
            .identity_layer_repository()
            .read_captured_identity(auth.user.id, work_id)
            .await
            .map_err(map_repository_error)?
            .active_routes
            .into_iter()
            .map(|route| RouteKey {
                provider: route.provider,
                kind: route.kind,
                value: route.provider_scoped_id,
            })
            .collect(),
        None => Vec::new(),
    };
    ctx.identity_road_service()
        .resolve_review(
            actor,
            livrarr_domain::identity_layer::ReviewResolutionCommand::IdentityConflict {
                card_id: pending.id,
                expected_generation: pending.generation,
                action: livrarr_domain::identity_layer::IdentityConflictResolution::Reject {
                    surviving_routes,
                },
            },
        )
        .await
        .map_err(map_road_error)?;

    Ok(StatusCode::NO_CONTENT)
}

fn map_repository_error(
    error: livrarr_domain::identity_layer::IdentityRepositoryError,
) -> ApiError {
    match error {
        livrarr_domain::identity_layer::IdentityRepositoryError::NotFound => ApiError::NotFound,
        livrarr_domain::identity_layer::IdentityRepositoryError::StaleGeneration => {
            ApiError::Conflict {
                reason: error.to_string(),
            }
        }
        livrarr_domain::identity_layer::IdentityRepositoryError::UnauthorizedScope => {
            ApiError::Forbidden
        }
        other => ApiError::Internal(other.to_string()),
    }
}

fn map_road_error(error: livrarr_domain::identity_layer::IdentityRoadError) -> ApiError {
    match error {
        livrarr_domain::identity_layer::IdentityRoadError::NotFound => ApiError::NotFound,
        livrarr_domain::identity_layer::IdentityRoadError::StaleGeneration => ApiError::Conflict {
            reason: error.to_string(),
        },
        livrarr_domain::identity_layer::IdentityRoadError::UnauthorizedScope => ApiError::Forbidden,
        other => ApiError::Internal(other.to_string()),
    }
}
