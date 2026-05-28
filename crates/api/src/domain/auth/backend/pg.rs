//! Postgres-backed [`AuthBackend`] implementation.
//!
//! Production identity backend wired by the composition root when
//! `API_AUTH_BACKEND=postgres` is selected. Mirrors the in-memory
//! backend's user-visible semantics method-for-method, with the
//! durability and crash-safety the PG identity tables provide.
//!
//! ## Storage layout
//!
//! - `users` (`0001_users.sql`) — backed by `PgUserRepo`.
//! - `sessions`, `personal_access_tokens`, `verification_tokens`
//!   (`0002_user_auth.sql`) — backed by `PgSessionRepo`,
//!   `PgPatRepo`, `PgVerificationTokenRepo`.
//! - `plane_a_oauth_states` (`0028_plane_a_oauth_state.sql`) — backed
//!   by `PgOAuthStateRepo`.
//!
//! ## Encoding seams (deliberate divergences)
//!
//! - `UserRow.id` (BYTEA, 16 bytes) is the raw ULID payload via
//!   `UserId::as_bytes` / `UserId::from_bytes`.
//! - `SessionRow.id` / `PersonalAccessTokenRow.id` / OAuth `state`
//!   are stored as the existing helper string outputs cast to
//!   `as_bytes().to_vec()`. The migration docstrings call these
//!   columns "`sess_` ULID" / "`pat_` ULID" — today we keep the
//!   helper-derived URL-safe base64 strings (43 chars) to avoid
//!   diverging from the in-memory backend or breaking the existing
//!   `me_e2e.rs` test surface. Refactoring the primitives to mint
//!   real ULIDs is a separate change.
//! - `users.mfa_secret` (BYTEA) holds the base32-encoded TOTP secret
//!   as raw UTF-8 bytes; on read we `String::from_utf8` back into the
//!   base32 form [`mfa::verify_code`] consumes. Encryption-at-rest
//!   is deferred to a follow-up that wires the credential service's
//!   master-key envelope into the identity surface.
//! - `OAuthStateRow.redirect_uri` is always `None` today —
//!   the [`AuthBackend::start_oauth`] trait signature does not yet
//!   accept a `redirect_uri` parameter. The column stays correctly
//!   nullable so a future trait-signature change picks it up without
//!   a migration.
//!
//! ## Transactional flows
//!
//! [`register_user`], [`verify_email`], and
//! [`complete_password_reset`] are the multi-step writes; each wraps
//! its statements in a single `sqlx::Transaction` and bypasses the
//! repo abstraction inside the tx because the repos are pool-bound
//! and not `Executor`-generic. Convert to `Executor`-generic repos
//! when a fourth multi-step flow appears. See the inline comments at
//! each method site.
//!
//! ## Background sweepers
//!
//! `cleanup_expired` exists on `SessionRepo`,
//! `VerificationTokenRepo`, and `OAuthStateRepo` but is not invoked
//! here. Wiring a sweeper task is deferred to a follow-up; production
//! deployments should run a periodic cleanup job against the three
//! TTL-bearing tables.
//!
//! [`register_user`]: AuthBackend::register_user
//! [`verify_email`]: AuthBackend::verify_email
//! [`complete_password_reset`]: AuthBackend::complete_password_reset

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use chrono::Utc;
use nebula_core::{Principal, UserId};
use nebula_metrics::{
    MetricsRegistry,
    naming::{
        NEBULA_API_AUTH_ATTEMPTS_TOTAL, NEBULA_API_AUTH_MFA_ATTEMPTS_TOTAL,
        NEBULA_API_AUTH_OAUTH_ATTEMPTS_TOTAL, auth_outcome,
    },
};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Postgres};

use nebula_storage::{
    pg::{PgOAuthStateRepo, PgPatRepo, PgSessionRepo, PgUserRepo, PgVerificationTokenRepo},
    repos::{OAuthStateRepo, PatRepo, SessionRepo, UserRepo, VerificationTokenRepo},
    rows::{OAuthStateRow, PersonalAccessTokenRow, SessionRow, UserRow, VerificationTokenRow},
};

use super::{
    dto::{SignupRequest, UserProfile},
    error::AuthError,
    mfa,
    oauth::{OAUTH_STATE_TTL, OAuthProvider, mint_pkce},
    password,
    pat::{self, MintedPat, PatRecord, compute_pat_expires_at},
    provider::{
        AuthBackend, CreatePatParams, MfaEnrollment, OAuthCompletion, OAuthStart, PasswordOutcome,
        ProfilePatch, metrics_emit,
    },
    session::{self, SESSION_TTL, SessionRecord, expires_at},
};
use crate::ports::email::{EmailKind, EmailMessage, EmailPort};

/// MFA-challenge lifetime — mirrors the in-memory backend constant exactly
/// so swapping backings does not change user-visible behaviour. NOTE: the
/// `verification_tokens.kind` column docstring in `0002_user_auth.sql`
/// does not include `'mfa_challenge'`; the column is plain `TEXT` with no
/// `CHECK` so storing it works today. The docstring catch-up is tracked
/// separately.
const MFA_CHALLENGE_TTL: Duration = Duration::from_mins(5);

/// Email-verification + password-reset token lifetime.
const VERIFICATION_TTL: Duration = Duration::from_hours(1);

/// Minimum password length accepted by [`register_user`] and
/// [`complete_password_reset`].
const MIN_PASSWORD_LEN: usize = 8;

/// `verification_tokens.kind` literal for email-verification tokens.
const KIND_EMAIL_VERIFICATION: &str = "email_verification";

/// `verification_tokens.kind` literal for password-reset tokens.
const KIND_PASSWORD_RESET: &str = "password_reset";

/// `verification_tokens.kind` literal for MFA-challenge tokens. NOTE: not
/// listed in the `0002_user_auth.sql` docstring; column is plain `TEXT`
/// with no `CHECK` so this stores correctly today.
const KIND_MFA_CHALLENGE: &str = "mfa_challenge";

/// `personal_access_tokens.principal_kind` literal for human users.
const PRINCIPAL_KIND_USER: &str = "user";

