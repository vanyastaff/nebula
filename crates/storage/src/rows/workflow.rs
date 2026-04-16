//! Row types for the workflow layer.

use chrono::{DateTime, Utc};
use serde_json::Value;

/// Table: `workflows`
///
/// Top-level workflow entity scoped to a workspace.
/// Points to a current published version.
#[derive(Debug, Clone)]
pub struct WorkflowRow {
    /// `wf_` ULID, 16-byte BYTEA.
    pub id: Vec<u8>,
    pub workspace_id: Vec<u8>,
    pub slug: String,
    pub display_name: String,
    pub description: Option<String>,
    /// FK to `workflow_versions.id`.
    pub current_version_id: Vec<u8>,
    /// `'Active'` / `'Paused'` / `'Archived'`.
    pub state: String,
    pub created_at: DateTime<Utc>,
    pub created_by: Vec<u8>,
    pub updated_at: DateTime<Utc>,
    /// Optimistic concurrency version.
    pub version: i64,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Table: `workflow_versions`
///
/// Immutable versioned snapshots of workflow definitions.
/// Executions are pinned to a specific version.
#[derive(Debug, Clone)]
pub struct WorkflowVersionRow {
    /// `wfv_` ULID, 16-byte BYTEA.
    pub id: Vec<u8>,
    pub workflow_id: Vec<u8>,
    pub version_number: i32,
    /// Full workflow definition (JSONB).
    pub definition: Value,
    pub schema_version: i32,
    /// `'Draft'` / `'Published'` / `'Archived'` / `'Deleted'`.
    pub state: String,
    pub created_at: DateTime<Utc>,
    pub created_by: Vec<u8>,
    pub description: Option<String>,
    /// Pre-compiled expression bytecode.
    pub compiled_expressions: Option<Vec<u8>>,
    /// Pre-compiled validation rules.
    pub compiled_validation: Option<Vec<u8>>,
    /// Whether this version is pinned from automatic GC.
    pub pinned: bool,
}
