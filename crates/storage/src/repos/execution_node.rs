//! Per-attempt node execution storage.

use std::future::Future;

use chrono::{DateTime, Utc};

use crate::{error::StorageError, rows::ExecutionNodeRow};

/// Storage for per-attempt node execution details.
///
/// Spec 16 layer 4. Each retry attempt gets its own row; state column
/// holds stateful-action state with schema hash for migration detection.
pub trait ExecutionNodeRepo: Send + Sync {
    /// Insert a new node attempt.
    fn create(
        &self,
        node: &ExecutionNodeRow,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Fetch a node attempt by ID.
    fn get(
        &self,
        id: &[u8],
    ) -> impl Future<Output = Result<Option<ExecutionNodeRow>, StorageError>> + Send;

    /// Fetch a node attempt by `(execution_id, logical_node_id, attempt)`.
    fn get_attempt(
        &self,
        execution_id: &[u8],
        logical_node_id: &str,
        attempt: i32,
    ) -> impl Future<Output = Result<Option<ExecutionNodeRow>, StorageError>> + Send;

    /// Update the status of a node attempt with CAS on `version`.
    fn transition(
        &self,
        id: &[u8],
        expected_version: i64,
        new_status: &str,
        finished_at: Option<DateTime<Utc>>,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Record the final output of a node attempt.
    fn set_output(
        &self,
        id: &[u8],
        expected_version: i64,
        output: serde_json::Value,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Record an error on a node attempt.
    fn set_error(
        &self,
        id: &[u8],
        expected_version: i64,
        error_kind: &str,
        error_message: &str,
        retry_hint_ms: Option<i64>,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    // ── Stateful action checkpoint ──────────────────────────────────────

    /// Persist `(iteration_count, state)` for a stateful action.
    /// Atomically updates both columns with CAS on `version`.
    fn save_checkpoint(
        &self,
        id: &[u8],
        expected_version: i64,
        iteration_count: i32,
        state: serde_json::Value,
        state_schema_hash: &[u8],
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Load the current checkpoint state of a node attempt.
    fn load_checkpoint(
        &self,
        id: &[u8],
    ) -> impl Future<Output = Result<Option<CheckpointSnapshot>, StorageError>> + Send;

    // ── Retry / wake scheduling ─────────────────────────────────────────

    /// Schedule the attempt to wake up at `wake_at`.
    fn schedule_wake_at(
        &self,
        id: &[u8],
        expected_version: i64,
        wake_at: DateTime<Utc>,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Suspend waiting for a named signal.
    fn schedule_wake_signal(
        &self,
        id: &[u8],
        expected_version: i64,
        signal_name: &str,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// List node attempts ready to wake up by timer.
    fn list_wake_ready(
        &self,
        now: DateTime<Utc>,
        batch_size: u32,
    ) -> impl Future<Output = Result<Vec<ExecutionNodeRow>, StorageError>> + Send;

    // ── Listing ─────────────────────────────────────────────────────────

    /// List all node attempts for an execution, ordered by `started_at`.
    fn list_for_execution(
        &self,
        execution_id: &[u8],
    ) -> impl Future<Output = Result<Vec<ExecutionNodeRow>, StorageError>> + Send;
}

/// Snapshot returned by [`ExecutionNodeRepo::load_checkpoint`].
#[derive(Debug, Clone)]
pub struct CheckpointSnapshot {
    /// Current iteration count.
    pub iteration_count: i32,
    /// Serialized state (or `None` if the action is stateless).
    pub state: Option<serde_json::Value>,
    /// Hash of the state schema for migration detection.
    pub state_schema_hash: Option<Vec<u8>>,
}
