//! Auth middleware for Axum — extracts Bearer token or X-Api-Key.

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

use crate::auth_crypto::{AuthCryptoService, RealAuthCrypto};
use crate::state::AppState;
use crate::{AuthContext, AuthType};
use livrarr_db::{SessionDb, UserDb};

/// Axum middleware: authenticate via Bearer token or X-Api-Key header.
/// Injects AuthContext as a request extension.
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let crypto = RealAuthCrypto;

    // Try Bearer token first
    if let Some(token) = extract_bearer(req.headers()) {
        let token_hash = crypto
            .hash_token(token)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        if let Ok(Some(session)) = state.db.get_session(&token_hash).await {
            if let Ok(user) = state.db.get_user(session.user_id).await {
                let ctx = AuthContext {
                    user,
                    auth_type: AuthType::Session,
                    session_token_hash: Some(token_hash),
                };
                req.extensions_mut().insert(ctx);
                return Ok(next.run(req).await);
            }
        }

        return Err(StatusCode::UNAUTHORIZED);
    }

    // Try X-Api-Key
    if let Some(api_key) = req.headers().get("x-api-key").and_then(|v| v.to_str().ok()) {
        let key_hash = crypto
            .hash_token(api_key)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        if let Ok(user) = state.db.get_user_by_api_key_hash(&key_hash).await {
            let ctx = AuthContext {
                user,
                auth_type: AuthType::ApiKey,
                session_token_hash: None,
            };
            req.extensions_mut().insert(ctx);
            return Ok(next.run(req).await);
        }

        return Err(StatusCode::UNAUTHORIZED);
    }

    Err(StatusCode::UNAUTHORIZED)
}

fn extract_bearer(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// Axum extractor: requires the authenticated user to be an admin.
/// Must be used after auth middleware (AuthContext must be in extensions).
pub struct RequireAdmin(pub AuthContext);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for RequireAdmin {
    type Rejection = crate::ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let ctx = parts
            .extensions
            .get::<AuthContext>()
            .cloned()
            .ok_or(crate::ApiError::Unauthorized)?;

        if ctx.user.role != livrarr_db::UserRole::Admin {
            return Err(crate::ApiError::Forbidden);
        }

        Ok(RequireAdmin(ctx))
    }
}
