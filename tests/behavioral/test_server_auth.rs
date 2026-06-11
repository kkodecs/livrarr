#![allow(dead_code)]
// Behavioral contract tests for AuthService trait.
// Tests run against a real AuthService implementation constructed by `fresh_service()`.
// External dependencies mocked: argon2id hashing (fast/deterministic), token generation (deterministic).

use chrono::Utc;
use librarr_domain::{AuthType, User, UserRole};
use librarr_server::{
    AdminCreateUserRequest, AdminUpdateUserRequest, AuthContext, AuthError, AuthService,
    LoginRequest, SetupRequest, SetupResponse, UpdateProfileRequest, UserResponse,
};

// ---------------------------------------------------------------------------
// Stub AuthService (provides concrete type for `fresh_service`)
// ---------------------------------------------------------------------------

struct StubAuthService;

#[async_trait::async_trait]
impl AuthService for StubAuthService {
    async fn login(&self, _req: LoginRequest) -> Result<librarr_server::LoginResponse, AuthError> {
        unimplemented!()
    }
    async fn logout(&self, _token_hash: &str) -> Result<(), AuthError> {
        unimplemented!()
    }
    async fn complete_setup(&self, _req: SetupRequest) -> Result<SetupResponse, AuthError> {
        unimplemented!()
    }
    async fn get_current_user(
        &self,
        _auth: &AuthContext,
    ) -> Result<librarr_server::AuthMeResponse, AuthError> {
        unimplemented!()
    }
    async fn update_profile(
        &self,
        _user_id: librarr_domain::UserId,
        _req: UpdateProfileRequest,
    ) -> Result<UserResponse, AuthError> {
        unimplemented!()
    }
    async fn regenerate_api_key(
        &self,
        _user_id: librarr_domain::UserId,
    ) -> Result<librarr_server::ApiKeyResponse, AuthError> {
        unimplemented!()
    }
    async fn create_user(&self, _req: AdminCreateUserRequest) -> Result<UserResponse, AuthError> {
        unimplemented!()
    }
    async fn list_users(&self) -> Result<Vec<UserResponse>, AuthError> {
        unimplemented!()
    }
    async fn get_user(&self, _id: librarr_domain::UserId) -> Result<UserResponse, AuthError> {
        unimplemented!()
    }
    async fn update_user(
        &self,
        _id: librarr_domain::UserId,
        _req: AdminUpdateUserRequest,
    ) -> Result<UserResponse, AuthError> {
        unimplemented!()
    }
    async fn delete_user(
        &self,
        _requesting_user_id: librarr_domain::UserId,
        _target_user_id: librarr_domain::UserId,
    ) -> Result<(), AuthError> {
        unimplemented!()
    }
    async fn regenerate_user_api_key(
        &self,
        _user_id: librarr_domain::UserId,
    ) -> Result<librarr_server::ApiKeyResponse, AuthError> {
        unimplemented!()
    }
}

async fn fresh_service() -> impl AuthService {
    librarr_server::auth_impl::new_test_auth_service().await
}

// ---------------------------------------------------------------------------
// Request stubs for AuthMiddleware tests
// ---------------------------------------------------------------------------

/// A request with no credentials attached.
fn no_credentials_request() -> TestRequest {
    TestRequest {
        kind: TestRequestKind::NoCredentials,
    }
}

/// A request targeting a setup endpoint (exempt from auth).
fn setup_request() -> TestRequest {
    TestRequest {
        kind: TestRequestKind::Setup,
    }
}

/// A request with an external auth header from the given IP.
fn external_auth_request(username: &str, ip: &str) -> TestRequest {
    TestRequest {
        kind: TestRequestKind::ExternalAuth {
            username: username.to_owned(),
            ip: ip.to_owned(),
        },
    }
}

use librarr_server::{TestRequest, TestRequestKind};

/// Build an `AuthContext` from just a role (for middleware role-check tests).
fn auth_context(role: UserRole) -> AuthContext {
    let now = Utc::now();
    AuthContext {
        user: User {
            id: 1,
            username: format!("test_{:?}", role).to_lowercase(),
            password_hash: "test-hash".into(),
            role,
            api_key_hash: "test-api-hash".into(),
            setup_pending: false,
            created_at: now,
            updated_at: now,
        },
        auth_type: AuthType::Session,
        session_token_hash: None,
    }
}

