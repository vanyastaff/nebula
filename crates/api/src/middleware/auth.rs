//! Authentication Middleware
//!
//! JWT Bearer token validation.

use axum::{
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::Response,
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

/// Standard JWT claims validated on every request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject — user ID.
    pub sub: String,
    /// Expiration time (Unix timestamp).
    pub exp: u64,
    /// Issued-at time (Unix timestamp).
    pub iat: u64,
}

/// Typed extension inserted into the request after successful auth.
#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    /// Authenticated user ID from the JWT `sub` claim.
    pub user_id: String,
}

/// JWT Bearer authentication middleware.
///
/// Expects `Authorization: Bearer <token>` header.
/// Validates the token signature and expiry against the server's JWT secret.
/// Inserts [`AuthenticatedUser`] into request extensions on success.
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let key = DecodingKey::from_secret(state.jwt_secret.as_bytes());
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;

    let token_data =
        decode::<Claims>(token, &key, &validation).map_err(|_| StatusCode::UNAUTHORIZED)?;

    request.extensions_mut().insert(AuthenticatedUser {
        user_id: token_data.claims.sub,
    });

    Ok(next.run(request).await)
}
