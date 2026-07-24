//! Row types for the identity layer.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::session_token::SessionTokenDigest;

/// Table: `users`
///
/// Registered humans in the system. Supports soft delete, MFA,
/// and brute-force lockout tracking.
pub struct UserRow {
    /// `user_` ULID, 16-byte BYTEA.
    pub id: Vec<u8>,
    /// Lowercased email, unique among active users.
    pub email: String,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub display_name: String,
    pub avatar_url: Option<String>,
    /// Argon2id encoded; `None` for OAuth-only accounts.
    pub password_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub locked_until: Option<DateTime<Utc>>,
    pub failed_login_count: i32,
    pub mfa_enabled: bool,
    /// Encrypted with master key.
    /// Versioned encrypted envelope for the active TOTP seed.
    pub mfa_secret_envelope: Option<Vec<u8>>,
    /// Optimistic concurrency version.
    pub version: i64,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for UserRow {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UserRow")
            .field("id", &"[redacted]")
            .field("email", &"[redacted]")
            .field("email_verified_at", &self.email_verified_at)
            .field("display_name", &"[redacted]")
            .field(
                "avatar_url",
                &self.avatar_url.as_ref().map(|_| "[redacted]"),
            )
            .field(
                "password_hash",
                &self.password_hash.as_ref().map(|_| "[redacted]"),
            )
            .field("created_at", &self.created_at)
            .field("last_login_at", &self.last_login_at)
            .field("locked_until", &self.locked_until)
            .field("failed_login_count", &self.failed_login_count)
            .field("mfa_enabled", &self.mfa_enabled)
            .field(
                "mfa_secret_envelope",
                &self.mfa_secret_envelope.as_ref().map(|_| "[redacted]"),
            )
            .field("version", &self.version)
            .field("deleted_at", &self.deleted_at)
            .finish()
    }
}

/// Table: `oauth_links`
///
/// Links external OAuth accounts to a user. The Plane-A runtime currently
/// emits only the reviewed `google` and `github` provider keys.
/// Primary key: `(provider, provider_user_id)`.
#[derive(Debug, Clone)]
pub struct OAuthLinkRow {
    pub user_id: Vec<u8>,
    /// Runtime-emitted provider key: currently `'google'` or `'github'`.
    /// The text storage shape is technical decoupling, not an
    /// operator-extensible provider profile.
    pub provider: String,
    pub provider_user_id: String,
    pub provider_email: Option<String>,
    pub linked_at: DateTime<Utc>,
}

/// Table: `sessions`
///
/// Active login sessions (browser cookies). Expired rows are
/// cleaned up daily.
pub struct SessionRow {
    /// Domain-separated digest of the presented browser-session token.
    pub token_digest: SessionTokenDigest,
    pub user_id: Vec<u8>,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    /// Stored as text (INET on Postgres, TEXT on SQLite).
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for SessionRow {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionRow")
            .field("token_digest", &"[redacted]")
            .field("user_id", &"[redacted]")
            .field("created_at", &self.created_at)
            .field("last_active_at", &self.last_active_at)
            .field("expires_at", &self.expires_at)
            .field(
                "ip_address",
                &self.ip_address.as_ref().map(|_| "[redacted]"),
            )
            .field(
                "user_agent",
                &self.user_agent.as_ref().map(|_| "[redacted]"),
            )
            .field("revoked_at", &self.revoked_at)
            .finish()
    }
}

/// New browser-session metadata independent of its one-time bearer token.
///
/// The plaintext token is supplied separately to the repository create
/// boundary and is never retained in this value.
pub struct SessionDraft {
    pub user_id: Vec<u8>,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    /// Stored as text (INET on Postgres, TEXT on SQLite).
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for SessionDraft {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionDraft")
            .field("user_id", &"[redacted]")
            .field("created_at", &self.created_at)
            .field("last_active_at", &self.last_active_at)
            .field("expires_at", &self.expires_at)
            .field(
                "ip_address",
                &self.ip_address.as_ref().map(|_| "[redacted]"),
            )
            .field(
                "user_agent",
                &self.user_agent.as_ref().map(|_| "[redacted]"),
            )
            .field("revoked_at", &self.revoked_at)
            .finish()
    }
}

#[cfg(test)]
mod identity_row_secret_tests {
    use chrono::Utc;

    use super::{OAuthStateRow, SessionDraft, SessionRow, UserRow};
    use crate::session_token::session_token_digest;

    static_assertions::assert_not_impl_any!(UserRow: Clone);
    static_assertions::assert_not_impl_any!(SessionRow: Clone);
    static_assertions::assert_not_impl_any!(SessionDraft: Clone);
    static_assertions::assert_not_impl_any!(OAuthStateRow: Clone);

    #[test]
    fn user_and_session_debug_redact_identity_authority() {
        const CANARY: &str = "IDENTITY_ROW_SECRET_CANARY-5e27";
        let now = Utc::now();
        let user = UserRow {
            id: CANARY.as_bytes().to_vec(),
            email: format!("{CANARY}@example.test"),
            email_verified_at: Some(now),
            display_name: CANARY.to_owned(),
            avatar_url: Some(format!("https://example.test/{CANARY}")),
            password_hash: Some(format!("$argon2id${CANARY}")),
            created_at: now,
            last_login_at: None,
            locked_until: None,
            failed_login_count: 0,
            mfa_enabled: true,
            mfa_secret_envelope: Some(CANARY.as_bytes().to_vec()),
            version: 1,
            deleted_at: None,
        };
        let session = SessionRow {
            token_digest: session_token_digest(CANARY.as_bytes()),
            user_id: CANARY.as_bytes().to_vec(),
            created_at: now,
            last_active_at: now,
            expires_at: now,
            ip_address: Some(CANARY.to_owned()),
            user_agent: Some(CANARY.to_owned()),
            revoked_at: None,
        };
        let draft = SessionDraft {
            user_id: CANARY.as_bytes().to_vec(),
            created_at: now,
            last_active_at: now,
            expires_at: now,
            ip_address: Some(CANARY.to_owned()),
            user_agent: Some(CANARY.to_owned()),
            revoked_at: None,
        };

        assert!(!format!("{user:?}").contains(CANARY));
        assert!(!format!("{session:?}").contains(CANARY));
        assert!(!format!("{draft:?}").contains(CANARY));
    }
}

