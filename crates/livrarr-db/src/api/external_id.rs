//! Typed external identifier data access: `ExternalIdDb` trait + request type.

use crate::{DbError, ExternalIdRowId, ExternalIdType, UserId, WorkId};

/// TEMP(pk-tdd): A typed external identifier for a work (DB layer).
/// Named ExternalId to match the behavioral test type expectations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalId {
    pub id: ExternalIdRowId,
    pub user_id: UserId,
    pub work_id: WorkId,
    pub id_type: ExternalIdType,
    pub id_value: String,
}

/// TEMP(pk-tdd): Request to upsert a typed external identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpsertExternalIdRequest {
    pub work_id: WorkId,
    pub id_type: ExternalIdType,
    pub id_value: String,
}

#[trait_variant::make(Send)]
pub trait ExternalIdDb: Send + Sync {
    async fn upsert_external_id(
        &self,
        user_id: UserId,
        req: UpsertExternalIdRequest,
    ) -> Result<(), DbError>;

    async fn upsert_external_ids_batch(
        &self,
        user_id: UserId,
        reqs: Vec<UpsertExternalIdRequest>,
    ) -> Result<(), DbError>;

    async fn list_external_ids(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<ExternalId>, DbError>;
}
