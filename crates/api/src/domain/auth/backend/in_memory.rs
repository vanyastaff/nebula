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
    /// In-memory mirror of the PG `external_identities` table per
    /// ADR-0085 D-8. Keyed by `(provider, subject)` to match the PG
    /// PK; value is the linked Nebula `user_id` (16-byte ULID raw
    /// bytes, same shape as `users.id` in PG). PR-4
    /// `complete_oauth` consumes this on the REQ-oauth-006
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
    /// Operator OAuth providers config (Plane A). Same shape as
    /// `PgAuthBackend.oauth_providers`. Defaults to empty so existing
    /// tests that construct the backend via `InMemoryAuthBackend::new`
    /// without OAuth config keep working (start_oauth returns
    /// `ProviderNotConfigured`); tests that exercise OAuth call
    /// [`InMemoryAuthBackend::with_oauth_providers`] explicitly.
    oauth_providers: Arc<crate::config::OAuthProvidersConfig>,
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
            external_identities: DashMap::default(),
            email_port: Arc::clone(&echo) as Arc<dyn EmailPort>,
            default_echo: Some(echo),
            metrics: None,
            oauth_providers: Arc::new(crate::config::OAuthProvidersConfig::default()),
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

    /// Attach the operator OAuth providers config. Mirrors
    /// `PgAuthBackend::with_oauth_providers`. Tests that exercise the
    /// real authorize-URL emission path (PR-3) wire this; tests that
    /// only need the trait surface keep the empty default.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_oauth_providers(
        mut self,
        providers: Arc<crate::config::OAuthProvidersConfig>,
    ) -> Self {
        self.oauth_providers = providers;
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

    // PR-3 T3.9 GREEN: rewrite to emit a REAL authorize URL via
    // `flow::build_authorization_uri` after resolving the provider's
    // endpoints. Symmetric to `PgAuthBackend::start_oauth`; the only
    // difference is the OAuth state row goes into the DashMap (with
    // `redirect_uri` captured in the entry for PR-4 verification).
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
                let provider_cfg = oauth_providers.providers.get(&provider).ok_or_else(|| {
                    AuthError::ProviderNotConfigured {
                        provider: provider.as_str().to_owned(),
                    }
                })?;
                let endpoints = resolve_provider_endpoints(
                    provider_cfg,
                    oauth_providers.oauth_allow_insecure_localhost,
                )
                .await
                .map_err(|e| AuthError::OAuthFailed(e.to_string()))?;
                let pkce = mint_pkce()?;
                let auth_req = AuthorizationUriRequest {
                    auth_url: endpoints.authorize_url.clone(),
                    token_url: endpoints.token_url.clone(),
                    client_id: provider_cfg.client_id.expose_secret().to_owned(),
                    client_secret: provider_cfg.client_secret.expose_secret().to_owned(),
                    redirect_uri: redirect_uri.clone(),
                    scopes: Some(endpoints.scopes),
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
                self.oauth_state.insert(
                    pkce.state.clone(),
                    OAuthStateEntry {
                        provider,
                        code_verifier: pkce.code_verifier,
                        expires_at: expiry_unix(OAUTH_STATE_TTL),
                        consumed: false,
                        // PR-3: capture handler-derived redirect_uri
                        // for PR-4 to re-verify against the user's
                        // callback.
                        redirect_uri: Some(redirect_uri),
                    },
                );
                Ok(OAuthStart {
                    authorize_url,
                    state: pkce.state,
                })
            },
            |result| match result {
                Ok(_) => auth_outcome::SUCCESS,
                // PR-3 wave-2 (CodeRabbit G.6): include
                // `ProviderNotConfigured` in the OAUTH_FAILED bucket
                // so the in-memory backend's metrics labels match the
                // PG backend (G.5) and the `default_outcome_for`
                // exhaustive table.
                Err(AuthError::OAuthFailed(_) | AuthError::ProviderNotConfigured { .. }) => {
                    auth_outcome::OAUTH_FAILED
                },
                Err(_) => auth_outcome::INTERNAL,
            },
        )
        .await
    }

    // PR-4 T4.18 GREEN: real complete_oauth via token POST + userinfo
    // GET + REQ-oauth-006 short-circuit + email truth-table. See
    // crates/api/src/domain/auth/backend/pg.rs for the matching PG
    // impl; the two share the same shape (resolve endpoints →
    // exchange code → fetch userinfo → maybe fetch verified emails
    // → REQ-oauth-006 lookup → email truth-table → mint session) and
    // differ only in where state/identity rows live (DashMap vs PG).
    async fn complete_oauth(
        &self,
        provider: OAuthProvider,
        state: &str,
        code: &str,
        redirect_uri: &str,
    ) -> Result<OAuthCompletion, AuthError> {
        use secrecy::ExposeSecret;

        use crate::transport::oauth::{
            discovery::resolve_provider_endpoints,
            flow::{TokenExchangeRequest, exchange_code},
            userinfo::{UserinfoClaims, fetch_primary_verified_email, fetch_userinfo},
        };

        let provider_label = metrics_emit::oauth_provider_label(provider);
        let redirect_uri = redirect_uri.to_owned();
        let code = code.to_owned();
        let state = state.to_owned();
        let oauth_providers = Arc::clone(&self.oauth_providers);
        metrics_emit::run_with_metrics(
            &self.metrics,
            NEBULA_API_AUTH_OAUTH_ATTEMPTS_TOTAL,
            Some(provider_label),
            async move {
                // Step 1: atomically consume the state entry. Mirror
                // the PG `consume_by_state_and_provider` semantics:
                // returns InvalidToken on missing / expired / replayed
                // / cross-provider. Mark consumed in-place so a
                // second `complete_oauth` with the same `state`
                // fails.
                let entry = {
                    let mut entry = self
                        .oauth_state
                        .get_mut(&state)
                        .ok_or(AuthError::InvalidToken)?;
                    if entry.consumed
                        || entry.expires_at <= Self::now_secs()
                        || entry.provider != provider
                    {
                        return Err(AuthError::InvalidToken);
                    }
                    entry.consumed = true;
                    entry.clone()
                };

                // Step 2: verify the row's redirect_uri matches the
                // handler-derived value (Scenario 3.10
                // public_url_changed_mid_flow defense per
                // REQ-oauth-003).
                match entry.redirect_uri.as_deref() {
                    Some(stored) if stored == redirect_uri => {},
                    _ => {
                        return Err(AuthError::OAuthFailed(
                            "public_url_changed_mid_flow".to_owned(),
                        ));
                    },
                }

                // Step 3: lookup provider config. The state entry was
                // valid so the provider was configured at start_oauth
                // time — a config removed between start and complete
                // surfaces as ProviderNotConfigured.
                let provider_cfg = oauth_providers.providers.get(&provider).ok_or_else(|| {
                    AuthError::ProviderNotConfigured {
                        provider: provider.as_str().to_owned(),
                    }
                })?;

                // Step 4: resolve endpoints (Oidc → cached discovery
                // doc; Manual → operator config). Same helper as
                // start_oauth (PR-3) keeps the two paths in lockstep.
                let endpoints = resolve_provider_endpoints(
                    provider_cfg,
                    oauth_providers.oauth_allow_insecure_localhost,
                )
                .await
                .map_err(|e| AuthError::OAuthFailed(e.to_string()))?;

                // Step 5: exchange code for token via `flow::exchange_code`.
                // The token endpoint goes through the strict
                // anti-SSRF gate inside exchange_code per D-9-WAVE6.
                let token_req = TokenExchangeRequest {
                    token_url: endpoints.token_url.clone(),
                    client_id: provider_cfg.client_id.expose_secret().to_owned(),
                    client_secret: provider_cfg.client_secret.expose_secret().to_owned(),
                    code: code.clone(),
                    redirect_uri: redirect_uri.clone(),
                    code_verifier: entry.code_verifier.clone(),
                    auth_style: nebula_credential::AuthStyle::Header,
                };
                let token_response = exchange_code(&token_req)
                    .await
                    .map_err(AuthError::OAuthFailed)?;

                // Step 6: extract access_token; log id_token presence
                // only (NO JWKS validation per D-16 — userinfo is
                // authoritative).
                let access_token = token_response
                    .get("access_token")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        AuthError::OAuthFailed("token response missing access_token".to_owned())
                    })?
                    .to_owned();
                let id_token_present = token_response
                    .get("id_token")
                    .map(|v| !v.is_null())
                    .unwrap_or(false);
                tracing::debug!(
                    provider = %provider.as_str(),
                    id_token_present,
                    "OAuth complete: token exchange succeeded"
                );

                // Step 7: GET userinfo. Strict anti-SSRF gate inside
                // fetch_userinfo per D-9-WAVE6.
                let UserinfoClaims {
                    sub,
                    email,
                    email_verified,
                } = fetch_userinfo(&endpoints.userinfo_url, &access_token)
                    .await
                    .map_err(|e| AuthError::OAuthFailed(e.to_string()))?;

                // Step 8: REQ-oauth-006 short-circuit BEFORE the
                // verified-emails fallback fetch — for repeat logins
                // we don't need to re-resolve the email; the
                // (provider, sub) linkage is the source of truth.
                let provider_key = provider.as_str().to_owned();
                let link_key = (provider_key.clone(), sub.clone());
                let linked_user_id = self
                    .external_identities
                    .get(&link_key)
                    .map(|r| r.value().clone());
                if let Some(user_id_bytes) = linked_user_id {
                    drop(access_token);
                    let user_id =
                        UserId::from_bytes(user_id_bytes.as_slice().try_into().map_err(|_| {
                            AuthError::Internal(
                                "linked user_id has unexpected byte length".to_owned(),
                            )
                        })?);
                    let record = self.users.get(&user_id).map(|r| r.clone()).ok_or_else(|| {
                        AuthError::Internal(
                            "external_identities link references missing user (CASCADE skipped?)"
                                .to_owned(),
                        )
                    })?;
                    let session = self.create_session(&record.id.to_string()).await?;
                    tracing::info!(
                        provider = %provider.as_str(),
                        cause = "existing_external_identity_linked",
                        "OAuth complete: REQ-oauth-006 short-circuit"
                    );
                    return Ok(OAuthCompletion {
                        user: record.profile(),
                        session,
                    });
                }

                // Step 9: GitHub-style fallback for providers whose
                // userinfo does not include email_verified — fetch the
                // verified_emails endpoint and pick the
                // primary+verified entry per ADR-0085 D-5 wave-6.
                // Only reached on first-login / cross-link branches
                // (the REQ-oauth-006 short-circuit above already
                // returned for repeat logins).
                let (resolved_email, resolved_verified) = match (email_verified, email) {
                    (Some(true), Some(e)) => (e, true),
                    (Some(false), Some(e)) => (e, false),
                    (Some(_), None) => {
                        return Err(AuthError::OAuthFailed(
                            "IdP userinfo missing email".to_owned(),
                        ));
                    },
                    (None, fallback_email) => match endpoints.verified_emails_url.as_deref() {
                        Some(url) => {
                            let e = fetch_primary_verified_email(url, &access_token)
                                .await
                                .map_err(|e| AuthError::OAuthFailed(e.to_string()))?;
                            (e, true)
                        },
                        None => (fallback_email.unwrap_or_default(), false),
                    },
                };
                drop(access_token);

                // Step 10: email truth-table (REQ-oauth-004 +
                // REQ-oauth-005). Both branches require
                // `resolved_verified == true`; an unverified IdP email
                // fails closed per the account-takeover defense.
                if !resolved_verified || resolved_email.is_empty() {
                    return Err(AuthError::EmailNotVerified);
                }
                let normalized_email = resolved_email.trim().to_lowercase();
                let existing = self.lookup_user_by_email(&normalized_email);
                let record = match existing {
                    None => {
                        // REQ-oauth-004: first login — create a new
                        // user with email_verified = true (IdP
                        // attested) and no password (OAuth-only).
                        let id = UserId::new();
                        let record = UserRecord {
                            id,
                            email: normalized_email.clone(),
                            display_name: normalized_email.clone(),
                            avatar_url: None,
                            password_hash: None,
                            email_verified: true,
                            failed_login_count: 0,
                            locked_until: None,
                            mfa_secret: None,
                            mfa_enabled: false,
                        };
                        self.put_user(record.clone());
                        record
                    },
                    Some(existing_record) => {
                        // REQ-oauth-005: link onto an existing user.
                        // Account-takeover defense (Scenario 5.2):
                        // if Nebula's email_verified is false even
                        // though the IdP attests verified, reject.
                        if !existing_record.email_verified {
                            return Err(AuthError::EmailNotVerified);
                        }
                        existing_record
                    },
                };

                // Step 11: persist the (provider, sub) -> user_id
                // link. The DashMap insert is idempotent at the
                // primary-key level for the in-memory backend; PG's
                // PK rejects duplicates via SQLSTATE 23505.
                self.external_identities
                    .insert(link_key, record.id.as_bytes().to_vec());

                // Step 12: mint a session.
                let session = self.create_session(&record.id.to_string()).await?;
                Ok(OAuthCompletion {
                    user: record.profile(),
                    session,
                })
            },
            |result| match result {
                Ok(_) => auth_outcome::SUCCESS,
                Err(AuthError::InvalidToken) => auth_outcome::TOKEN_INVALID,
                Err(AuthError::EmailNotVerified) => auth_outcome::EMAIL_UNVERIFIED,
                Err(AuthError::OAuthFailed(_) | AuthError::ProviderNotConfigured { .. }) => {
                    auth_outcome::OAUTH_FAILED
                },
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
        // PR-3: start_oauth now requires a configured provider per
        // ADR-0085 D-6. The legacy synthetic-URL path is gone;
        // declare a Manual provider with localhost endpoints so the
        // test exercises the real authorize-URL emission against the
        // operator-config path.
        use std::collections::HashMap;

        use crate::config::{OAuthEndpoints, OAuthProviderConfig, OAuthProvidersConfig};
        use secrecy::SecretString;

        let mut providers = HashMap::new();
        providers.insert(
            OAuthProvider::Google,
            OAuthProviderConfig {
                client_id: SecretString::new("test-client".into()),
                client_secret: SecretString::new("test-secret".into()),
                endpoints: OAuthEndpoints::Manual {
                    authorize_url: "https://example.invalid/authorize".to_owned(),
                    token_url: "https://example.invalid/token".to_owned(),
                    userinfo_url: "https://example.invalid/userinfo".to_owned(),
                    verified_emails_url: None,
                    jwks_url: None,
                    scopes: vec!["openid".to_owned(), "email".to_owned()],
                },
            },
        );
        let cfg = Arc::new(OAuthProvidersConfig {
            providers,
            oauth_allow_insecure_localhost: false,
        });
        let b = InMemoryAuthBackend::new().with_oauth_providers(cfg);
        let start = b
            .start_oauth(
                OAuthProvider::Google,
                "https://nebula.test/api/v1/auth/oauth/google/callback",
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
        assert!(b.oauth_state.contains_key(&start.state));
        let entry = b.oauth_state.get(&start.state).unwrap();
        assert_eq!(
            entry.redirect_uri.as_deref(),
            Some("https://nebula.test/api/v1/auth/oauth/google/callback"),
            "PR-3 must persist the handler-derived redirect_uri"
        );
    }

    /// T3.6 RED-then-GREEN: start_oauth returns ProviderNotConfigured
    /// when the provider is absent from `oauth.providers` map.
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
        match err {
            AuthError::ProviderNotConfigured { provider } => {
                assert_eq!(provider, "google");
            },
            other => panic!("expected ProviderNotConfigured, got: {other:?}"),
        }
    }

    /// T3.3 RED-then-GREEN: OIDC provider returns real authorize URL
    /// via the flow helper (PKCE S256 markers + state query param).
    #[tokio::test]
    async fn start_oauth_emits_real_authorize_url_with_pkce_s256_for_oidc_provider() {
        use std::collections::HashMap;

        use crate::config::{OAuthEndpoints, OAuthProviderConfig, OAuthProvidersConfig};
        use secrecy::SecretString;

        // Oidc arm bypassed via a Manual fixture pointing at a fake
        // domain — the test does not actually fetch the discovery
        // doc (that's the discovery::tests path); it exercises the
        // authorize-URL construction step which is identical for both
        // arms after `resolve_provider_endpoints` returns.
        let mut providers = HashMap::new();
        providers.insert(
            OAuthProvider::Microsoft,
            OAuthProviderConfig {
                client_id: SecretString::new("my-client-id".into()),
                client_secret: SecretString::new("my-client-secret".into()),
                endpoints: OAuthEndpoints::Manual {
                    authorize_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize"
                        .to_owned(),
                    token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token"
                        .to_owned(),
                    userinfo_url: "https://graph.microsoft.com/oidc/userinfo".to_owned(),
                    verified_emails_url: None,
                    jwks_url: None,
                    scopes: vec![
                        "openid".to_owned(),
                        "email".to_owned(),
                        "profile".to_owned(),
                    ],
                },
            },
        );
        let cfg = Arc::new(OAuthProvidersConfig {
            providers,
            oauth_allow_insecure_localhost: false,
        });
        let b = InMemoryAuthBackend::new().with_oauth_providers(cfg);
        let start = b
            .start_oauth(
                OAuthProvider::Microsoft,
                "https://nebula.test/api/v1/auth/oauth/microsoft/callback",
            )
            .await
            .unwrap();
        let url = start.authorize_url;
        assert!(url.starts_with("https://login.microsoftonline.com"));
        assert!(url.contains("client_id=my-client-id"));
        assert!(url.contains("code_challenge="));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("response_type=code"));
        assert!(
            url.contains("scope=openid+email+profile")
                || url.contains("scope=openid%20email%20profile"),
            "scopes joined: {url}"
        );
        assert!(url.contains("state="));
    }

    /// T3.4 RED-then-GREEN: Manual provider with explicit endpoints
    /// builds authorize URL against the operator-configured
    /// `authorize_url` (e.g. GitHub).
    #[tokio::test]
    async fn start_oauth_emits_real_authorize_url_for_manual_provider_with_explicit_endpoints() {
        use std::collections::HashMap;

        use crate::config::{OAuthEndpoints, OAuthProviderConfig, OAuthProvidersConfig};
        use secrecy::SecretString;

        let mut providers = HashMap::new();
        providers.insert(
            OAuthProvider::GitHub,
            OAuthProviderConfig {
                client_id: SecretString::new("gh-app-id".into()),
                client_secret: SecretString::new("gh-app-secret".into()),
                endpoints: OAuthEndpoints::Manual {
                    authorize_url: "https://github.com/login/oauth/authorize".to_owned(),
                    token_url: "https://github.com/login/oauth/access_token".to_owned(),
                    userinfo_url: "https://api.github.com/user".to_owned(),
                    verified_emails_url: Some("https://api.github.com/user/emails".to_owned()),
                    jwks_url: None,
                    scopes: vec!["user:email".to_owned()],
                },
            },
        );
        let cfg = Arc::new(OAuthProvidersConfig {
            providers,
            oauth_allow_insecure_localhost: false,
        });
        let b = InMemoryAuthBackend::new().with_oauth_providers(cfg);
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

// PR-4 stage C: RED tests for `complete_oauth` using a TcpListener
// fake responder that serves the token endpoint POST + userinfo GET
// + optional GitHub-style `/user/emails` GET. The pattern mirrors
// `transport::oauth::discovery::tests` (PR-3 wave-1).
#[cfg(test)]
mod complete_oauth_tests {
    use super::*;
    use crate::config::{OAuthEndpoints, OAuthProviderConfig, OAuthProvidersConfig};
    use crate::domain::auth::backend::oauth::OAuthProvider;
    use ::secrecy::SecretString as RealSecretString;
    use std::collections::HashMap;
    use std::io::Write;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn spawn_idp_responder(
        token_body: Vec<u8>,
        userinfo_body: Vec<u8>,
        emails_body: Option<Vec<u8>>,
    ) -> (String, String, String) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let token_url = format!("http://{addr}/token");
        let userinfo_url = format!("http://{addr}/userinfo");
        let emails_url = format!("http://{addr}/emails");

        tokio::spawn(async move {
            for _ in 0..3_u8 {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = vec![0u8; 8192];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = std::str::from_utf8(&buf[..n]).unwrap_or("");
                let body = if req.contains("/token") {
                    token_body.clone()
                } else if req.contains("/userinfo") {
                    userinfo_body.clone()
                } else if req.contains("/emails") && emails_body.is_some() {
                    emails_body.clone().unwrap()
                } else {
                    let nf = b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
                    let _ = sock.write_all(nf).await;
                    let _ = sock.shutdown().await;
                    continue;
                };
                let _ = sock.write_all(&body).await;
                let _ = sock.shutdown().await;
            }
        });

        (token_url, userinfo_url, emails_url)
    }

    fn http_json(body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        write!(
            &mut out,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .unwrap();
        out.extend_from_slice(body);
        out
    }

    fn cfg_pointing_at(
        provider: OAuthProvider,
        token_url: &str,
        userinfo_url: &str,
        verified_emails_url: Option<&str>,
    ) -> Arc<OAuthProvidersConfig> {
        let mut providers = HashMap::new();
        providers.insert(
            provider,
            OAuthProviderConfig {
                client_id: RealSecretString::new("test-client".into()),
                client_secret: RealSecretString::new("test-secret".into()),
                endpoints: OAuthEndpoints::Manual {
                    // Authorize URL not fetched by these tests; flag-aware
                    // gate runs at validate_at_load time (PR-2) which we
                    // bypass here by constructing the config directly.
                    authorize_url: "https://idp.example.com/authorize".to_owned(),
                    token_url: token_url.to_owned(),
                    userinfo_url: userinfo_url.to_owned(),
                    verified_emails_url: verified_emails_url.map(str::to_owned),
                    jwks_url: None,
                    scopes: vec!["openid".to_owned()],
                },
            },
        );
        Arc::new(OAuthProvidersConfig {
            providers,
            oauth_allow_insecure_localhost: true,
        })
    }

    fn seed_started_flow(
        backend: &InMemoryAuthBackend,
        state: &str,
        provider: OAuthProvider,
        redirect_uri: &str,
    ) {
        backend.oauth_state.insert(
            state.to_owned(),
            OAuthStateEntry {
                provider,
                code_verifier: "verifier-1234567890abcdef".to_owned(),
                expires_at: InMemoryAuthBackend::now_secs() + 600,
                consumed: false,
                redirect_uri: Some(redirect_uri.to_owned()),
            },
        );
    }

    fn token_response(access: &str, with_id_token: bool) -> Vec<u8> {
        let body = if with_id_token {
            format!(
                "{{\"access_token\":\"{access}\",\"token_type\":\"Bearer\",\"id_token\":\"x.y.z\"}}"
            )
        } else {
            format!("{{\"access_token\":\"{access}\",\"token_type\":\"Bearer\"}}")
        };
        http_json(body.as_bytes())
    }

    fn userinfo_response(sub: &str, email: Option<&str>, email_verified: Option<bool>) -> Vec<u8> {
        let mut body = serde_json::Map::new();
        body.insert("sub".to_owned(), serde_json::Value::String(sub.to_owned()));
        if let Some(e) = email {
            body.insert("email".to_owned(), serde_json::Value::String(e.to_owned()));
        }
        if let Some(v) = email_verified {
            body.insert("email_verified".to_owned(), serde_json::Value::Bool(v));
        }
        http_json(serde_json::to_string(&body).unwrap().as_bytes())
    }

    fn emails_response(entries: &[(&str, bool, bool)]) -> Vec<u8> {
        let arr: Vec<serde_json::Value> = entries
            .iter()
            .map(|(e, primary, verified)| {
                serde_json::json!({
                    "email": e, "primary": primary, "verified": verified,
                })
            })
            .collect();
        http_json(serde_json::to_string(&arr).unwrap().as_bytes())
    }

    /// T4.4 RED-then-GREEN: first-login via OIDC provider where the
    /// userinfo response includes `email_verified: true` inline.
    #[tokio::test]
    async fn complete_oauth_succeeds_with_valid_code_oidc_provider_first_login() {
        let token = token_response("at-1", true);
        let userinfo = userinfo_response("oidc-sub-1", Some("alice@example.com"), Some(true));
        let (token_url, userinfo_url, _) = spawn_idp_responder(token, userinfo, None).await;
        let cfg = cfg_pointing_at(OAuthProvider::Google, &token_url, &userinfo_url, None);

        let b = InMemoryAuthBackend::new().with_oauth_providers(cfg);
        let redirect = "https://nebula.test/api/v1/auth/oauth/google/callback";
        seed_started_flow(&b, "state-1", OAuthProvider::Google, redirect);

        let result = b
            .complete_oauth(OAuthProvider::Google, "state-1", "code-1", redirect)
            .await
            .expect("happy path must succeed");
        assert_eq!(result.user.email, "alice@example.com");
        assert!(result.user.email_verified);
        assert!(
            b.external_identities
                .contains_key(&("google".to_owned(), "oidc-sub-1".to_owned()))
        );
    }

    /// T4.5 + T4.13b RED-then-GREEN: GitHub-style Manual provider
    /// triggers `/user/emails` fallback per ADR-0085 D-5 wave-6.
    #[tokio::test]
    async fn complete_oauth_succeeds_with_valid_code_manual_provider_github() {
        let token = token_response("gh-at-1", false);
        let userinfo = userinfo_response("gh-sub-1", Some("charlie@example.com"), None);
        let emails = emails_response(&[
            ("charlie-old@example.com", false, true),
            ("charlie@example.com", true, true),
        ]);
        let (token_url, userinfo_url, emails_url) =
            spawn_idp_responder(token, userinfo, Some(emails)).await;
        let cfg = cfg_pointing_at(
            OAuthProvider::GitHub,
            &token_url,
            &userinfo_url,
            Some(&emails_url),
        );
        let b = InMemoryAuthBackend::new().with_oauth_providers(cfg);
        let redirect = "https://nebula.test/api/v1/auth/oauth/github/callback";
        seed_started_flow(&b, "state-gh", OAuthProvider::GitHub, redirect);

        let result = b
            .complete_oauth(OAuthProvider::GitHub, "state-gh", "code-gh", redirect)
            .await
            .expect("GitHub flow must succeed");
        assert_eq!(result.user.email, "charlie@example.com");
        assert!(result.user.email_verified);
    }

    /// T4.6 RED-then-GREEN: replay rejection.
    #[tokio::test]
    async fn complete_oauth_rejects_replay() {
        let token = token_response("at-r", false);
        let userinfo = userinfo_response("sub-r", Some("dave@example.com"), Some(true));
        let (t1, u1, _) = spawn_idp_responder(token, userinfo, None).await;
        let cfg = cfg_pointing_at(OAuthProvider::Google, &t1, &u1, None);
        let b = InMemoryAuthBackend::new().with_oauth_providers(cfg);
        let redirect = "https://nebula.test/api/v1/auth/oauth/google/callback";
        seed_started_flow(&b, "state-r", OAuthProvider::Google, redirect);

        let _ = b
            .complete_oauth(OAuthProvider::Google, "state-r", "code-r", redirect)
            .await
            .expect("first call must succeed");
        let replay = b
            .complete_oauth(OAuthProvider::Google, "state-r", "code-r", redirect)
            .await
            .expect_err("replay must be rejected");
        assert!(matches!(replay, AuthError::InvalidToken));
    }

    /// T4.8 RED-then-GREEN: unknown state value rejected.
    #[tokio::test]
    async fn complete_oauth_rejects_mismatched_state_token() {
        let cfg = cfg_pointing_at(
            OAuthProvider::Google,
            "https://idp.example.com/token",
            "https://idp.example.com/userinfo",
            None,
        );
        let b = InMemoryAuthBackend::new().with_oauth_providers(cfg);
        let err = b
            .complete_oauth(
                OAuthProvider::Google,
                "state-never-seen",
                "code-x",
                "https://nebula.test/api/v1/auth/oauth/google/callback",
            )
            .await
            .expect_err("unknown state must be rejected");
        assert!(matches!(err, AuthError::InvalidToken));
    }

    /// T4.9 RED-then-GREEN: cross-provider state replay defense.
    #[tokio::test]
    async fn complete_oauth_rejects_mismatched_provider() {
        let cfg = cfg_pointing_at(
            OAuthProvider::GitHub,
            "https://idp.example.com/token",
            "https://idp.example.com/userinfo",
            None,
        );
        let b = InMemoryAuthBackend::new().with_oauth_providers(cfg);
        let redirect = "https://nebula.test/api/v1/auth/oauth/google/callback";
        seed_started_flow(&b, "state-cross", OAuthProvider::Google, redirect);
        let err = b
            .complete_oauth(OAuthProvider::GitHub, "state-cross", "code-x", redirect)
            .await
            .expect_err("cross-provider state must be rejected");
        assert!(matches!(err, AuthError::InvalidToken));
        let entry = b.oauth_state.get("state-cross").unwrap();
        assert!(!entry.consumed);
    }

    /// T4.10 RED-then-GREEN: public_url_changed_mid_flow defense
    /// (Scenario 3.10).
    #[tokio::test]
    async fn complete_oauth_rejects_public_url_changed_mid_flow() {
        let cfg = cfg_pointing_at(
            OAuthProvider::Google,
            "https://idp.example.com/token",
            "https://idp.example.com/userinfo",
            None,
        );
        let b = InMemoryAuthBackend::new().with_oauth_providers(cfg);
        seed_started_flow(
            &b,
            "state-pu",
            OAuthProvider::Google,
            "https://nebula.test/api/v1/auth/oauth/google/callback",
        );
        let err = b
            .complete_oauth(
                OAuthProvider::Google,
                "state-pu",
                "code-pu",
                "https://nebula-CHANGED.test/api/v1/auth/oauth/google/callback",
            )
            .await
            .expect_err("mismatched redirect_uri must fail closed");
        match err {
            AuthError::OAuthFailed(cause) => {
                assert_eq!(cause, "public_url_changed_mid_flow");
            },
            other => panic!("expected OAuthFailed, got: {other:?}"),
        }
    }

    /// T4.11 RED-then-GREEN: token endpoint 500 → OAuthFailed.
    #[tokio::test]
    async fn complete_oauth_handles_idp_token_endpoint_500_with_redacted_log() {
        let token_err = b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n".to_vec();
        let userinfo_dummy = http_json(b"{}");
        let (t1, u1, _) = spawn_idp_responder(token_err, userinfo_dummy, None).await;
        let cfg = cfg_pointing_at(OAuthProvider::Google, &t1, &u1, None);
        let b = InMemoryAuthBackend::new().with_oauth_providers(cfg);
        let redirect = "https://nebula.test/api/v1/auth/oauth/google/callback";
        seed_started_flow(&b, "state-500", OAuthProvider::Google, redirect);
        let err = b
            .complete_oauth(OAuthProvider::Google, "state-500", "code-500", redirect)
            .await
            .expect_err("token endpoint 500 must propagate as OAuthFailed");
        assert!(matches!(err, AuthError::OAuthFailed(_)));
    }

    /// T4.12 RED-then-GREEN: token response missing access_token.
    #[tokio::test]
    async fn complete_oauth_rejects_malformed_token_response_missing_access_token() {
        let bad_token = http_json(br#"{"token_type":"Bearer"}"#);
        let userinfo = userinfo_response("x", Some("e@e.com"), Some(true));
        let (t1, u1, _) = spawn_idp_responder(bad_token, userinfo, None).await;
        let cfg = cfg_pointing_at(OAuthProvider::Google, &t1, &u1, None);
        let b = InMemoryAuthBackend::new().with_oauth_providers(cfg);
        let redirect = "https://nebula.test/api/v1/auth/oauth/google/callback";
        seed_started_flow(&b, "state-mt", OAuthProvider::Google, redirect);
        let err = b
            .complete_oauth(OAuthProvider::Google, "state-mt", "code-mt", redirect)
            .await
            .expect_err("missing access_token must error");
        match err {
            AuthError::OAuthFailed(msg) => {
                assert!(msg.contains("access_token"), "msg: {msg}");
            },
            other => panic!("expected OAuthFailed, got: {other:?}"),
        }
    }

    /// T4.14 RED-then-GREEN: first-login rejected when IdP says
    /// `email_verified: false` AND no verified_emails_url fallback.
    #[tokio::test]
    async fn complete_oauth_rejects_first_login_when_idp_email_unverified() {
        let token = token_response("at-uv", false);
        let userinfo = userinfo_response("sub-uv", Some("erin@example.com"), Some(false));
        let (t1, u1, _) = spawn_idp_responder(token, userinfo, None).await;
        let cfg = cfg_pointing_at(OAuthProvider::Google, &t1, &u1, None);
        let b = InMemoryAuthBackend::new().with_oauth_providers(cfg);
        let redirect = "https://nebula.test/api/v1/auth/oauth/google/callback";
        seed_started_flow(&b, "state-uv", OAuthProvider::Google, redirect);
        let err = b
            .complete_oauth(OAuthProvider::Google, "state-uv", "code-uv", redirect)
            .await
            .expect_err("unverified IdP email must reject");
        assert!(matches!(err, AuthError::EmailNotVerified));
    }
}
