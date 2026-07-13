//! Per-field provenance data access: `ProvenanceDb` trait + request type.

use crate::{
    DbError, FieldProvenance, MetadataProvider, ProvenanceSetter, UserId, WorkField, WorkId,
};

/// TEMP(pk-tdd): Request to set field provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetFieldProvenanceRequest {
    pub user_id: UserId,
    pub work_id: WorkId,
    pub field: WorkField,
    pub source: Option<MetadataProvider>,
    pub setter: ProvenanceSetter,
    pub cleared: bool,
}

#[trait_variant::make(Send)]
pub trait ProvenanceDb: Send + Sync {
    async fn set_field_provenance(&self, req: SetFieldProvenanceRequest) -> Result<(), DbError>;

    async fn set_field_provenance_batch(
        &self,
        reqs: Vec<SetFieldProvenanceRequest>,
    ) -> Result<(), DbError>;

    async fn get_field_provenance(
        &self,
        user_id: UserId,
        work_id: WorkId,
        field: WorkField,
    ) -> Result<Option<FieldProvenance>, DbError>;

    async fn list_work_provenance(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<FieldProvenance>, DbError>;

    async fn delete_field_provenance_batch(
        &self,
        user_id: UserId,
        work_id: WorkId,
        fields: Vec<WorkField>,
    ) -> Result<(), DbError>;

    async fn clear_work_provenance(&self, user_id: UserId, work_id: WorkId) -> Result<(), DbError>;
}
