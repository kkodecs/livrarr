use crate::identity::{LatencyTier, Resolution, WorkSeed};
use crate::services::work_identity::WorkIdentityError;
use crate::UserId;

#[trait_variant::make(Send)]
pub trait IdentityResolver: Send + Sync {
    async fn resolve(
        &self,
        user_id: UserId,
        seed: &WorkSeed,
        tier: LatencyTier,
    ) -> Result<Resolution, WorkIdentityError>;
}
