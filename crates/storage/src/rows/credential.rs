//! Row types for the credential layer.

use chrono::{DateTime, Utc};
use serde_json::Value;

/// Table: `credentials`
///
/// Encrypted credential storage scoped to org or workspace.
/// The `encrypted_secret` column is ciphertext — decryption
/// happens exclusively in `nebula-credential`.
#[derive(Debug, Clone)]
pub struct CredentialRow {
    /// `cred_` ULID, 16-byte BYTEA.
    pub id: Vec<u8>,
    pub org_id: Vec<u8>,
    /// `None` for org-level credentials.
    pub workspace_id: Option<Vec<u8>>,
    pub slug: String,
    pub display_name: String,
    /// Credential type (e.g. `'oauth2_google'`, `'api_key'`, `'basic_auth'`).
    pub kind: String,
    /// `'workspace'` or `'org'`.
    pub scope: String,
    /// Encrypted with org master key. Never logged.
    pub encrypted_secret: Vec<u8>,
    /// Supports key rotation.
    pub encryption_version: i32,
    /// For org-level: serialized list of allowed workspace IDs.
    pub allowed_workspaces: Option<Value>,
    /// Non-secret data (client_id, scopes, etc.).
    pub metadata: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub created_by: Vec<u8>,
    pub last_rotated_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    /// Optimistic concurrency version.
    pub version: i64,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Table: `pending_credentials`
///
/// Credentials in an incomplete OAuth flow or pending approval.
/// Promoted to `credentials` on completion, or expired and cleaned up.
#[derive(Debug, Clone)]
pub struct PendingCredentialRow {
    /// 16-byte BYTEA primary key.
    pub id: Vec<u8>,
    pub org_id: Vec<u8>,
    pub workspace_id: Option<Vec<u8>>,
    /// Credential type being set up.
    pub kind: String,
    /// Transient flow state (OAuth state param, PKCE verifier, etc.).
    pub flow_state: Value,
    pub created_at: DateTime<Utc>,
    pub created_by: Vec<u8>,
    pub expires_at: DateTime<Utc>,
}

/// Table: `credential_audit`
///
/// Append-only log of credential access and mutations for
/// security auditing.
#[derive(Debug, Clone)]
pub struct CredentialAuditRow {
    /// 16-byte BYTEA primary key (ULID).
    pub id: Vec<u8>,
    pub credential_id: Vec<u8>,
    /// `'accessed'` / `'rotated'` / `'created'` / `'deleted'`.
    pub action: String,
    /// `'user'` / `'service_account'` / `'system'`.
    pub actor_kind: String,
    pub actor_id: Option<Vec<u8>>,
    pub details: Option<Value>,
    pub performed_at: DateTime<Utc>,
}
