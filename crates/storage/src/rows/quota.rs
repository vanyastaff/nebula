//! Row types for the quota and rate-limiting layer.

use chrono::{DateTime, Utc};

/// Table: `org_quotas`
///
/// Plan-based limits for an organization. One row per org.
/// Primary key: `org_id`.
#[derive(Debug, Clone)]
pub struct OrgQuotaRow {
    pub org_id: Vec<u8>,
    pub plan: String,
    pub concurrent_executions_limit: i32,
    pub executions_per_month_limit: Option<i64>,
    pub active_workflows_limit: Option<i32>,
    pub total_workflows_limit: Option<i32>,
    pub workspaces_limit: Option<i32>,
    pub org_members_limit: Option<i32>,
    pub service_accounts_limit: Option<i32>,
    pub storage_bytes_limit: Option<i64>,
    pub updated_at: DateTime<Utc>,
}

/// Table: `org_quota_usage`
///
/// Current resource usage counters for an organization.
/// Primary key: `org_id`.
#[derive(Debug, Clone)]
pub struct OrgQuotaUsageRow {
    pub org_id: Vec<u8>,
    pub concurrent_executions: i32,
    pub active_workflows: i32,
    pub total_workflows: i32,
    pub workspaces: i32,
    pub org_members: i32,
    pub service_accounts: i32,
    pub storage_bytes: i64,
    pub executions_this_month: i64,
    pub month_reset_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Table: `workspace_quota_usage`
///
/// Current resource usage counters for a workspace.
/// Primary key: `workspace_id`.
#[derive(Debug, Clone)]
pub struct WorkspaceQuotaUsageRow {
    pub workspace_id: Vec<u8>,
    pub concurrent_executions: i32,
    pub active_workflows: i32,
    pub updated_at: DateTime<Utc>,
}
