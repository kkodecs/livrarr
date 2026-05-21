use crate::identity::*;
use crate::UserId;

#[derive(Debug, thiserror::Error)]
pub enum ConflictError {
    #[error("conflict not found")]
    NotFound,
    #[error("conflict already resolved")]
    AlreadyResolved,
    #[error("serialization failed: {0}")]
    SerializationFailed(String),
    #[error("work service failed: {0}")]
    WorkServiceFailed(String),
    #[error("corrupted payload: {0}")]
    CorruptedPayload(String),
    #[error("database error: {0}")]
    Db(String),
}

#[trait_variant::make(Send)]
pub trait IdentityConflictService: Send + Sync {
    async fn raise(&self, conflict: NewIdentityConflict) -> Result<i64, ConflictError>;

    async fn list_open(&self, user_id: UserId) -> Result<Vec<IdentityConflict>, ConflictError>;

    async fn get(
        &self,
        id: i64,
        user_id: UserId,
    ) -> Result<Option<IdentityConflict>, ConflictError>;

    async fn resolve(
        &self,
        id: i64,
        user_id: UserId,
        action: ConflictResolutionAction,
        notes: Option<String>,
    ) -> Result<(), ConflictError>;

    async fn dismiss(&self, id: i64, user_id: UserId) -> Result<(), ConflictError>;
}
