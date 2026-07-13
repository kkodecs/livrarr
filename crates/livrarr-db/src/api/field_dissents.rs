//! Per-work field dissent data access: `FieldDissentDb` trait.

use crate::{DbError, FieldDissent, UserId, WorkId};

/// Per-work field dissents (REQ-014): queryable record of excluded merge
/// contributions. A work's new merge generation supersedes its prior rows.
#[trait_variant::make(Send)]
pub trait FieldDissentDb: Send + Sync {
    async fn record_field_dissents(
        &self,
        user_id: UserId,
        work_id: WorkId,
        dissents: Vec<FieldDissent>,
    ) -> Result<(), DbError>;

    async fn list_field_dissents(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<FieldDissent>, DbError>;
}
