//! Request and response DTOs for auth endpoints.
//!
//! These are deserialized by the handlers and validated before reaching the
//! [`AuthBackend`](super::AuthBackend). Keeping them outside `state.rs`
//! avoids cross-handler coupling and lets new fields be added without a
//! state-lock dance.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use zeroize::ZeroizeOnDrop;

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
#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    /// Account email.
    pub email: String,
    /// Plaintext password — handed straight to the Argon2id verifier.
    #[schema(value_type = String, format = "password", write_only = true)]
    pub password: SecretString,
    /// Optional 6-digit TOTP code when the account has MFA enabled.
    #[serde(default)]
    pub totp: Option<String>,
}

/// `POST /auth/forgot-password` request body.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ForgotPasswordRequest {
    /// Account email — endpoint always responds 202 Accepted to avoid
    /// account enumeration.
    pub email: String,
}

/// `POST /auth/reset-password` request body.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ResetPasswordRequest {
    /// One-time reset token previously emailed to the user.
    pub token: String,
    /// New plaintext password.
    #[schema(value_type = String, format = "password", write_only = true)]
    pub new_password: SecretString,
}

/// `POST /auth/verify-email` request body.
#[derive(Debug, Deserialize, ToSchema)]
pub struct VerifyEmailRequest {
    /// One-time verification token previously emailed to the user.
    pub token: String,
}

/// `POST /auth/mfa/enroll` request body — empty; identity comes from the
/// authenticated session.
#[derive(Debug, Deserialize, Default, ToSchema)]
pub struct MfaEnrollRequest {}

/// `POST /auth/mfa/verify` request body — enrollment-confirm path.
///
/// This endpoint is session-bearing and CSRF-gated; identity comes from
/// the `nebula_session` cookie. The cookie-less second-factor login
/// completion path lives at `POST /auth/login/mfa` with
/// [`MfaLoginCompleteRequest`].
#[derive(Debug, Deserialize, ToSchema)]
pub struct MfaConfirmEnrollRequest {
    /// 6-digit TOTP code from the user's authenticator app.
    pub code: String,
}

/// `POST /auth/login/mfa` request body — second-factor login completion.
///
/// This endpoint is cookie-less (the caller has no session yet) and
/// therefore CSRF-exempt by construction; the `challenge_token` issued by
/// the password-step `/auth/login` response is the only authority.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MfaLoginCompleteRequest {
    /// 6-digit TOTP code from the user's authenticator app.
    pub code: String,
    /// MFA-challenge token returned by `/auth/login` when MFA is required.
    pub challenge_token: String,
}

/// Response after a successful login (no MFA required).
#[derive(Debug, Serialize, ToSchema)]
pub struct LoginResponse {
    /// Resolved user profile (no secrets).
    pub user: UserProfile,
    /// Opaque session ID — also sent as the `nebula_session` cookie.
    pub session_id: String,
    /// CSRF token paired with the session — sent as the `nebula_csrf` cookie.
    pub csrf_token: String,
}

/// Response when login succeeded the password step but MFA is required.
#[derive(Debug, Serialize, ToSchema)]
pub struct MfaChallengeResponse {
    /// MFA-required flag for the client.
    #[serde(rename = "mfa_required")]
    pub mfa_required: bool,
    /// Opaque, single-use challenge token to be passed back to `mfa/verify`.
    pub challenge_token: String,
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
#[derive(Debug, Serialize, ToSchema)]
pub struct MfaEnrollResponse {
    /// `otpauth://totp/...` URI to be displayed as a QR code.
    pub otpauth_uri: String,
    /// Base32 secret in case the authenticator app rejects the URI form.
    pub secret_base32: String,
}

/// Response for the OAuth start endpoint.
#[derive(Debug, Serialize, ToSchema)]
pub struct OAuthStartResponse {
    /// Provider authorization URL the client should redirect to.
    pub authorize_url: String,
    /// Opaque state token (also stored server-side, single-use).
    pub state: String,
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
#[derive(Clone, Default, ZeroizeOnDrop)]
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
}
