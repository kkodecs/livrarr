use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::context::HasIdentityConflictService;
use crate::{ApiError, AuthContext};
use livrarr_domain::identity::*;
use livrarr_domain::services::IdentityConflictService;

#[derive(Debug, Serialize)]
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
pub struct ResolveRequest {
    pub action: ConflictResolutionAction,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize)]
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
            kind: format!("{:?}", c.kind),
            incoming_title: c.incoming.title.clone(),
            incoming_author: c.incoming.author_name.clone(),
            incoming_ol_key: c.incoming.ol_key.clone(),
            raised_at: c.raised_at.to_rfc3339(),
            raised_by: format!("{:?}", c.raised_by),
            status: format!("{:?}", c.status),
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

pub async fn resolve<S: HasIdentityConflictService>(
    State(ctx): State<S>,
    Path(id): Path<i64>,
    auth: AuthContext,
    Json(body): Json<ResolveRequest>,
) -> Result<Json<ResolveResponse>, ApiError> {
    ctx.identity_conflict_service()
        .resolve(id, auth.user.id, body.action, body.notes)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(ResolveResponse {
        status: "resolved".to_string(),
        action: body.action,
    }))
}

pub async fn dismiss<S: HasIdentityConflictService>(
    State(ctx): State<S>,
    Path(id): Path<i64>,
    auth: AuthContext,
) -> Result<StatusCode, ApiError> {
    ctx.identity_conflict_service()
        .dismiss(id, auth.user.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}
