//! In-memory [`AuthBackend`] implementation.
//!
//! Production-quality crypto (Argon2id passwords, RFC 6238 TOTP, SHA-256
//! PAT lookup) backed by per-process `DashMap` / `parking_lot::RwLock`
//! state. This is the **default backend** for tests and the local-first
//! `simple_server` binary; storage-backed implementations live in a future
//! Sprint-E follow-up that swaps out the storage for `nebula-storage`
//! repos without changing the trait surface.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use nebula_core::{Principal, UserId};
use nebula_metrics::{
    MetricsRegistry,
    naming::{
        NEBULA_API_AUTH_ATTEMPTS_TOTAL, NEBULA_API_AUTH_MFA_ATTEMPTS_TOTAL,
        NEBULA_API_AUTH_OAUTH_ATTEMPTS_TOTAL, auth_outcome,
    },
};
use nebula_storage::{
    repos::OAUTH_STATE_CAPACITY,
    session_token::{SessionTokenDigest, session_token_digest},
};
use parking_lot::Mutex;

use super::{
    dto::{SignupRequest, UserProfile},
    error::AuthError,
    mfa,
    oauth::{OAUTH_STATE_TTL, OAuthProvider, OAuthStateEntry, expiry_unix, mint_pkce},
    password,
    pat::{self, MintedPat, PatRecord, compute_pat_expires_at},
    provider::{
        AuthBackend, AuthenticatedSession, CreatePatParams, MFA_ENROLLMENT_TTL, MfaEnrollment,
        OAuthCompletion, OAuthStart, PasswordOutcome, ProfilePatch, metrics_emit,
    },
    session::{self, SESSION_TTL, SessionRecord, expires_at},
};
use crate::ports::email::{EchoSink, EmailKind, EmailMessage, EmailPort};

/// Threshold for brute-force lockout.
const LOCKOUT_THRESHOLD: i32 = 5;

/// Lockout duration after [`LOCKOUT_THRESHOLD`] failed logins.
const LOCKOUT_TTL: Duration = Duration::from_mins(15);

/// MFA-challenge lifetime — the user must verify within this window.
const MFA_CHALLENGE_TTL: Duration = Duration::from_mins(5);

/// Email verification + password reset token lifetime.
const VERIFICATION_TTL: Duration = Duration::from_hours(1);

#[derive(Clone)]
struct UserRecord {
    id: UserId,
    email: String,
    display_name: String,
    avatar_url: Option<String>,
    password_hash: Option<String>,
    email_verified: bool,
    failed_login_count: i32,
    locked_until: Option<DateTime<Utc>>,
    mfa_secret: Option<String>,
    mfa_enabled: bool,
}

impl UserRecord {
    fn profile(&self) -> UserProfile {
        UserProfile {
            user_id: self.id.to_string(),
            email: self.email.clone(),
            display_name: self.display_name.clone(),
            avatar_url: self.avatar_url.clone(),
            email_verified: self.email_verified,
            mfa_enabled: self.mfa_enabled,
        }
    }
}

#[derive(Clone)]
struct VerificationToken {
    user_id: UserId,
    kind: VerificationKind,
    expires_at: u64,
}

#[derive(Clone, PartialEq, Eq)]
enum VerificationKind {
    EmailVerify,
    PasswordReset,
}

#[derive(Clone)]
struct MfaChallenge {
    user_id: UserId,
    expires_at: u64,
}

struct PendingMfaEnrollment {
    secret_envelope: Vec<u8>,
    expires_at: u64,
}

#[derive(Clone, Copy)]
struct InMemorySession {
    user_id: UserId,
    authenticated_at: DateTime<Utc>,
    expires_at: u64,
}

enum InMemoryOAuthFinalizeOutcome {
    SessionCreated(UserRecord),
    MfaRequired,
    VerifiedEmailRequired,
}

struct PreparedOAuthLogin {
    session_id: String,
    csrf_token: String,
    session_expires_at: DateTime<Utc>,
    session_expires_unix: u64,
    session_authenticated_at: DateTime<Utc>,
    challenge_token: String,
    challenge_expires_unix: u64,
}

/// In-memory [`AuthBackend`].
pub struct InMemoryAuthBackend {
    /// Serializes identity convergence across password registration and
    /// OAuth finalization. DashMap makes individual operations safe, but
    /// the user/email/link/session invariant spans several maps.
    identity_finalize: Mutex<()>,
    users_by_email: DashMap<String, UserId>,
    users: DashMap<UserId, UserRecord>,
    sessions: DashMap<SessionTokenDigest, InMemorySession>,
    pats: DashMap<[u8; 32], PatRecord>,
    verification_tokens: DashMap<String, VerificationToken>,
    mfa_challenges: DashMap<String, MfaChallenge>,
    pending_mfa_enrollments: Mutex<HashMap<UserId, PendingMfaEnrollment>>,
    oauth_state: Mutex<HashMap<String, OAuthStateEntry>>,
    /// In-memory mirror of the PG `external_identities` table per
    /// ADR-0085 D-8. Keyed by `(provider, subject)` to match the PG
    /// PK; value is the linked Nebula `user_id` (16-byte ULID raw
    /// bytes, same shape as `users.id` in PG). `complete_oauth`
    /// consumes this on the REQ-oauth-006
    /// short-circuit + writes to it on first-login / cross-link
    /// branches.
    external_identities: DashMap<(String, String), Vec<u8>>,
    /// Outbound email delivery port. Defaults to a fresh
    /// [`EchoSink`] (see [`Self::default`]) so `Self::new()` keeps the
    /// previous in-process inbox semantics; production composition roots
    /// (and tests that want to assert against a custom transport) inject
    /// a real port via [`Self::with_email_port`].
    email_port: Arc<dyn EmailPort>,
    /// Side handle on the default [`EchoSink`] so [`Self::emails`] can
    /// still snapshot the in-process inbox without a downcast through
    /// `Arc<dyn EmailPort>`. `None` once [`Self::with_email_port`] swaps
    /// in a custom port — callers that need introspection in that mode
    /// must keep their own `EchoSink` reference.
    default_echo: Option<Arc<EchoSink>>,
    /// Optional `nebula_api_auth_*` emission seam (mirror of the
    /// `PgAuthBackend` slot so the trait-level contract stays uniform).
    /// `None` skips emission. Production composition root threads in
    /// the shared `Arc<MetricsRegistry>`; the existing in-memory test
    /// surface keeps `None` to preserve the previous no-emission
    /// semantics.
    metrics: Option<Arc<MetricsRegistry>>,
    /// Opaque Plane-A runtime. `None` means OAuth is disabled safely.
    oauth_runtime: Option<Arc<crate::transport::oauth::OAuthIdentityRuntime>>,
}

impl Default for InMemoryAuthBackend {
    fn default() -> Self {
        let echo = Arc::new(EchoSink::default());
        Self {
            identity_finalize: Mutex::new(()),
            users_by_email: DashMap::default(),
            users: DashMap::default(),
            sessions: DashMap::default(),
            pats: DashMap::default(),
            verification_tokens: DashMap::default(),
            mfa_challenges: DashMap::default(),
            pending_mfa_enrollments: Mutex::new(HashMap::new()),
            oauth_state: Mutex::new(HashMap::new()),
            external_identities: DashMap::default(),
            email_port: Arc::clone(&echo) as Arc<dyn EmailPort>,
            default_echo: Some(echo),
            metrics: None,
            oauth_runtime: None,
        }
    }
}

/// Legacy snapshot shape of an outbound email.
///
/// Kept for backward compatibility with tests that still assert on
/// `EmailEnvelope.token` + `EmailEnvelope.kind` via
/// [`InMemoryAuthBackend::emails`]. New code SHOULD use
/// [`crate::ports::email::EmailMessage`] directly through an
/// [`EchoSink::peek`] call.
#[derive(Clone)]
#[deprecated(
    since = "0.2.0",
    note = "Use crate::ports::email::EmailMessage and EchoSink::peek for new code"
)]
pub struct EmailEnvelope {
    /// Recipient address.
    pub to: String,
    /// Token included in the email link. Mirrors the dev `EchoSink`
    /// convention of putting the raw token in [`EmailMessage::body`].
    pub token: String,
    /// Email category label — [`EmailKind::as_str`] output
    /// (`"EmailVerify"` / `"PasswordReset"`).
    pub kind: &'static str,
}

