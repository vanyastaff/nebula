//! Request and response DTOs for auth endpoints.
//!
//! These are deserialized by the handlers and validated before reaching the
//! [`AuthBackend`](super::AuthBackend). Keeping them outside `state.rs`
//! avoids cross-handler coupling and lets new fields be added without a
//! state-lock dance.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// `POST /auth/signup` request body.
///
/// `password` is wrapped so it never lingers in memory after dropping.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SignupRequest {
    /// Caller-supplied email address — lowercased and trimmed before storage.
    pub email: String,
    /// Plaintext password — handed straight to the Argon2id hasher.
    #[schema(value_type = String, format = "password", write_only = true)]
    pub password: SecretString,
    /// Caller-chosen display name (1..=128 chars).
    pub display_name: String,
}

/// `POST /auth/login` request body.
#[derive(Deserialize, ToSchema)]
pub struct LoginRequest {
    /// Account email.
    pub email: String,
    /// Plaintext password — handed straight to the Argon2id verifier.
    #[schema(value_type = String, format = "password", write_only = true)]
    pub password: SecretString,
    /// Optional 6-digit TOTP code when the account has MFA enabled.
    #[serde(default)]
    #[schema(write_only = true)]
    pub totp: Option<String>,
}

impl std::fmt::Debug for LoginRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LoginRequest")
            .field("email", &"[redacted]")
            .field("password", &"[redacted]")
            .field("totp", &self.totp.as_ref().map(|_| "[redacted]"))
            .finish()
    }
}

/// `POST /auth/forgot-password` request body.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ForgotPasswordRequest {
    /// Account email — endpoint always responds 202 Accepted to avoid
    /// account enumeration.
    pub email: String,
}

/// `POST /auth/reset-password` request body.
#[derive(Deserialize, ToSchema)]
pub struct ResetPasswordRequest {
    /// One-time reset token previously emailed to the user.
    #[schema(format = "password", write_only = true)]
    pub token: String,
    /// New plaintext password.
    #[schema(value_type = String, format = "password", write_only = true)]
    pub new_password: SecretString,
}

impl std::fmt::Debug for ResetPasswordRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResetPasswordRequest")
            .field("token", &"[redacted]")
            .field("new_password", &"[redacted]")
            .finish()
    }
}

/// `POST /auth/verify-email` request body.
#[derive(Deserialize, ToSchema)]
pub struct VerifyEmailRequest {
    /// One-time verification token previously emailed to the user.
    #[schema(format = "password", write_only = true)]
    pub token: String,
}

impl std::fmt::Debug for VerifyEmailRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifyEmailRequest")
            .field("token", &"[redacted]")
            .finish()
    }
}

/// `POST /auth/mfa/enroll` request body — empty; identity comes from the
/// authenticated session.
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "a unit struct does not deserialize from a JSON `{}` body; the braces are the API contract"
)]
#[derive(Debug, Deserialize, Default, ToSchema)]
pub struct MfaEnrollRequest {}

/// `POST /auth/mfa/verify` request body — enrollment-confirm path.
///
/// This endpoint is session-bearing and CSRF-gated; identity comes from
/// the `__Host-nebula-session` cookie. The cookie-less second-factor login
/// completion path lives at `POST /auth/login/mfa` with
/// [`MfaLoginCompleteRequest`].
#[derive(Deserialize, ToSchema)]
pub struct MfaConfirmEnrollRequest {
    /// 6-digit TOTP code from the user's authenticator app.
    #[schema(write_only = true)]
    pub code: String,
}

impl std::fmt::Debug for MfaConfirmEnrollRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MfaConfirmEnrollRequest")
            .field("code", &"[redacted]")
            .finish()
    }
}

/// `POST /auth/login/mfa` request body — second-factor login completion.
///
/// This endpoint is cookie-less (the caller has no session yet) and
/// therefore CSRF-exempt by construction; the `challenge_token` issued by
/// `/auth/login` or an OAuth callback is the only authority.
#[derive(Deserialize, ToSchema)]
pub struct MfaLoginCompleteRequest {
    /// 6-digit TOTP code from the user's authenticator app.
    #[schema(write_only = true)]
    pub code: String,
    /// MFA-challenge token returned by a first-factor endpoint when MFA is required.
    #[schema(format = "password", write_only = true)]
    pub challenge_token: String,
}

