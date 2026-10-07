//! Authentication policy over deployment-owned durable account persistence.
//!
//! The composition root selects the storage adapter once. Password, session,
//! MFA and OAuth policy use the same repository contracts for every adapter;
//! database admission and identity-secret convergence precede exposing auth.
//!
//! ## Encoding seams (deliberate divergences)
//!
//! - `UserRow.id` (16 bytes) is the raw ULID payload via
//!   `UserId::as_bytes` / `UserId::from_bytes`.
//! - Presented session cookies are stored only as domain-separated SHA-256
//!   digests. PAT identifiers and OAuth state retain the URL-safe opaque
//!   values minted by their domain helpers.
//! - `users.mfa_secret_envelope` holds a versioned AES-256-GCM envelope
//!   authenticated for the exact user and active-TOTP purpose. Pending
//!   enrollment uses a distinct AAD purpose and is decrypted/re-sealed when
//!   promoted, so ciphertext cannot be copied across lifecycle authorities.
//! - `OAuthStateRow.redirect_uri` persists the exact handler-derived
//!   callback URL and is rechecked before provider egress.
//!
//! ## Transactional flows
//!
//! [`register_user`], [`verify_email`], and [`complete_password_reset`]
//! submit atomic account transitions to [`nebula_storage::auth::AccountLifecycle`].
//! Password hashing, token minting and email delivery remain outside those
//! transactions. This backend does not issue SQL or own transaction boundaries.
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
use nebula_storage::auth::{
    AccountTokenOutcome, AuthPersistence, MfaEnrollmentCandidate, MfaEnrollmentInstallOutcome,
    OAuthLoginFinalizeCommand, OAuthLoginFinalizeOutcome, OAuthLoginFinalized,
    OAuthLoginMfaChallengeDraft, OAuthLoginSessionDraft, OAuthLoginUserDraft, OAuthStateAdmission,
    OAuthStateRow, PasswordRegistration, PersonalAccessTokenRow, SessionDraft, UserRepo, UserRow,
    VerificationTokenRow, identity_secret::TotpSecretPurpose,
};
use rand::Rng;
use sha2::{Digest, Sha256};

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

/// MFA-challenge lifetime, matching the in-memory backend.
const MFA_CHALLENGE_TTL: Duration = Duration::from_mins(5);

/// Email-verification + password-reset token lifetime.
const VERIFICATION_TTL: Duration = Duration::from_hours(1);

/// Minimum password length accepted by [`register_user`] and
/// [`complete_password_reset`].
const MIN_PASSWORD_LEN: usize = 8;

/// `verification_tokens.kind` literal for password-reset tokens.
const KIND_PASSWORD_RESET: &str = "password_reset";

/// Verification-token kind for a local MFA challenge.
const KIND_MFA_CHALLENGE: &str = "mfa_challenge";

/// `personal_access_tokens.principal_kind` literal for human users.
const PRINCIPAL_KIND_USER: &str = "user";

/// Production [`AuthBackend`] using one coherent account persistence set.
///
/// Holds account repositories and atomic transition owners on the same
/// deployment pool, plus the shared [`EmailPort`].
pub struct DurableAuthBackend {
    persistence: AuthPersistence,
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
}