#[expect(
    deprecated,
    reason = "redaction is part of the deprecated shim's safety contract"
)]
impl std::fmt::Debug for EmailEnvelope {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EmailEnvelope")
            .field("to", &"[redacted]")
            .field("token", &"[redacted]")
            .field("kind", &self.kind)
            .finish()
    }
}

// guard-justified: the `From` impl exists exclusively to feed the
// deprecated `EmailEnvelope` type and cannot itself avoid touching it.
#[expect(deprecated, reason = "shim feeds the deprecated public type")]
impl From<EmailMessage> for EmailEnvelope {
    fn from(m: EmailMessage) -> Self {
        Self {
            to: m.to,
            token: m.body,
            kind: m.kind.as_str(),
        }
    }
}

impl InMemoryAuthBackend {
    /// Construct an empty backend with the default in-process
    /// [`EchoSink`] email port.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Wrap into an `Arc<dyn AuthBackend>` for [`crate::AppState`].
    #[must_use]
    pub fn into_arc(self) -> Arc<dyn AuthBackend> {
        Arc::new(self)
    }

    /// Replace the default [`EchoSink`] with a custom [`EmailPort`].
    ///
    /// Once a custom port is wired, [`Self::emails`] returns an empty
    /// vector (the in-memory inbox is no longer the source of truth);
    /// callers that still need introspection must hold their own handle
    /// to the injected port.
    #[must_use]
    pub fn with_email_port(mut self, port: Arc<dyn EmailPort>) -> Self {
        self.email_port = port;
        self.default_echo = None;
        self
    }

    /// Wire an optional [`MetricsRegistry`] so the backend records
    /// `nebula_api_auth_*` counters / histogram on every outcome
    /// branch. Mirrors the `IdempotencyLayer::with_metrics` precedent
    /// and the constructor injection on `super::pg::PgAuthBackend`
    /// (feature-gated under `postgres`); tests that don't care opt
    /// out by passing `None` (the default).
    #[must_use]
    pub fn with_metrics(mut self, metrics: Option<Arc<MetricsRegistry>>) -> Self {
        self.metrics = metrics;
        self
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

    /// Snapshot the captured outbound emails — used in tests.
    ///
    /// Returns an empty vector when [`Self::with_email_port`] has
    /// swapped the default [`EchoSink`] for a custom transport; the
    /// in-process inbox is only meaningful for the default port.
    // guard-justified: this accessor IS the back-compat shim and must
    // return the deprecated `EmailEnvelope` type by contract.
    #[must_use]
    #[expect(deprecated, reason = "deliberate back-compat shim over EmailEnvelope")]
    pub fn emails(&self) -> Vec<EmailEnvelope> {
        self.default_echo
            .as_ref()
            .map(|s| s.peek())
            .unwrap_or_default()
            .into_iter()
            .map(EmailEnvelope::from)
            .collect()
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn prepare_oauth_login() -> Result<PreparedOAuthLogin, AuthError> {
        let session_id = session::random_token(32)?;
        let csrf_token = session::random_token(24)?;
        let challenge_token = session::random_token(24)?;
        let session_expires_at = expires_at(SESSION_TTL);
        let session_expires_unix = u64::try_from(session_expires_at.timestamp())
            .map_err(|_| AuthError::Internal("invalid OAuth session expiry".to_owned()))?;
        Ok(PreparedOAuthLogin {
            session_id,
            csrf_token,
            session_expires_at,
            session_expires_unix,
            session_authenticated_at: Utc::now(),
            challenge_token,
            challenge_expires_unix: Self::now_secs() + MFA_CHALLENGE_TTL.as_secs(),
        })
    }

    fn lookup_user_by_email(&self, email: &str) -> Option<UserRecord> {
        let key = email.trim().to_lowercase();
        let id = *self.users_by_email.get(&key)?;
        self.users.get(&id).map(|u| u.clone())
    }

    fn put_user(&self, user: UserRecord) {
        // Publish the record before its secondary index. Readers may
        // transiently miss a brand-new user, but can never observe an
        // email index pointing at a record that is not present yet.
        let email = user.email.clone();
        let id = user.id;
        self.users.insert(user.id, user);
        self.users_by_email.insert(email, id);
    }

    fn finalize_oauth_login(
        &self,
        provider: OAuthProvider,
        subject: &str,
        verified_email: Option<&str>,
        candidate_user_id: UserId,
        prepared: &PreparedOAuthLogin,
    ) -> Result<InMemoryOAuthFinalizeOutcome, AuthError> {
        let _identity_guard = self.identity_finalize.lock();
        let link_key = (provider.as_str().to_owned(), subject.to_owned());

        if let Some(linked_user_id) = self
            .external_identities
            .get(&link_key)
            .map(|entry| entry.value().clone())
        {
            let raw_id: [u8; 16] = linked_user_id.as_slice().try_into().map_err(|_| {
                AuthError::Internal("OAuth identity link has an invalid user id".to_owned())
            })?;
            let user_id = UserId::from_bytes(raw_id);
            let user = self
                .users
                .get(&user_id)
                .map(|entry| entry.clone())
                .ok_or_else(|| {
                    AuthError::Internal("OAuth identity link is unavailable".to_owned())
                })?;
            return self.finalize_oauth_artifact(user, prepared);
        }

        let Some(email) = verified_email else {
            return Ok(InMemoryOAuthFinalizeOutcome::VerifiedEmailRequired);
        };
        if self.lookup_user_by_email(email).is_some() {
            return Err(AuthError::AccountLinkRequired);
        }
        let user = UserRecord {
            id: candidate_user_id,
            email: email.to_owned(),
            display_name: email.to_owned(),
            avatar_url: None,
            password_hash: None,
            email_verified: true,
            failed_login_count: 0,
            locked_until: None,
            mfa_secret: None,
            mfa_enabled: false,
        };
        self.put_user(user.clone());

        self.external_identities
            .insert(link_key, user.id.as_bytes().to_vec());
        self.finalize_oauth_artifact(user, prepared)
    }

    fn finalize_oauth_artifact(
        &self,
        user: UserRecord,
        prepared: &PreparedOAuthLogin,
    ) -> Result<InMemoryOAuthFinalizeOutcome, AuthError> {
        if user.mfa_enabled {
            if user.mfa_secret.as_deref().is_none_or(str::is_empty) {
                return Err(AuthError::Internal(
                    "OAuth identity has MFA enabled without a secret".to_owned(),
                ));
            }
            self.mfa_challenges.insert(
                prepared.challenge_token.clone(),
                MfaChallenge {
                    user_id: user.id,
                    expires_at: prepared.challenge_expires_unix,
                },
            );
            return Ok(InMemoryOAuthFinalizeOutcome::MfaRequired);
        }

        self.sessions.insert(
            session_token_digest(prepared.session_id.as_bytes()),
            InMemorySession {
                user_id: user.id,
                authenticated_at: prepared.session_authenticated_at,
                expires_at: prepared.session_expires_unix,
            },
        );
        Ok(InMemoryOAuthFinalizeOutcome::SessionCreated(user))
    }

    fn consume_oauth_state(
        &self,
        provider: OAuthProvider,
        state: &str,
        redirect_uri: &str,
    ) -> Result<OAuthStateEntry, AuthError> {
        // Remove only a live provider-matching entry. A cross-provider
        // callback cannot burn the legitimate transaction.
        let entry = {
            let now = Self::now_secs();
            let mut states = self.oauth_state.lock();
            let matches = states
                .get(state)
                .is_some_and(|entry| entry.expires_at > now && entry.provider == provider);
            if !matches {
                states.retain(|_, entry| entry.expires_at > now);
                return Err(AuthError::InvalidToken);
            }
            states.remove(state).ok_or(AuthError::InvalidToken)?
        };
        if entry.redirect_uri != redirect_uri {
            return Err(AuthError::from_oauth_failure(
                crate::transport::oauth::OAuthFailureCode::RedirectUriMismatch,
            ));
        }
        Ok(entry)
    }

    fn issue_verification_token(
        &self,
        user_id: UserId,
        kind: VerificationKind,
    ) -> Result<String, AuthError> {
        let token = session::random_token(24)?;
        self.verification_tokens.insert(
            token.clone(),
            VerificationToken {
                user_id,
                kind,
                expires_at: Self::now_secs() + VERIFICATION_TTL.as_secs(),
            },
        );
        Ok(token)
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, token),
        fields(email.kind = ?kind, to_len = to.len())
    )]
    async fn record_email(&self, to: &str, token: &str, kind: EmailKind) -> Result<(), AuthError> {
        debug_assert!(
            !to.is_empty() && to.contains('@'),
            "record_email: recipient address must be a non-empty email-shaped string",
        );
        debug_assert!(!token.is_empty(), "record_email: token must not be empty");
        let subject = match kind {
            EmailKind::Verification => "Verify your email",
            EmailKind::PasswordReset => "Reset your password",
            EmailKind::Generic => "Notification",
        }
        .to_owned();
        let msg = EmailMessage {
            to: to.to_owned(),
            subject,
            // Dev convention: the body is the raw token so tests can
            // pull it back out via `EchoSink::peek` / `Self::emails`.
            // Production transports will replace the `EmailPort` impl
            // with one that renders a real template.
            body: token.to_owned(),
            kind,
        };
        self.email_port.send(msg).await?;
        Ok(())
    }
}

