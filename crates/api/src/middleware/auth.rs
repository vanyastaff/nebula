//! Authentication middleware — **Plane A** (host / Nebula API).
//!
//! Accepts session cookies, PAT tokens, static API keys, or JWT Bearer tokens.
//! Both [`AuthenticatedUser`] (legacy) and [`AuthContext`] are inserted into
//! request extensions so downstream middleware and handlers can use either.
//!
//! This is **not** integration credential OAuth (**Plane B**). Integration OAuth client routes
//! live in the `credential` module (feature `credential-oauth`); see ADR-0033.

use std::str::FromStr;

use axum::{
    extract::{Request, State},
    http::{HeaderName, StatusCode, header},
    middleware::Next,
    response::Response,
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use nebula_core::UserId;
use serde::{Deserialize, Serialize};

use crate::state::AppState;

/// The canonical prefix for Nebula API keys.
pub const API_KEY_PREFIX: &str = "nbl_sk_";

/// The canonical prefix for personal access tokens.
pub const PAT_PREFIX: &str = "pat_";

/// Cookie name for session-based authentication.
pub const SESSION_COOKIE: &str = "nebula_session";

/// Custom header name for API key authentication.
///
/// Exposed so the CORS layer in `app::build_cors_layer` references
/// the same header constant as the auth middleware — there is
/// exactly one place the `x-api-key` string lives.
pub(crate) static X_API_KEY: HeaderName = HeaderName::from_static("x-api-key");

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
///
/// Kept for backward compatibility — new code should prefer [`AuthContext`].
#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    /// Authenticated user ID from the JWT `sub` claim, or `"api_key"` when
    /// the request was authenticated via `X-API-Key`.
    pub user_id: String,
}

/// Authentication context extracted by auth middleware.
///
/// Inserted into request extensions for downstream middleware and handlers.
#[derive(Debug, Clone)]
pub struct AuthContext {
    /// The resolved principal identity.
    pub principal: nebula_core::Principal,
    /// Which authentication method was used.
    pub auth_method: AuthMethod,
}

/// The authentication mechanism that was used for the current request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    /// Session cookie (`nebula_session`).
    Session,
    /// Personal access token (`pat_…`).
    Pat,
    /// Static API key (`nbl_sk_…`).
    ApiKey,
    /// JWT Bearer token.
    Jwt,
}

/// Combined authentication middleware supporting four auth methods.
///
/// The middleware tries each path in order:
///
/// 1. **Session cookie** (`nebula_session`) — resolved via [`SessionStore`] port.
/// 2. **PAT** — `Authorization: Bearer pat_…`, resolved via hash lookup (stub).
/// 3. **`X-API-Key` header** — compared in constant time against configured keys.
/// 4. **JWT Bearer** — validated against the server JWT secret with HS256.
///
/// At least one must succeed, otherwise 401 is returned.
///
/// Both [`AuthenticatedUser`] (legacy) and [`AuthContext`] are inserted into
/// request extensions on success.
///
/// [`SessionStore`]: crate::state::SessionStore
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // ── Path 1: Session cookie ──────────────────────────────────────────────
    if let Some(session_id) = extract_cookie(&request, SESSION_COOKIE)
        && let Some(ref store) = state.session_store
    {
        match store.get_principal_by_session(&session_id).await {
            Ok(Some(principal)) => {
                let user_id = principal_user_id(&principal);
                request.extensions_mut().insert(AuthenticatedUser {
                    user_id: user_id.clone(),
                });
                request.extensions_mut().insert(AuthContext {
                    principal,
                    auth_method: AuthMethod::Session,
                });
                return Ok(next.run(request).await);
            },
            Ok(None) => {
                // Session not found or expired — fall through to other methods
            },
            Err(_) => {
                // Store error — fall through
            },
        }
    }

    // ── Path 2: PAT (Authorization: Bearer pat_…) ───────────────────────────
    if let Some(bearer_value) = extract_bearer(&request)
        && bearer_value.starts_with(PAT_PREFIX)
    {
        // TODO: Resolve PAT via hash lookup when PAT store is available.
        // For now, parse the suffix as a user ID for development purposes.
        // In production, PATs will be hashed and looked up in the database.
        return Err(StatusCode::UNAUTHORIZED);
    }

    // ── Path 3: X-API-Key header ─────────────────────────────────────────────
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
        request.extensions_mut().insert(AuthContext {
            principal: nebula_core::Principal::System,
            auth_method: AuthMethod::ApiKey,
        });
        return Ok(next.run(request).await);
    }

    // ── Path 4: JWT Bearer token ──────────────────────────────────────────────
    let token = extract_bearer(&request).ok_or(StatusCode::UNAUTHORIZED)?;

    let key = DecodingKey::from_secret(state.jwt_secret.as_bytes());
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;

    let token_data =
        decode::<Claims>(token, &key, &validation).map_err(|_| StatusCode::UNAUTHORIZED)?;

    let user_id_str = token_data.claims.sub.clone();
    let principal = if let Ok(uid) = UserId::from_str(&user_id_str) {
        nebula_core::Principal::User(uid)
    } else {
        // Fallback: treat as system principal for non-parseable subjects
        nebula_core::Principal::System
    };

    request.extensions_mut().insert(AuthenticatedUser {
        user_id: user_id_str,
    });
    request.extensions_mut().insert(AuthContext {
        principal,
        auth_method: AuthMethod::Jwt,
    });

    Ok(next.run(request).await)
}

/// Extract the Bearer token value from the `Authorization` header.
fn extract_bearer(request: &Request) -> Option<&str> {
    request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

/// Extract a named cookie value from the request.
fn extract_cookie(request: &Request, name: &str) -> Option<String> {
    let cookie_header = request.headers().get(header::COOKIE)?;
    let cookie_str = cookie_header.to_str().ok()?;

    for pair in cookie_str.split(';') {
        let pair = pair.trim();
        if let Some(value) = pair.strip_prefix(name)
            && let Some(value) = value.strip_prefix('=')
        {
            return Some(value.to_string());
        }
    }

    None
}

/// Extract a user-facing ID string from a [`Principal`].
fn principal_user_id(principal: &nebula_core::Principal) -> String {
    match principal {
        nebula_core::Principal::User(uid) => uid.to_string(),
        nebula_core::Principal::ServiceAccount(sid) => sid.to_string(),
        nebula_core::Principal::Workflow { workflow_id, .. } => workflow_id.to_string(),
        nebula_core::Principal::System => "system".to_string(),
    }
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
