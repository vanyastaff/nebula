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
use parking_lot::RwLock;

use super::{
    dto::{SignupRequest, UserProfile},
    error::AuthError,
    mfa,
    oauth::{OAUTH_STATE_TTL, OAuthProvider, OAuthStateEntry, expiry_unix, mint_pkce},
    password,
    pat::{self, PatRecord},
    provider::{AuthBackend, MfaEnrollment, OAuthCompletion, OAuthStart, PasswordOutcome},
    session::{self, SESSION_TTL, SessionRecord, expires_at},
};

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
#[derive(Default)]
pub struct InMemoryAuthBackend {
    users_by_email: DashMap<String, UserId>,
    users: DashMap<UserId, UserRecord>,
    sessions: DashMap<String, (UserId, u64)>,
    pats: DashMap<[u8; 32], PatRecord>,
    verification_tokens: DashMap<String, VerificationToken>,
    mfa_challenges: DashMap<String, MfaChallenge>,
    oauth_state: DashMap<String, OAuthStateEntry>,
    /// In-memory capture of outbound emails for tests and local inspection.
    ///
    /// Every verification / password-reset email is appended here; there is
    /// no separate "disabled" mode — use [`Self::emails`] to assert flows.
    email_sink: Arc<RwLock<Vec<EmailEnvelope>>>,
}

/// Emitted email — captured by the test hook so integration tests can
/// assert reset / verification flows without a real SMTP transport.
#[derive(Debug, Clone)]
pub struct EmailEnvelope {
    /// Recipient address.
    pub to: String,
    /// Token included in the email link.
    pub token: String,
    /// Email category — `EmailVerify` or `PasswordReset`.
    pub kind: &'static str,
}

impl InMemoryAuthBackend {
    /// Construct an empty backend.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Wrap into an `Arc<dyn AuthBackend>` for [`crate::AppState`].
    #[must_use]
    pub fn into_arc(self) -> Arc<dyn AuthBackend> {
        Arc::new(self)
    }

    /// Snapshot the captured outbound emails — used in tests.
    #[must_use]
    pub fn emails(&self) -> Vec<EmailEnvelope> {
        self.email_sink.read().clone()
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

    fn record_email(&self, to: &str, token: &str, kind: &'static str) {
        self.email_sink.write().push(EmailEnvelope {
            to: to.to_owned(),
            token: token.to_owned(),
            kind,
        });
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
            password_hash: Some(hash),
            email_verified: false,
            failed_login_count: 0,
            locked_until: None,
            mfa_secret: None,
            mfa_enabled: false,
        };
        let profile = record.profile();
        self.put_user(record);

        let token = self.issue_verification_token(id, VerificationKind::EmailVerify)?;
        self.record_email(&email, &token, "EmailVerify");

        tracing::info!(user_id = %id, "user registered");
        Ok(profile)
    }

    async fn authenticate_password(
        &self,
        email: &str,
        password_input: &str,
        totp: Option<&str>,
    ) -> Result<PasswordOutcome, AuthError> {
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
                    let until =
                        Utc::now() + chrono::Duration::from_std(LOCKOUT_TTL).unwrap_or_default();
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
                let secret = user
                    .mfa_secret
                    .as_deref()
                    .ok_or_else(|| AuthError::Internal("mfa enabled without secret".to_owned()))?;
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
    }

    async fn verify_mfa(
        &self,
        challenge_token: &str,
        code: &str,
    ) -> Result<UserProfile, AuthError> {
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
        let secret = user
            .mfa_secret
            .as_deref()
            .ok_or_else(|| AuthError::Internal("mfa challenge for non-mfa user".to_owned()))?;
        if !mfa::verify_code(secret, code)? {
            return Err(AuthError::InvalidMfaCode);
        }
        Ok(user.profile())
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

    async fn request_password_reset(&self, email: &str) -> Result<(), AuthError> {
        if let Some(user) = self.lookup_user_by_email(email) {
            match self.issue_verification_token(user.id, VerificationKind::PasswordReset) {
                Ok(token) => {
                    self.record_email(&user.email, &token, "PasswordReset");
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
    }

    async fn verify_email(&self, token: &str) -> Result<(), AuthError> {
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
    }

    async fn start_mfa_enrollment(&self, user_id: &str) -> Result<MfaEnrollment, AuthError> {
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
    }

    async fn confirm_mfa_enrollment(&self, user_id: &str, code: &str) -> Result<(), AuthError> {
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
    }

    async fn start_oauth(&self, provider: OAuthProvider) -> Result<OAuthStart, AuthError> {
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
    }

    async fn complete_oauth(
        &self,
        provider: OAuthProvider,
        state: &str,
        _code: &str,
    ) -> Result<OAuthCompletion, AuthError> {
        let entry = self
            .oauth_state
            .get_mut(state)
            .ok_or(AuthError::InvalidToken)?;
        if entry.consumed || entry.expires_at <= Self::now_secs() || entry.provider != provider {
            return Err(AuthError::InvalidToken);
        }
        // The in-memory backend cannot actually exchange a code with a
        // real provider; return NotImplemented so callers know they need
        // a configured backend.
        drop(entry);
        Err(AuthError::NotImplemented(
            "complete_oauth requires a configured provider backend",
        ))
    }
}

#[cfg(test)]
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
        b.email_sink.write().clear();
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
}