#[async_trait]
impl AuthBackend for InMemoryAuthBackend {
    async fn get_principal_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<AuthenticatedSession>, crate::ApiError> {
        let now = Self::now_secs();
        let digest = session_token_digest(session_id.as_bytes());
        if let Some(entry) = self.sessions.get(&digest) {
            let session = *entry;
            drop(entry);
            if session.expires_at <= now {
                self.sessions.remove(&digest);
                return Ok(None);
            }
            return Ok(Some(AuthenticatedSession {
                principal: Principal::User(session.user_id),
                authenticated_at: session.authenticated_at,
            }));
        }
        Ok(None)
    }

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
                if req.password.len() < 8 {
                    return Err(AuthError::InvalidCredentials);
                }
                let display_name = req.display_name.trim();
                if display_name.is_empty() || display_name.len() > 128 {
                    return Err(AuthError::InvalidCredentials);
                }
                if self.users_by_email.contains_key(&email) {
                    return Err(AuthError::EmailAlreadyRegistered);
                }
                let hash = password::hash_password(req.password.expose())?;
                let id = UserId::new();
                let record = UserRecord {
                    id,
                    email: email.clone(),
                    display_name: display_name.to_owned(),
                    avatar_url: None,
                    password_hash: Some(hash),
                    email_verified: false,
                    failed_login_count: 0,
                    locked_until: None,
                    mfa_secret: None,
                    mfa_enabled: false,
                };
                let profile = record.profile();
                {
                    let _identity_guard = self.identity_finalize.lock();
                    // The optimistic check above avoids an unnecessary
                    // password hash in the common duplicate case. This
                    // locked recheck is authoritative against concurrent
                    // signup and OAuth account-creation paths.
                    if self.users_by_email.contains_key(&email) {
                        return Err(AuthError::EmailAlreadyRegistered);
                    }
                    self.put_user(record);
                }

                // Signup deliberately commits the user record before queueing
                // the verification email. If email dispatch fails the user
                // still exists in an unverified state and can recover via
                // `request_password_reset` — the reset flow does not require
                // `email_verified` to be set to issue its cooldown-bounded
                // reset token. Rolling back the user on transient transport
                // failure would silently destroy the durable account on every
                // retry and is the wrong default for a bring-up backend.
                let token = self.issue_verification_token(id, VerificationKind::EmailVerify)?;
                self.record_email(&email, &token, EmailKind::Verification)
                    .await?;

