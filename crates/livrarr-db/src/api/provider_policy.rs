//! Provider policy snapshot loading: `ProviderPolicyDb` trait.

use crate::DbError;

/// Loads the provider-policy table into the in-memory snapshot (REQ-003). The
/// server holds + atomically swaps the snapshot; runtime lookup is a memory read.
#[trait_variant::make(Send)]
pub trait ProviderPolicyDb: Send + Sync {
    async fn load_provider_policy_snapshot(
        &self,
    ) -> Result<livrarr_domain::services::ProviderPolicySnapshot, DbError>;
}
