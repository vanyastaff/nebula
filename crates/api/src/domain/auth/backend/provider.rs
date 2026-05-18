//! The [`AuthBackend`] trait — Plane-A identity + session contract.
//!
//! Replaces the older `SessionStore` trait. All auth-domain operations
//! (signup / login / MFA / OAuth / sessions / PATs) flow through this single
//! trait so callers never have to ask "which slot is the right one?".

use async_trait::async_trait;
use nebula_core::Principal;

use super::{
    dto::{SignupRequest, UserProfile},
    error::AuthError,
    oauth::OAuthProvider,
    pat::{MintedPat, PatRecord},
    session::SessionRecord,
};

/// Partial profile mutation for [`AuthBackend::update_user_profile`].
///
/// This is the **port-level** patch shape — deliberately decoupled from the
/// `me` HTTP DTO (`UpdateMeRequest`) so storage/transport schemas never leak
/// across the API boundary (3). The handler maps the wire DTO onto
/// this struct. Each `Some` field is applied; `None` leaves the field
/// unchanged.
#[derive(Debug, Clone, Default)]
pub struct ProfilePatch {
    /// Replacement display name, if present.
    pub display_name: Option<String>,
    /// Replacement avatar URL, if present.
    pub avatar_url: Option<String>,
}

/// PAT-creation parameters for [`AuthBackend::create_pat`].
///
/// Port-level shape decoupled from the `me` HTTP DTO (`CreateTokenRequest`),
/// per 3. The handler maps the wire DTO onto this struct after
/// validation.
#[derive(Debug, Clone)]
pub struct CreatePatParams {
    /// Caller-chosen friendly name.
    pub name: String,
    /// Granted scopes (`[]` = full access).
    pub scopes: Vec<String>,
    /// Optional time-to-live in seconds; `None` = non-expiring.
    pub ttl_seconds: Option<u64>,
}

/// Outcome of a password-step authentication.
#[derive(Debug, Clone)]
pub enum PasswordOutcome {
    /// Password verified and no MFA is required — caller may mint a session.
    Authenticated(UserProfile),
    /// Password verified but MFA is required. Caller stores the challenge
    /// token and surfaces it to the client; the client then calls
    /// `verify_mfa` with `code` + the same `challenge_token`.
    MfaRequired {
        /// Single-use challenge token paired to the user.
        challenge_token: String,
    },
}

/// Outcome of a successful MFA enrollment.
#[derive(Debug, Clone)]
pub struct MfaEnrollment {
    /// `otpauth://totp/...` URI rendered as a QR code by the client.
    pub otpauth_uri: String,
    /// Base32 secret in case the authenticator app rejects the URI.
    pub secret_base32: String,
}

/// Result of starting a Plane-A OAuth flow.
#[derive(Debug, Clone)]
pub struct OAuthStart {
    /// Provider authorize URL (state + PKCE challenge already included).
    pub authorize_url: String,
    /// Opaque state token (also stored server-side).
    pub state: String,
}

/// Result of completing a Plane-A OAuth flow.
#[derive(Debug, Clone)]
pub struct OAuthCompletion {
    /// User profile resolved from the provider response.
    pub user: UserProfile,
    /// Newly created session.
    pub session: SessionRecord,
}

/// Plane-A authentication contract.
///
/// **Required methods only — no defaults.** Tests / dev runtimes provide an
/// implementation (typically [`super::InMemoryAuthBackend`]); production
/// composition wires a storage-backed impl.
///
/// `dyn AuthBackend` is the production injection shape inside
/// [`crate::AppState`]; the trait is therefore `Send + Sync` and uses
/// `#[async_trait]` so a single `Box<dyn AuthBackend>` works across all
/// handlers.
#[async_trait]
pub trait AuthBackend: Send + Sync {
    /// Look up a session by ID — entry point shared with the auth middleware.
    /// Returns the resolved [`Principal`] when the session is live, `None`
    /// for unknown / expired / revoked sessions.
    async fn get_principal_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<Principal>, crate::ApiError>;

    /// Register a new user from the signup form. Returns the freshly
    /// minted profile; the implementation is responsible for queueing
    /// the verification email.
    async fn register_user(&self, req: SignupRequest) -> Result<UserProfile, AuthError>;

    /// Verify password (and TOTP, if supplied) for `email`. Returns a
    /// [`PasswordOutcome`] indicating whether MFA is still pending.
    async fn authenticate_password(
        &self,
        email: &str,
        password: &str,
        totp: Option<&str>,
    ) -> Result<PasswordOutcome, AuthError>;