                tracing::info!(user_id = %id, "user registered");
                Ok(profile)
            },
            |result| match result {
                Ok(_) => auth_outcome::SUCCESS,
                Err(AuthError::EmailAlreadyRegistered) => auth_outcome::CONFLICT,
                Err(AuthError::InvalidCredentials) => auth_outcome::INVALID_CREDS,
                Err(_) => auth_outcome::INTERNAL,
            },
        )
        .await
    }

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
                    .lookup_user_by_email(email)
                    .ok_or(AuthError::InvalidCredentials)?;

                if let Some(until) = user.locked_until
                    && until > Utc::now()
                {
                    return Err(AuthError::AccountLocked);
                }

                let stored_hash = user
                    .password_hash
                    .as_ref()
                    .ok_or(AuthError::InvalidCredentials)?;
                if !password::verify_password(stored_hash, password_input)? {
                    self.users.alter(&user.id, |_, mut u| {
                        u.failed_login_count += 1;
                        if u.failed_login_count >= LOCKOUT_THRESHOLD {
                            let until = Utc::now()
                                + chrono::Duration::from_std(LOCKOUT_TTL).unwrap_or_default();
                            u.locked_until = Some(until);
                        }
                        u
                    });
                    return Err(AuthError::InvalidCredentials);
                }

                // Reset failure counter on success.
                self.users.alter(&user.id, |_, mut u| {
                    u.failed_login_count = 0;
                    u.locked_until = None;
                    u
                });

                if user.mfa_enabled {
                    if let Some(code) = totp {
                        let secret = user.mfa_secret.as_deref().ok_or_else(|| {
                            AuthError::Internal("mfa enabled without secret".to_owned())
                        })?;
                        if !mfa::verify_code(secret, code)? {
                            return Err(AuthError::InvalidMfaCode);
                        }
                        Ok(PasswordOutcome::Authenticated(user.profile()))
                    } else {
                        let challenge_token = session::random_token(24)?;
                        self.mfa_challenges.insert(
                            challenge_token.clone(),
                            MfaChallenge {
                                user_id: user.id,
                                expires_at: Self::now_secs() + MFA_CHALLENGE_TTL.as_secs(),
                            },
                        );
                        Ok(PasswordOutcome::MfaRequired { challenge_token })
                    }
                } else {
                    Ok(PasswordOutcome::Authenticated(user.profile()))
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
                let now = Self::now_secs();
                let entry = self
                    .mfa_challenges
                    .remove(challenge_token)
                    .ok_or(AuthError::InvalidToken)?
                    .1;
                if entry.expires_at <= now {
                    return Err(AuthError::InvalidToken);
                }
                let user = self
                    .users
                    .get(&entry.user_id)
                    .ok_or(AuthError::UserNotFound)?
                    .clone();
                let secret = user.mfa_secret.as_deref().ok_or_else(|| {
                    AuthError::Internal("mfa challenge for non-mfa user".to_owned())
                })?;
                if !mfa::verify_code(secret, code)? {
                    return Err(AuthError::InvalidMfaCode);
                }
                Ok(user.profile())
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

    async fn create_session(&self, user_id: &str) -> Result<SessionRecord, AuthError> {
        let id = session::random_token(32)?;
        let csrf = session::random_token(24)?;
        let exp = Self::now_secs() + SESSION_TTL.as_secs();

        // Resolve UserId from string form for the principal.
        let parsed: UserId = user_id
            .parse()
            .map_err(|_| AuthError::Internal("invalid user_id".to_owned()))?;
        if !self.users.contains_key(&parsed) {
            return Err(AuthError::UserNotFound);
        }
        self.sessions.insert(
            session_token_digest(id.as_bytes()),
            InMemorySession {
                user_id: parsed,
                authenticated_at: Utc::now(),
                expires_at: exp,
            },
        );

        Ok(SessionRecord {
            id,
            principal: Principal::User(parsed),
            csrf_token: csrf,
            expires_at: expires_at(SESSION_TTL),
        })
    }

    async fn revoke_session(&self, session_id: &str) -> Result<(), AuthError> {
        self.sessions
            .remove(&session_token_digest(session_id.as_bytes()));
        Ok(())
    }

    async fn lookup_pat(&self, presented: &str) -> Result<Option<PatRecord>, AuthError> {
        let hash = pat::hash_for_lookup(presented)?;
        let now = Utc::now();
        if let Some(entry) = self.pats.get(&hash) {
            let record = entry.clone();
            drop(entry);
            if !record.is_active(now) {
                return Ok(None);
            }
            return Ok(Some(record));
        }
        Ok(None)
    }

    async fn get_user_profile(&self, user_id: &str) -> Result<UserProfile, AuthError> {
        let parsed: UserId = user_id.parse().map_err(|_| AuthError::UserNotFound)?;
        let profile = self
            .users
            .get(&parsed)
            .ok_or(AuthError::UserNotFound)?
            .profile();
        Ok(profile)
    }

    async fn update_user_profile(
        &self,
        user_id: &str,
        patch: ProfilePatch,
    ) -> Result<UserProfile, AuthError> {
        let parsed: UserId = user_id.parse().map_err(|_| AuthError::UserNotFound)?;
        if !self.users.contains_key(&parsed) {
            return Err(AuthError::UserNotFound);
        }
        // Validate before mutating so a rejected patch leaves state intact.
        if let Some(name) = patch.display_name.as_deref() {
            let trimmed = name.trim();
            if trimmed.is_empty() || trimmed.len() > 128 {
                return Err(AuthError::InvalidInput(
                    "display_name must be 1..=128 non-blank characters",
                ));
            }
        }
        self.users.alter(&parsed, |_, mut u| {
            if let Some(name) = patch.display_name.as_deref() {
                name.trim().clone_into(&mut u.display_name);
            }
            if let Some(avatar) = patch.avatar_url.as_deref() {
                let trimmed = avatar.trim();
                u.avatar_url = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                };
            }
            u
        });
        let profile = self
            .users
            .get(&parsed)
            .ok_or(AuthError::UserNotFound)?
            .profile();
        tracing::info!(user_id = %parsed, "user profile updated");
        Ok(profile)
    }

    async fn list_pats(&self, user_id: &str) -> Result<Vec<PatRecord>, AuthError> {
        let parsed: UserId = user_id.parse().map_err(|_| AuthError::UserNotFound)?;
        if !self.users.contains_key(&parsed) {
            return Err(AuthError::UserNotFound);
        }
        let now = Utc::now();
        let mut out: Vec<PatRecord> = self
            .pats
            .iter()
            .filter(|e| e.user_id == parsed && e.is_active(now))
            .map(|e| e.clone())
            .collect();
        // Stable, deterministic order (newest first) so list responses and
        // the e2e assertions don't depend on DashMap shard iteration order.
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    async fn create_pat(
        &self,
        user_id: &str,
        params: CreatePatParams,
    ) -> Result<MintedPat, AuthError> {
        let parsed: UserId = user_id.parse().map_err(|_| AuthError::UserNotFound)?;
        if !self.users.contains_key(&parsed) {
            return Err(AuthError::UserNotFound);
        }
        let name = params.name.trim();
        if name.is_empty() || name.len() > 128 {
            return Err(AuthError::InvalidInput(
                "token name must be 1..=128 non-blank characters",
            ));
        }
        let expires_at = compute_pat_expires_at(params.ttl_seconds)?;
        let minted = pat::mint_pat(parsed, name.to_owned(), params.scopes, expires_at)?;
        self.pats.insert(minted.record.hash, minted.record.clone());
        tracing::info!(user_id = %parsed, pat_id = %minted.record.id, "personal access token created");
        Ok(minted)
    }

    async fn revoke_pat(&self, user_id: &str, pat_id: &str) -> Result<(), AuthError> {
        let parsed: UserId = user_id.parse().map_err(|_| AuthError::UserNotFound)?;
        // Find the hash key whose record id matches AND is owned by the
        // caller. A token owned by a different principal is reported as
        // not-found (no cross-user existence disclosure).
        let key = self
            .pats
            .iter()
            .find(|e| e.id == pat_id && e.user_id == parsed)
            .map(|e| *e.key());
        let Some(key) = key else {
            return Err(AuthError::UserNotFound);
        };
        let mut entry = self.pats.get_mut(&key).ok_or(AuthError::UserNotFound)?;
        if entry.revoked_at.is_none() {
            entry.revoked_at = Some(Utc::now());
        }
        drop(entry);
        tracing::info!(user_id = %parsed, pat_id = %pat_id, "personal access token revoked");
        Ok(())
    }

    async fn request_password_reset(&self, email: &str) -> Result<(), AuthError> {
        if let Some(user) = self.lookup_user_by_email(email) {
            match self.issue_verification_token(user.id, VerificationKind::PasswordReset) {
                Ok(token) => {
                    if let Err(err) = self
                        .record_email(&user.email, &token, EmailKind::PasswordReset)
                        .await
                    {
                        // Enumeration-safe: never surface delivery failures.
                        // Logged so operators can correlate transport faults.
                        tracing::error!(
                            error = %err,
                            user_id = %user.id,
                            "failed to dispatch password reset email",
                        );
                    }
                },
                Err(err) => {
                    // Do not surface token-mint failures to the caller (enumeration-safe),
                    // but never fall back to a predictable token.
                    tracing::error!(
                        error = %err,
                        user_id = %user.id,
                        "failed to mint password reset token",
                    );
                },
            }
        }
        // Always return Ok to avoid account-existence enumeration.
        Ok(())
    }

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
                let entry = self
                    .verification_tokens
                    .remove(token)
                    .ok_or(AuthError::InvalidToken)?
                    .1;
                if entry.kind != VerificationKind::PasswordReset {
                    return Err(AuthError::InvalidToken);
                }
                if entry.expires_at <= Self::now_secs() {
                    return Err(AuthError::InvalidToken);
                }
                if new_password.len() < 8 {
                    return Err(AuthError::InvalidCredentials);
                }
                let new_hash = password::hash_password(new_password)?;
                self.users.alter(&entry.user_id, |_, mut u| {
                    u.password_hash = Some(new_hash.clone());
                    u.failed_login_count = 0;
                    u.locked_until = None;
                    u
                });
                Ok(())
            },
            |result| match result {
                Ok(()) => auth_outcome::SUCCESS,
                Err(AuthError::InvalidToken) => auth_outcome::TOKEN_INVALID,
                // Per oracle per-method map: `complete_password_reset`
                // collapses `InvalidCredentials` to `invalid_input`
                // because the failure is shape-validation of
                // `new_password` (short / blank).
                Err(AuthError::InvalidCredentials) => auth_outcome::INVALID_INPUT,
                Err(_) => auth_outcome::INTERNAL,
            },
        )
        .await
    }

    async fn verify_email(&self, token: &str) -> Result<(), AuthError> {
        metrics_emit::run_with_metrics(
            &self.metrics,
            NEBULA_API_AUTH_ATTEMPTS_TOTAL,
            None,
            async move {
                let entry = self
                    .verification_tokens
                    .remove(token)
                    .ok_or(AuthError::InvalidToken)?
                    .1;
                if entry.kind != VerificationKind::EmailVerify {
                    return Err(AuthError::InvalidToken);
                }
                if entry.expires_at <= Self::now_secs() {
                    return Err(AuthError::InvalidToken);
                }
                self.users.alter(&entry.user_id, |_, mut u| {
                    u.email_verified = true;
                    u
                });
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

    async fn start_mfa_enrollment(&self, user_id: &str) -> Result<MfaEnrollment, AuthError> {
        metrics_emit::run_with_metrics(
            &self.metrics,
            NEBULA_API_AUTH_MFA_ATTEMPTS_TOTAL,
            None,
            async move {
                let parsed: UserId = user_id
                    .parse()
                    .map_err(|_| AuthError::Internal("invalid user_id".to_owned()))?;
                let user_email = self
                    .users
                    .get(&parsed)
                    .ok_or(AuthError::UserNotFound)?
                    .email
                    .clone();
                let (secret, uri) = mfa::mint_secret(&user_email)?;
                self.pending_mfa_enrollments.lock().insert(
                    parsed,
                    PendingMfaEnrollment {
                        secret_envelope: secret.as_bytes().to_vec(),
                        expires_at: Self::now_secs() + MFA_ENROLLMENT_TTL.as_secs(),
                    },
                );
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

    async fn confirm_mfa_enrollment(&self, user_id: &str, code: &str) -> Result<(), AuthError> {
        metrics_emit::run_with_metrics(
            &self.metrics,
            NEBULA_API_AUTH_MFA_ATTEMPTS_TOTAL,
            None,
            async move {
                let parsed: UserId = user_id
                    .parse()
                    .map_err(|_| AuthError::Internal("invalid user_id".to_owned()))?;
                let mut candidates = self.pending_mfa_enrollments.lock();
                let Some(candidate) = candidates.get(&parsed) else {
                    return Err(AuthError::InvalidMfaCode);
                };
                if candidate.expires_at <= Self::now_secs() {
                    candidates.remove(&parsed);
                    return Err(AuthError::InvalidMfaCode);
                }
                let secret = std::str::from_utf8(&candidate.secret_envelope)
                    .map_err(|_| AuthError::Internal("invalid MFA secret envelope".to_owned()))?;
                if !mfa::verify_code(secret, code)? {
                    return Err(AuthError::InvalidMfaCode);
                }
                let mut user = self.users.get_mut(&parsed).ok_or(AuthError::UserNotFound)?;
                let candidate = candidates
                    .remove(&parsed)
                    .ok_or(AuthError::InvalidMfaCode)?;
                let installed_secret = String::from_utf8(candidate.secret_envelope)
                    .map_err(|_| AuthError::Internal("invalid MFA secret envelope".to_owned()))?;
                user.mfa_secret = Some(installed_secret);
                user.mfa_enabled = true;
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

    // Resolve the provider under the fixed runtime policy, then persist the
    // PKCE state and exact callback URL in the bounded in-memory store.
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
                {
                    let now = Self::now_secs();
                    let mut states = self.oauth_state.lock();
                    states.retain(|_, entry| entry.expires_at > now);
                    if states.len() >= OAUTH_STATE_CAPACITY as usize {
                        return Err(AuthError::RateLimit);
                    }
                }
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

                let mut states = self.oauth_state.lock();
                states.retain(|_, entry| entry.expires_at > Self::now_secs());
                if states.len() >= OAUTH_STATE_CAPACITY as usize {
                    return Err(AuthError::RateLimit);
                }
                states.insert(
                    pkce.state.clone(),
                    OAuthStateEntry {
                        provider,
                        code_verifier: pkce.code_verifier,
                        expires_at: expiry_unix(OAUTH_STATE_TTL),
                        redirect_uri,
                    },
                );
                Ok(OAuthStart {
                    authorize_url,
                    state: pkce.state,
                })
            },
            |result| match result {
                Ok(_) => auth_outcome::SUCCESS,
                // Deployment-state and upstream OAuth failures share the
                // closed OAuth-failed metric label.
                Err(AuthError::OAuthFailed | AuthError::ProviderNotConfigured) => {
                    auth_outcome::OAUTH_FAILED
                },
                Err(AuthError::RateLimit) => auth_outcome::RATE_LIMIT,
                Err(_) => auth_outcome::INTERNAL,
            },
        )
        .await
    }

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
                self.consume_oauth_state(provider, state, redirect_uri)?;
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
    // mirror the PG implementation; only persistence differs.
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
                let entry = self.consume_oauth_state(provider, &state, &redirect_uri)?;
                let deadline = runtime.begin_deadline();
                let pending = runtime
                    .begin_identity_completion(
                        deadline,
                        provider,
                        &state,
                        &code,
                        &redirect_uri,
                        &entry.code_verifier,
                    )
                    .await
                    .map_err(AuthError::from_oauth_failure)?;
                let sub = pending.subject().to_owned();
                // Prepare independent session and MFA-continuation material
                // before entering the short identity critical section. The
                // mutex is never held across provider egress.
                let prepared = Self::prepare_oauth_login()?;

                // Existing subject links are the source of truth and do not
                // require another verified-email fetch. Exactly one local
                // authority artifact is created in the same critical section
                // as link resolution.
                let outcome =
                    self.finalize_oauth_login(provider, &sub, None, UserId::new(), &prepared)?;
                match outcome {
                    InMemoryOAuthFinalizeOutcome::SessionCreated(record) => {
                        drop(pending);
                        return Ok(OAuthCompletion::SessionCreated {
                            user: record.profile(),
                            session: SessionRecord {
                                id: prepared.session_id,
                                principal: Principal::User(record.id),
                                csrf_token: prepared.csrf_token,
                                expires_at: prepared.session_expires_at,
                            },
                        });
                    },
                    InMemoryOAuthFinalizeOutcome::MfaRequired => {
                        drop(pending);
                        return Ok(OAuthCompletion::MfaRequired {
                            challenge_token: prepared.challenge_token,
                        });
                    },
                    InMemoryOAuthFinalizeOutcome::VerifiedEmailRequired => drop(prepared),
                }

                // No subject link exists. Fetch provider-attested email,
                // then re-enter the same finalizer; it rechecks both link
                // and email under the lock to converge concurrent callbacks.
                let resolved_email = runtime
                    .resolve_verified_identity(pending)
                    .await
                    .map_err(AuthError::from_oauth_failure)?
                    .into_string();
                if resolved_email.is_empty() {
                    return Err(AuthError::EmailNotVerified);
                }
                let prepared = Self::prepare_oauth_login()?;
                match self.finalize_oauth_login(
                    provider,
                    &sub,
                    Some(&resolved_email),
                    UserId::new(),
                    &prepared,
                )? {
                    InMemoryOAuthFinalizeOutcome::SessionCreated(record) => {
                        Ok(OAuthCompletion::SessionCreated {
                            user: record.profile(),
                            session: SessionRecord {
                                id: prepared.session_id,
                                principal: Principal::User(record.id),
                                csrf_token: prepared.csrf_token,
                                expires_at: prepared.session_expires_at,
                            },
                        })
                    },
                    InMemoryOAuthFinalizeOutcome::MfaRequired => Ok(OAuthCompletion::MfaRequired {
                        challenge_token: prepared.challenge_token,
                    }),
                    InMemoryOAuthFinalizeOutcome::VerifiedEmailRequired => Err(
                        AuthError::Internal("OAuth email finalization was incomplete".to_owned()),
                    ),
                }
            },
            metrics_emit::oauth_completion_outcome,
        )
        .await
    }
}

// guard-justified: the tests below intentionally exercise the
// deprecated `EmailEnvelope` shim and `emails()` accessor.
#[cfg(test)]
#[expect(
    deprecated,
    reason = "tests still assert on the deprecated `EmailEnvelope` back-compat shim"
)]
mod tests {
    use super::*;
    use crate::domain::auth::backend::dto::SecretString;

