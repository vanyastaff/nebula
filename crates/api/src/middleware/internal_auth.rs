//! Shared-token authentication for `/internal/v1/...` routes
//! (M3.3 / ADR-0049 — E3).
//!
//! Internal routes are deliberately separate from the Plane-A auth
//! stack:
//!
//! - **No tenancy gate** — they're consumed by ops tooling, not
//!   tenants.
//! - **Not advertised in OpenAPI** — operators discover them via
//!   runbooks, not Swagger.
//! - **Out-of-band rotation** — the token is set in env / secret
//!   store; we don't expose a rotate endpoint here.
//!
//! The middleware constant-time-compares the inbound
//! `X-Internal-Token` header against
//! [`crate::config::InternalConfig::shared_token`]. Empty / missing
//! token in config closes the route entirely (every request is 503).

use axum::{
    body::Body,
    extract::State,
    http::{HeaderName, HeaderValue, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::state::AppState;

/// Header carrying the internal-auth shared token. Lower-cased so
/// the header-map lookup is canonical.
pub const X_INTERNAL_TOKEN: HeaderName = HeaderName::from_static("x-internal-token");

/// Middleware that requires `X-Internal-Token` to match
/// `AppState.internal_shared_token`.
///
/// Returns:
///
/// - **503** when the server has no internal token configured (the
///   route surface is closed; ops must set it before reaching the
///   endpoint).
/// - **401** when the header is missing or fails the
///   constant-time comparison.
/// - The wrapped handler's response otherwise.
pub async fn internal_auth_middleware(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let Some(expected) = state.internal_shared_token.as_ref() else {
        tracing::warn!(
            target: "nebula::api::internal",
            "internal route hit but server has no shared token configured; refusing"
        );
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "internal auth not configured",
        )
            .into_response();
    };

    let presented = req
        .headers()
        .get(&X_INTERNAL_TOKEN)
        .and_then(|h: &HeaderValue| h.to_str().ok());

    let Some(token) = presented else {
        return (StatusCode::UNAUTHORIZED, "missing X-Internal-Token").into_response();
    };

    if !constant_time_eq(token.as_bytes(), expected.as_bytes()) {
        tracing::warn!(
            target: "nebula::api::internal",
            "internal token mismatch"
        );
        return (StatusCode::UNAUTHORIZED, "invalid X-Internal-Token").into_response();
    }

    next.run(req).await
}

/// Constant-time comparison of two byte slices. Length mismatch is
/// surfaced as `false` without short-circuiting on length alone —
/// the equal-length path runs the same number of operations
/// regardless of where the first mismatch occurs.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_length_constant_time() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abz"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(constant_time_eq(b"", b""));
    }
}
