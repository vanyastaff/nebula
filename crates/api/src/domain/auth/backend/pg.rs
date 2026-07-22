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
//! - Presented session cookies are stored only as domain-separated SHA-256
//!   digests. `PersonalAccessTokenRow.id` / OAuth `state`
//!   are stored as the existing helper string outputs cast to
//!   `as_bytes().to_vec()`. The migration docstrings call these
//!   columns "`sess_` ULID" / "`pat_` ULID" — today we keep the
//!   helper-derived URL-safe base64 strings (43 chars) to avoid
//!   diverging from the in-memory backend or breaking the existing
//!   `me_e2e.rs` test surface. Refactoring the primitives to mint
//!   real ULIDs is a separate change.
//! - `users.mfa_secret_envelope` holds a versioned AES-256-GCM envelope
//!   authenticated for the exact user and active-TOTP purpose. Pending
//!   enrollment uses a distinct AAD purpose and is decrypted/re-sealed when
//!   promoted, so ciphertext cannot be copied across lifecycle authorities.
//! - `OAuthStateRow.redirect_uri` persists the exact handler-derived
//!   callback URL and is rechecked before provider egress.
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
//! OAuth start performs activity-driven cleanup of expired OAuth state.
//! Session and verification-token cleanup still require the deployment's
//! periodic maintenance job.
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
use rand::Rng;
use sha2::{Digest, Sha256};
use sqlx::{Pool, Postgres};

use nebula_storage::{
    identity_secret::{IdentitySecretCodec, TotpSecretPurpose},
    pg::{
        PgMfaEnrollmentRepo, PgOAuthLoginFinalizer, PgOAuthStateRepo, PgPatRepo, PgSessionRepo,
        PgUserRepo, PgVerificationTokenRepo,
    },
    repos::{
        MfaEnrollmentCandidate, MfaEnrollmentInstallOutcome, MfaEnrollmentRepo,
        OAuthLoginFinalizeCommand, OAuthLoginFinalizeOutcome, OAuthLoginFinalized,
        OAuthLoginMfaChallengeDraft, OAuthLoginSessionDraft, OAuthLoginUserDraft,
        OAuthStateAdmission, OAuthStateRepo, PatRepo, SessionRepo, UserRepo, VerificationTokenRepo,
    },
    rows::{OAuthStateRow, PersonalAccessTokenRow, SessionDraft, UserRow, VerificationTokenRow},
};