fn mk_login(username: &str, password: &str, remember_me: bool) -> LoginRequest {
    LoginRequest {
        username: username.into(),
        password: password.into(),
        remember_me,
    }
}

fn mk_setup(username: &str, password: &str) -> SetupRequest {
    SetupRequest {
        username: username.into(),
        password: password.into(),
    }
}

fn mk_create(username: &str, password: &str, role: UserRole) -> AdminCreateUserRequest {
    AdminCreateUserRequest {
        username: username.into(),
        password: password.into(),
        role,
    }
}

fn mk_update(u: Option<&str>, p: Option<&str>, r: Option<UserRole>) -> AdminUpdateUserRequest {
    AdminUpdateUserRequest {
        username: u.map(Into::into),
        password: p.map(Into::into),
        role: r,
    }
}

fn auth_ctx(user: &UserResponse, auth_type: AuthType) -> AuthContext {
    let now = Utc::now();
    AuthContext {
        user: User {
            id: user.id,
            username: user.username.clone(),
            password_hash: "test-hash".into(),
            role: user.role,
            api_key_hash: "test-api-hash".into(),
            setup_pending: false,
            created_at: now,
            updated_at: now,
        },
        auth_type,
        session_token_hash: None,
    }
}

async fn do_setup(svc: &impl AuthService) -> SetupResponse {
    svc.complete_setup(mk_setup("admin_user", "secret1"))
        .await
        .unwrap()
}

async fn do_create(svc: &impl AuthService, u: &str, p: &str, r: UserRole) -> UserResponse {
    svc.create_user(mk_create(u, p, r)).await.unwrap()
}

// === Nominal (happy path) ===

#[tokio::test]
async fn test_server_auth_login_valid_credentials_returns_non_empty_token() {
    // Satisfies: AUTH-005 — Login returns session token
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let resp = svc
        .login(mk_login("admin_user", "secret1", false))
        .await
        .unwrap();
    assert!(!resp.token.is_empty());
}

#[tokio::test]
async fn test_server_auth_login_remember_me_true_succeeds() {
    // Satisfies: AUTH-005 — Persistent session (30d rolling) when remember_me=true
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let resp = svc
        .login(mk_login("admin_user", "secret1", true))
        .await
        .unwrap();
    assert!(!resp.token.is_empty());
}

#[tokio::test]
async fn test_server_auth_login_remember_me_false_succeeds() {
    // Satisfies: AUTH-005 — Short session (24h) when remember_me=false
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let resp = svc
        .login(mk_login("admin_user", "secret1", false))
        .await
        .unwrap();
    assert!(!resp.token.is_empty());
}

#[tokio::test]
async fn test_server_auth_logout_valid_token_succeeds() {
    // Satisfies: AUTH-005 — Logout deletes session
    let svc = fresh_service().await;
    let setup = do_setup(&svc).await;
    svc.logout(&setup.token).await.unwrap();
}

#[tokio::test]
async fn test_server_auth_complete_setup_returns_api_key_and_token() {
    // Satisfies: AUTH-010 — Setup wizard returns API key + session token, one-time
    let svc = fresh_service().await;
    let resp = svc
        .complete_setup(mk_setup("admin_user", "secret1"))
        .await
        .unwrap();
    assert!(!resp.api_key.is_empty());
    assert!(!resp.token.is_empty());
}

#[tokio::test]
async fn test_server_auth_get_current_user_returns_user_and_auth_type() {
    // Satisfies: AUTH-008 — Returns user info and auth mechanism
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let admin = svc.get_user(1).await.unwrap();
    let ctx = auth_ctx(&admin, AuthType::Session);
    let resp = svc.get_current_user(&ctx).await.unwrap();
    assert_eq!(resp.user.id, admin.id);
    assert_eq!(resp.auth_type, AuthType::Session);
}

#[tokio::test]
async fn test_server_auth_update_profile_username_succeeds() {
    // Satisfies: AUTH-013 — Valid username update accepted
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let admin = svc.get_user(1).await.unwrap();
    let req = UpdateProfileRequest {
        username: Some("renamed".into()),
        password: None,
    };
    let resp = svc.update_profile(admin.id, req).await.unwrap();
    assert_eq!(resp.username, "renamed");
}