/// Production [`AuthBackend`] backed by the spec-16 PG identity repos.
///
/// Holds an `Arc` of each repo plus the underlying `Pool<Postgres>`
/// (used by the transactional flows that bypass the repo abstraction)
/// and the shared [`EmailPort`].
pub struct PgAuthBackend {
    user_repo: Arc<PgUserRepo>,
    session_repo: Arc<PgSessionRepo>,
    pat_repo: Arc<PgPatRepo>,
    verification_token_repo: Arc<PgVerificationTokenRepo>,
    oauth_state_repo: Arc<PgOAuthStateRepo>,
    /// Held alongside the repos because the multi-step flows
    /// ([`register_user`], [`verify_email`],
    /// [`complete_password_reset`]) call `pool.begin()` directly: the
    /// repos themselves are pool-bound and not yet `Executor`-generic.
    /// Convert when a fourth multi-step flow appears.
    ///
    /// [`register_user`]: AuthBackend::register_user
    /// [`verify_email`]: AuthBackend::verify_email
    /// [`complete_password_reset`]: AuthBackend::complete_password_reset
    pool: Pool<Postgres>,
    /// Shared outbound-email port. The composition root injects the
    /// same `Arc<dyn EmailPort>` into both `AppState::email_port` and
    /// here, so the slot is always consumed by exactly the same
    /// transport.
    email_port: Arc<dyn EmailPort>,
    /// Optional `nebula_api_auth_*` emission seam. `None` skips
    /// emission (mirrors the `IdempotencyLayer::with_metrics`
    /// `Option<Arc<MetricsRegistry>>` precedent at
    /// `crates/api/src/middleware/idempotency/layer.rs`). Production
    /// composition always populates this with the shared
    /// `Arc<MetricsRegistry>` so the closed-set counters are observable
    /// from operator dashboards; tests that don't exercise the emission
    /// seam pass `None`.
    metrics: Option<Arc<MetricsRegistry>>,
    /// Operator-supplied OAuth providers config (Plane A). Defaults
    /// to empty per `OAuthProvidersConfig::default()` so backends
    /// constructed without an explicit `with_oauth_providers` call
    /// behave as if no provider is declared (start_oauth returns
    /// `ProviderNotConfigured`, matching the boot fail-closed
    /// posture of REQ-compose-001 Invariant 1). Production
    /// composition root wires it from `api_config.auth.oauth`.
    ///
    /// Wrapped in `Arc` because the backend itself is wrapped in an
    /// `Arc<dyn AuthBackend>` and the config is read-only at runtime
    /// — sharing one allocation across the backend's clones avoids
    /// repeated deep-copies of the secrets map.
    oauth_providers: Arc<crate::config::OAuthProvidersConfig>,
}

impl PgAuthBackend {
    /// Construct a backend from a live `sqlx::Pool<Postgres>`, a
    /// shared `Arc<dyn EmailPort>`, and an optional `Arc<MetricsRegistry>`
    /// for the `nebula_api_auth_*` emission seam.
    ///
    /// The five PG identity repos are built internally from the pool
    /// (each holds its own clone, which is cheap — `Pool` is an `Arc`
    /// internally). `metrics` follows the `IdempotencyLayer::with_metrics`
    /// precedent: `None` for tests that do not exercise the emission
    /// path, `Some(_)` from the production composition root.
    #[must_use]
    pub fn new(
        pool: Pool<Postgres>,
        email_port: Arc<dyn EmailPort>,
        metrics: Option<Arc<MetricsRegistry>>,
    ) -> Self {
        Self {
            user_repo: Arc::new(PgUserRepo::new(pool.clone())),
            session_repo: Arc::new(PgSessionRepo::new(pool.clone())),
            pat_repo: Arc::new(PgPatRepo::new(pool.clone())),
            verification_token_repo: Arc::new(PgVerificationTokenRepo::new(pool.clone())),
            oauth_state_repo: Arc::new(PgOAuthStateRepo::new(pool.clone())),
            pool,
            email_port,
            metrics,
            oauth_providers: Arc::new(crate::config::OAuthProvidersConfig::default()),
        }
    }

    /// Attach the operator OAuth providers config. Composition root
    /// calls this with `Arc::new(api_config.auth.oauth.clone())` so
    /// the backend can serve real authorize URLs (PR-3) and complete
    /// OAuth flows (PR-4). Tests skip this when they don't exercise
    /// OAuth — the default empty config makes start_oauth return
    /// `ProviderNotConfigured` for any provider.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_oauth_providers(
        mut self,
        providers: Arc<crate::config::OAuthProvidersConfig>,
    ) -> Self {
        self.oauth_providers = providers;
        self
    }

    /// Wrap into an `Arc<dyn AuthBackend>` for [`crate::AppState`].
    #[must_use]
    pub fn into_arc(self) -> Arc<dyn AuthBackend> {
        Arc::new(self)
    }
}

// ── private helpers ─────────────────────────────────────────────────────

/// SHA-256 a plaintext token to its storage shape (32-byte digest).
fn sha256_token(plaintext: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(plaintext.as_bytes());
    hasher.finalize().into()
}

/// Parse a `usr_<ULID>`-prefixed string into the raw 16-byte ULID
/// payload the PG identity tables expect.
fn user_id_bytes(s: &str) -> Result<[u8; 16], AuthError> {
    let parsed: UserId = s
        .parse()
        .map_err(|_| AuthError::Internal("invalid user_id".to_owned()))?;
    Ok(parsed.as_bytes())
}

/// Reconstruct a [`UserId`] from a 16-byte BYTEA payload read out of
/// `users.id` / `sessions.user_id` / `personal_access_tokens.principal_id`.
fn user_id_from_bytes(bytes: &[u8]) -> Result<UserId, AuthError> {
    let arr: [u8; 16] = bytes
        .try_into()
        .map_err(|_| AuthError::Internal("user id is not 16 bytes".to_owned()))?;
    Ok(UserId::from_bytes(arr))
}

/// Project a [`UserRow`] onto the API-facing [`UserProfile`].
fn row_to_profile(row: &UserRow) -> Result<UserProfile, AuthError> {
    let user_id = user_id_from_bytes(&row.id)?;
    Ok(UserProfile {
        user_id: user_id.to_string(),
        email: row.email.clone(),
        display_name: row.display_name.clone(),
        avatar_url: row.avatar_url.clone(),
        email_verified: row.email_verified_at.is_some(),
        mfa_enabled: row.mfa_enabled,
    })
}