impl std::fmt::Debug for MfaLoginCompleteRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MfaLoginCompleteRequest")
            .field("code", &"[redacted]")
            .field("challenge_token", &"[redacted]")
            .finish()
    }
}

/// Response after a successful login (no MFA required).
#[derive(Serialize, ToSchema)]
pub struct LoginResponse {
    /// Resolved user profile (no secrets).
    pub user: UserProfile,
    /// CSRF token paired with the session — sent as the readable
    /// `__Host-nebula-csrf` cookie. The session bearer itself is deliberately absent
    /// from JSON and exists only in the `HttpOnly` session cookie.
    #[schema(read_only = true)]
    pub csrf_token: String,
}

impl std::fmt::Debug for LoginResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LoginResponse")
            .field("user", &"[redacted]")
            .field("csrf_token", &"[redacted]")
            .finish()
    }
}

impl Drop for LoginResponse {
    fn drop(&mut self) {
        self.csrf_token.zeroize();
    }
}

/// Response when a password or OAuth first factor succeeded but MFA is required.
#[derive(Serialize, ToSchema)]
pub struct MfaChallengeResponse {
    /// MFA-required flag for the client.
    #[serde(rename = "mfa_required")]
    pub mfa_required: bool,
    /// Opaque, single-use challenge token to pass to `/auth/login/mfa`.
    #[schema(format = "password", read_only = true)]
    pub challenge_token: String,
}

impl std::fmt::Debug for MfaChallengeResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MfaChallengeResponse")
            .field("mfa_required", &self.mfa_required)
            .field("challenge_token", &"[redacted]")
            .finish()
    }
}

impl Drop for MfaChallengeResponse {
    fn drop(&mut self) {
        self.challenge_token.zeroize();
    }
}

/// Response after a successful signup.
#[derive(Debug, Serialize, ToSchema)]
pub struct SignupResponse {
    /// Resolved user profile (no secrets).
    pub user: UserProfile,
    /// `true` when an email-verification message was queued for delivery.
    pub verification_email_sent: bool,
}

/// Response after MFA enrollment — exposes the otpauth URI **once**
/// so the client can render a QR code.
#[derive(Serialize, ToSchema)]
pub struct MfaEnrollResponse {
    /// `otpauth://totp/...` URI to be displayed as a QR code.
    #[schema(format = "uri", read_only = true)]
    pub otpauth_uri: String,
    /// Base32 secret in case the authenticator app rejects the URI form.
    #[schema(format = "password", read_only = true)]
    pub secret_base32: String,
}

impl std::fmt::Debug for MfaEnrollResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MfaEnrollResponse")
            .field("otpauth_uri", &"[redacted]")
            .field("secret_base32", &"[redacted]")
            .finish()
    }
}

impl Drop for MfaEnrollResponse {
    fn drop(&mut self) {
        self.otpauth_uri.zeroize();
        self.secret_base32.zeroize();
    }
}

/// Response for the OAuth start endpoint.
#[derive(Serialize, ToSchema)]
pub struct OAuthStartResponse {
    /// Provider authorization URL the client should redirect to.
    #[schema(format = "uri", read_only = true)]
    pub authorize_url: String,
    /// Opaque state token (also stored server-side, single-use).
    #[schema(format = "password", read_only = true)]
    pub state: String,
}

impl std::fmt::Debug for OAuthStartResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthStartResponse")
            .field("authorize_url", &"[redacted]")
            .field("state", &"[redacted]")
            .finish()
    }
}

impl Drop for OAuthStartResponse {
    fn drop(&mut self) {
        self.authorize_url.zeroize();
        self.state.zeroize();
    }
}

/// User profile shape returned to the client. **Never** contains password
/// hashes, MFA secrets, or PAT material.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UserProfile {
    /// `user_<ULID>` string form.
    pub user_id: String,
    /// Lowercased email.
    pub email: String,
    /// Caller-chosen display name.
    pub display_name: String,
    /// Avatar URL, if the user has set one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// `true` once the user has verified their email.
    pub email_verified: bool,
    /// `true` when the account has TOTP enrolled.
    pub mfa_enabled: bool,
}

/// Wrapper around a plaintext secret that zeroes its memory on drop.
///
/// Implements [`Deserialize`] so request bodies can be parsed directly.
#[derive(Default, ZeroizeOnDrop)]
pub struct SecretString(String);

