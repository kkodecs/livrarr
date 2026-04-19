use crate::types::api_error::ApiError;
use crate::types::auth::AuthContext;

pub struct RequireAdmin(pub AuthContext);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for RequireAdmin {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let ctx = parts
            .extensions
            .get::<AuthContext>()
            .cloned()
            .ok_or(ApiError::Unauthorized)?;

        if ctx.user.role != livrarr_domain::UserRole::Admin {
            return Err(ApiError::Forbidden);
        }

        Ok(RequireAdmin(ctx))
    }
}
