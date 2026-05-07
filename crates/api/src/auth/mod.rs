//! Plane-A authentication backend — identity, sessions, MFA, PATs, and
//! user-facing OAuth sign-in.
//!
//! ## Layout
//!
//! - [`backend`]: the [`AuthBackend`] trait — production injection point on [`crate::AppState`].
//! - [`in_memory`]: production-quality default implementation (Argon2id, RFC 6238 TOTP, SHA-256 PAT
//!   lookup) for dev / tests / `simple_server`.
//! - [`dto`]: HTTP request/response shapes (with [`SecretString`] for plaintext passwords).
//! - [`error`]: [`AuthError`] type and the mapping into [`crate::ApiError`].
//! - [`session`] / [`pat`] / [`oauth`] / [`password`] / [`mfa`]: per-feature primitives reused by
//!   the in-memory backend.
//!
//! ## Why this module exists
//!
//! Per ADR-0033, **Plane A** (host / Nebula API auth) is kept disjoint from
//! **Plane B** (integration credential OAuth). Plane B lives under
//! `crates/api/src/services/oauth/` + `crates/credential/`; Plane A lives
//! here. New auth-domain features land in this module — never in the
//! credential / OAuth-integration tree.
//!
//! [`SecretString`]: dto::SecretString
//! [`AuthBackend`]: backend::AuthBackend
//! [`AuthError`]: error::AuthError

pub mod backend;
pub mod dto;
pub mod error;
pub mod in_memory;
pub mod mfa;
pub mod oauth;
pub mod password;
pub mod pat;
pub mod session;

pub use backend::{AuthBackend, MfaEnrollment, OAuthCompletion, OAuthStart, PasswordOutcome};
pub use dto::{
    ForgotPasswordRequest, LoginRequest, LoginResponse, MfaChallengeResponse, MfaEnrollRequest,
    MfaEnrollResponse, MfaVerifyRequest, MfaVerifyResponse, OAuthStartResponse,
    ResetPasswordRequest, SecretString, SignupRequest, SignupResponse, UserProfile,
    VerifyEmailRequest,
};
pub use error::AuthError;
pub use in_memory::InMemoryAuthBackend;
pub use oauth::OAuthProvider;
pub use pat::{MintedPat, PAT_PREFIX, PatRecord, hash_for_lookup, hashes_equal, mint_pat};
pub use session::{
    CSRF_COOKIE, SESSION_COOKIE, SESSION_TTL, SessionRecord, cleared_cookie, csrf_cookie,
    random_token, session_cookie,
};
