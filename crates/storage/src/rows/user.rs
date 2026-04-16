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
