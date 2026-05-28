//! Row types for the identity layer.

use chrono::{DateTime, Utc};
use serde_json::Value;

/// Table: `users`
///
/// Registered humans in the system. Supports soft delete, MFA,
/// and brute-force lockout tracking.
#[derive(Debug, Clone)]
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
    pub mfa_secret: Option<Vec<u8>>,
    /// Optimistic concurrency version.
    pub version: i64,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Table: `oauth_links`
///
/// Links external OAuth accounts (Google, GitHub, Microsoft) to a user.
/// Primary key: `(provider, provider_user_id)`.
#[derive(Debug, Clone)]
pub struct OAuthLinkRow {
    pub user_id: Vec<u8>,
    /// `'google'` / `'github'` / `'microsoft'`.
    pub provider: String,
    pub provider_user_id: String,
    pub provider_email: Option<String>,
    pub linked_at: DateTime<Utc>,
}

/// Table: `sessions`
///
/// Active login sessions (browser cookies). Expired rows are
/// cleaned up daily.
#[derive(Debug, Clone)]
pub struct SessionRow {
    /// `sess_` ULID, 16-byte BYTEA.
    pub id: Vec<u8>,
    pub user_id: Vec<u8>,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    /// Stored as text (INET on Postgres, TEXT on SQLite).
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub revoked_at: Option<DateTime<Utc>>,
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
#[derive(Debug, Clone)]
pub struct OAuthStateRow {
    /// Random url-safe state value (primary key). Not a ULID — callers
    /// generate this with a CSPRNG.
    pub state: String,
    /// Provider identifier, e.g. `'google'` / `'github'` / `'microsoft'`.
    pub provider: String,
    /// PKCE `code_verifier` to be sent on token exchange.
    pub code_verifier: String,
    /// Optional `redirect_uri` requested at authorize time.
    pub redirect_uri: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
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
    /// IdP identifier, e.g. `'google'` / `'github'` / `'microsoft'`.
    /// Snake-case matches the `OAuthProvider` enum serialization.
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
