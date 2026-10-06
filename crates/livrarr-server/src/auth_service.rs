//! Production AuthService, generic over crypto backend.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use chrono::{Duration, Utc};
use subtle::ConstantTimeEq;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::auth_crypto::AuthCryptoService;
use crate::setup_token::{remove_setup_token_file, SetupToken};
use crate::*;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{
    CompleteSetupDbRequest, CreateUserDbRequest, SessionDb, UpdateUserDbRequest, UserDb,
};

/// Maximum number of entries in the lockout map before eviction.
const MAX_LOCKOUT_ENTRIES: usize = 10_000;
/// Number of entries to evict when the map exceeds the maximum.
const EVICT_COUNT: usize = 1_000;

pub struct ServerAuthService<C: AuthCryptoService> {
    db: SqliteDb,
    crypto: C,
    lockouts: Arc<RwLock<HashMap<String, LockoutState>>>,
    setup_token: Mutex<SetupTokenState>,
}

/// The first-run setup token as the service holds it.
enum SetupTokenState {
    /// No token was given: every setup request is refused.
    Absent,
    /// Setup requests must carry this token.
    Active(SetupToken),
    /// A setup request carrying the token claimed the account.
    Consumed,
}

struct LockoutState {
    failures: u32,
    locked_until: Option<chrono::DateTime<Utc>>,
}

impl<C: AuthCryptoService> ServerAuthService<C> {
    /// A service with no setup token, which refuses every setup request.
    pub fn new(db: SqliteDb, crypto: C) -> Self {
        Self {
            db,
            crypto,
            lockouts: Arc::new(RwLock::new(HashMap::new())),
            setup_token: Mutex::new(SetupTokenState::Absent),
        }
    }

    /// The service that accepts a setup request only when it carries `token`.
    pub fn with_setup_token(self, token: SetupToken) -> Self {
        Self {
            setup_token: Mutex::new(SetupTokenState::Active(token)),
            ..self
        }
    }

    /// Accepts `offered` only when, trimmed of surrounding whitespace, it
    /// equals the active setup token, compared in constant time.
    fn check_setup_token(&self, offered: Option<&str>) -> Result<(), AuthError> {
        let state = self
            .setup_token
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let expected = match &*state {
            SetupTokenState::Active(token) => token.value(),
            SetupTokenState::Consumed => return Err(AuthError::SetupCompleted),
            SetupTokenState::Absent => return Err(AuthError::SetupTokenRejected),
        };
        let offered = offered.map(str::trim).unwrap_or_default();
        let matches: bool = offered.as_bytes().ct_eq(expected.as_bytes()).into();
        if offered.is_empty() || !matches {
            return Err(AuthError::SetupTokenRejected);
        }
        Ok(())
    }