    fn signup_req(email: &str) -> SignupRequest {
        SignupRequest {
            email: email.to_owned(),
            password: SecretString::new("hunter22".to_owned()),
            display_name: "Test User".to_owned(),
        }
    }

    async fn enroll_mfa_user(backend: &InMemoryAuthBackend, email: &str) -> (UserProfile, String) {
        let profile = backend
            .register_user(signup_req(email))
            .await
            .expect("seed MFA user");
        let enrollment = backend
            .start_mfa_enrollment(&profile.user_id)
            .await
            .expect("start MFA enrollment");
        let code = mfa::current_code(&enrollment.secret_base32).expect("current MFA code");
        backend
            .confirm_mfa_enrollment(&profile.user_id, &code)
            .await
            .expect("confirm MFA enrollment");
        (profile, enrollment.secret_base32)
    }

    fn oauth_material(
        session_id: &str,
        challenge_token: &str,
        challenge_expires_unix: u64,
    ) -> PreparedOAuthLogin {
        PreparedOAuthLogin {
            session_id: session_id.to_owned(),
            csrf_token: "test-csrf".to_owned(),
            session_expires_at: Utc::now(),
            session_expires_unix: u64::MAX,
            session_authenticated_at: Utc::now(),
            challenge_token: challenge_token.to_owned(),
            challenge_expires_unix,
        }
    }

    fn backend_with_oauth(providers: crate::config::OAuthProvidersConfig) -> InMemoryAuthBackend {
        let runtime = Arc::new(
            crate::OAuthIdentityRuntime::from_config(providers)
                .expect("test OAuth runtime must build")
                .expect("test OAuth provider set must enable the runtime"),
        );
        InMemoryAuthBackend::new().with_oauth_runtime(runtime)
    }

    #[tokio::test]
    async fn register_then_login_returns_authenticated() {
        let b = InMemoryAuthBackend::new();
        let profile = b
            .register_user(signup_req("alice@nebula.dev"))
            .await
            .unwrap();
        assert_eq!(profile.email, "alice@nebula.dev");
        assert!(!profile.email_verified);
        assert!(!profile.mfa_enabled);

        let outcome = b
            .authenticate_password("alice@nebula.dev", "hunter22", None)
            .await
            .unwrap();
        match outcome {
            PasswordOutcome::Authenticated(p) => assert_eq!(p.user_id, profile.user_id),
            PasswordOutcome::MfaRequired { .. } => panic!("MFA not enabled"),
        }
    }

    #[tokio::test]
    async fn signup_emits_verification_email() {
        let b = InMemoryAuthBackend::new();
        b.register_user(signup_req("a@b.c")).await.unwrap();
        let emails = b.emails();
        assert_eq!(emails.len(), 1);
        assert_eq!(emails[0].kind, "EmailVerify");
        assert_eq!(emails[0].to, "a@b.c");
    }

    #[test]
    fn legacy_email_envelope_debug_redacts_token_and_recipient() {
        const CANARY: &str = "LEGACY_EMAIL_AUTHORITY_CANARY-e9b2";
        let envelope = EmailEnvelope {
            to: format!("{CANARY}@example.test"),
            token: CANARY.to_owned(),
            kind: "PasswordReset",
        };

        let debug = format!("{envelope:?}");
        assert!(!debug.contains(CANARY));
        assert!(debug.contains("PasswordReset"));
    }