/// Table: `personal_access_tokens`
///
/// API tokens for CLI, CI, and automation. Usable by both
/// users and service accounts.
#[derive(Debug, Clone)]
pub struct PersonalAccessTokenRow {
    /// `pat_` ULID, 16-byte BYTEA.
    pub id: Vec<u8>,
    /// `'user'` or `'service_account'`.
    pub principal_kind: String,
    pub principal_id: Vec<u8>,
    pub name: String,
    /// First 12 chars of the token for display.
    pub prefix: String,
    /// SHA-256 of the full token.
    pub hash: Vec<u8>,
    /// `[]` = full access, or `['read', 'workflows', ...]`.
    pub scopes: Value,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Table: `verification_tokens`
///
/// One-time tokens for email verification, password reset,
/// invitations, and MFA recovery.
#[derive(Debug, Clone)]
pub struct VerificationTokenRow {
    /// SHA-256 of the token value (primary key).
    pub token_hash: Vec<u8>,
    pub user_id: Vec<u8>,
    /// `'email_verification'` / `'password_reset'` / `'org_invite'` / `'mfa_recovery'`.
    pub kind: String,
    /// Kind-specific data (invite details, etc.).
    pub payload: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
}

/// Table: `plane_a_oauth_states`
///
/// Server-side PKCE state for Plane-A (sign-in-with-OAuth). Each row
/// holds the `code_verifier` minted by `start_oauth`; the matching
/// `complete_oauth` consumes the row atomically to validate the
/// callback and recover the verifier. Named `plane_a_*` to avoid
/// clashing with the Plane-B credential OAuth pending-state surface
/// (`pending_credentials`).
pub struct OAuthStateRow {
    /// Random url-safe state value (primary key). Not a ULID — callers
    /// generate this with a CSPRNG.
    pub state: String,
    /// Runtime-emitted provider key: currently `'google'` or `'github'`.
    /// Persisted text does not authorize custom endpoint profiles.
    pub provider: String,
    /// PKCE `code_verifier` to be sent on token exchange.
    pub code_verifier: String,
    /// Optional `redirect_uri` requested at authorize time.
    pub redirect_uri: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for OAuthStateRow {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthStateRow")
            .field("state", &"[redacted]")
            .field("provider", &self.provider)
            .field("code_verifier", &"[redacted]")
            .field(
                "redirect_uri",
                &self.redirect_uri.as_ref().map(|_| "[redacted]"),
            )
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .field("consumed_at", &self.consumed_at)
            .finish()
    }
}

#[cfg(test)]
mod oauth_state_debug_tests {
    use super::OAuthStateRow;
    use chrono::Utc;

    #[test]
    fn oauth_state_row_debug_redacts_state_verifier_and_redirect_uri() {
        let now = Utc::now();
        let row = OAuthStateRow {
            state: "STATE_CANARY-6087".to_owned(),
            provider: "google".to_owned(),
            code_verifier: "VERIFIER_CANARY-c815".to_owned(),
            redirect_uri: Some(
                "https://app.example/callback?canary=REDIRECT_CANARY-d96c".to_owned(),
            ),
            created_at: now,
            expires_at: now,
            consumed_at: None,
        };

        let debug = format!("{row:?}");
        assert!(!debug.contains("STATE_CANARY-6087"));
        assert!(!debug.contains("VERIFIER_CANARY-c815"));
        assert!(!debug.contains("REDIRECT_CANARY-d96c"));
    }
}

/// External identity row: a stable per-IdP linkage between
/// `(provider, subject)` (the IdP's `sub` claim is the source of
/// truth for "same human") and a Nebula `user_id`.
///
/// Per ADR-0085 D-8: PK is `(provider, subject)`; `user_id` is the
/// FK with `ON DELETE CASCADE`, so deleting a user atomically purges
/// every external link. `email` is the IdP-side email captured at
/// link time — audit only, NOT refreshed on subsequent logins per
/// REQ-oauth-006 Scenario 6.2.
#[derive(Debug, Clone)]
pub struct ExternalIdentityRow {
    /// Runtime-emitted IdP key: currently `'google'` or `'github'`.
    /// Snake-case matches the `OAuthProvider` enum serialization; the text
    /// column is not an operator-extensible profile registry.
    pub provider: String,
    /// IdP `sub` claim. Opaque to Nebula; treated as a stable string.
    pub subject: String,
    /// Nebula `user_id` (16-byte ULID, raw bytes — matches `users.id`).
    /// `Vec<u8>` because the repos layer is generic over storage shape
    /// and `users.id` is `BYTEA` per `0001_users.sql`.
    pub user_id: Vec<u8>,
    /// IdP-side email at link time (NULL when the IdP did not return
    /// one, e.g. GitHub with no `user:email` scope). NOT refreshed on
    /// subsequent logins.
    pub email: Option<String>,
    /// When the link was first established.
    pub linked_at: DateTime<Utc>,
}