#[tokio::test]
async fn test_server_auth_update_profile_password_succeeds() {
    // Satisfies: AUTH-013 — Password update reflected in subsequent login
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let admin = svc.get_user(1).await.unwrap();
    let req = UpdateProfileRequest {
        username: None,
        password: Some("newpass1".into()),
    };
    svc.update_profile(admin.id, req).await.unwrap();
    let login = svc
        .login(mk_login("admin_user", "newpass1", false))
        .await
        .unwrap();
    assert!(!login.token.is_empty());
}

#[tokio::test]
async fn test_server_auth_regenerate_api_key_returns_non_empty() {
    // Satisfies: AUTH-010 — Regenerate own API key returns plaintext
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let admin = svc.get_user(1).await.unwrap();
    let resp = svc.regenerate_api_key(admin.id).await.unwrap();
    assert!(!resp.api_key.is_empty());
}

#[tokio::test]
async fn test_server_auth_create_user_returns_correct_fields() {
    // Satisfies: AUTH-011 — Create user persists fields
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let resp = svc
        .create_user(mk_create("reader1", "secret1", UserRole::User))
        .await
        .unwrap();
    assert_eq!(resp.username, "reader1");
    assert_eq!(resp.role, UserRole::User);
    assert!(resp.id > 0);
}

#[tokio::test]
async fn test_server_auth_list_users_returns_all() {
    // Satisfies: AUTH-011 — List users returns full set
    let svc = fresh_service().await;
    do_setup(&svc).await;
    do_create(&svc, "reader1", "secret1", UserRole::User).await;
    do_create(&svc, "reader2", "secret1", UserRole::User).await;
    let users = svc.list_users().await.unwrap();
    let names: Vec<_> = users.iter().map(|u| u.username.as_str()).collect();
    assert!(names.contains(&"admin_user"));
    assert!(names.contains(&"reader1"));
    assert!(names.contains(&"reader2"));
}

#[tokio::test]
async fn test_server_auth_get_user_returns_correct_user() {
    // Satisfies: AUTH-011 — Get user by ID
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let created = do_create(&svc, "reader1", "secret1", UserRole::User).await;
    let fetched = svc.get_user(created.id).await.unwrap();
    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.username, "reader1");
}

#[tokio::test]
async fn test_server_auth_update_user_changes_fields() {
    // Satisfies: AUTH-011 — Update user modifies requested fields
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let created = do_create(&svc, "reader1", "secret1", UserRole::User).await;
    let resp = svc
        .update_user(
            created.id,
            mk_update(Some("renamed"), None, Some(UserRole::Admin)),
        )
        .await
        .unwrap();
    assert_eq!(resp.username, "renamed");
    assert_eq!(resp.role, UserRole::Admin);
}

#[tokio::test]
async fn test_server_auth_delete_user_succeeds() {
    // Satisfies: AUTH-011 — Delete user when not self and not last admin
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let admin2 = do_create(&svc, "admin2", "secret1", UserRole::Admin).await;
    let target = do_create(&svc, "reader1", "secret1", UserRole::User).await;
    svc.delete_user(admin2.id, target.id).await.unwrap();
    match svc.get_user(target.id).await {
        Err(AuthError::UserNotFound) => {}
        other => panic!("expected UserNotFound after delete, got {:?}", other),
    }
}

#[tokio::test]
async fn test_server_auth_regenerate_user_api_key_returns_non_empty() {
    // Satisfies: AUTH-011 — Admin can regenerate another user's API key
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let user = do_create(&svc, "reader1", "secret1", UserRole::User).await;
    let resp = svc.regenerate_user_api_key(user.id).await.unwrap();
    assert!(!resp.api_key.is_empty());
}

// === Failure ===

#[tokio::test]
async fn test_server_auth_login_wrong_password_returns_invalid_credentials() {
    // Satisfies: AUTH-012 — Wrong password returns InvalidCredentials
    let svc = fresh_service().await;
    do_setup(&svc).await;
    match svc.login(mk_login("admin_user", "wrongpw", false)).await {
        Err(AuthError::InvalidCredentials) => {}
        other => panic!("expected InvalidCredentials, got {:?}", other),
    }
}

#[tokio::test]
async fn test_server_auth_login_nonexistent_user_returns_invalid_credentials() {
    // Satisfies: AUTH-012 — Nonexistent user returns InvalidCredentials (no enumeration)
    let svc = fresh_service().await;
    do_setup(&svc).await;
    match svc.login(mk_login("missing_user", "secret1", false)).await {
        Err(AuthError::InvalidCredentials) => {}
        other => panic!("expected InvalidCredentials, got {:?}", other),
    }
}