/// Project a [`PersonalAccessTokenRow`] onto the API-facing
/// [`PatRecord`]. Returns `Err(AuthError::Internal)` if any field is
/// shape-incorrect (32-byte hash, parseable id bytes, JSON scope list);
/// these are operator-side invariant breaks rather than caller faults.
fn row_to_pat_record(row: PersonalAccessTokenRow) -> Result<PatRecord, AuthError> {
    let id = String::from_utf8(row.id)
        .map_err(|_| AuthError::Internal("pat id is not utf-8".to_owned()))?;
    let user_id = user_id_from_bytes(&row.principal_id)?;
    let hash: [u8; 32] = row
        .hash
        .try_into()
        .map_err(|_: Vec<u8>| AuthError::Internal("pat hash is not 32 bytes".to_owned()))?;
    let scopes: Vec<String> = serde_json::from_value(row.scopes)
        .map_err(|e| AuthError::Internal(format!("pat scopes deserialize: {e}")))?;
    Ok(PatRecord {
        id,
        user_id,
        name: row.name,
        prefix: row.prefix,
        hash,
        scopes,
        created_at: row.created_at,
        expires_at: row.expires_at,
        last_used_at: row.last_used_at,
        revoked_at: row.revoked_at,
    })
}

/// Fetch a user by parsed-string id; returns `Err(UserNotFound)` for
/// missing or soft-deleted rows so callers can `?`-propagate cleanly.
async fn fetch_user_by_id(repo: &PgUserRepo, id: &str) -> Result<UserRow, AuthError> {
    let bytes = user_id_bytes(id)?;
    repo.get(&bytes).await?.ok_or(AuthError::UserNotFound)
}

#[async_trait]
impl AuthBackend for PgAuthBackend {
    #[tracing::instrument(level = "info", skip(self, session_id))]
    async fn get_principal_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<Principal>, crate::ApiError> {
        let row = self
            .session_repo
            .get(session_id.as_bytes())
            .await
            .map_err(crate::ApiError::from)?;
        match row {
            Some(row) => {
                let user_id = user_id_from_bytes(&row.user_id).map_err(crate::ApiError::from)?;
                Ok(Some(Principal::User(user_id)))
            },
            None => Ok(None),
        }
    }

