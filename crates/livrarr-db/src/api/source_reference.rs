//! Source-reference data access: the typed provider context saved with a
//! Work at creation. Rows are source facts, never identity routes.

use crate::{DbError, SourceReference, UserId, WorkId};

#[trait_variant::make(Send)]
pub trait SourceReferenceDb: Send + Sync {
    /// Every saved source reference for the Work, in a stable order.
    async fn list_source_references(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<SourceReference>, DbError>;
}
