use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::context::{HasHistoryService, HasWorkIdentityRepository, HasWorkService};
use crate::{ApiError, AuthContext};
use livrarr_domain::history_events;
use livrarr_domain::identity::{AnchorSetter, Candidate};
use livrarr_domain::services::{
    HistoryService, WorkIdentityError, WorkIdentityRepository, WorkService, WorkServiceError,
};

/// One ranked candidate behind a `NeedsReview` park (AC-013) — a genuinely
/// computed similarity score, never the historical hardcoded 1.0 (REQ-010).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCandidateDto {
    pub candidate_id: String,
    pub title: String,
    pub author_name: String,
    pub language: Option<String>,
    pub ol_key: Option<String>,
    pub gr_key: Option<String>,
    pub hc_key: Option<String>,
    pub isbn_13: Option<String>,
    pub asin: Option<String>,
    pub cover_url: Option<String>,
    pub sources: Vec<String>,
    pub title_jaccard: f64,
    pub author_overlap: u32,
    /// Set when this candidate's identity is already claimed by another work
    /// in the library — informational only; picking it does not merge or
    /// otherwise touch that other work.
    pub existing_work_id: Option<i64>,
}

/// A work parked `NeedsReview`, with its persisted candidate set (AC-013).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewParkDto {
    pub work_id: i64,
    pub title: String,
    pub author_name: String,
    pub cover_url: Option<String>,
    pub candidates: Vec<ReviewCandidateDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveReviewRequest {
    pub candidate_id: String,
}

fn candidate_to_dto(c: Candidate) -> ReviewCandidateDto {
    ReviewCandidateDto {
        candidate_id: c.candidate_id.0,
        title: c.anchors.title,
        author_name: c.anchors.author_name,
        language: c.anchors.language,
        ol_key: c.anchors.ol_key,
        gr_key: c.anchors.gr_key,
        hc_key: c.anchors.hc_key,
        isbn_13: c.anchors.isbn_13,
        asin: c.anchors.asin,
        cover_url: c.cover_url,
        sources: c
            .sources
            .iter()
            .filter_map(|s| {
                serde_json::to_value(s)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
            })
            .collect(),
        title_jaccard: c.score.title_jaccard,
        author_overlap: c.score.author_overlap,
        existing_work_id: c.existing_work_id,
    }
}

/// List every work parked `NeedsReview` with its persisted, real-scored
/// candidates (AC-013 review surface).
pub async fn list<S: HasWorkIdentityRepository>(
    State(state): State<S>,
    ctx: AuthContext,
) -> Result<Json<Vec<ReviewParkDto>>, ApiError> {
    let works = state
        .work_identity_repo()
        .list_needs_review_works(ctx.user.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut out = Vec::with_capacity(works.len());
    for w in works {
        let candidates = state
            .work_identity_repo()
            .get_review_candidates(w.id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .unwrap_or_default();

        out.push(ReviewParkDto {
            work_id: w.id,
            title: w.title,
            author_name: w.author_name,
            cover_url: w.cover_url,
            candidates: candidates.into_iter().map(candidate_to_dto).collect(),
        });
    }
    Ok(Json(out))
}

pub async fn resolve<S: HasWorkIdentityRepository + HasWorkService + HasHistoryService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(work_id): Path<i64>,
    Json(body): Json<ResolveReviewRequest>,
) -> Result<StatusCode, ApiError> {
    // Ownership check before any mutation (P4 — no cross-user resolve).
    let work = state
        .work_service()
        .get(ctx.user.id, work_id)
        .await
        .map_err(|e| match e {
            WorkServiceError::NotFound => ApiError::NotFound,
            other => ApiError::Internal(other.to_string()),
        })?;

    // Candidates + identity generation read together (one transaction): the
    // coherent basis for the apply's first-statement claim (identity-edit r4
    // §Writer coverage — review apply).
    let (expected_generation, candidates) = state
        .work_identity_repo()
        .read_review_candidates_with_generation(work_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let candidates = candidates.ok_or(ApiError::NotFound)?;

    let chosen = candidates
        .into_iter()
        .find(|c| c.candidate_id.0 == body.candidate_id)
        .ok_or(ApiError::NotFound)?;

    state
        .work_identity_repo()
        .apply_review_candidate_claimed(work_id, &chosen, AnchorSetter::User, expected_generation)
        .await
        .map_err(|e| match e {
            // A different identity mutation won the generation claim since
            // the read above — the dedicated stale 409, never NotParked.
            WorkIdentityError::StaleIdentity => ApiError::ConflictDetailed {
                message: "identity changed; reload review candidates".into(),
                details: crate::types::api_error::ErrorDetails::code("identity_review_stale"),
            },
            // The park settled between our read and the apply (or was never
            // parked despite a stale candidates row) — 409, mirroring the
            // ConflictError::AlreadyResolved mapping.
            WorkIdentityError::NotParked => ApiError::Conflict {
                reason: e.to_string(),
            },
            other => ApiError::Internal(other.to_string()),
        })?;

    state
        .history_service()
        .record(
            ctx.user.id,
            history_events::identity_resolved(
                work_id,
                &work.title,
                "review-candidate-apply",
                format!("{} — {}", chosen.anchors.title, chosen.anchors.author_name),
            ),
        )
        .await;

    Ok(StatusCode::NO_CONTENT)
}

/// Dismiss a park without adopting any candidate (AC-013): the work reverts
/// to Pending, standing alone — no merge. A duplicate surfaced this way is
/// one click from the separate merge-two-works action.
pub async fn dismiss<S: HasWorkIdentityRepository + HasWorkService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(work_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state
        .work_service()
        .get(ctx.user.id, work_id)
        .await
        .map_err(|e| match e {
            WorkServiceError::NotFound => ApiError::NotFound,
            other => ApiError::Internal(other.to_string()),
        })?;

    // Generation read coherently with the park state; the dismiss's first
    // statement is the conditional claim (same contract as resolve).
    let (expected_generation, _candidates) = state
        .work_identity_repo()
        .read_review_candidates_with_generation(work_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    state
        .work_identity_repo()
        .dismiss_review_claimed(work_id, expected_generation)
        .await
        .map_err(|e| match e {
            WorkIdentityError::StaleIdentity => ApiError::ConflictDetailed {
                message: "identity changed; reload review candidates".into(),
                details: crate::types::api_error::ErrorDetails::code("identity_review_stale"),
            },
            // Not parked (or no longer parked) — never downgrade a settled
            // work; 409, same mapping as resolve.
            WorkIdentityError::NotParked => ApiError::Conflict {
                reason: e.to_string(),
            },
            other => ApiError::Internal(other.to_string()),
        })?;

    Ok(StatusCode::NO_CONTENT)
}