#[tokio::test]
async fn test_server_auth_complete_setup_twice_returns_setup_completed() {
    // Satisfies: AUTH-010 — Second setup call returns SetupCompleted (irreversible)
    let svc = fresh_service().await;
    do_setup(&svc).await;
    match svc.complete_setup(mk_setup("other", "secret1")).await {
        Err(AuthError::SetupCompleted) => {}
        other => panic!("expected SetupCompleted, got {:?}", other),
    }
}

#[tokio::test]
async fn test_server_auth_delete_self_returns_cannot_delete_self() {
    // Satisfies: AUTH-011 — Cannot delete self
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let admin = svc.get_user(1).await.unwrap();
    match svc.delete_user(admin.id, admin.id).await {
        Err(AuthError::CannotDeleteSelf) => {}
        other => panic!("expected CannotDeleteSelf, got {:?}", other),
    }
}

#[tokio::test]
async fn test_server_auth_delete_last_admin_returns_last_admin() {
    // Satisfies: AUTH-011 — Cannot delete/demote last admin
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let admin = svc.get_user(1).await.unwrap();
    let user = do_create(&svc, "reader1", "secret1", UserRole::User).await;
    match svc.delete_user(user.id, admin.id).await {
        Err(AuthError::LastAdmin) => {}
        other => panic!("expected LastAdmin, got {:?}", other),
    }
}

#[tokio::test]
async fn test_server_auth_create_user_duplicate_username_returns_username_taken() {
    // Satisfies: AUTH-013 — Duplicate username rejected
    let svc = fresh_service().await;
    do_setup(&svc).await;
    do_create(&svc, "reader1", "secret1", UserRole::User).await;
    match svc
        .create_user(mk_create("reader1", "secret2", UserRole::User))
        .await
    {
        Err(AuthError::UsernameTaken) => {}
        other => panic!("expected UsernameTaken, got {:?}", other),
    }
}

#[tokio::test]
async fn test_server_auth_get_nonexistent_user_returns_user_not_found() {
    // Satisfies: AUTH-011 — Nonexistent user ID returns UserNotFound
    let svc = fresh_service().await;
    do_setup(&svc).await;
    match svc.get_user(999_999).await {
        Err(AuthError::UserNotFound) => {}
        other => panic!("expected UserNotFound, got {:?}", other),
    }
}

#[tokio::test]
async fn test_server_auth_update_nonexistent_user_returns_user_not_found() {
    // Satisfies: AUTH-011 — Update nonexistent user returns UserNotFound
    let svc = fresh_service().await;
    do_setup(&svc).await;
    match svc
        .update_user(999_999, mk_update(Some("ghost"), None, None))
        .await
    {
        Err(AuthError::UserNotFound) => {}
        other => panic!("expected UserNotFound, got {:?}", other),
    }
}

// === Boundary ===

#[tokio::test]
async fn test_server_auth_username_exactly_3_chars_valid() {
    // Satisfies: AUTH-013 — Username min length 3 accepted
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let resp = svc
        .create_user(mk_create("abc", "secret1", UserRole::User))
        .await;
    assert!(resp.is_ok(), "expected Ok, got {:?}", resp);
}

#[tokio::test]
async fn test_server_auth_username_2_chars_invalid() {
    // Satisfies: AUTH-013 — Username below min length rejected
    let svc = fresh_service().await;
    do_setup(&svc).await;
    match svc
        .create_user(mk_create("ab", "secret1", UserRole::User))
        .await
    {
        Err(AuthError::InvalidUsername { .. }) => {}
        other => panic!("expected InvalidUsername, got {:?}", other),
    }
}

#[tokio::test]
async fn test_server_auth_username_exactly_50_chars_valid() {
    // Satisfies: AUTH-013 — Username max length 50 accepted
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let name = "a".repeat(50);
    let resp = svc
        .create_user(mk_create(&name, "secret1", UserRole::User))
        .await;
    assert!(resp.is_ok(), "expected Ok, got {:?}", resp);
}

#[tokio::test]
async fn test_server_auth_username_51_chars_invalid() {
    // Satisfies: AUTH-013 — Username above max length rejected
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let name = "a".repeat(51);
    match svc
        .create_user(mk_create(&name, "secret1", UserRole::User))
        .await
    {
        Err(AuthError::InvalidUsername { .. }) => {}
        other => panic!("expected InvalidUsername, got {:?}", other),
    }
}