impl DurableAuthBackend {
    /// Bind authentication policy to deployment-owned persistence, email and
    /// optional metrics. Storage supplies the same identity codec used by its
    /// MFA owner. The application must complete storage startup before serving.
    #[must_use]
    pub fn new(
        persistence: AuthPersistence,
        email_port: Arc<dyn EmailPort>,
        metrics: Option<Arc<MetricsRegistry>>,
    ) -> Self {
        Self {
            persistence,
            email_port,
            metrics,
            oauth_runtime: None,
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
            .persistence
            .oauth_states()
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
            .persistence
            .identity_secrets()
            .open_totp_seed(TotpSecretPurpose::Active, &user.id, envelope)
            .map_err(identity_secret_auth_error)?;
        let secret = std::str::from_utf8(&opened.plaintext)
            .map_err(|_| AuthError::Internal("MFA secret encoding is invalid".to_owned()))?;
        let valid = mfa::verify_code(secret, code)?;
        if let Some(replacement) = opened.replacement_envelope.as_deref() {
            self.persistence
                .users()
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

/// Session repository errors can include storage-driver detail. Collapse them
/// before crossing the public API boundary.
fn session_repo_error(_: nebula_storage::StorageError) -> AuthError {
    AuthError::Internal("session storage operation failed".to_owned())
}

fn identity_secret_auth_error(
    _: nebula_storage::auth::identity_secret::IdentitySecretError,
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
async fn fetch_user_by_id(repo: &dyn UserRepo, id: &str) -> Result<UserRow, AuthError> {
    let bytes = user_id_bytes(id)?;
    repo.get(&bytes).await?.ok_or(AuthError::UserNotFound)
}

#[async_trait]
impl AuthBackend for DurableAuthBackend {
    #[tracing::instrument(level = "info", skip(self, session_id))]
    async fn get_principal_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<AuthenticatedSession>, crate::ApiError> {
        let row = self
            .persistence
            .sessions()
            .get(session_id.as_bytes())
            .await
            .map_err(session_repo_error)
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

                self.persistence
                    .accounts()
                    .register_password_user(&PasswordRegistration {
                        user_id: user_bytes.as_slice(),
                        email: &email,
                        display_name,
                        password_hash: &password_hash,
                        verification_hash: &verification_hash,
                        created_at: now,
                        verification_expires_at: expires_at,
                    })
                    .await?;

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
                    .persistence
                    .users()
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
                    self.persistence
                        .users()
                        .record_login_failure(&user.id)
                        .await?;
                    return Err(AuthError::InvalidCredentials);
                }

                // record_login_success ONLY; no `update` call — a profile
                // update would CAS-conflict with concurrent patches and
                // spuriously bump `version` on every login.
                self.persistence
                    .users()
                    .record_login_success(&user.id)
                    .await?;

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
                        self.persistence
                            .verification_tokens()
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
                    .persistence
                    .verification_tokens()
                    .consume_by_hash_and_kind(&challenge_hash, KIND_MFA_CHALLENGE)
                    .await?
                    .ok_or(AuthError::InvalidToken)?;
                let user = self
                    .persistence
                    .users()
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
        if self.persistence.users().get(&user_bytes).await?.is_none() {
            return Err(AuthError::UserNotFound);
        }
        let session_id = session::random_token(32)?;
        let csrf = session::random_token(24)?;
        let now = Utc::now();
        let exp = now + chrono_duration(SESSION_TTL)?;
        self.persistence
            .sessions()
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
        self.persistence
            .sessions()
            .revoke(session_id.as_bytes())
            .await?;
        Ok(())
    }

    #[tracing::instrument(level = "info", skip(self, presented))]
    async fn lookup_pat(&self, presented: &str) -> Result<Option<PatRecord>, AuthError> {
        let hash = pat::hash_for_lookup(presented)?;
        let row = self.persistence.pats().get_by_hash(&hash).await?;
        match row {
            Some(row) => Ok(Some(row_to_pat_record(row)?)),
            None => Ok(None),
        }
    }

    #[tracing::instrument(level = "info", skip(self), fields(user_id))]
    async fn get_user_profile(&self, user_id: &str) -> Result<UserProfile, AuthError> {
        let row = fetch_user_by_id(self.persistence.users(), user_id).await?;
        row_to_profile(&row)
    }

    #[tracing::instrument(level = "info", skip(self, patch), fields(user_id))]
    async fn update_user_profile(
        &self,
        user_id: &str,
        patch: ProfilePatch,
    ) -> Result<UserProfile, AuthError> {
        let mut row = fetch_user_by_id(self.persistence.users(), user_id).await?;
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
        self.persistence
            .users()
            .update(&row, expected_version)
            .await?;
        // Re-fetch so the post-update version + side fields propagate
        // (e.g. for a future caller that reads `version` from the
        // returned profile).
        let refreshed = fetch_user_by_id(self.persistence.users(), user_id).await?;
        tracing::info!(user_id = %user_id, "user profile updated");
        row_to_profile(&refreshed)
    }

    #[tracing::instrument(level = "info", skip(self), fields(user_id))]
    async fn list_pats(&self, user_id: &str) -> Result<Vec<PatRecord>, AuthError> {
        let bytes = user_id_bytes(user_id)?;
        if self.persistence.users().get(&bytes).await?.is_none() {
            return Err(AuthError::UserNotFound);
        }
        let rows = self
            .persistence
            .pats()
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
        if self.persistence.users().get(&bytes).await?.is_none() {
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
        self.persistence
            .pats()
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
        // Ownership and revocation are one storage decision. Repeats succeed
        // for the owner; a foreign token is indistinguishable from a missing one.
        let owned = self
            .persistence
            .pats()
            .revoke_for_principal(pat_id.as_bytes(), PRINCIPAL_KIND_USER, &bytes)
            .await?;
        if !owned {
            return Err(AuthError::UserNotFound);
        }
        tracing::info!(user_id = %user_id, pat_id = %pat_id, "personal access token revoked");
        Ok(())
    }

    #[tracing::instrument(level = "info", skip(self, email))]
    async fn request_password_reset(&self, email: &str) -> Result<(), AuthError> {
        // Enumeration-safe: every internal failure is logged + swallowed.
        let user = match self.persistence.users().get_by_email(email).await {
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
        if let Err(err) = self.persistence.verification_tokens().create(&row).await {
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

                account_token_result(
                    self.persistence
                        .accounts()
                        .reset_password(&token_hash, &new_hash)
                        .await?,
                )
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

                account_token_result(
                    self.persistence
                        .accounts()
                        .verify_email(&token_hash)
                        .await?,
                )
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
                let row = fetch_user_by_id(self.persistence.users(), user_id).await?;
                let (secret, uri) = mfa::mint_secret(&row.email)?;
                let secret_envelope = self
                    .persistence
                    .identity_secrets()
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
                self.persistence
                    .mfa_enrollments()
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
                    .persistence
                    .mfa_enrollments()
                    .get_live_candidate(&user_bytes)
                    .await
                    .map_err(mfa_enrollment_repo_error)?
                    .ok_or(AuthError::InvalidMfaCode)?;
                let opened = self
                    .persistence
                    .identity_secrets()
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
                    .persistence
                    .mfa_enrollments()
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
                    .persistence
                    .oauth_states()
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
                    .persistence
                    .oauth_login()
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
                    .persistence
                    .oauth_login()
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

fn account_token_result(outcome: AccountTokenOutcome) -> Result<(), AuthError> {
    match outcome {
        AccountTokenOutcome::Applied => Ok(()),
        AccountTokenOutcome::InvalidToken => Err(AuthError::InvalidToken),
        AccountTokenOutcome::UserUnavailable => Err(AuthError::UserNotFound),
    }
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