    #[tokio::test]
    async fn login_with_wrong_password_is_invalid_credentials() {
        let b = InMemoryAuthBackend::new();
        b.register_user(signup_req("c@d.e")).await.unwrap();
        let err = b
            .authenticate_password("c@d.e", "wrong", None)
            .await
            .unwrap_err();
        assert!(matches!(err, AuthError::InvalidCredentials));
    }

    #[tokio::test]
    async fn five_failures_lock_account() {
        let b = InMemoryAuthBackend::new();
        b.register_user(signup_req("locked@e.f")).await.unwrap();
        for _ in 0..5 {
            let _ = b.authenticate_password("locked@e.f", "wrong", None).await;
        }
        let err = b
            .authenticate_password("locked@e.f", "hunter22", None)
            .await
            .unwrap_err();
        assert!(matches!(err, AuthError::AccountLocked));
    }

    #[tokio::test]
    async fn duplicate_signup_conflicts() {
        let b = InMemoryAuthBackend::new();
        b.register_user(signup_req("dup@e.f")).await.unwrap();
        let err = b.register_user(signup_req("dup@e.f")).await.unwrap_err();
        assert!(matches!(err, AuthError::EmailAlreadyRegistered));
    }

    #[tokio::test]
    async fn create_session_then_resolve_principal() {
        let b = InMemoryAuthBackend::new();
        let profile = b.register_user(signup_req("s@e.f")).await.unwrap();
        let session = b.create_session(&profile.user_id).await.unwrap();
        let session_auth = b
            .get_principal_by_session(&session.id)
            .await
            .unwrap()
            .expect("session is live");
        assert!(matches!(session_auth.principal, Principal::User(_)));
        assert!(session_auth.authenticated_at <= Utc::now());
    }

    #[tokio::test]
    async fn revoke_session_clears_lookup() {
        let b = InMemoryAuthBackend::new();
        let profile = b.register_user(signup_req("r@e.f")).await.unwrap();
        let session = b.create_session(&profile.user_id).await.unwrap();
        b.revoke_session(&session.id).await.unwrap();
        let resolved = b.get_principal_by_session(&session.id).await.unwrap();
        assert!(resolved.is_none());
    }

    #[tokio::test]
    async fn email_verification_flips_flag() {
        let b = InMemoryAuthBackend::new();
        b.register_user(signup_req("v@e.f")).await.unwrap();
        let token = b.emails()[0].token.clone();
        b.verify_email(&token).await.unwrap();
        let user = b.lookup_user_by_email("v@e.f").unwrap();
        assert!(user.email_verified);
        // Replay rejected.
        let err = b.verify_email(&token).await.unwrap_err();
        assert!(matches!(err, AuthError::InvalidToken));
    }

    #[tokio::test]
    async fn password_reset_round_trips() {
        let b = InMemoryAuthBackend::new();
        b.register_user(signup_req("p@e.f")).await.unwrap();
        // Drain the verification email so we only see the reset email next.
        b.default_echo
            .as_ref()
            .expect("default echo sink is wired when no custom port is injected")
            .drain();
        b.request_password_reset("p@e.f").await.unwrap();
        let token = b.emails()[0].token.clone();
        b.complete_password_reset(&token, "newpass1").await.unwrap();
        let outcome = b
            .authenticate_password("p@e.f", "newpass1", None)
            .await
            .unwrap();
        assert!(matches!(outcome, PasswordOutcome::Authenticated(_)));
    }

    #[tokio::test]
    async fn mfa_enrollment_then_login_with_code() {
        let b = InMemoryAuthBackend::new();
        let profile = b.register_user(signup_req("m@e.f")).await.unwrap();
        let enrol = b.start_mfa_enrollment(&profile.user_id).await.unwrap();
        let code = mfa::current_code(&enrol.secret_base32).unwrap();
        b.confirm_mfa_enrollment(&profile.user_id, &code)
            .await
            .unwrap();

        let login_no_code = b
            .authenticate_password("m@e.f", "hunter22", None)
            .await
            .unwrap();
        let challenge = match login_no_code {
            PasswordOutcome::MfaRequired { challenge_token } => challenge_token,
            PasswordOutcome::Authenticated(_) => panic!("MFA should be required"),
        };
        let new_code = mfa::current_code(&enrol.secret_base32).unwrap();
        let final_profile = b.verify_mfa(&challenge, &new_code).await.unwrap();
        assert_eq!(final_profile.user_id, profile.user_id);
    }

    #[tokio::test]
    async fn starting_reenrollment_preserves_the_active_factor_until_confirmation() {
        let b = InMemoryAuthBackend::new();
        let (profile, active_secret) = enroll_mfa_user(&b, "mfa-reenroll@e.f").await;

        let candidate = b
            .start_mfa_enrollment(&profile.user_id)
            .await
            .expect("start replacement MFA enrollment");
        assert_ne!(candidate.secret_base32, active_secret);

        let challenge_token = match b
            .authenticate_password("mfa-reenroll@e.f", "hunter22", None)
            .await
            .expect("active MFA must still gate password login")
        {
            PasswordOutcome::MfaRequired { challenge_token } => challenge_token,
            PasswordOutcome::Authenticated(_) => {
                panic!("starting a replacement must not disable active MFA")
            },
        };
        let active_code = mfa::current_code(&active_secret).expect("active MFA code");
        b.verify_mfa(&challenge_token, &active_code)
            .await
            .expect("the active factor must survive an abandoned enrollment");

        let candidate_code =
            mfa::current_code(&candidate.secret_base32).expect("replacement candidate MFA code");
        let wrong_candidate_code = format!(
            "{:06}",
            (candidate_code.parse::<u32>().expect("numeric TOTP") + 1) % 1_000_000
        );
        let error = b
            .confirm_mfa_enrollment(&profile.user_id, &wrong_candidate_code)
            .await
            .expect_err("wrong replacement code must reject");
        assert!(matches!(error, AuthError::InvalidMfaCode));

        let second_challenge = match b
            .authenticate_password("mfa-reenroll@e.f", "hunter22", None)
            .await
            .expect("failed replacement must preserve active MFA")
        {
            PasswordOutcome::MfaRequired { challenge_token } => challenge_token,
            PasswordOutcome::Authenticated(_) => {
                panic!("failed replacement must not disable active MFA")
            },
        };
        let active_code = mfa::current_code(&active_secret).expect("active MFA code");
        b.verify_mfa(&second_challenge, &active_code)
            .await
            .expect("the original factor must remain authoritative");
    }

    #[tokio::test]
    async fn confirmed_enrollment_is_single_use() {
        let b = InMemoryAuthBackend::new();
        let profile = b
            .register_user(signup_req("mfa-single-use@e.f"))
            .await
            .expect("register user");
        let candidate = b
            .start_mfa_enrollment(&profile.user_id)
            .await
            .expect("start MFA enrollment");
        let code = mfa::current_code(&candidate.secret_base32).expect("candidate MFA code");

        b.confirm_mfa_enrollment(&profile.user_id, &code)
            .await
            .expect("first confirmation installs candidate");
        let replay = b
            .confirm_mfa_enrollment(&profile.user_id, &code)
            .await
            .expect_err("confirmed candidate must not be replayable");
        assert!(matches!(replay, AuthError::InvalidMfaCode));
    }

