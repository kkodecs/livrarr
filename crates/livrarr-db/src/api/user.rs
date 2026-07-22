//! User data access: `UserDb` trait + request types.

use crate::{DbError, User, UserId, UserRole};

/// User data access.
///
/// Satisfies: AUTH-010, AUTH-011, AUTH-012, AUTH-013
#[trait_variant::make(Send)]
pub trait UserDb: Send + Sync {
    /// Get user by ID.
    async fn get_user(&self, id: UserId) -> Result<User, DbError>;

    /// Get user by username (case-insensitive).
    async fn get_user_by_username(&self, username: &str) -> Result<User, DbError>;

    /// Get user by API key hash.
    ///
    /// Satisfies: AUTH-007
    async fn get_user_by_api_key_hash(&self, hash: &str) -> Result<User, DbError>;

    /// List all users.
    async fn list_users(&self) -> Result<Vec<User>, DbError>;

    /// Create user. Returns created user with generated ID.
    async fn create_user(&self, req: CreateUserDbRequest) -> Result<User, DbError>;

    /// Update user fields. Null fields mean "keep existing."
    async fn update_user(&self, id: UserId, req: UpdateUserDbRequest) -> Result<User, DbError>;

    /// Delete user by ID. Cascades to all user-scoped data.
    ///
    /// Satisfies: AUTH-011
    async fn delete_user(&self, id: UserId) -> Result<(), DbError>;

    /// Count users with admin role (for last-admin check).
    async fn count_admins(&self) -> Result<i64, DbError>;

    /// True if a pending-setup principal exists (setup not yet completed).
    ///
    /// Hash-free existence check — safe to call before any password hashing.
    /// This is the true authority for "is setup complete," independent of
    /// which user id happens to hold the placeholder row (that row can be
    /// deleted later while another admin remains).
    async fn has_pending_setup(&self) -> Result<bool, DbError>;

    /// Complete setup: update placeholder admin with real credentials.
    /// Atomic conditional: only succeeds if setup_pending = true.
    ///
    /// Satisfies: AUTH-010
    async fn complete_setup(&self, req: CompleteSetupDbRequest) -> Result<User, DbError>;

    /// Update API key hash for a user.
    async fn update_api_key_hash(&self, user_id: UserId, hash: &str) -> Result<(), DbError>;
}

pub struct CreateUserDbRequest {
    pub username: String,
    pub password_hash: String,
    pub role: UserRole,
    pub api_key_hash: String,
}

pub struct UpdateUserDbRequest {
    pub username: Option<String>,
    pub password_hash: Option<String>,
    pub role: Option<UserRole>,
}

pub struct CompleteSetupDbRequest {
    pub username: String,
    pub password_hash: String,
    pub api_key_hash: String,
}
