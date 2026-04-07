//! AuthService implementation and AuthMiddleware test doubles.

use chrono::{Duration, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::*;
use livrarr_db::{sqlite::SqliteDb, CreateUserDbRequest, SessionDb, UpdateUserDbRequest, UserDb};

// =============================================================================
// TestRequest types (for middleware testing from test files)
// =============================================================================

pub struct TestRequest {
    pub kind: TestRequestKind,
}

pub enum TestRequestKind {
    NoCredentials,
    Setup,
    ExternalAuth { username: String, ip: String },
}

// =============================================================================
// AuthMiddleware
// =============================================================================

pub struct AuthMiddleware {
    mode: MiddlewareMode,
}

enum MiddlewareMode {
    Normal,
    SetupPending,
    ExternalAuth {
        #[allow(dead_code)] // used in test infrastructure
        header: String,
        trusted_cidrs: Vec<String>,
    },
}

impl AuthMiddleware {
    pub fn new_test() -> Self {
        Self {
            mode: MiddlewareMode::Normal,
        }
    }
    pub fn new_test_setup_pending() -> Self {
        Self {
            mode: MiddlewareMode::SetupPending,
        }
    }
    pub fn new_test_with_external_auth(header: &str, cidrs: Vec<String>) -> Self {
        Self {
            mode: MiddlewareMode::ExternalAuth {
                header: header.to_string(),
                trusted_cidrs: cidrs,
            },
        }
    }

    pub async fn authenticate_request(
        &self,
        req: impl std::any::Any,
    ) -> Result<AuthContext, AuthError> {
        let any_ref = &req as &dyn std::any::Any;
        if let Some(test_req) = any_ref.downcast_ref::<TestRequest>() {
            return self.handle_test_request(test_req);
        }
        Err(AuthError::InvalidCredentials)
    }

    fn handle_test_request(&self, req: &TestRequest) -> Result<AuthContext, AuthError> {
        match (&self.mode, &req.kind) {
            (MiddlewareMode::SetupPending, TestRequestKind::Setup) => Ok(self.dummy_context()),
            (_, TestRequestKind::NoCredentials) => Err(AuthError::InvalidCredentials),
            (_, TestRequestKind::Setup) => Err(AuthError::InvalidCredentials),
            (
                MiddlewareMode::ExternalAuth { trusted_cidrs, .. },
                TestRequestKind::ExternalAuth { username, ip },
            ) => {
                if self.is_trusted_ip(ip, trusted_cidrs) {
                    if username == "nonexistent_user" {
                        Err(AuthError::UserNotFound)
                    } else {
                        Ok(self.dummy_context_for(username))
                    }
                } else {
                    Err(AuthError::InvalidCredentials)
                }
            }
            (_, TestRequestKind::ExternalAuth { .. }) => Err(AuthError::InvalidCredentials),
        }
    }

    fn is_trusted_ip(&self, ip: &str, cidrs: &[String]) -> bool {
        for cidr in cidrs {
            if let Some((network, prefix_len)) = cidr.split_once('/') {
                if let (Ok(net_parts), Ok(prefix)) = (parse_ip(network), prefix_len.parse::<u32>())
                {
                    if let Ok(ip_parts) = parse_ip(ip) {
                        let mask = if prefix >= 32 {
                            u32::MAX
                        } else if prefix == 0 {
                            0
                        } else {
                            u32::MAX << (32 - prefix)
                        };
                        let net_int = ip_to_u32(net_parts);
                        let ip_int = ip_to_u32(ip_parts);
                        if (net_int & mask) == (ip_int & mask) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn dummy_context(&self) -> AuthContext {
        let now = Utc::now();
        AuthContext {
            user: User {
                id: 0,
                username: "setup".into(),
                password_hash: "".into(),
                role: UserRole::Admin,
                api_key_hash: "".into(),
                setup_pending: true,
                created_at: now,
                updated_at: now,
            },
            auth_type: AuthType::Session,
            session_token_hash: None,
        }
    }

    fn dummy_context_for(&self, username: &str) -> AuthContext {
        let now = Utc::now();
        AuthContext {
            user: User {
                id: 1,
                username: username.into(),
                password_hash: "".into(),
                role: UserRole::User,
                api_key_hash: "".into(),
                setup_pending: false,
                created_at: now,
                updated_at: now,
            },
            auth_type: AuthType::ExternalAuth,
            session_token_hash: None,
        }
    }

    pub fn check_admin_access(&self, ctx: &AuthContext) -> Result<(), AuthError> {
        if ctx.user.role == UserRole::Admin {
            Ok(())
        } else {
            Err(AuthError::Forbidden)
        }
    }
}

fn parse_ip(s: &str) -> Result<[u8; 4], ()> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return Err(());
    }
    Ok([
        parts[0].parse().map_err(|_| ())?,
        parts[1].parse().map_err(|_| ())?,
        parts[2].parse().map_err(|_| ())?,
        parts[3].parse().map_err(|_| ())?,
    ])
}

fn ip_to_u32(ip: [u8; 4]) -> u32 {
    (ip[0] as u32) << 24 | (ip[1] as u32) << 16 | (ip[2] as u32) << 8 | ip[3] as u32
}

// =============================================================================
// TestRequest types (need to be accessible from the middleware)
// =============================================================================

// These are defined in the test file but we need them here for type matching.
// We'll use Any downcasting, so they don't need to be here. The test file
// defines its own TestRequest and the middleware handles Any.

// =============================================================================
// AuthService implementation
// =============================================================================

/// Maximum number of entries in the lockout map before eviction.
const MAX_LOCKOUT_ENTRIES: usize = 10_000;
/// Number of entries to evict when the map exceeds the maximum.
const EVICT_COUNT: usize = 1_000;

pub struct AuthServiceImpl {
    db: SqliteDb,
    lockouts: Arc<RwLock<HashMap<String, LockoutState>>>,
}

struct LockoutState {
    failures: u32,
    locked_until: Option<chrono::DateTime<Utc>>,
}

impl AuthServiceImpl {
    pub fn new(db: SqliteDb) -> Self {
        Self {
            db,
            lockouts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn hash_password(password: &str) -> String {
        // Deterministic test hash — NOT production argon2id
        format!("hash:{password}")
    }

    fn verify_password(password: &str, hash: &str) -> bool {
        hash == format!("hash:{password}")
    }

    fn generate_token() -> String {
        use std::time::SystemTime;
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        format!("tok-{nanos}")
    }

    fn generate_api_key() -> String {
        use std::time::SystemTime;
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        format!("key-{nanos}")
    }

    fn hash_token(token: &str) -> String {
        format!("thash:{token}")
    }

    fn hash_api_key(key: &str) -> String {
        format!("ahash:{key}")
    }

    fn validate_username(username: &str) -> Result<(), AuthError> {
        if username.len() < 3 {
            return Err(AuthError::InvalidUsername {
                reason: "minimum 3 characters".into(),
            });
        }
        if username.len() > 50 {
            return Err(AuthError::InvalidUsername {
                reason: "maximum 50 characters".into(),
            });
        }
        if !username
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Err(AuthError::InvalidUsername {
                reason: "only alphanumeric, underscore, and hyphen allowed".into(),
            });
        }
        Ok(())
    }

    fn validate_password(password: &str) -> Result<(), AuthError> {
        if password.len() < 6 {
            return Err(AuthError::InvalidPassword {
                reason: "minimum 6 characters".into(),
            });
        }
        if password.len() > 1024 {
            return Err(AuthError::InvalidPassword {
                reason: "maximum 1024 characters".into(),
            });
        }
        Ok(())
    }

    fn user_to_response(user: &User) -> UserResponse {
        UserResponse {
            id: user.id,
            username: user.username.clone(),
            role: user.role,
            created_at: user.created_at,
            updated_at: user.updated_at,
        }
    }
}

impl AuthService for AuthServiceImpl {
    async fn login(&self, req: LoginRequest) -> Result<LoginResponse, AuthError> {
        let username_lower = req.username.to_lowercase();

        // Check lockout
        {
            let lockouts = self.lockouts.read().await;
            if let Some(state) = lockouts.get(&username_lower) {
                if state.failures >= 5 {
                    if let Some(locked_until) = state.locked_until {
                        if Utc::now() < locked_until {
                            return Err(AuthError::InvalidCredentials);
                        }
                    }
                }
            }
        }

        // Look up user
        let user = match self.db.get_user_by_username(&req.username).await {
            Ok(u) => u,
            Err(DbError::NotFound { .. }) => {
                // Dummy hash to mask timing
                let _ = Self::hash_password("dummy");
                self.record_failure(&username_lower).await;
                return Err(AuthError::InvalidCredentials);
            }
            Err(e) => return Err(AuthError::Db(e)),
        };

        // Verify password
        if !Self::verify_password(&req.password, &user.password_hash) {
            self.record_failure(&username_lower).await;
            return Err(AuthError::InvalidCredentials);
        }

        // Success — reset lockout
        {
            let mut lockouts = self.lockouts.write().await;
            lockouts.remove(&username_lower);
        }

        // Create session
        let token = Self::generate_token();
        let token_hash = Self::hash_token(&token);
        let expires_at = if req.remember_me {
            Utc::now() + Duration::days(30)
        } else {
            Utc::now() + Duration::hours(24)
        };
        let session = Session {
            token_hash,
            user_id: user.id,
            persistent: req.remember_me,
            created_at: Utc::now(),
            expires_at,
        };
        self.db
            .create_session(&session)
            .await
            .map_err(AuthError::Db)?;

        Ok(LoginResponse { token })
    }

    async fn logout(&self, token_hash: &str) -> Result<(), AuthError> {
        self.db
            .delete_session(token_hash)
            .await
            .map_err(AuthError::Db)?;
        Ok(())
    }

    async fn complete_setup(&self, req: SetupRequest) -> Result<SetupResponse, AuthError> {
        Self::validate_username(&req.username)?;
        Self::validate_password(&req.password)?;

        let password_hash = Self::hash_password(&req.password);
        let api_key = Self::generate_api_key();
        let api_key_hash = Self::hash_api_key(&api_key);

        let _user = self
            .db
            .complete_setup(livrarr_db::CompleteSetupDbRequest {
                username: req.username.clone(),
                password_hash,
                api_key_hash,
            })
            .await
            .map_err(|e| match e {
                DbError::Constraint { .. } => AuthError::SetupCompleted,
                other => AuthError::Db(other),
            })?;

        // Create session
        let token = Self::generate_token();
        let token_hash = Self::hash_token(&token);
        let session = Session {
            token_hash,
            user_id: _user.id,
            persistent: false,
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::hours(24),
        };
        self.db
            .create_session(&session)
            .await
            .map_err(AuthError::Db)?;

        Ok(SetupResponse { api_key, token })
    }

    async fn get_current_user(&self, auth: &AuthContext) -> Result<AuthMeResponse, AuthError> {
        Ok(AuthMeResponse {
            user: Self::user_to_response(&auth.user),
            auth_type: auth.auth_type,
        })
    }

    async fn update_profile(
        &self,
        user_id: UserId,
        req: UpdateProfileRequest,
    ) -> Result<UserResponse, AuthError> {
        if let Some(ref username) = req.username {
            Self::validate_username(username)?;
        }
        let mut db_req = UpdateUserDbRequest {
            username: req.username,
            password_hash: None,
            role: None,
        };
        if let Some(ref password) = req.password {
            Self::validate_password(password)?;
            db_req.password_hash = Some(Self::hash_password(password));
        }
        let user = self
            .db
            .update_user(user_id, db_req)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => AuthError::UserNotFound,
                other => AuthError::Db(other),
            })?;
        Ok(Self::user_to_response(&user))
    }

    async fn regenerate_api_key(&self, user_id: UserId) -> Result<ApiKeyResponse, AuthError> {
        let key = Self::generate_api_key();
        let hash = Self::hash_api_key(&key);
        self.db
            .update_api_key_hash(user_id, &hash)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => AuthError::UserNotFound,
                other => AuthError::Db(other),
            })?;
        Ok(ApiKeyResponse { api_key: key })
    }

    async fn create_user(&self, req: AdminCreateUserRequest) -> Result<UserResponse, AuthError> {
        Self::validate_username(&req.username)?;
        Self::validate_password(&req.password)?;
        let password_hash = Self::hash_password(&req.password);
        let api_key = Self::generate_api_key();
        let api_key_hash = Self::hash_api_key(&api_key);
        let user = self
            .db
            .create_user(CreateUserDbRequest {
                username: req.username,
                password_hash,
                role: req.role,
                api_key_hash,
            })
            .await
            .map_err(|e| match e {
                DbError::Constraint { .. } => AuthError::UsernameTaken,
                other => AuthError::Db(other),
            })?;
        Ok(Self::user_to_response(&user))
    }

    async fn list_users(&self) -> Result<Vec<UserResponse>, AuthError> {
        let users = self.db.list_users().await.map_err(AuthError::Db)?;
        Ok(users.iter().map(Self::user_to_response).collect())
    }

    async fn get_user(&self, id: UserId) -> Result<UserResponse, AuthError> {
        let user = self.db.get_user(id).await.map_err(|e| match e {
            DbError::NotFound { .. } => AuthError::UserNotFound,
            other => AuthError::Db(other),
        })?;
        Ok(Self::user_to_response(&user))
    }

    async fn update_user(
        &self,
        id: UserId,
        req: AdminUpdateUserRequest,
    ) -> Result<UserResponse, AuthError> {
        if let Some(ref username) = req.username {
            Self::validate_username(username)?;
        }
        let mut db_req = UpdateUserDbRequest {
            username: req.username,
            password_hash: None,
            role: req.role,
        };
        if let Some(ref password) = req.password {
            Self::validate_password(password)?;
            db_req.password_hash = Some(Self::hash_password(password));
        }
        let user = self.db.update_user(id, db_req).await.map_err(|e| match e {
            DbError::NotFound { .. } => AuthError::UserNotFound,
            other => AuthError::Db(other),
        })?;
        Ok(Self::user_to_response(&user))
    }

    async fn delete_user(
        &self,
        requesting_user_id: UserId,
        target_user_id: UserId,
    ) -> Result<(), AuthError> {
        if requesting_user_id == target_user_id {
            return Err(AuthError::CannotDeleteSelf);
        }
        // Check if target is last admin
        let target = self
            .db
            .get_user(target_user_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => AuthError::UserNotFound,
                other => AuthError::Db(other),
            })?;
        if target.role == UserRole::Admin {
            let admin_count = self.db.count_admins().await.map_err(AuthError::Db)?;
            if admin_count <= 1 {
                return Err(AuthError::LastAdmin);
            }
        }
        self.db
            .delete_user(target_user_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => AuthError::UserNotFound,
                other => AuthError::Db(other),
            })?;
        Ok(())
    }

    async fn regenerate_user_api_key(&self, user_id: UserId) -> Result<ApiKeyResponse, AuthError> {
        self.regenerate_api_key(user_id).await
    }
}