    #[tracing::instrument(
        level = "info",
        skip(self, req),
        fields(display_name_len = req.display_name.len()),
    )]
    async fn register_user(&self, req: SignupRequest) -> Result<UserProfile, AuthError> {
        metrics_emit::run_with_metrics(
            &self.metrics,
            NEBULA_API_AUTH_ATTEMPTS_TOTAL,
            None,
            async move {
                let email = req.email.trim().to_lowercase();
                if email.is_empty() || !email.contains('@') {
                    return Err(AuthError::InvalidCredentials);
                }
                if req.password.len() < MIN_PASSWORD_LEN {
                    return Err(AuthError::InvalidCredentials);
                }
                let display_name = req.display_name.trim();
                if display_name.is_empty() || display_name.len() > 128 {
                    return Err(AuthError::InvalidCredentials);
                }

                // Argon2id outside the tx — the work is slow and the row-level
                // lock window must stay short.
                let password_hash = password::hash_password(req.password.expose())?;

                let user_id = UserId::new();
                let user_bytes = user_id.as_bytes();
                let verification_plaintext = session::random_token(24)?;
                let verification_hash = sha256_token(&verification_plaintext);
                let now = Utc::now();
                let expires_at = now + chrono_duration(VERIFICATION_TTL)?;

                // Two-step tx bypasses the repo abstraction: orphan-row
                // prevention requires user + verification-token INSERT
                // atomicity. The repos are pool-bound; convert to
                // `Executor`-generic when a fourth multi-step flow appears.
                let mut tx = self.pool.begin().await.map_err(map_sqlx_err)?;

                let user_insert = sqlx::query(
                    "INSERT INTO users \
             (id, email, email_verified_at, display_name, avatar_url, password_hash, \
              created_at, last_login_at, locked_until, failed_login_count, mfa_enabled, \
              mfa_secret, version, deleted_at) \
             VALUES ($1, $2, NULL, $3, NULL, $4, $5, NULL, NULL, 0, FALSE, NULL, 0, NULL)",
                )
                .bind(user_bytes.as_slice())
                .bind(&email)
                .bind(display_name)
                .bind(&password_hash)
                .bind(now)
                .execute(&mut *tx)
                .await;

                if let Err(err) = user_insert {
                    // Roll back implicitly by dropping the tx without commit;
                    // surface a typed conflict for the unique-email index.
                    if is_unique_violation(&err) {
                        return Err(AuthError::EmailAlreadyRegistered);
                    }
                    return Err(map_sqlx_err(err));
                }

                sqlx::query(
                    "INSERT INTO verification_tokens \
             (token_hash, user_id, kind, payload, created_at, expires_at, consumed_at) \
             VALUES ($1, $2, $3, NULL, $4, $5, NULL)",
                )
                .bind(verification_hash.as_slice())
                .bind(user_bytes.as_slice())
                .bind(KIND_EMAIL_VERIFICATION)
                .bind(now)
                .bind(expires_at)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx_err)?;

                tx.commit().await.map_err(map_sqlx_err)?;

                // Email send happens AFTER the tx commits. A delivery failure
                // here returns `AuthError::Internal` — the user still exists in
                // an unverified state and can recover by requesting a password
                // reset (the reset flow does not require an email-verified
                // account to issue the cooldown-bounded token).
                // Signup deliberately commits the user record before queueing
                // the verification email so a transient transport failure does
                // not destroy the durable account on retry.
                if let Err(err) = self
                    .email_port
                    .send(EmailMessage {
                        to: email.clone(),
                        subject: "Verify your email".to_owned(),
                        body: verification_plaintext,
                        kind: EmailKind::Verification,
                    })
                    .await
                {
                    tracing::error!(
                        error = %err,
                        user_id = %user_id,
                        "failed to deliver verification email after user-create commit",
                    );
                    return Err(AuthError::Internal(format!("email: {err}")));
                }

                tracing::info!(user_id = %user_id, "user registered");
                Ok(UserProfile {
                    user_id: user_id.to_string(),
                    email,
                    display_name: display_name.to_owned(),
                    avatar_url: None,
                    email_verified: false,
                    mfa_enabled: false,
                })
            },
            |result| match result {
                Ok(_) => auth_outcome::SUCCESS,
                Err(AuthError::EmailAlreadyRegistered) => auth_outcome::CONFLICT,
                // Register-side validation rejections (short password,
                // missing @, blank display name) come back as
                // `InvalidCredentials` from the existing implementation;
                // per oracle locked spec map them to `invalid_creds` on
                // the attempts counter (no `invalid_input` split for the
                // register path).
                Err(AuthError::InvalidCredentials) => auth_outcome::INVALID_CREDS,
                Err(_) => auth_outcome::INTERNAL,
            },
        )
        .await
    }

    #[tracing::instrument(level = "info", skip(self, email, password_input, totp), fields(email_len = email.len()))]
    async fn authenticate_password(
        &self,
        email: &str,
        password_input: &str,
        totp: Option<&str>,
    ) -> Result<PasswordOutcome, AuthError> {
        metrics_emit::run_with_metrics(
            &self.metrics,
            NEBULA_API_AUTH_ATTEMPTS_TOTAL,
            None,
            async move {
                let user = self
                    .user_repo
                    .get_by_email(email)
                    .await?
                    .ok_or(AuthError::InvalidCredentials)?;

                if let Some(until) = user.locked_until
                    && until > Utc::now()
                {
                    return Err(AuthError::AccountLocked);
                }

                let stored_hash = user
                    .password_hash
                    .as_deref()
                    .ok_or(AuthError::InvalidCredentials)?;
                if !password::verify_password(stored_hash, password_input)? {
                    self.user_repo.record_login_failure(&user.id).await?;
                    return Err(AuthError::InvalidCredentials);
                }

                // record_login_success ONLY; no `update` call — a profile
                // update would CAS-conflict with concurrent patches and
                // spuriously bump `version` on every login.
                self.user_repo.record_login_success(&user.id).await?;

                if user.mfa_enabled {
                    let secret = decode_mfa_secret(&user)?;
                    if let Some(code) = totp {
                        if !mfa::verify_code(&secret, code)? {
                            return Err(AuthError::InvalidMfaCode);
                        }
                        Ok(PasswordOutcome::Authenticated(row_to_profile(&user)?))
                    } else {
                        let challenge_plaintext = session::random_token(24)?;
                        let challenge_hash = sha256_token(&challenge_plaintext);
                        let now = Utc::now();
                        let expires_at = now + chrono_duration(MFA_CHALLENGE_TTL)?;
                        self.verification_token_repo
                            .create(&VerificationTokenRow {
                                token_hash: challenge_hash.to_vec(),
                                user_id: user.id.clone(),
                                kind: KIND_MFA_CHALLENGE.to_owned(),
                                payload: None,
                                created_at: now,
                                expires_at,
                                consumed_at: None,
                            })
                            .await?;
                        Ok(PasswordOutcome::MfaRequired {
                            challenge_token: challenge_plaintext,
                        })
                    }
                } else {
                    Ok(PasswordOutcome::Authenticated(row_to_profile(&user)?))
                }
            },
            |result| match result {
                Ok(PasswordOutcome::Authenticated(_)) => auth_outcome::SUCCESS,
                Ok(PasswordOutcome::MfaRequired { .. }) => auth_outcome::MFA_REQUIRED,
                Err(AuthError::AccountLocked) => auth_outcome::LOCKOUT,
                Err(AuthError::InvalidCredentials) => auth_outcome::INVALID_CREDS,
                Err(AuthError::InvalidMfaCode) => auth_outcome::INVALID_MFA_CODE,
                Err(_) => auth_outcome::INTERNAL,
            },
        )
        .await
    }

    #[tracing::instrument(level = "info", skip(self, challenge_token, code))]
    async fn verify_mfa(
        &self,
        challenge_token: &str,
        code: &str,
    ) -> Result<UserProfile, AuthError> {
        metrics_emit::run_with_metrics(
            &self.metrics,
            NEBULA_API_AUTH_MFA_ATTEMPTS_TOTAL,
            None,
            async move {
                let challenge_hash = sha256_token(challenge_token);
                // `consume_by_hash_and_kind` filters on `kind` inside the same
                // UPDATE so a non-MFA token (e.g. password_reset) sent to this
                // endpoint does NOT match and is NOT consumed; the row stays
                // available for the valid follow-up at its real route.
                let token_row = self
                    .verification_token_repo
                    .consume_by_hash_and_kind(&challenge_hash, KIND_MFA_CHALLENGE)
                    .await?
                    .ok_or(AuthError::InvalidToken)?;
                let user = self
                    .user_repo
                    .get(&token_row.user_id)
                    .await?
                    .ok_or(AuthError::UserNotFound)?;
                let secret = decode_mfa_secret(&user)?;
                if !mfa::verify_code(&secret, code)? {
                    return Err(AuthError::InvalidMfaCode);
                }
                row_to_profile(&user)
            },
            |result| match result {
                Ok(_) => auth_outcome::SUCCESS,
                Err(AuthError::InvalidMfaCode) => auth_outcome::INVALID_MFA_CODE,
                Err(AuthError::InvalidToken) => auth_outcome::TOKEN_INVALID,
                Err(_) => auth_outcome::INTERNAL,
            },
        )
        .await
    }

    #[tracing::instrument(level = "info", skip(self), fields(user_id))]
    async fn create_session(&self, user_id: &str) -> Result<SessionRecord, AuthError> {
        let user_bytes = user_id_bytes(user_id)?;
        // Ensure the user exists (else the FK on sessions.user_id will
        // reject the INSERT with an opaque error).
        if self.user_repo.get(&user_bytes).await?.is_none() {
            return Err(AuthError::UserNotFound);
        }
        let session_id = session::random_token(32)?;
        let csrf = session::random_token(24)?;
        let now = Utc::now();
        let exp = now + chrono_duration(SESSION_TTL)?;
        self.session_repo
            .create(&SessionRow {
                id: session_id.as_bytes().to_vec(),
                user_id: user_bytes.to_vec(),
                created_at: now,
                last_active_at: now,
                expires_at: exp,
                ip_address: None,
                user_agent: None,
                revoked_at: None,
            })
            .await?;
        Ok(SessionRecord {
            id: session_id,
            principal: Principal::User(UserId::from_bytes(user_bytes)),
            csrf_token: csrf,
            expires_at: expires_at(SESSION_TTL),
        })
    }

    #[tracing::instrument(level = "info", skip(self, session_id))]
    async fn revoke_session(&self, session_id: &str) -> Result<(), AuthError> {
        self.session_repo.revoke(session_id.as_bytes()).await?;
        Ok(())
    }

    #[tracing::instrument(level = "info", skip(self, presented))]
    async fn lookup_pat(&self, presented: &str) -> Result<Option<PatRecord>, AuthError> {
        let hash = pat::hash_for_lookup(presented)?;
        let row = self.pat_repo.get_by_hash(&hash).await?;
        match row {
            Some(row) => Ok(Some(row_to_pat_record(row)?)),
            None => Ok(None),
        }
    }

    #[tracing::instrument(level = "info", skip(self), fields(user_id))]
    async fn get_user_profile(&self, user_id: &str) -> Result<UserProfile, AuthError> {
        let row = fetch_user_by_id(&self.user_repo, user_id).await?;
        row_to_profile(&row)
    }

    #[tracing::instrument(level = "info", skip(self, patch), fields(user_id))]
    async fn update_user_profile(
        &self,
        user_id: &str,
        patch: ProfilePatch,
    ) -> Result<UserProfile, AuthError> {
        let mut row = fetch_user_by_id(&self.user_repo, user_id).await?;
        if let Some(name) = patch.display_name.as_deref() {
            let trimmed = name.trim();
            if trimmed.is_empty() || trimmed.len() > 128 {
                return Err(AuthError::InvalidInput(
                    "display_name must be 1..=128 non-blank characters",
                ));
            }
            row.display_name = trimmed.to_owned();
        }
        if let Some(avatar) = patch.avatar_url.as_deref() {
            let trimmed = avatar.trim();
            row.avatar_url = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            };
        }
        let expected_version = row.version;
        self.user_repo.update(&row, expected_version).await?;
        // Re-fetch so the post-update version + side fields propagate
        // (e.g. for a future caller that reads `version` from the
        // returned profile).
        let refreshed = fetch_user_by_id(&self.user_repo, user_id).await?;
        tracing::info!(user_id = %user_id, "user profile updated");
        row_to_profile(&refreshed)
    }

    #[tracing::instrument(level = "info", skip(self), fields(user_id))]
    async fn list_pats(&self, user_id: &str) -> Result<Vec<PatRecord>, AuthError> {
        let bytes = user_id_bytes(user_id)?;
        if self.user_repo.get(&bytes).await?.is_none() {
            return Err(AuthError::UserNotFound);
        }
        let rows = self
            .pat_repo
            .list_for_principal(PRINCIPAL_KIND_USER, &bytes)
            .await?;
        let mut out: Vec<PatRecord> = rows
            .into_iter()
            .map(row_to_pat_record)
            .collect::<Result<_, _>>()?;
        // Newest first (mirror in-memory test expectations).
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    #[tracing::instrument(level = "info", skip(self, params), fields(user_id, pat_name_len = params.name.len()))]
    async fn create_pat(
        &self,
        user_id: &str,
        params: CreatePatParams,
    ) -> Result<MintedPat, AuthError> {
        let bytes = user_id_bytes(user_id)?;
        if self.user_repo.get(&bytes).await?.is_none() {
            return Err(AuthError::UserNotFound);
        }
        let name = params.name.trim();
        if name.is_empty() || name.len() > 128 {
            return Err(AuthError::InvalidInput(
                "token name must be 1..=128 non-blank characters",
            ));
        }
        let expires_at = compute_pat_expires_at(params.ttl_seconds)?;
        let user_typed = UserId::from_bytes(bytes);
        let minted = pat::mint_pat(
            user_typed,
            name.to_owned(),
            params.scopes.clone(),
            expires_at,
        )?;
        let scopes_json = serde_json::to_value(&params.scopes)
            .map_err(|e| AuthError::Internal(format!("pat scopes serialize: {e}")))?;
        self.pat_repo
            .create(&PersonalAccessTokenRow {
                id: minted.record.id.as_bytes().to_vec(),
                principal_kind: PRINCIPAL_KIND_USER.to_owned(),
                principal_id: bytes.to_vec(),
                name: minted.record.name.clone(),
                prefix: minted.record.prefix.clone(),
                hash: minted.record.hash.to_vec(),
                scopes: scopes_json,
                created_at: minted.record.created_at,
                last_used_at: None,
                expires_at: minted.record.expires_at,
                revoked_at: None,
            })
            .await?;
        tracing::info!(user_id = %user_id, pat_id = %minted.record.id, "personal access token created");
        Ok(minted)
    }

    #[tracing::instrument(level = "info", skip(self), fields(user_id, pat_id))]
    async fn revoke_pat(&self, user_id: &str, pat_id: &str) -> Result<(), AuthError> {
        let bytes = user_id_bytes(user_id)?;
        // Cross-user existence is hidden — a PAT owned by a different
        // principal is reported as not-found, same as a missing token.
        // `list_for_principal` only returns ACTIVE tokens, so an
        // already-revoked PAT belonging to this user would fall through
        // to a `UserNotFound` 404 — wrong for a documented-idempotent
        // DELETE. Distinguish "unknown PAT" from "already revoked" with
        // a single ownership probe that ignores the revoked filter.
        let owned = self
            .pat_repo
            .list_for_principal(PRINCIPAL_KIND_USER, &bytes)
            .await?
            .into_iter()
            .find(|row| row.id == pat_id.as_bytes());
        if let Some(row) = owned {
            self.pat_repo.revoke(&row.id).await?;
            tracing::info!(user_id = %user_id, pat_id = %pat_id, "personal access token revoked");
            return Ok(());
        }
        // No live PAT matched. Check whether the row exists at all and
        // belongs to this principal — if so the second revoke is a no-op
        // (idempotent). If not, surface `UserNotFound` exactly as before.
        let existing: Option<(Vec<u8>,)> = sqlx::query_as(
            "SELECT id FROM personal_access_tokens \
             WHERE id = $1 AND principal_kind = $2 AND principal_id = $3",
        )
        .bind(pat_id.as_bytes())
        .bind(PRINCIPAL_KIND_USER)
        .bind(bytes.as_slice())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        if existing.is_some() {
            tracing::info!(
                user_id = %user_id,
                pat_id = %pat_id,
                "personal access token revoke is a no-op (already revoked)",
            );
            return Ok(());
        }
        Err(AuthError::UserNotFound)
    }

    #[tracing::instrument(level = "info", skip(self, email))]
    async fn request_password_reset(&self, email: &str) -> Result<(), AuthError> {
        // Enumeration-safe: every internal failure is logged + swallowed.
        let user = match self.user_repo.get_by_email(email).await {
            Ok(Some(u)) => u,
            Ok(None) => return Ok(()),
            Err(err) => {
                tracing::error!(error = %err, "password reset: failed to look up user");
                return Ok(());
            },
        };
        let user_id_typed = match user_id_from_bytes(&user.id) {
            Ok(id) => id,
            Err(err) => {
                tracing::error!(error = %err, "password reset: malformed user id row");
                return Ok(());
            },
        };
        let plaintext = match session::random_token(24) {
            Ok(t) => t,
            Err(err) => {
                tracing::error!(error = %err, user_id = %user_id_typed, "failed to mint password reset token");
                return Ok(());
            },
        };
        let now = Utc::now();
        let expires_at = match chrono_duration(VERIFICATION_TTL) {
            Ok(d) => now + d,
            Err(err) => {
                tracing::error!(error = %err, "verification TTL out of range");
                return Ok(());
            },
        };
        let row = VerificationTokenRow {
            token_hash: sha256_token(&plaintext).to_vec(),
            user_id: user.id.clone(),
            kind: KIND_PASSWORD_RESET.to_owned(),
            payload: None,
            created_at: now,
            expires_at,
            consumed_at: None,
        };
        if let Err(err) = self.verification_token_repo.create(&row).await {
            tracing::error!(error = %err, user_id = %user_id_typed, "failed to persist password reset token");
            return Ok(());
        }
        if let Err(err) = self
            .email_port
            .send(EmailMessage {
                to: user.email.clone(),
                subject: "Reset your password".to_owned(),
                body: plaintext,
                kind: EmailKind::PasswordReset,
            })
            .await
        {
            tracing::error!(error = %err, user_id = %user_id_typed, "failed to dispatch password reset email");
        }
        Ok(())
    }

    #[tracing::instrument(level = "info", skip(self, token, new_password))]
    async fn complete_password_reset(
        &self,
        token: &str,
        new_password: &str,
    ) -> Result<(), AuthError> {
        metrics_emit::run_with_metrics(
            &self.metrics,
            NEBULA_API_AUTH_ATTEMPTS_TOTAL,
            None,
            async move {
                // Validate length BEFORE the tx so a malformed input never
                // burns the reset token: the atomic UPDATE that consumes the
                // token is the serialization point, and we do not want a 400
                // path to leave the token marked consumed.
                if new_password.len() < MIN_PASSWORD_LEN {
                    return Err(AuthError::InvalidCredentials);
                }
                // Argon2id BEFORE the tx so the slow work stays outside the
                // row-lock window.
                let new_hash = password::hash_password(new_password)?;
                let token_hash = sha256_token(token);

                // Three-step tx bypasses the repo abstraction: the
                // consume-token / update-password / revoke-siblings sequence
                // must be atomic so a partial application cannot strand a
                // burned token against an unchanged password. The repos are
                // pool-bound; convert to `Executor`-generic when a fourth
                // multi-step flow appears.
                let mut tx = self.pool.begin().await.map_err(map_sqlx_err)?;

                let consumed: Option<(Vec<u8>,)> = sqlx::query_as(
                    "UPDATE verification_tokens SET consumed_at = NOW() \
             WHERE token_hash = $1 AND kind = $2 \
               AND consumed_at IS NULL AND expires_at > NOW() \
             RETURNING user_id",
                )
                .bind(token_hash.as_slice())
                .bind(KIND_PASSWORD_RESET)
                .fetch_optional(&mut *tx)
                .await
                .map_err(map_sqlx_err)?;

                let Some((user_id_bytes,)) = consumed else {
                    return Err(AuthError::InvalidToken);
                };

                // No CAS guard inside the tx — the consumed-by-hash row IS
                // the serialization point: only one caller can successfully
                // burn the token, so concurrent password-set races are
                // impossible for the same reset link. `version` is still
                // bumped so any concurrent reader sees the world advance.
                let updated = sqlx::query(
                    "UPDATE users SET \
                 password_hash = $2, failed_login_count = 0, locked_until = NULL, \
                 version = version + 1 \
             WHERE id = $1 AND deleted_at IS NULL",
                )
                .bind(user_id_bytes.as_slice())
                .bind(&new_hash)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx_err)?
                .rows_affected();
                if updated == 0 {
                    return Err(AuthError::UserNotFound);
                }

                // Revoke any in-flight sibling reset tokens so a stolen second
                // link cannot be replayed after a successful reset.
                sqlx::query(
                    "UPDATE verification_tokens SET consumed_at = NOW() \
                     WHERE user_id = $1 AND kind = $2 AND consumed_at IS NULL",
                )
                .bind(user_id_bytes.as_slice())
                .bind(KIND_PASSWORD_RESET)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx_err)?;

                tx.commit().await.map_err(map_sqlx_err)?;
                Ok(())
            },
            |result| match result {
                Ok(()) => auth_outcome::SUCCESS,
                Err(AuthError::InvalidToken) => auth_outcome::TOKEN_INVALID,
                // Per oracle per-method map: `complete_password_reset`
                // collapses `InvalidCredentials` to `invalid_input`
                // because the failure is shape-validation of
                // `new_password` (short / blank), not a credential
                // mismatch.
                Err(AuthError::InvalidCredentials) => auth_outcome::INVALID_INPUT,
                Err(_) => auth_outcome::INTERNAL,
            },
        )
        .await
    }

    #[tracing::instrument(level = "info", skip(self, token))]
    async fn verify_email(&self, token: &str) -> Result<(), AuthError> {
        metrics_emit::run_with_metrics(
            &self.metrics,
            NEBULA_API_AUTH_ATTEMPTS_TOTAL,
            None,
            async move {
                let token_hash = sha256_token(token);

                // Two-step tx bypasses the repo abstraction: the token-consume
                // + email-verified flip must be atomic so a CAS-loss or vanished
                // user row cannot strand a burned verification token against an
                // unchanged `email_verified_at`. The repos are pool-bound;
                // convert to `Executor`-generic when a fourth multi-step flow
                // appears.
                let mut tx = self.pool.begin().await.map_err(map_sqlx_err)?;

                let consumed: Option<(Vec<u8>,)> = sqlx::query_as(
                    "UPDATE verification_tokens SET consumed_at = NOW() \
             WHERE token_hash = $1 AND kind = $2 \
               AND consumed_at IS NULL AND expires_at > NOW() \
             RETURNING user_id",
                )
                .bind(token_hash.as_slice())
                .bind(KIND_EMAIL_VERIFICATION)
                .fetch_optional(&mut *tx)
                .await
                .map_err(map_sqlx_err)?;

                let Some((user_id_bytes,)) = consumed else {
                    return Err(AuthError::InvalidToken);
                };

                // No CAS guard inside the tx — the consumed-by-hash row IS
                // the serialization point for this user's email-verify flow.
                // Bump `version` so concurrent readers see the world advance.
                let updated = sqlx::query(
                    "UPDATE users SET \
                         email_verified_at = NOW(), \
                         version = version + 1 \
                     WHERE id = $1 AND deleted_at IS NULL",
                )
                .bind(user_id_bytes.as_slice())
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx_err)?
                .rows_affected();
                if updated == 0 {
                    return Err(AuthError::UserNotFound);
                }

                tx.commit().await.map_err(map_sqlx_err)?;
                Ok(())
            },
            |result| match result {
                Ok(()) => auth_outcome::SUCCESS,
                Err(AuthError::InvalidToken) => auth_outcome::TOKEN_INVALID,
                Err(_) => auth_outcome::INTERNAL,
            },
        )
        .await
    }

    #[tracing::instrument(level = "info", skip(self), fields(user_id))]
    async fn start_mfa_enrollment(&self, user_id: &str) -> Result<MfaEnrollment, AuthError> {
        metrics_emit::run_with_metrics(
            &self.metrics,
            NEBULA_API_AUTH_MFA_ATTEMPTS_TOTAL,
            None,
            async move {
                let mut row = fetch_user_by_id(&self.user_repo, user_id).await?;
                let (secret, uri) = mfa::mint_secret(&row.email)?;
                // Save secret but DO NOT flip mfa_enabled until
                // confirm_mfa_enrollment. Encryption-at-rest of the MFA secret
                // is deferred to a follow-up that wires the credential
                // service's master-key envelope into the identity surface.
                row.mfa_secret = Some(secret.as_bytes().to_vec());
                row.mfa_enabled = false;
                let expected_version = row.version;
                self.user_repo.update(&row, expected_version).await?;
                Ok(MfaEnrollment {
                    otpauth_uri: uri,
                    secret_base32: secret,
                })
            },
            |result| match result {
                Ok(_) => auth_outcome::SUCCESS,
                Err(_) => auth_outcome::INTERNAL,
            },
        )
        .await
    }

    #[tracing::instrument(level = "info", skip(self, code), fields(user_id))]
    async fn confirm_mfa_enrollment(&self, user_id: &str, code: &str) -> Result<(), AuthError> {
        metrics_emit::run_with_metrics(
            &self.metrics,
            NEBULA_API_AUTH_MFA_ATTEMPTS_TOTAL,
            None,
            async move {
                let mut row = fetch_user_by_id(&self.user_repo, user_id).await?;
                let secret = decode_mfa_secret(&row).map_err(|_| AuthError::InvalidMfaCode)?;
                if !mfa::verify_code(&secret, code)? {
                    return Err(AuthError::InvalidMfaCode);
                }
                if !row.mfa_enabled {
                    row.mfa_enabled = true;
                    let expected_version = row.version;
                    self.user_repo.update(&row, expected_version).await?;
                }
                Ok(())
            },
            |result| match result {
                Ok(()) => auth_outcome::SUCCESS,
                Err(AuthError::InvalidMfaCode) => auth_outcome::INVALID_MFA_CODE,
                Err(_) => auth_outcome::INTERNAL,
            },
        )
        .await
    }

    #[tracing::instrument(level = "info", skip(self, redirect_uri), fields(provider = %provider.as_str()))]
    // PR-3 T3.8 GREEN: rewrite to emit a REAL authorize URL via
    // `flow::build_authorization_uri` after resolving the provider's
    // endpoints (Oidc → discovery doc cache; Manual → operator
    // config). Replaces the synthetic `https://nebula.local/...`
    // placeholder from before PR-3.
    async fn start_oauth(
        &self,
        provider: OAuthProvider,
        redirect_uri: &str,
    ) -> Result<OAuthStart, AuthError> {
        use secrecy::ExposeSecret;

        use crate::transport::oauth::{
            discovery::resolve_provider_endpoints,
            flow::{AuthorizationUriRequest, build_authorization_uri},
        };

        let provider_label = metrics_emit::oauth_provider_label(provider);
        let redirect_uri = redirect_uri.to_owned();
        let oauth_providers = Arc::clone(&self.oauth_providers);
        metrics_emit::run_with_metrics(
            &self.metrics,
            NEBULA_API_AUTH_OAUTH_ATTEMPTS_TOTAL,
            Some(provider_label),
            async move {
                // Step 1: lookup the operator config for `provider`;
                // 503 ProviderNotConfigured per ADR-0085 D-6 if absent.
                let provider_cfg = oauth_providers.providers.get(&provider).ok_or_else(|| {
                    AuthError::ProviderNotConfigured {
                        provider: provider.as_str().to_owned(),
                    }
                })?;

                // Step 2: resolve endpoints (Oidc → discovery cache;
                // Manual → operator config). Any SSRF / fetch failure
                // bubbles up as AuthError::OAuthFailed with the
                // typed DiscoveryError formatted.
                let endpoints = resolve_provider_endpoints(
                    provider_cfg,
                    oauth_providers.oauth_allow_insecure_localhost,
                )
                .await
                .map_err(|e| AuthError::OAuthFailed(e.to_string()))?;

                // Step 3: mint PKCE pair.
                let pkce = mint_pkce()?;

                // Step 4: build the authorize URL with PKCE S256
                // params. `build_authorization_uri` is the same
                // helper Plane-B uses, ensuring uniform query-string
                // construction across the two planes.
                let auth_req = AuthorizationUriRequest {
                    auth_url: endpoints.authorize_url.clone(),
                    token_url: endpoints.token_url.clone(),
                    client_id: provider_cfg.client_id.expose_secret().to_owned(),
                    client_secret: provider_cfg.client_secret.expose_secret().to_owned(),
                    redirect_uri: redirect_uri.clone(),
                    scopes: Some(endpoints.scopes.clone()),
                    auth_style: None,
                };
                let authorize_url =
                    build_authorization_uri(&auth_req, &pkce.state, &pkce.code_challenge)
                        .map_err(|e| {
                            AuthError::OAuthFailed(format!(
                                "authorize URL construction failed: {e}"
                            ))
                        })?
                        .to_string();

                // Step 5: persist the OAuth state row with the
                // handler-derived redirect_uri so complete_oauth can
                // re-verify it (Scenario 3.10 public_url_changed_mid_flow).
                let now = Utc::now();
                let expires_at = now + chrono_duration(OAUTH_STATE_TTL)?;
                self.oauth_state_repo
                    .create(&OAuthStateRow {
                        state: pkce.state.clone(),
                        provider: provider.as_str().to_owned(),
                        code_verifier: pkce.code_verifier,
                        redirect_uri: Some(redirect_uri),
                        created_at: now,
                        expires_at,
                        consumed_at: None,
                    })
                    .await?;
                Ok(OAuthStart {
                    authorize_url,
                    state: pkce.state,
                })
            },
            |result| match result {
                Ok(_) => auth_outcome::SUCCESS,
                // PR-3 wave-2 (CodeRabbit G.5): classify the new
                // fail-closed `ProviderNotConfigured` variant as
                // OAUTH_FAILED in the inline closure too, mirroring
                // the `default_outcome_for` table in error.rs. Without
                // this, an unconfigured-provider error gets the
                // `internal` label and looks like a server bug in
                // dashboards instead of a deployment-state issue.
                Err(AuthError::OAuthFailed(_) | AuthError::ProviderNotConfigured { .. }) => {
                    auth_outcome::OAUTH_FAILED
                },
                Err(_) => auth_outcome::INTERNAL,
            },
        )
        .await
    }

    // PR-2 T2.9: trait sig gained `redirect_uri: &str`. PR-4 verifies
    // it against the OAuthStateRow's persisted redirect_uri to close
    // the `public_url_changed_mid_flow` defense (REQ-oauth-003
    // Scenario 3.10). For now the param is accepted and ignored —
    // complete_oauth still returns NotImplemented until PR-4.
    #[tracing::instrument(level = "info", skip(self, state, _code, _redirect_uri), fields(provider = %provider.as_str(), state_len = state.len()))]
    async fn complete_oauth(
        &self,
        provider: OAuthProvider,
        state: &str,
        _code: &str,
        _redirect_uri: &str,
    ) -> Result<OAuthCompletion, AuthError> {
        let provider_label = metrics_emit::oauth_provider_label(provider);
        metrics_emit::run_with_metrics(
            &self.metrics,
            NEBULA_API_AUTH_OAUTH_ATTEMPTS_TOTAL,
            Some(provider_label),
            async move {
                // `consume_by_state_and_provider` filters on `provider` inside
                // the same UPDATE — a state crossed between providers does NOT
                // match and is NOT consumed; the row stays available for the
                // valid callback at the correct provider. The single-statement
                // UPDATE is the PKCE replay defence: a second callback at the
                // same `(state, provider)` is the loser and sees None.
                //
                // The PG path enforces the replay window even though the
                // actual provider code-exchange is still `NotImplemented`;
                // when a follow-up wires real provider configs and
                // `CredentialService::get::<OAuth2Credential>`, it replaces
                // the `NotImplemented` return with the real exchange without
                // changing any storage semantics.
                let _row = self
                    .oauth_state_repo
                    .consume_by_state_and_provider(state, provider.as_str())
                    .await?
                    .ok_or(AuthError::InvalidToken)?;
                Err(AuthError::NotImplemented(
                    "oauth provider code exchange is not yet wired; complete_oauth \
                     currently consumes the state row but does not exchange the \
                     authorization code",
                ))
            },
            |result| match result {
                Ok(_) => auth_outcome::SUCCESS,
                Err(AuthError::InvalidToken) => auth_outcome::TOKEN_INVALID,
                Err(AuthError::OAuthFailed(_)) => auth_outcome::OAUTH_FAILED,
                // Per oracle per-method map: `complete_oauth` collapses
                // `NotImplemented` to `internal` because the PG path is
                // wired-but-incomplete until PR-C lands the real
                // provider code-exchange.
                Err(_) => auth_outcome::INTERNAL,
            },
        )
        .await
    }
}