use super::{
    dto::{SignupRequest, UserProfile},
    error::AuthError,
    mfa,
    oauth::{OAUTH_STATE_TTL, OAuthProvider, mint_pkce},
    password,
    pat::{self, MintedPat, PatRecord, compute_pat_expires_at},
    provider::{
        AuthBackend, AuthenticatedSession, CreatePatParams, MFA_ENROLLMENT_TTL, MfaEnrollment,
        OAuthCompletion, OAuthStart, PasswordOutcome, ProfilePatch, metrics_emit,
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
    mfa_enrollment_repo: Arc<PgMfaEnrollmentRepo>,
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
    /// Opaque Plane-A runtime. `None` means OAuth is disabled safely.
    oauth_runtime: Option<Arc<crate::transport::oauth::OAuthIdentityRuntime>>,
    /// Transaction owner for OAuth user/link/session convergence. Keeping
    /// this behind one storage-owned boundary prevents partial identities
    /// and duplicate users under concurrent callbacks.
    oauth_login_finalizer: Arc<PgOAuthLoginFinalizer>,
    /// Shared credential/identity key authority, snapshotted by the identity
    /// codec and injected by the first-party composition root.
    identity_secrets: Arc<IdentitySecretCodec>,
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
        identity_secrets: Arc<IdentitySecretCodec>,
    ) -> Self {
        Self {
            user_repo: Arc::new(PgUserRepo::new(pool.clone())),
            session_repo: Arc::new(PgSessionRepo::new(pool.clone())),
            pat_repo: Arc::new(PgPatRepo::new(pool.clone())),
            verification_token_repo: Arc::new(PgVerificationTokenRepo::new(pool.clone())),
            mfa_enrollment_repo: Arc::new(PgMfaEnrollmentRepo::new(
                pool.clone(),
                Arc::clone(&identity_secrets),
            )),
            oauth_state_repo: Arc::new(PgOAuthStateRepo::new(pool.clone())),
            oauth_login_finalizer: Arc::new(PgOAuthLoginFinalizer::new(pool.clone())),
            pool,
            email_port,
            metrics,
            oauth_runtime: None,
            identity_secrets,
        }
    }

    /// Attach the single opaque Plane-A OAuth runtime.
    ///
    /// First-party composition only; this technical seam is not a supported
    /// `nebula-sdk` surface.
    #[doc(hidden)]
    #[must_use = "builder methods must be chained or built"]
    pub fn with_oauth_runtime(
        mut self,
        runtime: Arc<crate::transport::oauth::OAuthIdentityRuntime>,
    ) -> Self {
        self.oauth_runtime = Some(runtime);
        self
    }

    /// Wrap into an `Arc<dyn AuthBackend>` for [`crate::AppState`].
    #[must_use]
    pub fn into_arc(self) -> Arc<dyn AuthBackend> {
        Arc::new(self)
    }

    async fn consume_oauth_state(
        &self,
        provider: OAuthProvider,
        state: &str,
        redirect_uri: &str,
    ) -> Result<OAuthStateRow, AuthError> {
        let row = self
            .oauth_state_repo
            .consume_by_state_and_provider(state, provider.as_str())
            .await
            .map_err(oauth_state_repo_error)?
            .ok_or(AuthError::InvalidToken)?;
        match row.redirect_uri.as_deref() {
            Some(stored) if stored == redirect_uri => Ok(row),
            _ => Err(AuthError::from_oauth_failure(
                crate::transport::oauth::OAuthFailureCode::RedirectUriMismatch,
            )),
        }
    }

    async fn verify_active_mfa_code(&self, user: &UserRow, code: &str) -> Result<bool, AuthError> {
        let envelope = user
            .mfa_secret_envelope
            .as_deref()
            .ok_or(AuthError::InvalidMfaCode)?;
        let opened = self
            .identity_secrets
            .open_totp_seed(TotpSecretPurpose::Active, &user.id, envelope)
            .map_err(identity_secret_auth_error)?;
        let secret = std::str::from_utf8(&opened.plaintext)
            .map_err(|_| AuthError::Internal("MFA secret encoding is invalid".to_owned()))?;
        let valid = mfa::verify_code(secret, code)?;
        if let Some(replacement) = opened.replacement_envelope.as_deref() {
            self.user_repo
                .rotate_mfa_secret_envelope(&user.id, envelope, replacement)
                .await?;
        }
        Ok(valid)
    }
}

// ── private helpers ─────────────────────────────────────────────────────

/// SHA-256 a plaintext token to its storage shape (32-byte digest).
fn sha256_token(plaintext: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(plaintext.as_bytes());
    hasher.finalize().into()
}

/// Plane-A state values and PKCE verifiers are secrets. Repository errors can
/// contain PostgreSQL constraint detail with bound values, so this boundary
/// deliberately discards every underlying detail before auth logging.
fn oauth_state_repo_error(_: nebula_storage::StorageError) -> AuthError {
    AuthError::Internal("OAuth state storage operation failed".to_owned())
}

/// Pending enrollment rows carry opaque identity-secret envelopes and
/// candidate identifiers. Database constraint details must not cross the
/// Plane-A auth boundary.
fn mfa_enrollment_repo_error(_: nebula_storage::StorageError) -> AuthError {
    AuthError::Internal("MFA enrollment storage operation failed".to_owned())
}

fn identity_secret_auth_error(
    _: nebula_storage::identity_secret::IdentitySecretError,
) -> AuthError {
    AuthError::Internal("MFA secret envelope operation failed".to_owned())
}

