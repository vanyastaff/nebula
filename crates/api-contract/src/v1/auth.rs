//! Request and response DTOs for auth endpoints.
//!
//! These are deserialized by the handlers and validated before reaching the
//! `AuthBackend`. Keeping them outside `state.rs`
//! avoids cross-handler coupling and lets new fields be added without a
//! state-lock dance.

use serde::{Deserialize, Serialize};
#[cfg(feature = "openapi")]
use utoipa::ToSchema;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// `POST /auth/signup` request body.
///
/// `password` is wrapped so it never lingers in memory after dropping.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SignupRequest {
    /// Caller-supplied email address — lowercased and trimmed before storage.
    pub email: String,
    /// Plaintext password — handed straight to the Argon2id hasher.
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "password", write_only = true))]
    pub password: SecretString,
    /// Caller-chosen display name (1..=128 chars).
    pub display_name: String,
}

/// `POST /auth/login` request body.
#[derive(Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LoginRequest {
    /// Account email.
    pub email: String,
    /// Plaintext password — handed straight to the Argon2id verifier.
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "password", write_only = true))]
    pub password: SecretString,
    /// Optional 6-digit TOTP code when the account has MFA enabled.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(write_only = true))]
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
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ForgotPasswordRequest {
    /// Account email — endpoint always responds 202 Accepted to avoid
    /// account enumeration.
    pub email: String,
}

/// `POST /auth/reset-password` request body.
#[derive(Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ResetPasswordRequest {
    /// One-time reset token previously emailed to the user.
    #[cfg_attr(feature = "openapi", schema(format = "password", write_only = true))]
    pub token: String,
    /// New plaintext password.
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "password", write_only = true))]
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
#[derive(Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct VerifyEmailRequest {
    /// One-time verification token previously emailed to the user.
    #[cfg_attr(feature = "openapi", schema(format = "password", write_only = true))]
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
#[derive(Debug, Deserialize, Default, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MfaEnrollRequest {}

/// `POST /auth/mfa/verify` request body — enrollment-confirm path.
///
/// This endpoint is session-bearing and CSRF-gated; identity comes from
/// the `__Host-nebula-session` cookie. The cookie-less second-factor login
/// completion path lives at `POST /auth/login/mfa` with
/// [`MfaLoginCompleteRequest`].
#[derive(Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MfaConfirmEnrollRequest {
    /// 6-digit TOTP code from the user's authenticator app.
    #[cfg_attr(feature = "openapi", schema(write_only = true))]
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
#[derive(Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MfaLoginCompleteRequest {
    /// 6-digit TOTP code from the user's authenticator app.
    #[cfg_attr(feature = "openapi", schema(write_only = true))]
    pub code: String,
    /// MFA-challenge token returned by a first-factor endpoint when MFA is required.
    #[cfg_attr(feature = "openapi", schema(format = "password", write_only = true))]
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
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LoginResponse {
    /// Resolved user profile (no secrets).
    pub user: UserProfile,
    /// CSRF token paired with the session — sent as the readable
    /// `__Host-nebula-csrf` cookie. The session bearer itself is deliberately absent
    /// from JSON and exists only in the `HttpOnly` session cookie.
    #[cfg_attr(feature = "openapi", schema(read_only = true))]
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
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MfaChallengeResponse {
    /// MFA-required flag for the client.
    #[serde(rename = "mfa_required")]
    pub mfa_required: bool,
    /// Opaque, single-use challenge token to pass to `/auth/login/mfa`.
    #[cfg_attr(feature = "openapi", schema(format = "password", read_only = true))]
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
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SignupResponse {
    /// Resolved user profile (no secrets).
    pub user: UserProfile,
    /// `true` when an email-verification message was queued for delivery.
    pub verification_email_sent: bool,
}

/// Response after MFA enrollment — exposes the otpauth URI **once**
/// so the client can render a QR code.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MfaEnrollResponse {
    /// `otpauth://totp/...` URI to be displayed as a QR code.
    #[cfg_attr(feature = "openapi", schema(format = "uri", read_only = true))]
    pub otpauth_uri: String,
    /// Base32 secret in case the authenticator app rejects the URI form.
    #[cfg_attr(feature = "openapi", schema(format = "password", read_only = true))]
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
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct OAuthStartResponse {
    /// Provider authorization URL the client should redirect to.
    #[cfg_attr(feature = "openapi", schema(format = "uri", read_only = true))]
    pub authorize_url: String,
    /// Opaque state token (also stored server-side, single-use).
    #[cfg_attr(feature = "openapi", schema(format = "password", read_only = true))]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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

/// Supported Plane-A OAuth providers.
///
/// Serialize/Deserialize derived so `OAuthProvidersConfig` can use
/// `HashMap<OAuthProvider, OAuthProviderConfig>` keyed by enum value
/// (per ADR-0085 D-5). Each variant pins its exact serde and OpenAPI token;
/// this matters for `GitHub`, whose mechanical snake-case spelling would be
/// the incompatible `git_hub`. Drift against [`Self::as_str`] and `FromStr`
/// is covered by a unit test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[non_exhaustive]
pub enum OAuthProvider {
    /// Sign in with Google.
    #[serde(rename = "google")]
    #[cfg_attr(feature = "openapi", schema(rename = "google"))]
    Google,
    /// Sign in with GitHub.
    #[serde(rename = "github")]
    #[cfg_attr(feature = "openapi", schema(rename = "github"))]
    GitHub,
}

impl std::str::FromStr for OAuthProvider {
    type Err = OAuthProviderParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "google" => Ok(Self::Google),
            "github" => Ok(Self::GitHub),
            _ => Err(OAuthProviderParseError),
        }
    }
}

impl OAuthProvider {
    /// Stable string representation for storage / logging.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::GitHub => "github",
        }
    }
}

/// An unrecognized Plane-A identity-provider token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OAuthProviderParseError;

impl std::fmt::Display for OAuthProviderParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("unknown OAuth provider")
    }
}

impl std::error::Error for OAuthProviderParseError {}
/// Query string for the OAuth callback.
#[derive(Deserialize, Serialize)]
pub struct OAuthCallbackParams {
    /// Opaque state token previously issued by `start_oauth`.
    pub state: String,
    /// Authorization code returned by the provider, mutually exclusive with
    /// `error`.
    pub code: Option<String>,
    /// Provider error identifier, mutually exclusive with `code`. Its value
    /// is validated for shape but never surfaced or logged.
    pub error: Option<String>,
}

impl std::fmt::Debug for OAuthCallbackParams {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthCallbackParams")
            .field("state", &"[redacted]")
            .field("code", &"[redacted]")
            .field("error", &"[redacted]")
            .finish()
    }
}

impl Serialize for SecretString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.expose())
    }
}