/// Decode the `users.mfa_secret` BYTEA back to the base32 string
/// [`mfa::verify_code`] expects. The BYTEA column holds the base32
/// secret as raw UTF-8 bytes — see the file-level module docs for the
/// "TODO: encrypt with master key" caveat.
fn decode_mfa_secret(row: &UserRow) -> Result<String, AuthError> {
    let bytes = row.mfa_secret.as_deref().ok_or(AuthError::InvalidMfaCode)?;
    String::from_utf8(bytes.to_vec())
        .map_err(|_| AuthError::Internal("mfa secret not valid utf-8".to_owned()))
}

/// Convert a `std::time::Duration` into a `chrono::Duration` for use
/// as a Postgres `TIMESTAMPTZ` offset.
fn chrono_duration(d: Duration) -> Result<chrono::Duration, AuthError> {
    chrono::Duration::from_std(d)
        .map_err(|e| AuthError::Internal(format!("duration out of range: {e}")))
}

/// Map a raw `sqlx::Error` into [`AuthError::Internal`] for the
/// transactional flows that bypass the repo abstraction.
fn map_sqlx_err(err: sqlx::Error) -> AuthError {
    AuthError::Internal(format!("storage: {err}"))
}

/// Detect the SQLSTATE `23505` (unique violation) inside a raw
/// `sqlx::Error` — used by [`register_user`] to translate the
/// `users` email-unique-index conflict into
/// [`AuthError::EmailAlreadyRegistered`].
fn is_unique_violation(err: &sqlx::Error) -> bool {
    if let sqlx::Error::Database(db_err) = err {
        return db_err.code().as_deref() == Some("23505");
    }
    false
}