    /// Drops the in-memory setup token once the account is claimed, then
    /// removes its file as a best effort.
    fn consume_setup_token(&self) {
        let previous = std::mem::replace(
            &mut *self
                .setup_token
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
            SetupTokenState::Consumed,
        );
        if let SetupTokenState::Active(token) = previous {
            remove_setup_token_file(token.file());
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
        if password.chars().count() < 8 {
            return Err(AuthError::InvalidPassword {
                reason: "minimum 8 characters".into(),
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
        if lockouts.len() >= MAX_LOCKOUT_ENTRIES {
            // Never evict an active lockout — that's the attack vector.
            // Evict expired lockouts and not-yet-locked entries only.
            let now = Utc::now();
            let evictable: Vec<String> = lockouts
                .iter()
                .filter(|(_, s)| s.locked_until.is_none_or(|t| t <= now))
                .map(|(k, _)| k.clone())
                .take(EVICT_COUNT)
                .collect();
            for key in evictable {
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
        if state.failures >= 5 && state.locked_until.is_none() {
            state.locked_until = Some(Utc::now() + Duration::minutes(15));
            warn!(username = %username, "account locked out after 5 failed attempts");
        }
    }
}

impl<C: AuthCryptoService> AuthService for ServerAuthService<C> {
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
                warn!(username = %req.username, "login failed: user not found");
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
            warn!(username = %req.username, "login failed: invalid password");
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

        info!(username = %req.username, user_id = user.id, "login successful");
        Ok(LoginResponse { token })
    }

    async fn logout(&self, token_hash: &str) -> Result<(), AuthError> {
        self.db
            .delete_session(token_hash)
            .await
            .map_err(AuthError::Db)?;
        info!("session deleted (logout)");
        Ok(())
    }

    async fn complete_setup(&self, req: SetupRequest) -> Result<SetupResponse, AuthError> {
        Self::validate_username(&req.username)?;
        Self::validate_password(&req.password)?;

        // Hash-free authority check BEFORE any Argon2 hashing runs. Without
        // this gate, a repeated POST to an already-completed setup still
        // pays full hashing cost on every request (CPU-DoS). The DB-level
        // conditional UPDATE below remains as the atomic race-winner for two
        // requests that both observe setup as pending.
        if self.is_setup_complete().await? {
            return Err(AuthError::SetupCompleted);
        }

        self.check_setup_token(req.setup_token.as_deref())?;

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

        let user_id = self
            .db
            .claim_setup(CompleteSetupDbRequest {
                username: req.username,
                password_hash,
                api_key_hash,
            })
            .await
            .map_err(|e| match e {
                DbError::Constraint { .. } => AuthError::SetupCompleted,
                other => AuthError::Db(other),
            })?;
        self.consume_setup_token();

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
            user_id,
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

            // Delete sessions BEFORE updating password. If deletion fails,
            // password is untouched and user can safely retry. If deletion
            // succeeds but password update fails, user is logged out but
            // password unchanged — a safe failure mode.
            self.db
                .delete_user_sessions(user_id)
                .await
                .map_err(AuthError::Db)?;
            info!(user_id = user_id, "password changed via profile update");
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
        let password_changed = req.password.is_some();
        if let Some(ref password) = req.password {
            Self::validate_password(password)?;
            let hash = self
                .crypto
                .hash_password(password)
                .await
                .map_err(|e| AuthError::Db(DbError::Io(Box::new(e))))?;
            db_req.password_hash = Some(hash);
        }

        // Apply the guarded DB write FIRST. Session invalidation is a side
        // effect of a *successful* password change, so it must only run
        // after this commits — otherwise a rejected update (e.g. the
        // sole-admin guard blocking a self-demote) would still log the
        // admin out despite nothing having changed.
        let user = self.db.update_user(id, db_req).await.map_err(|e| match e {
            DbError::NotFound { .. } => AuthError::UserNotFound,
            DbError::LastAdmin => AuthError::LastAdmin,
            other => AuthError::Db(other),
        })?;

        if password_changed {
            self.db
                .delete_user_sessions(id)
                .await
                .map_err(AuthError::Db)?;
        }

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
        // The sole-admin invariant is enforced atomically by the DB layer
        // (guarded in one statement against the live admin count) — no
        // separate get_user/count_admins pre-check here, which would only
        // race against a concurrent demote/delete anyway.
        self.db
            .delete_user(target_user_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => AuthError::UserNotFound,
                DbError::LastAdmin => AuthError::LastAdmin,
                other => AuthError::Db(other),
            })?;
        Ok(())
    }

    async fn regenerate_user_api_key(&self, user_id: UserId) -> Result<ApiKeyResponse, AuthError> {
        self.regenerate_api_key(user_id).await
    }

    async fn verify_credentials(&self, username: &str, password: &str) -> Result<User, AuthError> {
        let user = self
            .db
            .get_user_by_username(username)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => AuthError::InvalidCredentials,
                other => AuthError::Db(other),
            })?;
        let valid = self
            .crypto
            .verify_password(password, &user.password_hash)
            .await
            .map_err(|e| AuthError::Db(DbError::Io(Box::new(e))))?;
        if !valid {
            return Err(AuthError::InvalidCredentials);
        }
        Ok(user)
    }

    async fn is_setup_complete(&self) -> Result<bool, AuthError> {
        let pending = self.db.has_pending_setup().await.map_err(AuthError::Db)?;
        Ok(!pending)
    }

    async fn verify_token(&self, token: &str) -> Result<i64, AuthError> {
        let token_hash = self
            .crypto
            .hash_token(token)
            .await
            .map_err(|_| AuthError::InvalidCredentials)?;
        use livrarr_db::SessionDb;
        let session = self
            .db
            .get_session(&token_hash)
            .await
            .map_err(AuthError::Db)?
            .ok_or(AuthError::InvalidCredentials)?;
        Ok(session.user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_crypto::{AuthCryptoError, TestAuthCrypto};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Wraps `TestAuthCrypto` and counts `hash_password` calls — proves the
    /// CPU-DoS gate rejects a repeated setup POST before Argon2 would run.
    struct CountingCrypto {
        inner: TestAuthCrypto,
        hash_calls: Arc<AtomicUsize>,
    }

    impl AuthCryptoService for CountingCrypto {
        async fn hash_password(&self, password: &str) -> Result<String, AuthCryptoError> {
            self.hash_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.hash_password(password).await
        }
        async fn verify_password(
            &self,
            password: &str,
            hash: &str,
        ) -> Result<bool, AuthCryptoError> {
            self.inner.verify_password(password, hash).await
        }
        async fn generate_token(&self) -> Result<String, AuthCryptoError> {
            self.inner.generate_token().await
        }
        async fn hash_token(&self, token: &str) -> Result<String, AuthCryptoError> {
            self.inner.hash_token(token).await
        }
        async fn constant_time_eq(&self, a: &[u8], b: &[u8]) -> Result<bool, AuthCryptoError> {
            self.inner.constant_time_eq(a, b).await
        }
    }

    #[test]
    fn setup_request_debug_never_shows_the_password_or_setup_token() {
        let token = "6f1c2b9e0d4a7f3851c6e2b0a9d8f714";
        let request: SetupRequest = serde_json::from_value(serde_json::json!({
            "username": "owner",
            "password": "owner-secret-1",
            "setupToken": token,
        }))
        .expect("a setup body with a token deserializes");
        let shown = format!("{request:?}");
        assert!(shown.contains("owner"), "{shown}");
        assert!(!shown.contains("owner-secret-1"), "{shown}");
        assert!(!shown.contains(token), "{shown}");
    }

    const SETUP_TOKEN: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f0";

    #[tokio::test]
    async fn complete_setup_succeeds_on_virgin_db() {
        let db = livrarr_db::test_helpers::create_test_db().await;
        let token_dir = tempfile::tempdir().unwrap();
        let service = ServerAuthService::new(db, TestAuthCrypto).with_setup_token(SetupToken::new(
            SETUP_TOKEN,
            token_dir.path().join("setup-token"),
        ));

        let result = service
            .complete_setup(SetupRequest {
                username: "admin".to_string(),
                password: "firstpass1".to_string(),
                setup_token: Some(SETUP_TOKEN.to_string()),
            })
            .await;

        assert!(
            result.is_ok(),
            "setup must succeed on a virgin DB: {result:?}"
        );
        assert!(service.is_setup_complete().await.unwrap());
    }

    #[tokio::test]
    async fn setup_token_is_consumed_when_a_step_after_the_claim_fails() {
        let db = livrarr_db::test_helpers::create_test_db().await;
        let db_check = db.clone();
        let pending_id: i64 =
            sqlx::query_scalar("SELECT id FROM users WHERE setup_pending = 1 ORDER BY id LIMIT 1")
                .fetch_one(db.pool())
                .await
                .unwrap();
        // Constructed fault by necessity: no production writer stores an
        // unreadable timestamp or blocks session inserts, and both are needed
        // to make the steps after the conditional claim fail.
        sqlx::query("UPDATE users SET created_at = 'not-a-timestamp' WHERE setup_pending = 1")
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query(
            "CREATE TRIGGER fail_session_insert BEFORE INSERT ON sessions \
             BEGIN SELECT RAISE(ABORT, 'injected session failure'); END",
        )
        .execute(db.pool())
        .await
        .unwrap();
        let token_dir = tempfile::tempdir().unwrap();
        let token_file = token_dir.path().join("setup-token");
        std::fs::write(&token_file, SETUP_TOKEN).unwrap();
        let service = ServerAuthService::new(db, TestAuthCrypto)
            .with_setup_token(SetupToken::new(SETUP_TOKEN, token_file.clone()));
        let request = || SetupRequest {
            username: "owner".to_string(),
            password: "ownerpass1".to_string(),
            setup_token: Some(SETUP_TOKEN.to_string()),
        };

        let result = service.complete_setup(request()).await;
        assert!(result.is_err(), "the injected fault must fail the request");

        assert!(
            service.is_setup_complete().await.unwrap(),
            "the account must be claimed"
        );
        assert!(
            matches!(
                *service.setup_token.lock().unwrap(),
                SetupTokenState::Consumed
            ),
            "the service must no longer hold the setup token"
        );
        let repeat = service.complete_setup(request()).await;
        assert!(
            matches!(repeat, Err(AuthError::SetupCompleted)),
            "a repeat request must get SetupCompleted, got: {repeat:?}"
        );
        assert!(
            !token_file.exists(),
            "the setup token file must be removed once the account is claimed"
        );

        sqlx::query("UPDATE users SET created_at = ? WHERE id = ?")
            .bind(Utc::now().to_rfc3339())
            .bind(pending_id)
            .execute(db_check.pool())
            .await
            .unwrap();
        sqlx::query("DROP TRIGGER fail_session_insert")
            .execute(db_check.pool())
            .await
            .unwrap();

        let login = service
            .login(LoginRequest {
                username: "owner".to_string(),
                password: "ownerpass1".to_string(),
                remember_me: false,
            })
            .await
            .expect("the owner must sign in with the setup credentials");
        let session_user = service
            .verify_token(&login.token)
            .await
            .expect("the sign-in session must verify");
        assert_eq!(
            session_user, pending_id,
            "the session must belong to the claimed account"
        );
        assert!(
            service.is_setup_complete().await.unwrap(),
            "setup must still be complete after sign-in"
        );
        assert!(
            !token_file.exists(),
            "the setup token file must still be absent after sign-in"
        );
    }

    /// A repeated POST to an already-completed setup must be rejected
    /// WITHOUT paying Argon2 hashing cost, and the authority check must
    /// hold even after user id 1 (the original placeholder row) has been
    /// deleted while another admin remains.
    #[tokio::test]
    async fn double_post_rejects_before_hashing_with_user_one_absent() {
        let db = livrarr_db::test_helpers::create_test_db().await;
        let hash_calls = Arc::new(AtomicUsize::new(0));
        let crypto = CountingCrypto {
            inner: TestAuthCrypto,
            hash_calls: hash_calls.clone(),
        };
        let token_dir = tempfile::tempdir().unwrap();
        let service = ServerAuthService::new(db, crypto).with_setup_token(SetupToken::new(
            SETUP_TOKEN,
            token_dir.path().join("setup-token"),
        ));

        // First legitimate setup — converts the placeholder user 1.
        service
            .complete_setup(SetupRequest {
                username: "admin".to_string(),
                password: "firstpass1".to_string(),
                setup_token: Some(SETUP_TOKEN.to_string()),
            })
            .await
            .expect("first setup must succeed");
        assert_eq!(hash_calls.load(Ordering::SeqCst), 1);

        // Create a second admin, then delete the original admin (user 1).
        // Another admin remains, so the deletion is legitimate — this is
        // the exact drift the old id-1-based check mishandled.
        let second_admin = service
            .create_user(AdminCreateUserRequest {
                username: "admin2".to_string(),
                password: "secondpass1".to_string(),
                role: UserRole::Admin,
            })
            .await
            .expect("create second admin");
        service
            .delete_user(second_admin.id, 1)
            .await
            .expect("delete original admin (another admin remains)");

        // True authority: setup IS complete, even though user id 1 is gone.
        assert!(
            service.is_setup_complete().await.unwrap(),
            "is_setup_complete must reflect real state, not user id 1's presence"
        );

        let calls_before_second_post = hash_calls.load(Ordering::SeqCst);

        // Double-POST: setup is already complete — must reject without
        // hashing the attacker-supplied password.
        let result = service
            .complete_setup(SetupRequest {
                username: "attacker".to_string(),
                password: "whatever12".to_string(),
                setup_token: None,
            })
            .await;

        assert!(
            matches!(result, Err(AuthError::SetupCompleted)),
            "expected SetupCompleted, got: {result:?}"
        );
        assert_eq!(
            hash_calls.load(Ordering::SeqCst),
            calls_before_second_post,
            "second POST must be rejected before Argon2 hashing runs"
        );
    }

    /// #16 (MED): a rejected sole-admin self-demote must not have the side
    /// effect of logging the admin out. Session invalidation is a side
    /// effect of a successful password change — it must only run AFTER the
    /// guarded DB write actually commits, never before/regardless of
    /// whether the write is rejected.
    #[tokio::test]
    async fn rejected_self_demote_with_password_change_leaves_sessions_intact() {
        let db = livrarr_db::test_helpers::create_test_db().await;
        let db_check = db.clone();
        let service = ServerAuthService::new(db, TestAuthCrypto);

        let now = Utc::now();
        let session = Session {
            token_hash: "sole_admin_session".to_string(),
            user_id: 1,
            persistent: false,
            created_at: now,
            expires_at: now + Duration::hours(1),
        };
        db_check
            .create_session(&session)
            .await
            .expect("seed session for sole admin");

        // Sole admin submits a password change bundled with a self-demote.
        let result = service
            .update_user(
                1,
                AdminUpdateUserRequest {
                    username: None,
                    password: Some("newpass123".to_string()),
                    role: Some(UserRole::User),
                },
            )
            .await;

        assert!(
            matches!(result, Err(AuthError::LastAdmin)),
            "sole-admin self-demote must be rejected, got: {result:?}"
        );

        let still_there = db_check
            .get_session("sole_admin_session")
            .await
            .expect("session lookup should not error");
        assert!(
            still_there.is_some(),
            "a rejected update must not delete the admin's sessions"
        );
    }
}