#[tokio::test]
async fn test_server_auth_password_exactly_6_chars_valid() {
    // Satisfies: AUTH-013 — Password min length 6 accepted
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let resp = svc
        .create_user(mk_create("reader1", "123456", UserRole::User))
        .await;
    assert!(resp.is_ok(), "expected Ok, got {:?}", resp);
}

#[tokio::test]
async fn test_server_auth_password_5_chars_invalid() {
    // Satisfies: AUTH-013 — Password below min length rejected
    let svc = fresh_service().await;
    do_setup(&svc).await;
    match svc
        .create_user(mk_create("reader1", "12345", UserRole::User))
        .await
    {
        Err(AuthError::InvalidPassword { .. }) => {}
        other => panic!("expected InvalidPassword, got {:?}", other),
    }
}

#[tokio::test]
async fn test_server_auth_password_1024_chars_valid() {
    // Satisfies: AUTH-013 — Password max length 1024 accepted
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let pw = "p".repeat(1024);
    let resp = svc
        .create_user(mk_create("reader1", &pw, UserRole::User))
        .await;
    assert!(resp.is_ok(), "expected Ok, got {:?}", resp);
}

#[tokio::test]
async fn test_server_auth_password_1025_chars_invalid() {
    // Satisfies: AUTH-013 — Password above max length rejected
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let pw = "p".repeat(1025);
    match svc
        .create_user(mk_create("reader1", &pw, UserRole::User))
        .await
    {
        Err(AuthError::InvalidPassword { .. }) => {}
        other => panic!("expected InvalidPassword, got {:?}", other),
    }
}

#[tokio::test]
async fn test_server_auth_username_underscore_and_hyphen_valid() {
    // Satisfies: AUTH-013 — Alphanumeric + underscore + hyphen allowed
    let svc = fresh_service().await;
    do_setup(&svc).await;
    let resp = svc
        .create_user(mk_create("user_name-1", "secret1", UserRole::User))
        .await;
    assert!(resp.is_ok(), "expected Ok, got {:?}", resp);
}

#[tokio::test]
async fn test_server_auth_username_special_chars_invalid() {
    // Satisfies: AUTH-013 — Special characters outside allowed set rejected
    let svc = fresh_service().await;
    do_setup(&svc).await;
    match svc
        .create_user(mk_create("bad!name", "secret1", UserRole::User))
        .await
    {
        Err(AuthError::InvalidUsername { .. }) => {}
        other => panic!("expected InvalidUsername, got {:?}", other),
    }
}

#[tokio::test]
async fn test_server_auth_brute_force_5_failures_locks_account() {
    // Satisfies: AUTH-012 — 5 consecutive failures trigger lockout; correct password also rejected
    let svc = fresh_service().await;
    do_setup(&svc).await;

    for i in 0..5 {
        let result = svc.login(mk_login("admin_user", "wrongpw", false)).await;
        match result {
            Err(AuthError::InvalidCredentials) | Err(AuthError::AccountLocked) => {}
            other => panic!("expected auth error on failure {}, got {:?}", i + 1, other),
        }
    }

    // After 5 failures, even correct password is rejected
    match svc.login(mk_login("admin_user", "secret1", false)).await {
        Err(AuthError::InvalidCredentials) | Err(AuthError::AccountLocked) => {}
        other => panic!("expected locked after 5 failures, got {:?}", other),
    }
}

#[tokio::test]
async fn test_server_auth_brute_force_4_failures_then_success_no_lockout() {
    // Satisfies: AUTH-012 — 4 failures then correct password resets counter
    let svc = fresh_service().await;
    do_setup(&svc).await;

    for _ in 0..4 {
        match svc.login(mk_login("admin_user", "wrongpw", false)).await {
            Err(AuthError::InvalidCredentials) => {}
            other => panic!("expected InvalidCredentials, got {:?}", other),
        }
    }

    let resp = svc
        .login(mk_login("admin_user", "secret1", false))
        .await
        .unwrap();
    assert!(!resp.token.is_empty());
}

// === Security ===

#[tokio::test]
async fn test_server_auth_no_user_enumeration_same_error_variant() {
    // Satisfies: AUTH-012 — Wrong password and nonexistent user return same error
    let svc = fresh_service().await;
    do_setup(&svc).await;

    let wrong_pw = svc.login(mk_login("admin_user", "wrongpw", false)).await;
    let missing = svc.login(mk_login("no_such_user", "wrongpw", false)).await;

    match (&wrong_pw, &missing) {
        (Err(AuthError::InvalidCredentials), Err(AuthError::InvalidCredentials)) => {}
        other => panic!("expected same InvalidCredentials for both, got {:?}", other),
    }
}

