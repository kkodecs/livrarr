use chrono::{DateTime, Utc};
use livrarr_domain::{AuthType, DbError, User, UserId, UserRole};
use serde::{Deserialize, Serialize};

use super::api_error::ApiError;

#[derive(Debug, Clone)]
pub struct AuthContext {
    pub user: User,
    pub auth_type: AuthType,
    pub session_token_hash: Option<String>,
}

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for AuthContext {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(
            parts
                .extensions
                .get::<AuthContext>()
                .cloned()
                .ok_or(ApiError::Unauthorized),
        )
    }
}

#[trait_variant::make(Send)]
pub trait AuthService: Send + Sync {
    async fn login(&self, req: LoginRequest) -> Result<LoginResponse, AuthError>;
    async fn logout(&self, token_hash: &str) -> Result<(), AuthError>;
    async fn complete_setup(&self, req: SetupRequest) -> Result<SetupResponse, AuthError>;
    async fn get_current_user(&self, auth: &AuthContext) -> Result<AuthMeResponse, AuthError>;
    async fn update_profile(
        &self,
        user_id: UserId,
        req: UpdateProfileRequest,
    ) -> Result<UserResponse, AuthError>;
    async fn regenerate_api_key(&self, user_id: UserId) -> Result<ApiKeyResponse, AuthError>;
    async fn create_user(&self, req: AdminCreateUserRequest) -> Result<UserResponse, AuthError>;
    async fn list_users(&self) -> Result<Vec<UserResponse>, AuthError>;
    async fn get_user(&self, id: UserId) -> Result<UserResponse, AuthError>;
    async fn update_user(
        &self,
        id: UserId,
        req: AdminUpdateUserRequest,
    ) -> Result<UserResponse, AuthError>;
    async fn delete_user(
        &self,
        requesting_user_id: UserId,
        target_user_id: UserId,
    ) -> Result<(), AuthError>;
    async fn regenerate_user_api_key(&self, user_id: UserId) -> Result<ApiKeyResponse, AuthError>;
    async fn verify_credentials(&self, username: &str, password: &str) -> Result<User, AuthError>;
    async fn is_setup_complete(&self) -> Result<bool, AuthError>;
    async fn verify_token(&self, token: &str) -> Result<i64, AuthError>;
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    pub remember_me: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupResponse {
    pub api_key: String,
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatusResponse {
    pub setup_required: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProfileRequest {
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyResponse {
    pub api_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminCreateUserRequest {
    pub username: String,
    pub password: String,
    pub role: UserRole,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminUpdateUserRequest {
    pub username: Option<String>,
    pub password: Option<String>,
    pub role: Option<UserRole>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserResponse {
    pub id: UserId,
    pub username: String,
    pub role: UserRole,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthMeResponse {
    pub user: UserResponse,
    pub auth_type: AuthType,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("account locked")]
    AccountLocked,
    #[error("setup already completed")]
    SetupCompleted,
    #[error("setup required")]
    SetupRequired,
    #[error("cannot delete self")]
    CannotDeleteSelf,
    #[error("cannot remove last admin")]
    LastAdmin,
    #[error("user not found")]
    UserNotFound,
    #[error("username already taken")]
    UsernameTaken,
    #[error("invalid username: {reason}")]
    InvalidUsername { reason: String },
    #[error("invalid password: {reason}")]
    InvalidPassword { reason: String },
    #[error("session expired")]
    SessionExpired,
    #[error("forbidden")]
    Forbidden,
    #[error("database error: {0}")]
    Db(#[from] DbError),
}