impl SecretString {
    /// Wrap a plaintext value. Prefer the [`Deserialize`] path for HTTP
    /// inputs; this is for tests and trusted construction.
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// Borrow the inner plaintext for crypto operations.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Length of the wrapped value in bytes (used for validation).
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the wrapped string is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretString(***)")
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer).map(SecretString)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::assert_not_impl_any!(SecretString: Clone);

    #[test]
    fn secret_string_debug_redacts() {
        let s = SecretString::new("hunter2".to_owned());
        assert_eq!(format!("{s:?}"), "SecretString(***)");
        assert_eq!(s.expose(), "hunter2");
        assert_eq!(s.len(), 7);
        assert!(!s.is_empty());
    }

    #[test]
    fn login_request_deserializes_secret() {
        let req: LoginRequest =
            serde_json::from_str(r#"{"email":"a@b.c","password":"secret","totp":"123456"}"#)
                .expect("parse");
        assert_eq!(req.email, "a@b.c");
        assert_eq!(req.password.expose(), "secret");
        assert_eq!(req.totp.as_deref(), Some("123456"));
    }

    #[test]
    fn oauth_start_response_debug_redacts_url_and_state() {
        let response = OAuthStartResponse {
            authorize_url: "https://idp.example/authorize?state=URL_CANARY-a62b".to_owned(),
            state: "STATE_CANARY-c77e".to_owned(),
        };

        let debug = format!("{response:?}");
        assert!(!debug.contains("URL_CANARY-a62b"));
        assert!(!debug.contains("STATE_CANARY-c77e"));

        let wire = serde_json::to_value(&response).expect("OAuth start response serializes");
        assert_eq!(
            wire["authorize_url"],
            "https://idp.example/authorize?state=URL_CANARY-a62b"
        );
        assert_eq!(wire["state"], "STATE_CANARY-c77e");
    }

    #[test]
    fn mfa_challenge_response_debug_redacts_plaintext_but_wire_keeps_it() {
        const CANARY: &str = "MFA_CHALLENGE_CANARY-6f47";
        let response = MfaChallengeResponse {
            mfa_required: true,
            challenge_token: CANARY.to_owned(),
        };

        assert!(!format!("{response:?}").contains(CANARY));
        let wire = serde_json::to_value(&response).expect("MFA challenge response serializes");
        assert_eq!(wire["challenge_token"], CANARY);
    }

    #[test]
    fn auth_dto_debug_redacts_login_reset_mfa_and_session_authority() {
        const CANARY: &str = "AUTHORITY_CANARY-8f2c";
        let profile = || UserProfile {
            user_id: CANARY.to_owned(),
            email: format!("{CANARY}@example.test"),
            display_name: CANARY.to_owned(),
            avatar_url: Some(CANARY.to_owned()),
            email_verified: true,
            mfa_enabled: true,
        };
        let debug_values = [
            format!(
                "{:?}",
                LoginRequest {
                    email: format!("{CANARY}@example.test"),
                    password: SecretString::new(CANARY.to_owned()),
                    totp: Some(CANARY.to_owned()),
                }
            ),
            format!(
                "{:?}",
                ResetPasswordRequest {
                    token: CANARY.to_owned(),
                    new_password: SecretString::new(CANARY.to_owned()),
                }
            ),
            format!(
                "{:?}",
                VerifyEmailRequest {
                    token: CANARY.to_owned(),
                }
            ),
            format!(
                "{:?}",
                MfaConfirmEnrollRequest {
                    code: CANARY.to_owned(),
                }
            ),
            format!(
                "{:?}",
                MfaLoginCompleteRequest {
                    code: CANARY.to_owned(),
                    challenge_token: CANARY.to_owned(),
                }
            ),
            format!(
                "{:?}",
                LoginResponse {
                    user: profile(),
                    csrf_token: CANARY.to_owned(),
                }
            ),
            format!(
                "{:?}",
                MfaEnrollResponse {
                    otpauth_uri: format!("otpauth://totp/{CANARY}?secret={CANARY}"),
                    secret_base32: CANARY.to_owned(),
                }
            ),
        ];

        for debug in debug_values {
            assert!(!debug.contains(CANARY), "Debug leaked authority: {debug}");
        }

        let login_wire = serde_json::to_value(LoginResponse {
            user: profile(),
            csrf_token: CANARY.to_owned(),
        })
        .expect("login response serializes");
        assert!(login_wire.get("session_id").is_none());
        assert_eq!(login_wire["csrf_token"], CANARY);
    }
}
