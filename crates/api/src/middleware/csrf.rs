//! CSRF protection middleware using double-submit cookie pattern.
//!
//! For state-changing requests (POST, PUT, PATCH, DELETE):
//! - Verifies `X-CSRF-Token` header matches `nebula_csrf` cookie
//! - Skips for PAT/API-key auth (no cookie = no CSRF risk)
//! - GET/HEAD/OPTIONS requests are exempt

use axum::{
    extract::Request,
    http::{Method, header},
    middleware::Next,
    response::Response,
};

use crate::{
    errors::ApiError,
    middleware::auth::{AuthContext, AuthMethod},
};

/// CSRF verification middleware.
///
/// Must run AFTER auth middleware (needs [`AuthContext`] to check auth method).
///
/// Enforces double-submit cookie verification for state-changing requests
/// when the caller authenticated via session cookie or JWT. PAT and API-key
/// authenticated requests are exempt because they don't use cookies.
pub async fn csrf_middleware(request: Request, next: Next) -> Result<Response, ApiError> {
    // Only check state-changing methods
    let needs_csrf = matches!(
        *request.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );

    if !needs_csrf {
        return Ok(next.run(request).await);
    }

    // Skip CSRF for non-cookie auth methods (PAT, API key)
    if let Some(auth_ctx) = request.extensions().get::<AuthContext>() {
        match auth_ctx.auth_method {
            AuthMethod::Pat | AuthMethod::ApiKey => {
                return Ok(next.run(request).await);
            },
            AuthMethod::Session | AuthMethod::Jwt => {
                // Session/JWT auth uses cookies — need CSRF check
            },
        }
    }

    // Extract CSRF token from header
    let csrf_header = request
        .headers()
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        .map(ToOwned::to_owned);

    // Extract CSRF cookie
    let csrf_cookie = extract_cookie(&request, "nebula_csrf");

    // Verify they match
    match (csrf_header, csrf_cookie) {
        (Some(ref header_val), Some(ref cookie_val)) if header_val == cookie_val => {
            // Valid — proceed
            Ok(next.run(request).await)
        },
        (None, _) | (_, None) => {
            // Missing token
            Err(ApiError::Forbidden("CSRF token missing".to_string()))
        },
        _ => {
            // Mismatch
            Err(ApiError::Forbidden("CSRF token mismatch".to_string()))
        },
    }
}

/// Extract a named cookie from the request.
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
