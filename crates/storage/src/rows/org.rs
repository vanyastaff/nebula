//! Row types for the tenancy layer.

use chrono::{DateTime, Utc};
use serde_json::Value;

/// Table: `orgs`
///
/// Organizations — top-level tenant boundary. Every workspace,
/// credential, and quota lives under an org.
#[derive(Debug, Clone)]
pub struct OrgRow {
    /// `org_` ULID, 16-byte BYTEA.
    pub id: Vec<u8>,
    /// Globally unique, case-insensitive.
    pub slug: String,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
    /// First user who created the org (not FK, preserves history).
    pub created_by: Vec<u8>,
    /// `'self_host'` / `'free'` / `'team'` / `'business'` / `'enterprise'`.
    pub plan: String,
    pub billing_email: Option<String>,
    pub settings: Value,
    /// Optimistic concurrency version.
    pub version: i64,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Table: `workspaces`
///
/// Workspaces within an org — logical grouping for workflows,
/// credentials, and team members.
#[derive(Debug, Clone)]
pub struct WorkspaceRow {
    /// `ws_` ULID, 16-byte BYTEA.
    pub id: Vec<u8>,
    pub org_id: Vec<u8>,
    pub slug: String,
    pub display_name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub created_by: Vec<u8>,
    /// Only one default workspace per org.
    pub is_default: bool,
    pub settings: Value,
    /// Optimistic concurrency version.
    pub version: i64,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Table: `org_members`
///
/// Membership relation between principals (users, service accounts)
/// and organizations.
/// Primary key: `(org_id, principal_kind, principal_id)`.
#[derive(Debug, Clone)]
pub struct OrgMemberRow {
    pub org_id: Vec<u8>,
    /// `'user'` or `'service_account'`.
    pub principal_kind: String,
    pub principal_id: Vec<u8>,
    /// `'OrgOwner'` / `'OrgAdmin'` / `'OrgMember'` / `'OrgBilling'`.
    pub role: String,
    pub invited_at: DateTime<Utc>,
    pub invited_by: Option<Vec<u8>>,
    pub accepted_at: Option<DateTime<Utc>>,
}

/// Table: `workspace_members`
///
/// Membership relation between principals and workspaces.
/// Primary key: `(workspace_id, principal_kind, principal_id)`.
#[derive(Debug, Clone)]
pub struct WorkspaceMemberRow {
    pub workspace_id: Vec<u8>,
    /// `'user'` or `'service_account'`.
    pub principal_kind: String,
    pub principal_id: Vec<u8>,
    /// `'WorkspaceAdmin'` / `'Editor'` / `'Runner'` / `'Viewer'`.
    pub role: String,
    pub added_at: DateTime<Utc>,
    pub added_by: Vec<u8>,
}

/// Table: `service_accounts`
///
/// Non-human principals scoped to an org, used for automation
/// and CI/CD integrations.
#[derive(Debug, Clone)]
pub struct ServiceAccountRow {
    /// `sa_` ULID, 16-byte BYTEA.
    pub id: Vec<u8>,
    pub org_id: Vec<u8>,
    pub slug: String,
    pub display_name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub created_by: Vec<u8>,
    pub disabled_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}
