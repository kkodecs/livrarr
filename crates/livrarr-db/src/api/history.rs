//! History data access: `HistoryDb` trait + request type.

use livrarr_domain::history_events::HistoryDraft;

use crate::{DbError, EventType, HistoryEvent, HistoryFilter, UserId, WorkId};

/// History data access.
#[trait_variant::make(Send)]
pub trait HistoryDb: Send + Sync {
    /// List history events for a user, with optional filters (unbounded).
    async fn list_history(
        &self,
        user_id: UserId,
        filter: HistoryFilter,
    ) -> Result<Vec<HistoryEvent>, DbError>;

    /// List history events, paginated.
    async fn list_history_paginated(
        &self,
        user_id: UserId,
        filter: HistoryFilter,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<HistoryEvent>, i64), DbError>;

    /// Record a history event.
    async fn create_history_event(&self, req: CreateHistoryEventDbRequest) -> Result<(), DbError>;
}

pub struct CreateHistoryEventDbRequest {
    pub user_id: UserId,
    pub work_id: Option<WorkId>,
    pub event_type: EventType,
    pub data: serde_json::Value,
    /// `None` = the insert stamps the current time (live writers); `Some` =
    /// the caller-supplied historical fact date (backfill).
    pub date: Option<chrono::DateTime<chrono::Utc>>,
}

/// The one physical history-write chokepoint for live writers: builds the
/// insert request from a [`HistoryDraft`] and absorbs any insert error with a
/// logged warning. History is an observer, never an actor — this function
/// cannot fail, block, or retry the operation whose moment it records.
pub async fn record_history<D: HistoryDb>(db: &D, user_id: UserId, draft: HistoryDraft) {
    let event_type = draft.event_type;
    let work_id = draft.work_id;
    let req = CreateHistoryEventDbRequest {
        user_id,
        work_id,
        event_type,
        data: draft.data,
        date: draft.date,
    };
    if let Err(e) = db.create_history_event(req).await {
        tracing::warn!(?event_type, ?work_id, "history write failed: {e}");
    }
}
