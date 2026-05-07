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
//! `AppState::internal_shared_token`. Empty / missing
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
    // Fail-closed on both `None` and `Some("")`: a configured-but-empty
    // token is the canonical secret-injection misconfiguration
    // (`API_INTERNAL_SHARED_TOKEN=` in env, helm chart with empty
    // value, etc.) and would otherwise let an empty `X-Internal-Token`
    // header bypass auth.
    let expected = match state.internal_shared_token.as_ref() {
        Some(t) if !t.is_empty() => t,
        _ => {
            tracing::warn!(
                target: "nebula::api::internal",
                "internal route hit but server has no usable shared token configured; refusing"
            );
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "internal auth not configured",
            )
                .into_response();
        },
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

/// Constant-time comparison of two byte slices.
///
/// The equal-length path runs in time independent of where the first
/// byte mismatch occurs — every byte is XOR'd into the accumulator.
/// Length mismatch is folded into the same accumulator (via the
/// length difference), so the function does **not** early-return on
/// `a.len() != b.len()` and an attacker cannot use response timing
/// to distinguish "right length, wrong bytes" from "wrong length".
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    // Fold the length difference into the accumulator so we never
    // early-return on length mismatch alone.
    let len_diff = (a.len() ^ b.len()) as u64;
    let mut diff: u32 = u32::from((len_diff | (len_diff >> 32)) != 0);
    let len = a.len().min(b.len());
    for i in 0..len {
        diff |= u32::from(a[i] ^ b[i]);
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
        assert!(!constant_time_eq(b"abcd", b"abc"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
    }
}
