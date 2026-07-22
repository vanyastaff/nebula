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
use zeroize::Zeroize;

use super::error::AuthError;

/// Encode raw entropy as a URL-safe base64 string.
fn encode_url_safe(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Default session lifetime — 14 days.
pub const SESSION_TTL: Duration = Duration::from_hours(14 * 24);

/// Host-bound cookie name for the session ID.
///
/// The `__Host-` prefix is browser-enforced: the cookie must be `Secure`,
/// must use `Path=/`, and cannot carry `Domain`. This prevents a sibling
/// subdomain from shadowing Nebula's session authority.
pub const SESSION_COOKIE: &str = "__Host-nebula-session";

/// Host-bound cookie name for the double-submit CSRF token.
pub const CSRF_COOKIE: &str = "__Host-nebula-csrf";

/// Request header carrying the double-submit CSRF token.
///
/// HTTP field names are case-insensitive; the lower-case spelling is the
/// canonical programmatic form used by middleware and CORS policy.
pub const CSRF_HEADER: &str = "x-csrf-token";

/// A live session record returned by the backend after a successful login.
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

impl std::fmt::Debug for SessionRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionRecord")
            .field("id", &"[redacted]")
            .field("principal", &"[redacted]")
            .field("csrf_token", &"[redacted]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl Drop for SessionRecord {
    fn drop(&mut self) {
        self.id.zeroize();
        self.csrf_token.zeroize();
    }
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
    cookie(CSRF_COOKIE, csrf_token, false, SESSION_TTL)
}

/// Build a `Set-Cookie` header that clears the session cookie.
///
/// The deletion preserves the session cookie's host-bound and `HttpOnly`
/// policy so no response ever emits this cookie name with weaker attributes.
#[must_use]
pub fn cleared_session_cookie() -> String {
    cookie(SESSION_COOKIE, "", true, Duration::ZERO)
}

/// Build a `Set-Cookie` header that clears the readable CSRF cookie.
#[must_use]
pub fn cleared_csrf_cookie() -> String {
    cookie(CSRF_COOKIE, "", false, Duration::ZERO)
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
    use std::collections::HashSet;

    use super::*;

    static_assertions::assert_not_impl_any!(SessionRecord: Clone);

    #[test]
    fn random_token_unique_and_decodes() {
        let a = random_token(32).unwrap();
        let b = random_token(32).unwrap();
        assert_ne!(a, b, "two random tokens must differ");
        let decoded = URL_SAFE_NO_PAD.decode(&a).expect("decodes");
        assert_eq!(decoded.len(), 32);
    }

    #[test]
    fn session_record_debug_redacts_session_and_csrf_authority() {
        const CANARY: &str = "SESSION_AUTHORITY_CANARY-6cb1";
        let record = SessionRecord {
            id: CANARY.to_owned(),
            principal: Principal::System,
            csrf_token: CANARY.to_owned(),
            expires_at: Utc::now(),
        };

        let debug = format!("{record:?}");
        assert!(!debug.contains(CANARY));
        assert!(debug.contains("SessionRecord"));
    }

    fn attributes(cookie: &str) -> HashSet<&str> {
        cookie.split("; ").skip(1).collect()
    }

    #[test]
    fn session_and_csrf_cookies_share_one_host_bound_policy_and_ttl() {
        let session = session_cookie("abc123");
        let csrf = csrf_cookie("xyz789");
        let session_attributes = attributes(&session);
        let csrf_attributes = attributes(&csrf);
        let max_age = format!("Max-Age={}", SESSION_TTL.as_secs());

        assert_eq!(
            session.split("; ").next(),
            Some("__Host-nebula-session=abc123")
        );
        assert_eq!(csrf.split("; ").next(), Some("__Host-nebula-csrf=xyz789"));
        for attributes in [&session_attributes, &csrf_attributes] {
            assert!(attributes.contains("Path=/"));
            assert!(attributes.contains("Secure"));
            assert!(attributes.contains("SameSite=Lax"));
            assert!(attributes.contains(max_age.as_str()));
            assert!(
                !attributes
                    .iter()
                    .any(|attribute| attribute.to_ascii_lowercase().starts_with("domain=")),
                "__Host- cookies must never carry Domain"
            );
        }
        assert!(session_attributes.contains("HttpOnly"));
        assert!(
            !csrf_attributes.contains("HttpOnly"),
            "CSRF cookie must be readable by JS"
        );
    }

    #[test]
    fn cleared_cookies_preserve_their_security_attributes() {
        let session = cleared_session_cookie();
        let csrf = cleared_csrf_cookie();

        assert_eq!(
            session,
            "__Host-nebula-session=; Path=/; Max-Age=0; Secure; SameSite=Lax; HttpOnly"
        );
        assert_eq!(
            csrf,
            "__Host-nebula-csrf=; Path=/; Max-Age=0; Secure; SameSite=Lax"
        );
    }
}