fn require_oauth_state_admitted(admission: OAuthStateAdmission) -> Result<(), AuthError> {
    match admission {
        OAuthStateAdmission::Created => Ok(()),
        OAuthStateAdmission::AtCapacity | OAuthStateAdmission::Contended => {
            Err(AuthError::RateLimit)
        },
        _ => Err(AuthError::Internal(
            "OAuth state storage returned an unsupported admission outcome".to_owned(),
        )),
    }
}

fn oauth_start_outcome<T>(result: &Result<T, AuthError>) -> &'static str {
    match result {
        Ok(_) => auth_outcome::SUCCESS,
        Err(AuthError::OAuthFailed | AuthError::ProviderNotConfigured) => {
            auth_outcome::OAUTH_FAILED
        },
        Err(AuthError::RateLimit) => auth_outcome::RATE_LIMIT,
        Err(_) => auth_outcome::INTERNAL,
    }
}

/// OAuth finalization errors may contain constraint details and bound
/// identity values. Keep the auth boundary secret-free and stable.
fn oauth_login_finalize_error(_: nebula_storage::StorageError) -> AuthError {
    AuthError::Internal("OAuth login storage operation failed".to_owned())
}

struct PreparedOAuthFinalize {
    command: OAuthLoginFinalizeCommand,
    csrf_token: String,
    challenge_token: String,
}

fn build_oauth_finalize_command(
    provider: OAuthProvider,
    subject: &str,
    verified_email: Option<String>,
) -> Result<PreparedOAuthFinalize, AuthError> {
    let session_id = session::random_token(32)?;
    let csrf_token = session::random_token(24)?;
    let challenge_token = session::random_token(24)?;
    let now = Utc::now();
    let expires_at = now + chrono_duration(SESSION_TTL)?;
    let challenge_expires_at = now + chrono_duration(MFA_CHALLENGE_TTL)?;
    let display_name = verified_email.as_deref().unwrap_or("OAuth user").to_owned();
    Ok(PreparedOAuthFinalize {
        command: OAuthLoginFinalizeCommand {
            provider: provider.as_str().to_owned(),
            subject: subject.to_owned(),
            verified_email,
            candidate_user: OAuthLoginUserDraft {
                id: UserId::new().as_bytes().to_vec(),
                display_name,
                avatar_url: None,
                created_at: now,
            },
            session: OAuthLoginSessionDraft {
                token: session_id.into_bytes(),
                created_at: now,
                last_active_at: now,
                expires_at,
                ip_address: None,
                user_agent: None,
            },
            mfa_challenge: OAuthLoginMfaChallengeDraft {
                token_hash: sha256_token(&challenge_token),
                created_at: now,
                expires_at: challenge_expires_at,
            },
        },
        csrf_token,
        challenge_token,
    })
}

