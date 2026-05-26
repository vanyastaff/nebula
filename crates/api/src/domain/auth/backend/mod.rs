//! Plane-A authentication backend subsystem — identity, sessions, MFA,
//! PATs, and user-facing OAuth sign-in.
//!
//! This is the backend half of the [`crate::domain::auth`] domain (the HTTP
//! routes/handlers are siblings of this `backend/` directory).
//!
//! ## Layout
//!
//! - [`provider`]: the [`AuthBackend`] trait — production injection point on [`crate::AppState`].
//! - [`in_memory`]: production-quality default implementation (Argon2id, RFC 6238 TOTP, SHA-256 PAT
//!   lookup) for dev / tests / `simple_server`.
//! - [`dto`]: HTTP request/response shapes (with [`SecretString`] for plaintext passwords).
//! - [`error`]: [`AuthError`] type and the mapping into [`crate::ApiError`].
//! - [`session`] / [`pat`] / [`oauth`] / [`password`] / [`mfa`]: per-feature primitives reused by
//!   the in-memory backend.
//!
//! ## Why this module exists
//!
//! Per auth plane separation, **Plane A** (host / Nebula API auth) is kept disjoint from
//! **Plane B** (integration credential OAuth). Plane B lives under
//! [`crate::transport::oauth`] + `crates/credential/`; Plane A lives here.
//! New auth-domain features land in [`crate::domain::auth`] — never in the
//! credential / OAuth-integration tree.
//!
//! [`SecretString`]: dto::SecretString
//! [`AuthBackend`]: provider::AuthBackend
//! [`AuthError`]: error::AuthError

pub mod dto;
pub mod error;
pub mod in_memory;
pub mod mfa;
pub mod oauth;
pub mod password;
pub mod pat;
/// Postgres-backed [`AuthBackend`] implementation. Linked only when
/// the `postgres` cargo feature is enabled.
///
/// [`AuthBackend`]: provider::AuthBackend
#[cfg(feature = "postgres")]
pub mod pg;
pub mod provider;
pub mod session;

pub use dto::{
    ForgotPasswordRequest, LoginRequest, LoginResponse, MfaChallengeResponse,
    MfaConfirmEnrollRequest, MfaEnrollRequest, MfaEnrollResponse, MfaLoginCompleteRequest,
    OAuthStartResponse, ResetPasswordRequest, SecretString, SignupRequest, SignupResponse,
    UserProfile, VerifyEmailRequest,
};
pub use error::AuthError;
pub use in_memory::InMemoryAuthBackend;
pub use oauth::OAuthProvider;
pub use pat::{MintedPat, PAT_PREFIX, PatRecord, hash_for_lookup, hashes_equal, mint_pat};
#[cfg(feature = "postgres")]
pub use pg::PgAuthBackend;
pub use provider::{
    AuthBackend, CreatePatParams, MfaEnrollment, OAuthCompletion, OAuthStart, PasswordOutcome,
    ProfilePatch,
};
pub use session::{
    CSRF_COOKIE, SESSION_COOKIE, SESSION_TTL, SessionRecord, cleared_cookie, csrf_cookie,
    random_token, session_cookie,
};
