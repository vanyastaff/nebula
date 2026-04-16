//! Row types for the audit and slug-history layer.

use chrono::{DateTime, Utc};
use serde_json::Value;

/// Table: `slug_history`
///
/// Tracks old slugs so that renamed entities can be found by
/// their previous URL for a redirect grace period.
/// Primary key: `(kind, scope_id, old_slug)`.
#[derive(Debug, Clone)]
pub struct SlugHistoryRow {
    /// `'org'` / `'workspace'` / `'workflow'` / etc.
    pub kind: String,
    /// `None` for org-level slugs, otherwise the parent entity ID.
    pub scope_id: Option<Vec<u8>>,
    pub old_slug: String,
    /// The entity that was renamed.
    pub resource_id: Vec<u8>,
    pub renamed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Table: `audit_log`
///
/// High-level audit events (separate from the execution journal).
/// Append-only, retention is plan-configurable.
#[derive(Debug, Clone)]
pub struct AuditLogRow {
    /// ULID, 16-byte BYTEA.
    pub id: Vec<u8>,
    pub org_id: Vec<u8>,
    /// `None` for org-level events.
    pub workspace_id: Option<Vec<u8>>,
    /// `'user'` / `'service_account'` / `'system'`.
    pub actor_kind: String,
    /// `None` for system-initiated events.
    pub actor_id: Option<Vec<u8>>,
    /// Dotted action name, e.g. `'workflow.created'`, `'credential.rotated'`.
    pub action: String,
    pub target_kind: Option<String>,
    pub target_id: Option<Vec<u8>>,
    pub details: Option<Value>,
    /// Stored as text (INET on Postgres, TEXT on SQLite).
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub emitted_at: DateTime<Utc>,
}

/// Table: `blobs`
///
/// Generic binary object storage for large payloads that exceed
/// inline column limits (e.g. state snapshots, compiled artifacts).
#[derive(Debug, Clone)]
pub struct BlobRow {
    /// 16-byte BYTEA primary key.
    pub id: Vec<u8>,
    /// MIME type or application-defined kind.
    pub content_type: String,
    /// Raw bytes.
    pub data: Vec<u8>,
    /// Size in bytes (denormalized for queries without reading data).
    pub size: i64,
    pub created_at: DateTime<Utc>,
}