    #[tokio::test]
    async fn concurrent_enrollment_confirmation_has_exactly_one_winner() {
        let b = Arc::new(InMemoryAuthBackend::new());
        let profile = b
            .register_user(signup_req("mfa-concurrent@e.f"))
            .await
            .expect("register user");
        let candidate = b
            .start_mfa_enrollment(&profile.user_id)
            .await
            .expect("start MFA enrollment");
        let code = mfa::current_code(&candidate.secret_base32).expect("candidate MFA code");

        let left = {
            let b = Arc::clone(&b);
            let user_id = profile.user_id.clone();
            let code = code.clone();
            tokio::spawn(async move { b.confirm_mfa_enrollment(&user_id, &code).await })
        };
        let right = {
            let b = Arc::clone(&b);
            let user_id = profile.user_id.clone();
            tokio::spawn(async move { b.confirm_mfa_enrollment(&user_id, &code).await })
        };
        let outcomes = [
            left.await.expect("left join"),
            right.await.expect("right join"),
        ];

        assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            outcomes
                .iter()
                .filter(|result| matches!(result, Err(AuthError::InvalidMfaCode)))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn expired_enrollment_candidate_cannot_be_installed() {
        let b = InMemoryAuthBackend::new();
        let profile = b
            .register_user(signup_req("mfa-expired@e.f"))
            .await
            .expect("register user");
        let candidate = b
            .start_mfa_enrollment(&profile.user_id)
            .await
            .expect("start MFA enrollment");
        let user_id: UserId = profile.user_id.parse().expect("valid user id");
        b.pending_mfa_enrollments
            .lock()
            .get_mut(&user_id)
            .expect("pending candidate")
            .expires_at = 0;
        let code = mfa::current_code(&candidate.secret_base32).expect("candidate MFA code");

        let error = b
            .confirm_mfa_enrollment(&profile.user_id, &code)
            .await
            .expect_err("expired candidate must reject");
        assert!(matches!(error, AuthError::InvalidMfaCode));
        assert!(
            !b.get_user_profile(&profile.user_id)
                .await
                .expect("load profile")
                .mfa_enabled
        );
        assert!(!b.pending_mfa_enrollments.lock().contains_key(&user_id));
    }

    #[tokio::test]
    async fn wrong_code_does_not_consume_a_live_enrollment_candidate() {
        let b = InMemoryAuthBackend::new();
        let profile = b
            .register_user(signup_req("mfa-retry@e.f"))
            .await
            .expect("register user");
        let candidate = b
            .start_mfa_enrollment(&profile.user_id)
            .await
            .expect("start MFA enrollment");
        let code = mfa::current_code(&candidate.secret_base32).expect("candidate MFA code");
        let wrong_code = format!(
            "{:06}",
            (code.parse::<u32>().expect("numeric TOTP") + 1) % 1_000_000
        );

        let error = b
            .confirm_mfa_enrollment(&profile.user_id, &wrong_code)
            .await
            .expect_err("wrong code must reject");
        assert!(matches!(error, AuthError::InvalidMfaCode));
        b.confirm_mfa_enrollment(&profile.user_id, &code)
            .await
            .expect("a later correct code may consume the live candidate");
    }

    #[tokio::test]
    async fn pat_lookup_round_trip() {
        use crate::domain::auth::backend::pat::{self, MintedPat};
        let b = InMemoryAuthBackend::new();
        let profile = b.register_user(signup_req("t@e.f")).await.unwrap();
        let user_id: UserId = profile.user_id.parse().unwrap();
        let MintedPat { plaintext, record } =
            pat::mint_pat(user_id, "ci".to_owned(), vec![], None).unwrap();
        b.pats.insert(record.hash, record.clone());

        let resolved = b.lookup_pat(&plaintext).await.unwrap().expect("active");
        assert_eq!(resolved.id, record.id);
        // Wrong prefix is rejected by hash_for_lookup before the map probe.
        let bad = b
            .lookup_pat("nbl_sk_zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz")
            .await;
        assert!(matches!(bad, Err(AuthError::InvalidCredentials)));
    }

    #[tokio::test]
    async fn oauth_start_persists_state_entry() {
        // GitHub's fixed runtime profile needs no discovery request, so this
        // unit test can exercise real authorization-URL emission offline.
        use std::collections::HashMap;

        use crate::config::{OAuthProviderConfig, OAuthProvidersConfig};
        use secrecy::SecretString;

        let mut providers = HashMap::new();
        providers.insert(
            OAuthProvider::GitHub,
            OAuthProviderConfig {
                client_id: SecretString::new("test-client".into()),
                client_secret: SecretString::new("test-secret".into()),
            },
        );
        let cfg = OAuthProvidersConfig { providers };
        let b = backend_with_oauth(cfg);
        let start = b
            .start_oauth(
                OAuthProvider::GitHub,
                "https://nebula.test/api/v1/auth/oauth/github/callback",
            )
            .await
            .unwrap();
        assert!(
            start.authorize_url.contains("state="),
            "authorize URL must include state query param: {}",
            start.authorize_url
        );
        assert!(
            start.authorize_url.contains("code_challenge_method=S256"),
            "authorize URL must include PKCE S256 marker"
        );
        let states = b.oauth_state.lock();
        let entry = states.get(&start.state).expect("state persisted");
        assert_eq!(
            entry.redirect_uri, "https://nebula.test/api/v1/auth/oauth/github/callback",
            "start must persist the handler-derived redirect_uri"
        );
    }

    #[tokio::test]
    async fn oauth_start_fails_closed_at_the_storage_owned_capacity_before_egress() {
        use std::collections::HashMap;

        use crate::config::{OAuthProviderConfig, OAuthProvidersConfig};
        use secrecy::SecretString;

        let mut providers = HashMap::new();
        providers.insert(
            OAuthProvider::GitHub,
            OAuthProviderConfig {
                client_id: SecretString::new("test-client".into()),
                client_secret: SecretString::new("test-secret".into()),
            },
        );
        let backend = backend_with_oauth(OAuthProvidersConfig { providers });
        {
            let mut states = backend.oauth_state.lock();
            let expires_at = expiry_unix(OAUTH_STATE_TTL);
            for index in 0..OAUTH_STATE_CAPACITY {
                states.insert(
                    format!("capacity-state-{index}"),
                    OAuthStateEntry {
                        provider: OAuthProvider::GitHub,
                        code_verifier: "verifier".to_owned(),
                        expires_at,
                        redirect_uri: "https://nebula.test/api/v1/auth/oauth/github/callback"
                            .to_owned(),
                    },
                );
            }
        }

        let error = backend
            .start_oauth(
                OAuthProvider::GitHub,
                "https://nebula.test/api/v1/auth/oauth/github/callback",
            )
            .await
            .expect_err("hard-cap saturation must fail closed");

        assert!(matches!(error, AuthError::RateLimit));
        assert_eq!(
            backend.oauth_state.lock().len(),
            OAUTH_STATE_CAPACITY as usize
        );
    }

    /// `start_oauth` returns `ProviderNotConfigured` when the provider is absent.
    #[tokio::test]
    async fn start_oauth_returns_provider_not_configured_when_provider_absent() {
        let b = InMemoryAuthBackend::new();
        let err = b
            .start_oauth(
                OAuthProvider::Google,
                "https://nebula.test/api/v1/auth/oauth/google/callback",
            )
            .await
            .expect_err("missing provider config must error");
        assert!(matches!(err, AuthError::ProviderNotConfigured));
    }

    /// GitHub uses the runtime-owned canonical authorize endpoint and scope.
    #[tokio::test]
    async fn start_oauth_emits_canonical_github_authorize_url() {
        use std::collections::HashMap;

        use crate::config::{OAuthProviderConfig, OAuthProvidersConfig};
        use secrecy::SecretString;

        let mut providers = HashMap::new();
        providers.insert(
            OAuthProvider::GitHub,
            OAuthProviderConfig {
                client_id: SecretString::new("gh-app-id".into()),
                client_secret: SecretString::new("gh-app-secret".into()),
            },
        );
        let cfg = OAuthProvidersConfig { providers };
        let b = backend_with_oauth(cfg);
        let start = b
            .start_oauth(
                OAuthProvider::GitHub,
                "https://nebula.test/api/v1/auth/oauth/github/callback",
            )
            .await
            .unwrap();
        let url = start.authorize_url;
        assert!(url.starts_with("https://github.com/login/oauth/authorize"));
        assert!(url.contains("client_id=gh-app-id"));
        assert!(url.contains("scope=user%3Aemail"));
        assert!(url.contains("code_challenge_method=S256"));
    }

    #[test]
    fn concurrent_same_oauth_subject_converges_to_one_user() {
        let backend = Arc::new(InMemoryAuthBackend::new());
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let mut resolved = Vec::new();
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for (email, session_id) in [
                ("left@example.test", "left-session"),
                ("right@example.test", "right-session"),
            ] {
                let backend = Arc::clone(&backend);
                let barrier = Arc::clone(&barrier);
                handles.push(scope.spawn(move || {
                    barrier.wait();
                    let prepared = oauth_material(session_id, "challenge", u64::MAX);
                    backend
                        .finalize_oauth_login(
                            OAuthProvider::Google,
                            "stable-subject",
                            Some(email),
                            UserId::new(),
                            &prepared,
                        )
                        .map(|outcome| match outcome {
                            InMemoryOAuthFinalizeOutcome::SessionCreated(user) => user,
                            _ => panic!("verified email must create a session"),
                        })
                        .expect("finalization succeeds")
                }));
            }
            for handle in handles {
                resolved.push(handle.join().expect("worker does not panic"));
            }
        });

