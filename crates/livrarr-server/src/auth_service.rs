//! Production AuthService using real crypto and SqliteDb.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{Duration, Utc};
use tokio::sync::RwLock;

use crate::auth_crypto::{AuthCryptoService, RealAuthCrypto};
use crate::*;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{
    CompleteSetupDbRequest, CreateUserDbRequest, SessionDb, UpdateUserDbRequest, UserDb,
};

/// Maximum number of entries in the lockout map before eviction.
const MAX_LOCKOUT_ENTRIES: usize = 10_000;
/// Number of entries to evict when the map exceeds the maximum.
const EVICT_COUNT: usize = 1_000;

pub struct ServerAuthService {
    db: SqliteDb,
    crypto: RealAuthCrypto,
    lockouts: Arc<RwLock<HashMap<String, LockoutState>>>,
}

struct LockoutState {
    failures: u32,
    locked_until: Option<chrono::DateTime<Utc>>,
}

impl ServerAuthService {
    pub fn new(db: SqliteDb) -> Self {
        Self {
            db,
            crypto: RealAuthCrypto,
            lockouts: Arc::new(RwLock::new(HashMap::new())),
        }
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

impl AuthService for ServerAuthService {
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
                let _ = self.crypto.hash_password("dummy").await;
                self.record_failure(&username_lower).await;
                return Err(AuthError::InvalidCredentials);
            }
            Err(e) => return Err(AuthError::Db(e)),
        };

        // Verify password with real argon2id
        let valid = self
            .crypto
            .verify_password(&req.password, &user.password_hash)
            .await
            .map_err(|e| AuthError::Db(DbError::Io(Box::new(e))))?;

        if !valid {
            self.record_failure(&username_lower).await;
            return Err(AuthError::InvalidCredentials);
        }

        // Success — reset lockout
        {
            let mut lockouts = self.lockouts.write().await;
            lockouts.remove(&username_lower);
        }

        // Create session — plaintext token returned to client, hash stored in DB
        let token = self
            .crypto
            .generate_token()
            .await
            .map_err(|e| AuthError::Db(DbError::Io(Box::new(e))))?;
        let token_hash = self
            .crypto
            .hash_token(&token)
            .await
            .map_err(|e| AuthError::Db(DbError::Io(Box::new(e))))?;

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

        let password_hash = self
            .crypto
            .hash_password(&req.password)
            .await
            .map_err(|e| AuthError::Db(DbError::Io(Box::new(e))))?;

        let api_key = self
            .crypto
            .generate_token()
            .await
            .map_err(|e| AuthError::Db(DbError::Io(Box::new(e))))?;
        let api_key_hash = self
            .crypto
            .hash_token(&api_key)
            .await
            .map_err(|e| AuthError::Db(DbError::Io(Box::new(e))))?;

        let user = self
            .db
            .complete_setup(CompleteSetupDbRequest {
                username: req.username,
                password_hash,
                api_key_hash,
            })
            .await
            .map_err(|e| match e {
                DbError::Constraint { .. } => AuthError::SetupCompleted,
                other => AuthError::Db(other),
            })?;

        // Create session
        let token = self
            .crypto
            .generate_token()
            .await
            .map_err(|e| AuthError::Db(DbError::Io(Box::new(e))))?;
        let token_hash = self
            .crypto
            .hash_token(&token)
            .await
            .map_err(|e| AuthError::Db(DbError::Io(Box::new(e))))?;

        let session = Session {
            token_hash,
            user_id: user.id,
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
            let hash = self
                .crypto
                .hash_password(password)
                .await
                .map_err(|e| AuthError::Db(DbError::Io(Box::new(e))))?;
            db_req.password_hash = Some(hash);
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
        let key = self
            .crypto
            .generate_token()
            .await
            .map_err(|e| AuthError::Db(DbError::Io(Box::new(e))))?;
        let hash = self
            .crypto
            .hash_token(&key)
            .await
            .map_err(|e| AuthError::Db(DbError::Io(Box::new(e))))?;
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
        let password_hash = self
            .crypto
            .hash_password(&req.password)
            .await
            .map_err(|e| AuthError::Db(DbError::Io(Box::new(e))))?;
        let api_key = self
            .crypto
            .generate_token()
            .await
            .map_err(|e| AuthError::Db(DbError::Io(Box::new(e))))?;
        let api_key_hash = self
            .crypto
            .hash_token(&api_key)
            .await
            .map_err(|e| AuthError::Db(DbError::Io(Box::new(e))))?;
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
            let hash = self
                .crypto
                .hash_password(password)
                .await
                .map_err(|e| AuthError::Db(DbError::Io(Box::new(e))))?;
            db_req.password_hash = Some(hash);
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
