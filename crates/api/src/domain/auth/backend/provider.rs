//! The [`AuthBackend`] trait — Plane-A identity + session contract.
//!
//! Replaces the older `SessionStore` trait. All auth-domain operations
//! (signup / login / MFA / OAuth / sessions / PATs) flow through this single
//! trait so callers never have to ask "which slot is the right one?".
//!
//! ## Metrics emission helpers (`metrics_emit`)
//!
//! Both [`super::InMemoryAuthBackend`] and (under `feature = "postgres"`)
//! `super::pg::PgAuthBackend` share the closed-set emission discipline for
//! the `nebula_api_auth_*` family. The shared helpers live in a private
//! `metrics_emit` submodule below so the two backends cannot drift on
//! label key/value strings (the module is `pub(super)` so it is hidden
//! from the public API surface; its existence is documented here for
//! the maintainer audience).

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nebula_core::Principal;

use super::{
    dto::{SignupRequest, UserProfile},
    error::AuthError,
    oauth::OAuthProvider,
    pat::{MintedPat, PatRecord},
    session::SessionRecord,
};

/// Shared `nebula_api_auth_*` emission helpers consumed by
/// [`super::InMemoryAuthBackend`] and `super::pg::PgAuthBackend`
/// (the latter is feature-gated under `postgres` so an intra-doc
/// link cannot resolve unconditionally — kept as plain code for that
/// reason).
///
/// The single `run_with_metrics` entry point wraps a backend method's
/// future, classifies the resolved `Result` into a closed-set
/// `auth_outcome::*` label, bumps the per-method counter, and observes
/// the duration histogram in a single place — mirrors the
/// `record_outcome` / `LatencyGuard` pattern in
/// `crates/api/src/middleware/idempotency/layer.rs` but lifted into a
/// helper module so both backends share one wire and cannot drift.
///
/// Closed-set guarantee: every emission path here builds labels from
/// `&'static str` constants in `nebula_metrics::naming::auth_outcome`
/// / `naming::auth_oauth_provider`; no `format!` / `to_string` value
/// can reach a label key or value at the call site (oracle locked spec
/// decision 3 — closed cardinality enforced by-construction at the
/// call site, no `LabelAllowlist::only(...)`).
pub(super) mod metrics_emit {
    use std::sync::Arc;

    use nebula_metrics::{
        MetricsRegistry,
        naming::{NEBULA_API_AUTH_DURATION_SECONDS, auth_oauth_provider, auth_outcome},
    };

    use super::{AuthError, OAuthCompletion, OAuthProvider};

    /// Map an [`OAuthProvider`] enum value to its closed-set provider
    /// label string from
    /// [`nebula_metrics::naming::auth_oauth_provider`].
    ///
    /// Returning the constant directly (not `provider.as_str()`)
    /// makes the closed-set guarantee visible to the reviewer: the
    /// `match` cannot produce a non-constant value. If a new
    /// `OAuthProvider` variant lands without a matching constant, this
    /// fails compilation.
    #[must_use]
    pub(crate) fn oauth_provider_label(provider: OAuthProvider) -> &'static str {
        match provider {
            OAuthProvider::Google => auth_oauth_provider::GOOGLE,
            OAuthProvider::GitHub => auth_oauth_provider::GITHUB,
        }
    }

    /// Classify a completed OAuth backend call into the shared closed outcome
    /// set. Keeping this branch in one place prevents the in-memory and
    /// PostgreSQL backends from treating an MFA continuation as success.
    #[must_use]
    pub(crate) fn oauth_completion_outcome(
        result: &Result<OAuthCompletion, AuthError>,
    ) -> &'static str {
        match result {
            Ok(OAuthCompletion::SessionCreated { .. }) => auth_outcome::SUCCESS,
            Ok(OAuthCompletion::MfaRequired { .. }) => auth_outcome::MFA_REQUIRED,
            Err(AuthError::InvalidToken) => auth_outcome::TOKEN_INVALID,
            Err(AuthError::EmailNotVerified) => auth_outcome::EMAIL_UNVERIFIED,
            Err(AuthError::AccountLinkRequired) => auth_outcome::CONFLICT,
            Err(AuthError::OAuthFailed | AuthError::ProviderNotConfigured) => {
                auth_outcome::OAUTH_FAILED
            },
            Err(_) => auth_outcome::INTERNAL,
        }
    }

    /// Run `body` under the auth metrics wire and return its `Result`.
    ///
    /// On completion the resolved `Result` is passed to `classify` to
    /// pick an `auth_outcome::*` label (the per-method closed-set
    /// derivation lives at the call site); the resulting outcome is
    /// then used to:
    ///
    /// 1. Bump `counter_name` with labels `{outcome}` (or
    ///    `{outcome, provider}` if `provider` is `Some`).
    /// 2. Observe [`NEBULA_API_AUTH_DURATION_SECONDS`] keyed by
    ///    `{outcome}` only (never `provider`) with the elapsed time
    ///    in seconds (NOT milliseconds).
    ///
    /// When `metrics` is `None`, emission is a no-op and the helper
    /// returns the inner `Result` unchanged. Mirrors the
    /// `let Some(reg) = self.metrics.as_ref() else { return; };`
    /// early-return pattern in `idempotency/layer.rs`.
    pub(crate) async fn run_with_metrics<T, F, C>(
        metrics: &Option<Arc<MetricsRegistry>>,
        counter_name: &'static str,
        provider: Option<&'static str>,
        body: F,
        classify: C,
    ) -> Result<T, AuthError>
    where
        F: Future<Output = Result<T, AuthError>>,
        C: FnOnce(&Result<T, AuthError>) -> &'static str,
    {
        let start = std::time::Instant::now();
        let result = body.await;
        let outcome = classify(&result);
        let Some(registry) = metrics.as_ref() else {
            return result;
        };

        // Counter labels: oauth path adds `provider`; everything else
        // is `outcome` only. All values are `&'static str` constants
        // — no runtime stringification can reach the label.
        let counter_labels = match provider {
            Some(provider_label) => registry
                .interner()
                .label_set(&[("outcome", outcome), ("provider", provider_label)]),
            None => registry.interner().single("outcome", outcome),
        };
        if let Ok(counter) = registry.counter_labeled(counter_name, &counter_labels) {
            counter.inc();
        }

        // Histogram: outcome-only labels regardless of which counter
        // family fired — the duration view is always keyed by outcome
        // to keep the histogram cardinality at the floor of
        // `len(auth_outcome::*)` series.
        let hist_labels = if provider.is_some() {
            registry.interner().single("outcome", outcome)
        } else {
            counter_labels
        };
        if let Ok(histogram) =
            registry.histogram_labeled(NEBULA_API_AUTH_DURATION_SECONDS, &hist_labels)
        {
            // Seconds, not milliseconds. Default seconds-shaped buckets
            // span 5 ms ... 10 s which is correct for Argon2id-dominated
            // auth duration.
            histogram.observe(start.elapsed().as_secs_f64());
        }

        result
    }
}

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
    /// Granted scopes. Use `full_access` for complete access; empty scopes
    /// are invalid at the API auth boundary.
    pub scopes: Vec<String>,
    /// Optional time-to-live in seconds; `None` = non-expiring.
    pub ttl_seconds: Option<u64>,
}

