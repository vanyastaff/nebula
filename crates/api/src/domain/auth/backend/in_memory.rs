//! In-memory [`AuthBackend`] implementation.
//!
//! Production-quality crypto (Argon2id passwords, RFC 6238 TOTP, SHA-256
//! PAT lookup) backed by per-process `DashMap` / `parking_lot::RwLock`
//! state. This is the **default backend** for tests and the local-first
//! `simple_server` binary; storage-backed implementations live in a future
//! Sprint-E follow-up that swaps out the storage for `nebula-storage`
//! repos without changing the trait surface.

use std::{
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

use super::{
    dto::{SignupRequest, UserProfile},
    error::AuthError,
    mfa,
    oauth::{OAUTH_STATE_TTL, OAuthProvider, OAuthStateEntry, expiry_unix, mint_pkce},
    password,
    pat::{self, MintedPat, PatRecord, compute_pat_expires_at},
    provider::{
        AuthBackend, CreatePatParams, MfaEnrollment, OAuthCompletion, OAuthStart, PasswordOutcome,
        ProfilePatch, metrics_emit,
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

/// In-memory [`AuthBackend`].
pub struct InMemoryAuthBackend {
    users_by_email: DashMap<String, UserId>,
    users: DashMap<UserId, UserRecord>,
    sessions: DashMap<String, (UserId, u64)>,
    pats: DashMap<[u8; 32], PatRecord>,
    verification_tokens: DashMap<String, VerificationToken>,
    mfa_challenges: DashMap<String, MfaChallenge>,
    oauth_state: DashMap<String, OAuthStateEntry>,
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
}

impl Default for InMemoryAuthBackend {
    fn default() -> Self {
        let echo = Arc::new(EchoSink::default());
        Self {
            users_by_email: DashMap::default(),
            users: DashMap::default(),
            sessions: DashMap::default(),
            pats: DashMap::default(),
            verification_tokens: DashMap::default(),
            mfa_challenges: DashMap::default(),
            oauth_state: DashMap::default(),
            email_port: Arc::clone(&echo) as Arc<dyn EmailPort>,
            default_echo: Some(echo),
            metrics: None,
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
#[derive(Debug, Clone)]
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

// guard-justified: the `From` impl exists exclusively to feed the
// deprecated `EmailEnvelope` type and cannot itself avoid touching it.
#[allow(deprecated, reason = "shim feeds the deprecated public type")]
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

    /// Snapshot the captured outbound emails — used in tests.
    ///
    /// Returns an empty vector when [`Self::with_email_port`] has
    /// swapped the default [`EchoSink`] for a custom transport; the
    /// in-process inbox is only meaningful for the default port.
    // guard-justified: this accessor IS the back-compat shim and must
    // return the deprecated `EmailEnvelope` type by contract.
    #[must_use]
    #[allow(deprecated, reason = "deliberate back-compat shim over EmailEnvelope")]
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

    fn lookup_user_by_email(&self, email: &str) -> Option<UserRecord> {
        let key = email.trim().to_lowercase();
        let id = *self.users_by_email.get(&key)?;
        self.users.get(&id).map(|u| u.clone())
    }

    fn put_user(&self, user: UserRecord) {
        self.users_by_email.insert(user.email.clone(), user.id);
        self.users.insert(user.id, user);
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
    ) -> Result<Option<Principal>, crate::ApiError> {
        let now = Self::now_secs();
        if let Some(entry) = self.sessions.get(session_id) {
            let (user_id, expires) = *entry;
            drop(entry);
            if expires <= now {
                self.sessions.remove(session_id);
                return Ok(None);
            }
            return Ok(Some(Principal::User(user_id)));
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
                self.put_user(record);

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
        self.sessions.insert(id.clone(), (parsed, exp));

        Ok(SessionRecord {
            id,
            principal: Principal::User(parsed),
            csrf_token: csrf,
            expires_at: expires_at(SESSION_TTL),
        })
    }

    async fn revoke_session(&self, session_id: &str) -> Result<(), AuthError> {
        self.sessions.remove(session_id);
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
                u.display_name = name.trim().to_owned();
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
                // Save secret but DO NOT flip mfa_enabled until confirm_mfa_enrollment.
                self.users.alter(&parsed, |_, mut u| {
                    u.mfa_secret = Some(secret.clone());
                    u.mfa_enabled = false;
                    u
                });
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
                let user = self
                    .users
                    .get(&parsed)
                    .ok_or(AuthError::UserNotFound)?
                    .clone();
                let secret = user
                    .mfa_secret
                    .as_deref()
                    .ok_or(AuthError::InvalidMfaCode)?;
                if !mfa::verify_code(secret, code)? {
                    return Err(AuthError::InvalidMfaCode);
                }
                self.users.alter(&parsed, |_, mut u| {
                    u.mfa_enabled = true;
                    u
                });
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

    async fn start_oauth(&self, provider: OAuthProvider) -> Result<OAuthStart, AuthError> {
        let provider_label = metrics_emit::oauth_provider_label(provider);
        metrics_emit::run_with_metrics(
            &self.metrics,
            NEBULA_API_AUTH_OAUTH_ATTEMPTS_TOTAL,
            Some(provider_label),
            async move {
                let pkce = mint_pkce()?;
                // No real provider config in the in-memory backend — return a
                // synthetic authorize URL so tests can verify the contract.
                let authorize_url = format!(
                    "https://nebula.local/oauth/{}/authorize?state={}&code_challenge={}&code_challenge_method=S256",
                    provider.as_str(),
                    pkce.state,
                    pkce.code_challenge,
                );
                self.oauth_state.insert(
                    pkce.state.clone(),
                    OAuthStateEntry {
                        provider,
                        code_verifier: pkce.code_verifier,
                        expires_at: expiry_unix(OAUTH_STATE_TTL),
                        consumed: false,
                    },
                );
                Ok(OAuthStart {
                    authorize_url,
                    state: pkce.state,
                })
            },
            |result| match result {
                Ok(_) => auth_outcome::SUCCESS,
                Err(AuthError::OAuthFailed(_)) => auth_outcome::OAUTH_FAILED,
                Err(_) => auth_outcome::INTERNAL,
            },
        )
        .await
    }

    async fn complete_oauth(
        &self,
        provider: OAuthProvider,
        state: &str,
        _code: &str,
    ) -> Result<OAuthCompletion, AuthError> {
        let provider_label = metrics_emit::oauth_provider_label(provider);
        metrics_emit::run_with_metrics(
            &self.metrics,
            NEBULA_API_AUTH_OAUTH_ATTEMPTS_TOTAL,
            Some(provider_label),
            async move {
                let entry = self
                    .oauth_state
                    .get_mut(state)
                    .ok_or(AuthError::InvalidToken)?;
                if entry.consumed
                    || entry.expires_at <= Self::now_secs()
                    || entry.provider != provider
                {
                    return Err(AuthError::InvalidToken);
                }
                // The in-memory backend cannot actually exchange a code with a
                // real provider; return NotImplemented so callers know they need
                // a configured backend.
                drop(entry);
                Err(AuthError::NotImplemented(
                    "complete_oauth requires a configured provider backend",
                ))
            },
            |result| match result {
                Ok(_) => auth_outcome::SUCCESS,
                Err(AuthError::InvalidToken) => auth_outcome::TOKEN_INVALID,
                Err(AuthError::OAuthFailed(_)) => auth_outcome::OAUTH_FAILED,
                Err(_) => auth_outcome::INTERNAL,
            },
        )
        .await
    }
}

// guard-justified: the tests below intentionally exercise the
// deprecated `EmailEnvelope` shim and `emails()` accessor.
#[cfg(test)]
#[allow(
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
        let principal = b
            .get_principal_by_session(&session.id)
            .await
            .unwrap()
            .expect("session is live");
        assert!(matches!(principal, Principal::User(_)));
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
        let b = InMemoryAuthBackend::new();
        let start = b.start_oauth(OAuthProvider::Google).await.unwrap();
        assert!(start.authorize_url.contains("state="));
        assert!(b.oauth_state.contains_key(&start.state));
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
