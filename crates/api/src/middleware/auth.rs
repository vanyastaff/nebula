//! Authentication Middleware
//!
//! Accepts either a JWT Bearer token (`Authorization: Bearer <token>`) or a
//! static API key (`X-API-Key: nbl_sk_…`). Both paths inject [`AuthenticatedUser`]
//! into request extensions so downstream handlers are auth-mechanism agnostic.

use axum::{
    extract::{Request, State},
    http::{HeaderName, StatusCode, header},
    middleware::Next,
    response::Response,
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

/// The canonical prefix for Nebula API keys.
pub const API_KEY_PREFIX: &str = "nbl_sk_";

/// Custom header name for API key authentication.
static X_API_KEY: HeaderName = HeaderName::from_static("x-api-key");

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
    /// Authenticated user ID from the JWT `sub` claim, or `"api_key"` when
    /// the request was authenticated via `X-API-Key`.
    pub user_id: String,
}

/// Combined JWT Bearer + API key authentication middleware.
///
/// The middleware tries two paths in order:
///
/// 1. **`X-API-Key` header** — if present, the value is compared in constant time against the
///    configured `api_keys`. On match the request is allowed; on mismatch a 401 is returned
///    immediately (no JWT fallback).
/// 2. **`Authorization: Bearer <token>`** — validated against the server JWT secret with HS256.
///    Expiry is enforced.
///
/// At least one must succeed, otherwise 401 is returned.
///
/// API keys must use the [`API_KEY_PREFIX`] prefix (`nbl_sk_`). Keys without the
/// prefix are silently rejected.
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // ── Path 1: X-API-Key header ─────────────────────────────────────────────
    if let Some(api_key_value) = request.headers().get(&X_API_KEY) {
        let provided = api_key_value.to_str().unwrap_or("");

        // Keys without the canonical prefix are always invalid.
        if !provided.starts_with(API_KEY_PREFIX) {
            return Err(StatusCode::UNAUTHORIZED);
        }

        // Fold over ALL keys without short-circuiting so the number of keys and
        // which key matched cannot be inferred from elapsed time (timing oracle).
        let matched = state.api_keys.iter().fold(false, |found, k| {
            found | constant_time_eq(k.as_bytes(), provided.as_bytes())
        });

        if !matched {
            return Err(StatusCode::UNAUTHORIZED);
        }

        request.extensions_mut().insert(AuthenticatedUser {
            user_id: "api_key".to_string(),
        });
        return Ok(next.run(request).await);
    }

    // ── Path 2: JWT Bearer token ──────────────────────────────────────────────
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

/// Constant-time byte-slice equality.
///
/// Both slices are compared in O(max(a.len(), b.len())) regardless of where
/// they first differ, preventing timing side-channels.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        // Still touch every byte of `a` to avoid length oracle leaks.
        let _ = a.iter().fold(0u8, |acc, x| acc ^ x);
        return false;
    }
    // XOR all bytes together; any difference leaves a non-zero result.
    let diff = a
        .iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y));
    diff == 0
}