/// Outcome of a password-step authentication.
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

impl std::fmt::Debug for PasswordOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Authenticated(_) => formatter
                .debug_tuple("Authenticated")
                .field(&"[redacted]")
                .finish(),
            Self::MfaRequired { .. } => formatter
                .debug_struct("MfaRequired")
                .field("challenge_token", &"[redacted]")
                .finish(),
        }
    }
}

/// Outcome of a successful MFA enrollment.
pub struct MfaEnrollment {
    /// `otpauth://totp/...` URI rendered as a QR code by the client.
    pub otpauth_uri: String,
    /// Base32 secret in case the authenticator app rejects the URI.
    pub secret_base32: String,
}

/// Lifetime of a pending MFA enrollment candidate.
pub(crate) const MFA_ENROLLMENT_TTL: Duration = Duration::from_mins(10);

/// Maximum age of the primary authentication accepted for MFA enrollment.
pub(crate) const MFA_ENROLLMENT_REAUTH_TTL: Duration = Duration::from_mins(10);

/// Principal and primary-authentication time resolved from a live session.
///
/// `authenticated_at` is the session creation time. Carrying it through the
/// auth boundary lets sensitive handlers enforce reauthentication freshness
/// without treating a long-lived session as permanently fresh authority.
#[derive(Debug, Clone)]
pub struct AuthenticatedSession {
    /// Principal that owns the live session.
    pub principal: Principal,
    /// Time at which primary authentication created the session.
    pub authenticated_at: DateTime<Utc>,
}

impl std::fmt::Debug for MfaEnrollment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MfaEnrollment")
            .field("otpauth_uri", &"[redacted]")
            .field("secret_base32", &"[redacted]")
            .finish()
    }
}

/// Result of starting a Plane-A OAuth flow.
pub struct OAuthStart {
    /// Provider authorize URL (state + PKCE challenge already included).
    pub authorize_url: String,
    /// Opaque state token (also stored server-side).
    pub state: String,
}

impl std::fmt::Debug for OAuthStart {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthStart")
            .field("authorize_url", &"[redacted]")
            .field("state", &"[redacted]")
            .finish()
    }
}

/// Result of completing the provider first factor for a Plane-A OAuth login.
#[non_exhaustive]
pub enum OAuthCompletion {
    /// Authentication is complete and the session was created atomically with
    /// any new local user and external-identity link.
    SessionCreated {
        /// User profile resolved from the authoritative identity link.
        user: UserProfile,
        /// Newly created session.
        session: SessionRecord,
    },
    /// The linked local user has Nebula MFA enabled. No session exists yet;
    /// the client must complete the one-time challenge at `/auth/login/mfa`.
    MfaRequired {
        /// Opaque single-use MFA challenge plaintext.
        challenge_token: String,
    },
}

