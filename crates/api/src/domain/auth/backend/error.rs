//! Auth-domain errors.
//!
//! These map cleanly into [`crate::ApiError`] so handlers can `?`-propagate.

use nebula_storage::StorageError;
use thiserror::Error;

use crate::{error::ApiError, ports::email::EmailError};

/// Failure modes for the auth backend.
#[derive(Debug, Error)]
pub enum AuthError {
    /// Capability is wired into the trait but not provided by this backend.
    #[error("auth capability not implemented: {0}")]
    NotImplemented(&'static str),

    /// Operator has not declared an OAuth identity provider for the
    /// requested key.
    ///
    /// Per ADR-0085 D-6: returned when `start_oauth` / `complete_oauth`
    /// runs for a `provider` that is absent from
    /// `ApiConfig::auth.oauth.providers` (env var
    /// `API_AUTH_OAUTH_<PROVIDER>_CLIENT_ID` is unset). Maps to
    /// HTTP 503 Service Unavailable (NOT 400) because the operator can
    /// fix it by setting the env vars without any code change — it's
    /// a deployment state, not a caller error.
    #[error("OAuth provider `{provider}` is not configured on this Nebula instance")]
    ProviderNotConfigured {
        /// Snake_case provider key (matches the `OAuthProvider` enum
        /// variant).
        provider: String,
    },

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
    /// Will be revisited once the storage-backed AuthBackend
    /// implementation needs a richer error mapping (e.g. distinguishing
    /// transient transport failures from hard rejects); for the dev
    /// `EchoSink` and the bring-up SMTP transport this is the right
    /// "never silently swallow" default.
    fn from(e: EmailError) -> Self {
        Self::Internal(format!("email: {e}"))
    }
}

impl From<StorageError> for AuthError {
    /// Translate storage-layer failures returned by the PG identity
    /// repos into [`AuthError`] so `PgAuthBackend` can `?`-propagate
    /// cleanly. The mapping is deliberately small:
    ///
    /// - `Duplicate { entity: "user", .. }` →
    ///   [`AuthError::EmailAlreadyRegistered`] (the only unique-key on
    ///   the auth tables a caller can collide with by sending the same
    ///   email twice; every other duplicate is an internal bug).
    /// - Everything else → [`AuthError::Internal`] with the
    ///   operator-facing detail preserved. The wider 503-mapping
    ///   (`ApiError::Storage`) is intentionally NOT reused here so the
    ///   auth backend stays in the 5xx-Internal lane the rest of the
    ///   auth surface already uses; the storage detail string is
    ///   carried through `Display` for the operator-side log.
    fn from(err: StorageError) -> Self {
        match err {
            StorageError::Duplicate { entity: "user", .. } => Self::EmailAlreadyRegistered,
            other => Self::Internal(format!("storage: {other}")),
        }
    }
}

impl From<AuthError> for ApiError {
    fn from(e: AuthError) -> Self {
        match e {
            AuthError::NotImplemented(what) => {
                ApiError::ServiceUnavailable(format!("not implemented: {what}"))
            },
            AuthError::ProviderNotConfigured { provider } => ApiError::ServiceUnavailable(format!(
                "OAuth provider `{provider}` is not configured; set API_AUTH_OAUTH_{}_CLIENT_ID and related env vars",
                provider.to_ascii_uppercase()
            )),
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
    use nebula_metrics::naming::auth_outcome;

    use super::*;

    fn status(e: AuthError) -> StatusCode {
        let api: ApiError = e.into();
        api.to_problem_details().0
    }

    /// Exhaustive mapping from [`AuthError`] to a closed
    /// `nebula_metrics::naming::auth_outcome::*` label.
    ///
    /// **This is the compile-time gate for the auth metrics cardinality
    /// budget.** The `match` deliberately has no catch-all arm: adding
    /// a new [`AuthError`] variant fails compilation here until it has
    /// a closed-set mapping. Per-method emission sites in
    /// `crates/api/src/domain/auth/backend/{pg,in_memory}.rs` use their
    /// own narrower `match`es to capture per-method overrides (e.g.
    /// `complete_password_reset` collapses `InvalidCredentials` to
    /// `invalid_input`, not the default `invalid_creds`); this function
    /// is the safety net that guarantees *every* variant has at least
    /// one valid label, not the source of truth for every emission site.
    ///
    /// Notable collapses documented in the oracle locked spec:
    /// - `UserNotFound` -> `invalid_creds` (user-enumeration defence;
    ///   see [`auth_outcome::INVALID_CREDS`] doc comment).
    /// - `NotImplemented` / `Crypto` / `Internal` -> `internal`
    ///   (operator-side, not caller-side).
    fn default_outcome_for(err: &AuthError) -> &'static str {
        match err {
            AuthError::NotImplemented(_) => auth_outcome::INTERNAL,
            AuthError::ProviderNotConfigured { .. } => auth_outcome::OAUTH_FAILED,
            AuthError::EmailAlreadyRegistered => auth_outcome::CONFLICT,
            AuthError::UserNotFound => auth_outcome::INVALID_CREDS,
            AuthError::InvalidCredentials => auth_outcome::INVALID_CREDS,
            AuthError::InvalidInput(_) => auth_outcome::INVALID_INPUT,
            AuthError::AccountLocked => auth_outcome::LOCKOUT,
            AuthError::EmailNotVerified => auth_outcome::EMAIL_UNVERIFIED,
            AuthError::MfaRequired => auth_outcome::MFA_REQUIRED,
            AuthError::InvalidMfaCode => auth_outcome::INVALID_MFA_CODE,
            AuthError::InvalidToken => auth_outcome::TOKEN_INVALID,
            AuthError::RateLimit => auth_outcome::RATE_LIMIT,
            AuthError::OAuthFailed(_) => auth_outcome::OAUTH_FAILED,
            AuthError::Crypto(_) => auth_outcome::INTERNAL,
            AuthError::Internal(_) => auth_outcome::INTERNAL,
        }
    }

    #[test]
    fn every_auth_error_variant_has_a_closed_outcome_label() {
        // Enumerates all 14 `AuthError` variants and confirms each maps
        // to a closed `auth_outcome::*` constant. The compile-time gate
        // is `default_outcome_for`'s exhaustive `match`; this test
        // additionally verifies the label values are non-empty closed
        // strings.
        let cases: [(&str, AuthError); 15] = [
            ("NotImplemented", AuthError::NotImplemented("x")),
            (
                "ProviderNotConfigured",
                AuthError::ProviderNotConfigured {
                    provider: "google".to_owned(),
                },
            ),
            ("EmailAlreadyRegistered", AuthError::EmailAlreadyRegistered),
            ("UserNotFound", AuthError::UserNotFound),
            ("InvalidCredentials", AuthError::InvalidCredentials),
            ("InvalidInput", AuthError::InvalidInput("x")),
            ("AccountLocked", AuthError::AccountLocked),
            ("EmailNotVerified", AuthError::EmailNotVerified),
            ("MfaRequired", AuthError::MfaRequired),
            ("InvalidMfaCode", AuthError::InvalidMfaCode),
            ("InvalidToken", AuthError::InvalidToken),
            ("RateLimit", AuthError::RateLimit),
            ("OAuthFailed", AuthError::OAuthFailed("x".into())),
            ("Crypto", AuthError::Crypto("x".into())),
            ("Internal", AuthError::Internal("x".into())),
        ];
        for (name, err) in &cases {
            let outcome = default_outcome_for(err);
            assert!(!outcome.is_empty(), "{name} produced empty outcome label");
            // Verify the outcome is one of the 12 closed-set values.
            let closed = [
                auth_outcome::SUCCESS,
                auth_outcome::INVALID_CREDS,
                auth_outcome::INVALID_INPUT,
                auth_outcome::INVALID_MFA_CODE,
                auth_outcome::MFA_REQUIRED,
                auth_outcome::TOKEN_INVALID,
                auth_outcome::LOCKOUT,
                auth_outcome::EMAIL_UNVERIFIED,
                auth_outcome::RATE_LIMIT,
                auth_outcome::OAUTH_FAILED,
                auth_outcome::CONFLICT,
                auth_outcome::INTERNAL,
            ];
            assert!(
                closed.contains(&outcome),
                "{name} -> {outcome:?} is not in the auth_outcome closed set",
            );
        }
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
