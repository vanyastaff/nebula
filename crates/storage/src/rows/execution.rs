//! Row types for the execution layer.

use chrono::{DateTime, Utc};
use serde_json::Value;

/// Table: `executions`
///
/// A single workflow execution run. Tracks the full lifecycle from
/// pending through terminal status, with lease/claim coordination,
/// cancel tracking, timeout, and restart chain.
#[derive(Debug, Clone)]
pub struct ExecutionRow {
    /// `exec_` ULID, 16-byte BYTEA.
    pub id: Vec<u8>,
    pub workspace_id: Vec<u8>,
    pub org_id: Vec<u8>,
    pub workflow_version_id: Vec<u8>,

    // --- Status ---
    /// `'Pending'` / `'Queued'` / `'Running'` / `'Suspended'` / `'Succeeded'`
    /// / `'Failed'` / `'Cancelled'` / `'Cancelling'` / `'Orphaned'`.
    pub status: String,
    /// `ExecutionSource` enum serialized as JSON.
    pub source: Value,
    /// Trigger payload or manual input.
    pub input: Option<Value>,
    /// Final workflow output.
    pub output: Option<Value>,
    /// Execution-wide `$vars`.
    pub vars: Option<Value>,
    /// `{done: 5, running: 2, pending: 3}`.
    pub progress_summary: Option<Value>,

    // --- Timing ---
    pub created_at: DateTime<Utc>,
    /// For delayed starts.
    pub scheduled_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,

    // --- Claim / lease (multi-process coordination) ---
    /// `node_id` of the worker holding the lease.
    pub claimed_by: Option<Vec<u8>>,
    pub claimed_until: Option<DateTime<Utc>>,

    // --- Cancel tracking ---
    pub cancel_requested_at: Option<DateTime<Utc>>,
    pub cancel_requested_by: Option<Vec<u8>>,
    pub cancel_reason: Option<String>,
    pub escalated: bool,

    // --- Restart tracking ---
    /// FK to `executions.id` if this is a restart.
    pub restarted_from: Option<Vec<u8>>,

    // --- Timeout ---
    /// Computed from `created_at + workflow_version.execution_timeout`.
    pub execution_timeout_at: Option<DateTime<Utc>>,

    // --- CAS ---
    /// Optimistic concurrency version.
    pub version: i64,
}

/// Table: `execution_nodes`
///
/// Per-attempt node execution details. Tracks status, I/O, error
/// info, retry/wake scheduling, stateful action state, and
/// cancel escalation.
#[derive(Debug, Clone)]
pub struct ExecutionNodeRow {
    /// `node_` ULID, 16-byte BYTEA.
    pub id: Vec<u8>,
    pub execution_id: Vec<u8>,
    /// Logical node ID from the workflow definition.
    pub logical_node_id: String,
    /// Attempt number (1, 2, 3, ...) per retry.
    pub attempt: i32,

    // --- Status ---
    /// `'Running'` / `'Succeeded'` / `'Failed'` / `'Cancelled'`
    /// / `'PendingRetry'` / `'Suspended'`.
    pub status: String,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,

    // --- Input/output ---
    pub input: Option<Value>,
    pub output: Option<Value>,

    // --- Error tracking ---
    /// `'Transient'` / `'Permanent'` / `'Cancelled'` / `'Fatal'` / `'Timeout'`.
    pub error_kind: Option<String>,
    pub error_message: Option<String>,
    /// Hint from `TransientWithHint`.
    pub error_retry_hint_ms: Option<i64>,
    /// `{exec_id}:{logical_node_id}:{attempt}`.
    pub idempotency_key: String,

    // --- Retry / wake tracking ---
    /// Non-null when `PendingRetry` or `Suspended` with a timer.
    pub wake_at: Option<DateTime<Utc>>,
    /// Non-null when `Suspended` waiting for a signal.
    pub wake_signal_name: Option<String>,

    // --- Stateful action state ---
    /// Inline state (up to ~1 MB).
    pub state: Option<Value>,
    /// Reference for larger state blobs (v1.5).
    pub state_blob_ref: Option<Vec<u8>>,
    /// Schema migration detection hash.
    pub state_schema_hash: Option<Vec<u8>>,
    pub iteration_count: i32,

    // --- Cancel escalation ---
    pub escalated: bool,

    // --- CAS ---
    /// Optimistic concurrency version.
    pub version: i64,
}