fn finalized_oauth_completion(
    finalized: OAuthLoginFinalized,
    csrf_token: String,
) -> Result<OAuthCompletion, AuthError> {
    let OAuthLoginFinalized {
        user,
        session_token,
        session_expires_at,
    } = finalized;
    let user_id = user_id_from_bytes(&user.id)?;
    let session_id = String::from_utf8(session_token)
        .map_err(|_| AuthError::Internal("OAuth session id is not valid UTF-8".to_owned()))?;
    let session_record = SessionRecord {
        id: session_id,
        principal: Principal::User(user_id),
        csrf_token,
        expires_at: session_expires_at,
    };
    Ok(OAuthCompletion::SessionCreated {
        user: row_to_profile(&user)?,
        session: session_record,
    })
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
    ) -> Result<Option<AuthenticatedSession>, crate::ApiError> {
        let row = self
            .session_repo
            .get(session_id.as_bytes())
            .await
            .map_err(crate::ApiError::from)?;
        match row {
            Some(row) => {
                let user_id = user_id_from_bytes(&row.user_id).map_err(crate::ApiError::from)?;
                Ok(Some(AuthenticatedSession {
                    principal: Principal::User(user_id),
                    authenticated_at: row.created_at,
                }))
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
              mfa_secret_envelope, version, deleted_at) \
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
                    if let Some(code) = totp {
                        if !self.verify_active_mfa_code(&user, code).await? {
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
                if !self.verify_active_mfa_code(&user, code).await? {
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
            .create(
                session_id.as_bytes(),
                &SessionDraft {
                    user_id: user_bytes.to_vec(),
                    created_at: now,
                    last_active_at: now,
                    expires_at: exp,
                    ip_address: None,
                    user_agent: None,
                    revoked_at: None,
                },
            )
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
                let row = fetch_user_by_id(&self.user_repo, user_id).await?;
                let (secret, uri) = mfa::mint_secret(&row.email)?;
                let secret_envelope = self
                    .identity_secrets
                    .seal_totp_seed(
                        TotpSecretPurpose::EnrollmentCandidate,
                        &row.id,
                        secret.as_bytes(),
                    )
                    .map_err(identity_secret_auth_error)?;
                let mut enrollment_id = [0_u8; 32];
                rand::rng().fill_bytes(&mut enrollment_id);
                let now = Utc::now();
                let candidate = MfaEnrollmentCandidate::new(
                    enrollment_id,
                    row.id,
                    secret_envelope,
                    now,
                    now + chrono_duration(MFA_ENROLLMENT_TTL)?,
                )
                .map_err(mfa_enrollment_repo_error)?;
                self.mfa_enrollment_repo
                    .replace_candidate(&candidate)
                    .await
                    .map_err(mfa_enrollment_repo_error)?;
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
                let user_bytes = user_id_bytes(user_id)?;
                let candidate = self
                    .mfa_enrollment_repo
                    .get_live_candidate(&user_bytes)
                    .await
                    .map_err(mfa_enrollment_repo_error)?
                    .ok_or(AuthError::InvalidMfaCode)?;
                let opened = self
                    .identity_secrets
                    .open_totp_seed(
                        TotpSecretPurpose::EnrollmentCandidate,
                        &user_bytes,
                        candidate.secret_envelope(),
                    )
                    .map_err(identity_secret_auth_error)?;
                let secret = std::str::from_utf8(&opened.plaintext).map_err(|_| {
                    AuthError::Internal("MFA secret encoding is invalid".to_owned())
                })?;
                if !mfa::verify_code(secret, code)? {
                    return Err(AuthError::InvalidMfaCode);
                }
                let enrollment_id = *candidate.enrollment_id();
                match self
                    .mfa_enrollment_repo
                    .install_candidate(&user_bytes, &enrollment_id)
                    .await
                    .map_err(mfa_enrollment_repo_error)?
                {
                    MfaEnrollmentInstallOutcome::Installed => Ok(()),
                    MfaEnrollmentInstallOutcome::CandidateUnavailable => {
                        Err(AuthError::InvalidMfaCode)
                    },
                    _ => Err(AuthError::Internal(
                        "unsupported MFA enrollment installation outcome".to_owned(),
                    )),
                }
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
    // Resolve provider endpoints under the fixed runtime policy and persist
    // the PKCE state plus exact callback URL durably.
    async fn start_oauth(
        &self,
        provider: OAuthProvider,
        redirect_uri: &str,
    ) -> Result<OAuthStart, AuthError> {
        let provider_label = metrics_emit::oauth_provider_label(provider);
        let redirect_uri = redirect_uri.to_owned();
        let runtime = self.oauth_runtime.as_ref().map(Arc::clone);
        metrics_emit::run_with_metrics(
            &self.metrics,
            NEBULA_API_AUTH_OAUTH_ATTEMPTS_TOTAL,
            Some(provider_label),
            async move {
                let runtime = runtime.ok_or(AuthError::ProviderNotConfigured)?;
                let pkce = mint_pkce()?;
                let deadline = runtime.begin_deadline();
                let authorize_url = runtime
                    .build_authorization_url(
                        &deadline,
                        provider,
                        &redirect_uri,
                        &pkce.state,
                        &pkce.code_challenge,
                    )
                    .await
                    .map_err(AuthError::from_oauth_failure)?;
                let now = Utc::now();
                let expires_at = now + chrono_duration(OAUTH_STATE_TTL)?;
                let admission = self
                    .oauth_state_repo
                    .admit(&OAuthStateRow {
                        state: pkce.state.clone(),
                        provider: provider.as_str().to_owned(),
                        code_verifier: pkce.code_verifier,
                        redirect_uri: Some(redirect_uri),
                        created_at: now,
                        expires_at,
                        consumed_at: None,
                    })
                    .await
                    .map_err(oauth_state_repo_error)?;
                require_oauth_state_admitted(admission)?;
                Ok(OAuthStart {
                    authorize_url,
                    state: pkce.state,
                })
            },
            oauth_start_outcome,
        )
        .await
    }

    #[tracing::instrument(
        level = "info",
        skip(self, state, redirect_uri),
        fields(provider = %provider.as_str(), state_len = state.len())
    )]
    async fn cancel_oauth(
        &self,
        provider: OAuthProvider,
        state: &str,
        redirect_uri: &str,
    ) -> Result<(), AuthError> {
        let provider_label = metrics_emit::oauth_provider_label(provider);
        metrics_emit::run_with_metrics(
            &self.metrics,
            NEBULA_API_AUTH_OAUTH_ATTEMPTS_TOTAL,
            Some(provider_label),
            async move {
                self.consume_oauth_state(provider, state, redirect_uri)
                    .await?;
                Ok(())
            },
            |result| match result {
                Ok(()) => auth_outcome::OAUTH_FAILED,
                Err(AuthError::InvalidToken) => auth_outcome::TOKEN_INVALID,
                Err(AuthError::OAuthFailed) => auth_outcome::OAUTH_FAILED,
                Err(_) => auth_outcome::INTERNAL,
            },
        )
        .await
    }

    // Token exchange, userinfo, verified-email policy and identity linking
    // mirror the in-memory implementation; persistence is durable PG.
    #[tracing::instrument(level = "info", skip(self, state, code, redirect_uri), fields(provider = %provider.as_str(), state_len = state.len()))]
    async fn complete_oauth(
        &self,
        provider: OAuthProvider,
        state: &str,
        code: &str,
        redirect_uri: &str,
    ) -> Result<OAuthCompletion, AuthError> {
        let provider_label = metrics_emit::oauth_provider_label(provider);
        let redirect_uri = redirect_uri.to_owned();
        let code = code.to_owned();
        let state = state.to_owned();
        let runtime = self.oauth_runtime.as_ref().map(Arc::clone);
        metrics_emit::run_with_metrics(
            &self.metrics,
            NEBULA_API_AUTH_OAUTH_ATTEMPTS_TOTAL,
            Some(provider_label),
            async move {
                let runtime = runtime.ok_or(AuthError::ProviderNotConfigured)?;
                let row = self
                    .consume_oauth_state(provider, &state, &redirect_uri)
                    .await?;
                let deadline = runtime.begin_deadline();
                let pending = runtime
                    .begin_identity_completion(
                        deadline,
                        provider,
                        &state,
                        &code,
                        &redirect_uri,
                        &row.code_verifier,
                    )
                    .await
                    .map_err(AuthError::from_oauth_failure)?;
                let sub = pending.subject().to_owned();

                // First ask storage to resolve an existing stable subject
                // link. This path atomically creates exactly one local auth
                // artifact and avoids an unnecessary verified-email request
                // on repeat login.
                let PreparedOAuthFinalize {
                    command,
                    csrf_token,
                    challenge_token,
                } = build_oauth_finalize_command(provider, &sub, None)?;
                match self
                    .oauth_login_finalizer
                    .finalize(command)
                    .await
                    .map_err(oauth_login_finalize_error)?
                {
                    OAuthLoginFinalizeOutcome::Finalized(finalized) => {
                        drop(pending);
                        drop(challenge_token);
                        return finalized_oauth_completion(*finalized, csrf_token);
                    },
                    OAuthLoginFinalizeOutcome::MfaRequired => {
                        drop(pending);
                        drop(csrf_token);
                        return Ok(OAuthCompletion::MfaRequired { challenge_token });
                    },
                    OAuthLoginFinalizeOutcome::VerifiedEmailRequired => {
                        drop(csrf_token);
                        drop(challenge_token);
                    },
                    OAuthLoginFinalizeOutcome::AccountLinkRequired => {
                        return Err(AuthError::AccountLinkRequired);
                    },
                    OAuthLoginFinalizeOutcome::LinkedUserUnavailable => {
                        return Err(AuthError::Internal(
                            "OAuth identity link is unavailable".to_owned(),
                        ));
                    },
                    _ => {
                        return Err(AuthError::Internal(
                            "OAuth login finalizer returned an unsupported outcome".to_owned(),
                        ));
                    },
                }

                // No subject link exists. Acquire provider-attested email
                // before asking the same transaction boundary to converge
                // user + link + session under all concurrent races.
                let resolved_email = runtime
                    .resolve_verified_identity(pending)
                    .await
                    .map_err(AuthError::from_oauth_failure)?
                    .into_string();
                if resolved_email.is_empty() {
                    return Err(AuthError::EmailNotVerified);
                }
                let PreparedOAuthFinalize {
                    command,
                    csrf_token,
                    challenge_token,
                } = build_oauth_finalize_command(provider, &sub, Some(resolved_email))?;
                match self
                    .oauth_login_finalizer
                    .finalize(command)
                    .await
                    .map_err(oauth_login_finalize_error)?
                {
                    OAuthLoginFinalizeOutcome::Finalized(finalized) => {
                        drop(challenge_token);
                        finalized_oauth_completion(*finalized, csrf_token)
                    },
                    OAuthLoginFinalizeOutcome::MfaRequired => {
                        drop(csrf_token);
                        Ok(OAuthCompletion::MfaRequired { challenge_token })
                    },
                    OAuthLoginFinalizeOutcome::VerifiedEmailRequired => {
                        Err(AuthError::EmailNotVerified)
                    },
                    OAuthLoginFinalizeOutcome::AccountLinkRequired => {
                        Err(AuthError::AccountLinkRequired)
                    },
                    OAuthLoginFinalizeOutcome::LinkedUserUnavailable => Err(AuthError::Internal(
                        "OAuth identity link is unavailable".to_owned(),
                    )),
                    _ => Err(AuthError::Internal(
                        "OAuth login finalizer returned an unsupported outcome".to_owned(),
                    )),
                }
            },
            metrics_emit::oauth_completion_outcome,
        )
        .await
    }
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

#[cfg(test)]
mod oauth_state_error_tests {
    use axum::http::StatusCode;

    use super::*;

    #[test]
    fn admission_pressure_maps_to_http_429_and_rate_limit_metric() {
        for admission in [
            OAuthStateAdmission::AtCapacity,
            OAuthStateAdmission::Contended,
        ] {
            let error = require_oauth_state_admitted(admission)
                .expect_err("admission pressure must fail closed");
            assert!(matches!(error, AuthError::RateLimit));
            assert_eq!(
                oauth_start_outcome(&Err::<(), _>(AuthError::RateLimit)),
                auth_outcome::RATE_LIMIT
            );
            let api_error: crate::ApiError = error.into();
            assert_eq!(
                api_error.to_problem_details().0,
                StatusCode::TOO_MANY_REQUESTS
            );
        }
    }

    #[test]
    fn created_admission_is_the_only_success_outcome() {
        assert!(require_oauth_state_admitted(OAuthStateAdmission::Created).is_ok());
    }

    #[test]
    fn oauth_state_repository_errors_discard_secret_bearing_detail() {
        const STATE_CANARY: &str = "STATE_REPO_ERROR_CANARY_DO_NOT_LOG";
        let error = oauth_state_repo_error(nebula_storage::StorageError::Duplicate {
            entity: "plane_a_oauth_state",
            detail: format!("Key (state)=({STATE_CANARY}) already exists"),
        });

        assert_eq!(
            error.to_string(),
            "internal: OAuth state storage operation failed"
        );
        assert!(!format!("{error:?}").contains(STATE_CANARY));
        let api_error: crate::ApiError = error.into();
        assert!(!api_error.to_string().contains(STATE_CANARY));
        assert!(!format!("{api_error:?}").contains(STATE_CANARY));
    }
}
