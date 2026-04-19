use livrarr_domain::{EventType, HistoryFilter, HistoryId, UserId, WorkId};
use serde::{Deserialize, Serialize};

use super::api_error::ApiError;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryResponse {
    pub id: HistoryId,
    pub work_id: Option<WorkId>,
    pub event_type: EventType,
    pub data: serde_json::Value,
    pub date: String,
}

#[trait_variant::make(Send)]
pub trait HistoryApi: Send + Sync {
    async fn list(
        &self,
        user_id: UserId,
        target_user_id: Option<UserId>,
        filter: HistoryFilter,
    ) -> Result<Vec<HistoryResponse>, ApiError>;
}
