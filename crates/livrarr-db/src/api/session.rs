//! Session data access: `SessionDb` trait.

use crate::{DbError, Session, UserId};

/// Session data access.
///
/// Satisfies: AUTH-005, AUTH-006, AUTH-014
#[trait_variant::make(Send)]
pub trait SessionDb: Send + Sync {
    /// Get session by token hash. Returns None if not found or expired.
    async fn get_session(&self, token_hash: &str) -> Result<Option<Session>, DbError>;

    /// Create session.
    async fn create_session(&self, session: &Session) -> Result<(), DbError>;

    /// Delete session (logout).
    async fn delete_session(&self, token_hash: &str) -> Result<(), DbError>;

    /// Extend session expiry (for rolling persistent sessions).
    ///
    /// Satisfies: AUTH-005 (debounced rolling -- extend only when <24h remaining)
    async fn extend_session(
        &self,
        token_hash: &str,
        new_expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), DbError>;

    /// Delete all sessions for a user (e.g. after password change).
    async fn delete_user_sessions(&self, user_id: UserId) -> Result<u64, DbError>;

    /// Delete all expired sessions.
    ///
    /// Satisfies: AUTH-014
    async fn delete_expired_sessions(&self) -> Result<u64, DbError>;
}