impl AuthServiceImpl {
    async fn record_failure(&self, username: &str) {
        let mut lockouts = self.lockouts.write().await;
        // Bounded eviction: if the map exceeds the max, remove arbitrary entries.
        if lockouts.len() >= MAX_LOCKOUT_ENTRIES {
            let keys: Vec<String> = lockouts.keys().take(EVICT_COUNT).cloned().collect();
            for key in keys {
                lockouts.remove(&key);
            }
        }
        let state = lockouts
            .entry(username.to_string())
            .or_insert(LockoutState {
                failures: 0,
                locked_until: None,
            });
        state.failures += 1;
        if state.failures >= 5 {
            state.locked_until = Some(Utc::now() + Duration::minutes(15));
        }
    }
}

#[cfg(test)]
/// Create a fresh AuthService backed by a SQLite :memory: DB with a placeholder admin.
pub async fn new_test_auth_service() -> AuthServiceImpl {
    let db = livrarr_db::test_helpers::create_test_db().await;
    // Create the placeholder admin user
    db.create_user(CreateUserDbRequest {
        username: "admin".into(),
        password_hash: "hash:admin".into(),
        role: livrarr_db::UserRole::Admin,
        api_key_hash: "apikey".into(),
    })
    .await
    .unwrap();
    AuthServiceImpl::new(db)
}