        assert_eq!(resolved[0].id, resolved[1].id);
        assert_eq!(backend.users.len(), 1);
        assert_eq!(backend.users_by_email.len(), 1);
        assert_eq!(backend.external_identities.len(), 1);
        assert_eq!(backend.sessions.len(), 2);
    }

    #[test]
    fn concurrent_oauth_subjects_with_same_email_require_explicit_linking() {
        let backend = Arc::new(InMemoryAuthBackend::new());
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let mut resolved = Vec::new();
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for (subject, session_id) in [
                ("left-subject", "left-session"),
                ("right-subject", "right-session"),
            ] {
                let backend = Arc::clone(&backend);
                let barrier = Arc::clone(&barrier);
                handles.push(scope.spawn(move || {
                    barrier.wait();
                    let prepared = oauth_material(session_id, "challenge", u64::MAX);
                    backend.finalize_oauth_login(
                        OAuthProvider::GitHub,
                        subject,
                        Some("shared@example.test"),
                        UserId::new(),
                        &prepared,
                    )
                }));
            }
            for handle in handles {
                resolved.push(handle.join().expect("worker does not panic"));
            }
        });

        assert!(matches!(
            (&resolved[0], &resolved[1]),
            (
                Ok(InMemoryOAuthFinalizeOutcome::SessionCreated(_)),
                Err(AuthError::AccountLinkRequired)
            ) | (
                Err(AuthError::AccountLinkRequired),
                Ok(InMemoryOAuthFinalizeOutcome::SessionCreated(_))
            )
        ));
        assert_eq!(backend.users.len(), 1);
        assert_eq!(backend.users_by_email.len(), 1);
        assert_eq!(backend.external_identities.len(), 1);
        assert_eq!(backend.sessions.len(), 1);
    }

    #[tokio::test]
    async fn existing_verified_local_email_is_never_auto_linked() {
        let backend = InMemoryAuthBackend::new();
        let profile = backend
            .register_user(signup_req("owned@example.test"))
            .await
            .expect("seed local account");
        let user_id: UserId = profile.user_id.parse().expect("profile id is valid");
        backend.users.alter(&user_id, |_, mut user| {
            user.email_verified = true;
            user
        });

        let prepared = oauth_material("must-not-persist", "must-not-persist-challenge", u64::MAX);
        let result = backend.finalize_oauth_login(
            OAuthProvider::Google,
            "new-provider-subject",
            Some("owned@example.test"),
            UserId::new(),
            &prepared,
        );

        assert!(matches!(result, Err(AuthError::AccountLinkRequired)));
        assert_eq!(backend.users.len(), 1);
        assert_eq!(backend.external_identities.len(), 0);
        assert_eq!(backend.sessions.len(), 0);
    }

    #[tokio::test]
    async fn linked_oauth_user_with_mfa_never_receives_a_session_before_totp() {
        let backend = InMemoryAuthBackend::new();
        let (profile, _) = enroll_mfa_user(&backend, "oauth-mfa@example.test").await;
        let user_id: UserId = profile.user_id.parse().expect("profile id is valid");
        backend.external_identities.insert(
            (
                OAuthProvider::GitHub.as_str().to_owned(),
                "linked-mfa-subject".to_owned(),
            ),
            user_id.as_bytes().to_vec(),
        );

        let prepared = oauth_material("must-not-persist", "oauth-mfa-challenge", u64::MAX);
        let outcome = backend
            .finalize_oauth_login(
                OAuthProvider::GitHub,
                "linked-mfa-subject",
                None,
                UserId::new(),
                &prepared,
            )
            .expect("linked identity resolves");

        assert!(matches!(outcome, InMemoryOAuthFinalizeOutcome::MfaRequired));
        assert_eq!(
            backend.sessions.len(),
            0,
            "Nebula MFA must gate session creation after OAuth first factor"
        );
        let challenge = backend
            .mfa_challenges
            .get("oauth-mfa-challenge")
            .expect("MFA challenge is persisted under the identity mutex");
        assert_eq!(challenge.user_id, user_id);
    }

    #[tokio::test]
    async fn oauth_mfa_challenge_is_user_bound_single_use_and_expiring() {
        let backend = InMemoryAuthBackend::new();
        let (owner, owner_secret) = enroll_mfa_user(&backend, "oauth-owner@example.test").await;
        let owner_id: UserId = owner.user_id.parse().expect("owner id is valid");
        backend.external_identities.insert(
            (
                OAuthProvider::GitHub.as_str().to_owned(),
                "oauth-owner-subject".to_owned(),
            ),
            owner_id.as_bytes().to_vec(),
        );

        let owner_code = mfa::current_code(&owner_secret).expect("current owner code");
        let mut other_code = owner_code.clone();
        for index in 0..4 {
            let (_, other_secret) =
                enroll_mfa_user(&backend, &format!("oauth-other-{index}@example.test")).await;
            other_code = mfa::current_code(&other_secret).expect("current other-user code");
            if other_code != owner_code {
                break;
            }
        }
        assert_ne!(other_code, owner_code, "independent TOTP fixtures collided");

        let cross_user = oauth_material("cross-user-session", "cross-user-challenge", u64::MAX);
        let outcome = backend
            .finalize_oauth_login(
                OAuthProvider::GitHub,
                "oauth-owner-subject",
                None,
                UserId::new(),
                &cross_user,
            )
            .expect("issue owner challenge");
        assert!(matches!(outcome, InMemoryOAuthFinalizeOutcome::MfaRequired));
        let wrong_user = backend
            .verify_mfa("cross-user-challenge", &other_code)
            .await
            .expect_err("another user's TOTP must not satisfy the challenge");
        assert!(matches!(wrong_user, AuthError::InvalidMfaCode));
        assert_eq!(backend.sessions.len(), 0);
        let burned = backend
            .verify_mfa("cross-user-challenge", &owner_code)
            .await
            .expect_err("a failed challenge is still single-use");
        assert!(matches!(burned, AuthError::InvalidToken));

        let expired_material = oauth_material("expired-session", "expired-challenge", 0);
        backend
            .finalize_oauth_login(
                OAuthProvider::GitHub,
                "oauth-owner-subject",
                None,
                UserId::new(),
                &expired_material,
            )
            .expect("issue expired challenge fixture");
        let expired = backend
            .verify_mfa("expired-challenge", &owner_code)
            .await
            .expect_err("expired challenge must fail closed");
        assert!(matches!(expired, AuthError::InvalidToken));
        assert_eq!(backend.sessions.len(), 0);

        let fresh = oauth_material("fresh-session", "fresh-challenge", u64::MAX);
        backend
            .finalize_oauth_login(
                OAuthProvider::GitHub,
                "oauth-owner-subject",
                None,
                UserId::new(),
                &fresh,
            )
            .expect("issue fresh challenge");
        let verified = backend
            .verify_mfa("fresh-challenge", &owner_code)
            .await
            .expect("owner TOTP verifies the OAuth challenge");
        assert_eq!(verified.user_id, owner.user_id);
        assert_eq!(
            backend.sessions.len(),
            0,
            "verification alone mints no session"
        );
        let session = backend
            .create_session(&verified.user_id)
            .await
            .expect("session is minted only after local MFA verification");
        assert_eq!(session.principal, Principal::User(owner_id));
        let replay = backend
            .verify_mfa("fresh-challenge", &owner_code)
            .await
            .expect_err("successful OAuth MFA challenge cannot be replayed");
        assert!(matches!(replay, AuthError::InvalidToken));
    }

    #[tokio::test]
    async fn with_email_port_routes_through_injected_port() {
        // Caller-owned EchoSink — the test keeps the Arc so it can
        // assert the injected port (not the default sink that was
        // dropped) actually saw the verification email.
        let custom = Arc::new(EchoSink::default());
        let custom_port: Arc<dyn EmailPort> = Arc::clone(&custom) as _;
        let backend = InMemoryAuthBackend::new().with_email_port(custom_port);

        backend
            .register_user(signup_req("inject@nebula.dev"))
            .await
            .expect("register must succeed against the injected port");

        // The injected port received the verification email.
        let captured = custom.peek();
        assert_eq!(
            captured.len(),
            1,
            "injected port must receive the verification email"
        );
        assert_eq!(captured[0].to, "inject@nebula.dev");
        assert_eq!(captured[0].kind, EmailKind::Verification);

        // The default echo handle was dropped by `with_email_port`, so
        // the back-compat `emails()` shim now returns an empty Vec —
        // proving the default sink is no longer the source of truth.
        assert!(
            backend.emails().is_empty(),
            "with_email_port must drop the default echo: `emails()` should be empty"
        );
    }
}
