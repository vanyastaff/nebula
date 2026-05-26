//! Auth-domain errors.
//!
//! These map cleanly into [`crate::ApiError`] so handlers can `?`-propagate.

use thiserror::Error;

use crate::{error::ApiError, ports::email::EmailError};

/// Failure modes for the auth backend.
#[derive(Debug, Error)]
pub enum AuthError {
    /// Capability is wired into the trait but not provided by this backend.
    #[error("auth capability not implemented: {0}")]
    NotImplemented(&'static str),

    /// Email already registered.
    #[error("email already registered")]
    EmailAlreadyRegistered,

    /// User not found by ID or email.
    #[error("user not found")]
    UserNotFound,

    /// Provided credentials did not verify.
    #[error("invalid credentials")]
    InvalidCredentials,

    /// A caller-supplied input field failed validation (blank / oversized
    /// / malformed) — a 400-class fault, **not** an auth failure. Distinct
    /// from [`Self::InvalidCredentials`] so the port enforces the correct
    /// status itself rather than relying on every caller to pre-validate.
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),

    /// Account is locked (too many failed attempts) until the moment in error.
    #[error("account locked")]
    AccountLocked,

    /// Email is not verified yet — caller must complete the verification flow.
    #[error("email not verified")]
    EmailNotVerified,

    /// MFA is required but not yet completed for this login.
    #[error("mfa challenge required")]
    MfaRequired,

    /// MFA code did not verify.
    #[error("invalid mfa code")]
    InvalidMfaCode,

    /// One-time token (verification / reset / mfa-challenge) is unknown,
    /// expired, or already consumed.
    #[error("token invalid or expired")]
    InvalidToken,

    /// Rate limit hit on a sensitive endpoint.
    #[error("rate limit exceeded")]
    RateLimit,

    /// OAuth provider returned an error or the state token failed.
    #[error("oauth failed: {0}")]
    OAuthFailed(String),

    /// Cryptographic operation failed (hash compute, RNG, etc.).
    #[error("crypto failure: {0}")]
    Crypto(String),

    /// Internal backend error (storage, lock poisoning, etc.).
    #[error("internal: {0}")]
    Internal(String),
}

impl From<EmailError> for AuthError {
    /// Collapse every transport-layer failure into
    /// [`AuthError::Internal`] so handler `?`-propagation stays uniform.
    /// PR2 commit 3 may revisit this once `PgAuthBackend` lands a richer
    /// mapping (e.g. distinguishing transient transport failures from
    /// hard rejects), but for the dev `EchoSink` and the bring-up SMTP
    /// transport this is the right "never silently swallow" default.
    fn from(e: EmailError) -> Self {
        Self::Internal(format!("email: {e}"))
    }
}

impl From<AuthError> for ApiError {
    fn from(e: AuthError) -> Self {
        match e {
            AuthError::NotImplemented(what) => {
                ApiError::ServiceUnavailable(format!("not implemented: {what}"))
            },
            AuthError::EmailAlreadyRegistered => {
                ApiError::Conflict("email already registered".to_owned())
            },
            AuthError::UserNotFound => ApiError::NotFound("user".to_owned()),
            AuthError::InvalidCredentials => {
                ApiError::Unauthorized("invalid credentials".to_owned())
            },
            AuthError::InvalidInput(what) => ApiError::validation_message(what),
            AuthError::AccountLocked => {
                ApiError::AccountLocked("too many failed attempts; try again later".to_owned())
            },
            AuthError::EmailNotVerified => {
                ApiError::Forbidden("email verification required".to_owned())
            },
            AuthError::MfaRequired => ApiError::MfaRequired,
            AuthError::InvalidMfaCode => ApiError::Unauthorized("invalid mfa code".to_owned()),
            AuthError::InvalidToken => ApiError::Unauthorized("token invalid".to_owned()),
            AuthError::RateLimit => ApiError::RateLimitExceeded,
            AuthError::OAuthFailed(msg) => ApiError::UpstreamError(msg),
            AuthError::Crypto(msg) => ApiError::Internal(format!("crypto: {msg}")),
            AuthError::Internal(msg) => ApiError::Internal(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;

    fn status(e: AuthError) -> StatusCode {
        let api: ApiError = e.into();
        api.to_problem_details().0
    }

    #[test]
    fn invalid_credentials_maps_to_401() {
        assert_eq!(
            status(AuthError::InvalidCredentials),
            StatusCode::UNAUTHORIZED
        );
    }

    #[test]
    fn email_conflict_maps_to_409() {
        assert_eq!(
            status(AuthError::EmailAlreadyRegistered),
            StatusCode::CONFLICT
        );
    }

    #[test]
    fn account_locked_maps_to_423() {
        assert_eq!(
            status(AuthError::AccountLocked),
            StatusCode::from_u16(423).unwrap()
        );
    }

    #[test]
    fn mfa_required_maps_to_401() {
        assert_eq!(status(AuthError::MfaRequired), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn rate_limit_maps_to_429() {
        assert_eq!(status(AuthError::RateLimit), StatusCode::TOO_MANY_REQUESTS);
    }
}
