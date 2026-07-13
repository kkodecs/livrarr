//! Provider retry state data access: `ProviderRetryStateDb` trait + request type.

use crate::{DbError, MetadataProvider, OutcomeClass, UserId, WorkId};

/// TEMP(pk-tdd): Provider retry state record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRetryState {
    pub user_id: UserId,
    pub work_id: WorkId,
    pub provider: MetadataProvider,
    pub attempts: u32,
    pub last_outcome: Option<OutcomeClass>,
    pub next_attempt_at: Option<chrono::DateTime<chrono::Utc>>,
    pub normalized_payload_json: Option<String>,
}

#[trait_variant::make(Send)]
pub trait ProviderRetryStateDb: Send + Sync {
    async fn get_retry_state(
        &self,
        user_id: UserId,
        work_id: WorkId,
        provider: MetadataProvider,
    ) -> Result<Option<ProviderRetryState>, DbError>;

    async fn list_retry_states(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<ProviderRetryState>, DbError>;

    async fn record_will_retry(
        &self,
        user_id: UserId,
        work_id: WorkId,
        provider: MetadataProvider,
        next_attempt_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<ProviderRetryState, DbError>;

    /// Same row shape as [`Self::record_will_retry`] but for a breaker-open
    /// pause (`WillRetryReason::CircuitOpen`, R-11): sets `next_attempt_at` /
    /// `last_attempt_at` / `last_outcome = 'will_retry'` WITHOUT incrementing
    /// `attempts` — a paused provider must not spend retry budget while its
    /// breaker is open.
    async fn record_will_retry_paused(
        &self,
        user_id: UserId,
        work_id: WorkId,
        provider: MetadataProvider,
        next_attempt_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<ProviderRetryState, DbError>;

    async fn record_terminal_outcome(
        &self,
        user_id: UserId,
        work_id: WorkId,
        provider: MetadataProvider,
        outcome: OutcomeClass,
        normalized_payload_json: Option<String>,
    ) -> Result<(), DbError>;

    async fn reset_all_retry_states(&self, user_id: UserId, work_id: WorkId)
        -> Result<(), DbError>;

    async fn list_works_due_for_retry(
        &self,
        user_id: UserId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<(WorkId, MetadataProvider)>, DbError>;

    async fn list_works_with_terminal_provider_rows(
        &self,
        user_id: UserId,
    ) -> Result<Vec<(WorkId, Vec<MetadataProvider>)>, DbError>;

    async fn reset_not_configured_outcomes(
        &self,
        provider: MetadataProvider,
    ) -> Result<u64, DbError>;
}