    /// Complete the MFA step against a previously issued challenge token.
    async fn verify_mfa(&self, challenge_token: &str, code: &str)
    -> Result<UserProfile, AuthError>;

    /// Mint a session for the verified user.
    async fn create_session(&self, user_id: &str) -> Result<SessionRecord, AuthError>;

    /// Revoke a session (logout). Idempotent: revoking an unknown session is
    /// a successful no-op so logouts are safe to retry.
    async fn revoke_session(&self, session_id: &str) -> Result<(), AuthError>;

    /// Look up a presented PAT by its hash. Returns the record on success;
    /// `None` for unknown / revoked / expired tokens. The caller is expected
    /// to constant-time-compare the hash inside this method.
    async fn lookup_pat(&self, presented: &str) -> Result<Option<PatRecord>, AuthError>;

    // ── `me/*` identity surface ──────────────────────────────────────────
    //
    // The endpoints under `/api/v1/me/*` are authenticated, no tenant
    // scope, and operate strictly on the *caller's own* identity. They
    // delegate here so the handler never touches storage/`nebula_storage`
    // types directly (3) and the same single Plane-A contract
    // backs profile reads, profile patches, and PAT lifecycle.

    /// Resolve a user's own profile by id (`GET /me`). `Err(UserNotFound)`
    /// when the id does not resolve to a live user.
    async fn get_user_profile(&self, user_id: &str) -> Result<UserProfile, AuthError>;

    /// Apply a partial profile update for a user (`PATCH /me`) and return
    /// the post-update profile. Only `Some` fields in `patch` are written.
    async fn update_user_profile(
        &self,
        user_id: &str,
        patch: ProfilePatch,
    ) -> Result<UserProfile, AuthError>;

    /// List the active PATs owned by a user (`GET /me/tokens`). Returns
    /// metadata records only — the plaintext secret is never recoverable
    /// (only its SHA-256 is stored).
    async fn list_pats(&self, user_id: &str) -> Result<Vec<PatRecord>, AuthError>;

    /// Mint a new PAT for a user (`POST /me/tokens`). The returned
    /// [`MintedPat`] carries the plaintext **once**; the backend persists
    /// only the hashed record. Callers must surface the plaintext exactly
    /// once and never log it.
    async fn create_pat(
        &self,
        user_id: &str,
        params: CreatePatParams,
    ) -> Result<MintedPat, AuthError>;

    /// Revoke a PAT the caller owns by its `pat_`-prefixed token id
    /// (`DELETE /me/tokens/{pat}`). Scoped to `user_id`: a token that
    /// exists but belongs to a different principal is reported as
    /// `Err(UserNotFound)` — the same outcome as a missing token — so PAT
    /// ownership is not disclosed across users.
    async fn revoke_pat(&self, user_id: &str, pat_id: &str) -> Result<(), AuthError>;

    /// Start a forgot-password flow. Always succeeds (does not reveal
    /// whether the email is registered) — sending the reset email is
    /// the implementation's responsibility.
    async fn request_password_reset(&self, email: &str) -> Result<(), AuthError>;

    /// Consume a previously issued password-reset token and set
    /// `new_password` on the associated user.
    async fn complete_password_reset(
        &self,
        token: &str,
        new_password: &str,
    ) -> Result<(), AuthError>;

    /// Consume an email-verification token; idempotent for already-verified
    /// users (token is invalidated either way).
    async fn verify_email(&self, token: &str) -> Result<(), AuthError>;

    /// Begin TOTP enrollment for an authenticated user. Returns the QR-able
    /// otpauth URI **once** — clients must capture it on the first call.
    async fn start_mfa_enrollment(&self, user_id: &str) -> Result<MfaEnrollment, AuthError>;

    /// Confirm TOTP enrollment by verifying the user's first code.
    async fn confirm_mfa_enrollment(&self, user_id: &str, code: &str) -> Result<(), AuthError>;

    /// Begin a Plane-A OAuth sign-in.
    async fn start_oauth(&self, provider: OAuthProvider) -> Result<OAuthStart, AuthError>;

    /// Complete a Plane-A OAuth sign-in. The implementation exchanges the
    /// provider's `code` for an access token, fetches the user profile,
    /// upserts the user, and mints a session.
    async fn complete_oauth(
        &self,
        provider: OAuthProvider,
        state: &str,
        code: &str,
    ) -> Result<OAuthCompletion, AuthError>;
}
