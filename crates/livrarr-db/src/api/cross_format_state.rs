//! Cross-format resume state data access: `CrossFormatStateDb` trait.

use crate::{CrossFormatState, DbError, MediaType, UserId};

/// Per-(user, link) cross-format resume state access.
///
/// `furthest_ts` ADVANCEMENT is not exposed here — it happens inside the
/// extended [`crate::PlaybackProgressDb::upsert_progress`] transaction (REQ-016).
#[trait_variant::make(Send)]
pub trait CrossFormatStateDb: Send + Sync {
    /// The state row, or a zero-value default (furthest 0, no declines)
    /// WITHOUT inserting one.
    async fn get_or_default(
        &self,
        user_id: UserId,
        kash_link_id: i64,
    ) -> Result<CrossFormatState, DbError>;

    /// Record a decline threshold for one format; the other format's
    /// threshold and `furthest_ts` are untouched (REQ-017).
    async fn set_decline(
        &self,
        user_id: UserId,
        kash_link_id: i64,
        format: MediaType,
        declined_at_ts: f64,
    ) -> Result<(), DbError>;

    /// Explicit override: set `furthest_ts` (may DECREASE — REQ-018) and
    /// clear both decline thresholds.
    async fn sync_to(&self, user_id: UserId, kash_link_id: i64, ts: f64) -> Result<(), DbError>;
}
