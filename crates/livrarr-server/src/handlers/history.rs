use axum::extract::{Query, State};
use axum::Json;

use crate::state::AppState;
use crate::{ApiError, AuthContext, HistoryResponse};
use livrarr_db::{HistoryDb, HistoryFilter};
use livrarr_domain::EventType;

#[derive(serde::Deserialize)]
pub struct HistoryQuery {
    #[serde(rename = "eventType")]
    pub event_type: Option<EventType>,
    #[serde(rename = "workId")]
    pub work_id: Option<i64>,
    #[serde(rename = "startDate")]
    pub start_date: Option<String>,
    #[serde(rename = "endDate")]
    pub end_date: Option<String>,
}

/// GET /api/v1/history
pub async fn list(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Vec<HistoryResponse>>, ApiError> {
    let start_date = q
        .start_date
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));
    let end_date = q
        .end_date
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));

    let filter = HistoryFilter {
        event_type: q.event_type,
        work_id: q.work_id,
        start_date,
        end_date,
    };

    let events = state.db.list_history(ctx.user.id, filter).await?;
    Ok(Json(
        events
            .iter()
            .map(|e| HistoryResponse {
                id: e.id,
                work_id: e.work_id,
                event_type: e.event_type,
                data: e.data.clone(),
                date: e.date.to_rfc3339(),
            })
            .collect(),
    ))
}
