//! Session helpers — opaque IDs, CSRF tokens, and Set-Cookie strings.
//!
//! The session record itself lives in the [`AuthBackend`](super::AuthBackend)
//! implementation; this module just owns the on-the-wire shape of session
//! cookies and the cryptographic primitives used to mint IDs.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use nebula_core::Principal;
use rand::Rng;

use super::error::AuthError;

/// Encode raw entropy as a URL-safe base64 string.
fn encode_url_safe(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Default session lifetime — 14 days.
pub const SESSION_TTL: Duration = Duration::from_hours(14 * 24);

/// Default CSRF token lifetime — matches session lifetime.
pub const CSRF_TTL: Duration = SESSION_TTL;

/// Cookie name for the session ID.
pub const SESSION_COOKIE: &str = "nebula_session";

/// Cookie name for the CSRF token.
pub const CSRF_COOKIE: &str = "nebula_csrf";

/// A live session record returned by the backend after a successful login.
#[derive(Debug, Clone)]
pub struct SessionRecord {
    /// Opaque, URL-safe base64 ID (32 bytes of entropy).
    pub id: String,
    /// Resolved principal (always [`Principal::User`] for password logins).
    pub principal: Principal,
    /// CSRF token paired with the session.
    pub csrf_token: String,
    /// Wall-clock expiry — clients are expected to honor `Expires`/`Max-Age`.
    pub expires_at: DateTime<Utc>,
}

/// Generate a fresh URL-safe base64 token of `bytes` bytes of entropy.
pub fn random_token(bytes: usize) -> Result<String, AuthError> {
    let mut buf = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buf);
    Ok(encode_url_safe(&buf))
}

/// Build a `Set-Cookie` header value for the session cookie.
///
/// Defaults: `Secure`, `HttpOnly`, `SameSite=Lax`, `Path=/`, `Max-Age=<TTL>`.
#[must_use]
pub fn session_cookie(session_id: &str) -> String {
    cookie(SESSION_COOKIE, session_id, true, SESSION_TTL)
}

/// Build a `Set-Cookie` header value for the CSRF cookie.
///
/// `HttpOnly` is **disabled** — the SPA must read the value to attach it to
/// requests via the `X-CSRF-Token` header.
#[must_use]
pub fn csrf_cookie(csrf_token: &str) -> String {
    cookie(CSRF_COOKIE, csrf_token, false, CSRF_TTL)
}

/// Build a `Set-Cookie` header that clears the named cookie.
#[must_use]
pub fn cleared_cookie(name: &str) -> String {
    format!("{name}=; Path=/; Max-Age=0; Secure; SameSite=Lax")
}

fn cookie(name: &str, value: &str, http_only: bool, ttl: Duration) -> String {
    let mut out = format!(
        "{name}={value}; Path=/; Max-Age={}; Secure; SameSite=Lax",
        ttl.as_secs()
    );
    if http_only {
        out.push_str("; HttpOnly");
    }
    out
}

/// Compute an absolute expiry time `ttl` from now.
#[must_use]
pub fn expires_at(ttl: Duration) -> DateTime<Utc> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    DateTime::<Utc>::from_timestamp(now + ttl.as_secs() as i64, 0).unwrap_or_else(Utc::now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_token_unique_and_decodes() {
        let a = random_token(32).unwrap();
        let b = random_token(32).unwrap();
        assert_ne!(a, b, "two random tokens must differ");
        let decoded = URL_SAFE_NO_PAD.decode(&a).expect("decodes");
        assert_eq!(decoded.len(), 32);
    }

    #[test]
    fn session_cookie_has_security_flags() {
        let cookie = session_cookie("abc123");
        assert!(cookie.starts_with("nebula_session=abc123;"));
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Path=/"));
        assert!(cookie.contains("Max-Age="));
    }

    #[test]
    fn csrf_cookie_omits_httponly() {
        let cookie = csrf_cookie("xyz789");
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(
            !cookie.contains("HttpOnly"),
            "CSRF cookie must be readable by JS"
        );
    }

    #[test]
    fn cleared_cookie_uses_max_age_zero() {
        let cookie = cleared_cookie(SESSION_COOKIE);
        assert!(cookie.contains("Max-Age=0"));
        assert!(cookie.starts_with("nebula_session=;"));
    }
}