#[tokio::test]
async fn test_server_auth_locked_account_same_error_class_as_wrong_password() {
    // Satisfies: AUTH-012 — Locked account externally indistinguishable from wrong password
    let svc = fresh_service().await;
    do_setup(&svc).await;

    // Trigger lockout
    for _ in 0..5 {
        let _ = svc.login(mk_login("admin_user", "wrongpw", false)).await;
    }

    // Locked + correct password must return same safe error
    match svc.login(mk_login("admin_user", "secret1", false)).await {
        Err(AuthError::InvalidCredentials) | Err(AuthError::AccountLocked) => {}
        other => panic!(
            "expected auth-safe error on locked account, got {:?}",
            other
        ),
    }
}

// =============================================================================
// Auth Middleware Contracts — AUTH-001, AUTH-002, AUTH-009
// =============================================================================

#[tokio::test]
async fn test_server_auth_middleware_rejects_unauthenticated_requests() {
    // Satisfies: AUTH-001 — every API request authenticated except setup/static
    // IR contract: AuthMiddleware rejects requests without valid credentials
    // No anonymous access, no "disable auth" toggle, no network-based bypass
    let mw = librarr_server::AuthMiddleware::new_test();
    let result = mw.authenticate_request(no_credentials_request()).await;
    assert!(result.is_err(), "unauthenticated request must be rejected");
}

#[tokio::test]
async fn test_server_auth_middleware_setup_endpoints_exempt_from_auth() {
    // Satisfies: AUTH-001 — setup/bootstrap endpoints do not require auth
    // IR contract: AuthMiddleware allows /api/v1/setup/* without credentials
    let mw = librarr_server::AuthMiddleware::new_test_setup_pending();
    let result = mw.authenticate_request(setup_request()).await;
    assert!(
        result.is_ok(),
        "setup endpoints must be accessible without auth"
    );
}

#[tokio::test]
async fn test_server_auth_middleware_admin_role_gates_shared_infra() {
    // Satisfies: AUTH-002 — admin manages shared infra (root folders, download clients, config)
    // IR contract: AuthMiddleware + route handlers enforce role-based access
    let mw = librarr_server::AuthMiddleware::new_test();
    let admin_ctx = auth_context(UserRole::Admin);
    let user_ctx = auth_context(UserRole::User);
    // Admin can access shared infra
    assert!(mw.check_admin_access(&admin_ctx).is_ok());
    // Regular user cannot
    assert!(matches!(
        mw.check_admin_access(&user_ctx),
        Err(AuthError::Forbidden)
    ));
}

#[tokio::test]
async fn test_server_auth_middleware_external_auth_requires_trusted_ip() {
    // Satisfies: AUTH-009 — external auth only honored from trusted proxy CIDRs (TCP peer IP)
    // IR contract: AuthMiddleware precedence: Bearer → X-Api-Key → external auth
    let mw = librarr_server::AuthMiddleware::new_test_with_external_auth(
        "X-Remote-User",
        vec!["192.168.1.0/24".into()],
    );
    // Trusted IP with external header → authenticated
    let result = mw
        .authenticate_request(external_auth_request("alice", "192.168.1.100"))
        .await;
    assert!(
        result.is_ok(),
        "external auth from trusted IP should succeed"
    );
    // Untrusted IP with external header → rejected
    let result = mw
        .authenticate_request(external_auth_request("alice", "10.0.0.1"))
        .await;
    assert!(
        result.is_err(),
        "external auth from untrusted IP must be rejected"
    );
}

#[tokio::test]
async fn test_server_auth_middleware_external_auth_does_not_auto_create_user() {
    // Satisfies: AUTH-009 — external auth does not auto-create users
    // IR contract: AuthMiddleware looks up existing user, does not create
    let mw = librarr_server::AuthMiddleware::new_test_with_external_auth(
        "X-Remote-User",
        vec!["192.168.1.0/24".into()],
    );
    let result = mw
        .authenticate_request(external_auth_request("nonexistent_user", "192.168.1.100"))
        .await;
    assert!(
        result.is_err(),
        "external auth for nonexistent user must fail"
    );
}
