use crate::identity::*;
use crate::WorkId;

#[derive(Debug, thiserror::Error)]
pub enum WorkIdentityError {
    #[error("invalid anchor value")]
    InvalidAnchorValue,
    #[error("anchor not found")]
    AnchorNotFound,
    #[error("database error: {0}")]
    Db(String),
}

#[trait_variant::make(Send)]
pub trait WorkIdentityRepository: Send + Sync {
    async fn confirm_ol_anchor(
        &self,
        work_id: WorkId,
        ol_key: &str,
        setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError>;

    async fn supersede_ol_anchor(
        &self,
        work_id: WorkId,
        old_ol_key: &str,
        new_ol_key: &str,
        setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError>;

    async fn set_identity_pending(
        &self,
        work_id: WorkId,
        reason: PendingReason,
        setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError>;

    async fn verify_anchor_cache_consistency(
        &self,
    ) -> Result<Vec<ConsistencyDivergence>, WorkIdentityError>;

    async fn find_work_by_anchor(
        &self,
        anchor_type: &AnchorType,
        anchor_value: &str,
    ) -> Result<Option<WorkId>, WorkIdentityError>;

    async fn list_anchors(
        &self,
        work_id: WorkId,
    ) -> Result<Vec<WorkIdentityAnchor>, WorkIdentityError>;
}
