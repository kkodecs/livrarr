//! History data access: `HistoryDb` trait + request type.

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
}