impl std::fmt::Debug for OAuthCompletion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionCreated { .. } => formatter
                .debug_struct("SessionCreated")
                .field("user", &"[redacted]")
                .field("session", &"[redacted]")
                .finish(),
            Self::MfaRequired { .. } => formatter
                .debug_struct("MfaRequired")
                .field("challenge_token", &"[redacted]")
                .finish(),
        }
    }
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
    ) -> Result<Option<AuthenticatedSession>, crate::ApiError>;

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
    ///
    /// `redirect_uri` is **handler-derived** from `ApiConfig::public_url`
    /// per ADR-0085 D-3. The canonical API callback path is appended to
    /// the validated external mount prefix. The trait accepts it so the
    /// derived value round-trips through the implementation's state row
    /// and is re-verified on `complete_oauth` against the row's stored
    /// value (closes the `public_url_changed_mid_flow` defense per
    /// REQ-oauth-003 Scenario 3.10). Implementations MUST NOT derive
    /// the redirect_uri themselves; the handler is the single source of
    /// truth.
    async fn start_oauth(
        &self,
        provider: OAuthProvider,
        redirect_uri: &str,
    ) -> Result<OAuthStart, AuthError>;

    /// Consume an OAuth state after the provider returns a standard error
    /// callback (for example, the resource owner denied authorization).
    ///
    /// Implementations must atomically consume only a live matching
    /// `(state, provider)` entry and re-check its persisted `redirect_uri`.
    /// This path never needs provider runtime configuration and must not make
    /// token or userinfo requests.
    async fn cancel_oauth(
        &self,
        provider: OAuthProvider,
        state: &str,
        redirect_uri: &str,
    ) -> Result<(), AuthError>;

    /// Complete a Plane-A OAuth sign-in. The implementation exchanges the
    /// provider's `code` for an access token, fetches the user profile,
    /// resolves or creates the user, and either mints a session or persists a
    /// local Nebula-MFA challenge. Provider assurance never substitutes for
    /// the local user's MFA policy.
    ///
    /// `redirect_uri` is **handler-derived** per the same formula as
    /// [`Self::start_oauth`]. The implementation MUST compare it against
    /// the persisted state-row value and return the fixed
    /// [`AuthError::OAuthFailed`] variant on mismatch.
    async fn complete_oauth(
        &self,
        provider: OAuthProvider,
        state: &str,
        code: &str,
        redirect_uri: &str,
    ) -> Result<OAuthCompletion, AuthError>;
}

#[cfg(test)]
mod oauth_debug_tests {
    use nebula_metrics::naming::auth_outcome;

    use super::{MfaEnrollment, OAuthCompletion, OAuthStart, PasswordOutcome, metrics_emit};
    use crate::domain::auth::backend::UserProfile;

    static_assertions::assert_not_impl_any!(PasswordOutcome: Clone);
    static_assertions::assert_not_impl_any!(MfaEnrollment: Clone);
    static_assertions::assert_not_impl_any!(OAuthStart: Clone);
    static_assertions::assert_not_impl_any!(OAuthCompletion: Clone);

    #[test]
    fn oauth_start_debug_redacts_url_and_state() {
        let start = OAuthStart {
            authorize_url: "https://idp.example/authorize?state=URL_CANARY-3c2a".to_owned(),
            state: "STATE_CANARY-7301".to_owned(),
        };

        let debug = format!("{start:?}");
        assert!(!debug.contains("URL_CANARY-3c2a"));
        assert!(!debug.contains("STATE_CANARY-7301"));
    }

    #[test]
    fn oauth_completion_debug_redacts_mfa_challenge_plaintext() {
        const CANARY: &str = "OAUTH_MFA_CHALLENGE_CANARY-c803";
        let completion = OAuthCompletion::MfaRequired {
            challenge_token: CANARY.to_owned(),
        };

        let debug = format!("{completion:?}");
        assert!(!debug.contains(CANARY));
        assert!(debug.contains("MfaRequired"));
        assert_eq!(
            metrics_emit::oauth_completion_outcome(&Ok(completion)),
            auth_outcome::MFA_REQUIRED
        );
    }

    #[test]
    fn password_and_enrollment_outcome_debug_redacts_authority() {
        const CANARY: &str = "MFA_AUTHORITY_CANARY-4a31";
        let profile = UserProfile {
            user_id: CANARY.to_owned(),
            email: format!("{CANARY}@example.test"),
            display_name: CANARY.to_owned(),
            avatar_url: Some(CANARY.to_owned()),
            email_verified: true,
            mfa_enabled: true,
        };
        let values = [
            format!("{:?}", PasswordOutcome::Authenticated(profile)),
            format!(
                "{:?}",
                PasswordOutcome::MfaRequired {
                    challenge_token: CANARY.to_owned(),
                }
            ),
            format!(
                "{:?}",
                MfaEnrollment {
                    otpauth_uri: format!("otpauth://totp/{CANARY}?secret={CANARY}"),
                    secret_base32: CANARY.to_owned(),
                }
            ),
        ];

        for debug in values {
            assert!(!debug.contains(CANARY), "Debug leaked authority: {debug}");
        }
    }
}
