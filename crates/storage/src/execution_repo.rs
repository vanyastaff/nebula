//! Persistence and coordination interface for workflow executions.
//!
//! State (versioned CAS), journal (append-only), leases. Used by API and engine;
//! implementations in this crate or in adapters.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use nebula_core::{ExecutionId, NodeKey, WorkflowId};
use thiserror::Error;
use tokio::{sync::RwLock, time::Instant};

/// Errors returned by execution repository operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ExecutionRepoError {
    /// Requested entity is absent in storage.
    #[error("{entity} not found: {id}")]
    NotFound {
        /// Entity kind (for example: `execution`).
        entity: String,
        /// Entity identifier.
        id: String,
    },

    /// Optimistic concurrency check failed.
    #[error("{entity} {id}: expected version {expected_version}, got {actual_version}")]
    Conflict {
        /// Entity kind (for example: `execution`).
        entity: String,
        /// Entity identifier.
        id: String,
        /// Version expected by caller.
        expected_version: u64,
        /// Actual persisted version.
        actual_version: u64,
    },

    /// Backend/network connection failure.
    #[error("connection error: {0}")]
    Connection(String),

    /// Serialization or deserialization failure.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Operation timed out.
    #[error("timeout: {operation} after {duration:?}")]
    Timeout {
        /// Name of the operation that timed out.
        operation: String,
        /// Timeout duration that elapsed.
        duration: Duration,
    },

    /// Execution lease is currently held by another owner.
    #[error("lease unavailable for execution {execution_id}")]
    LeaseUnavailable {
        /// Execution identifier for the contested lease.
        execution_id: String,
    },

    /// Unexpected internal repository failure.
    #[error("internal error: {0}")]
    Internal(String),

    /// Persisted node-result carries a schema version this binary cannot decode.
    ///
    /// Emitted by [`ExecutionRepo::load_node_result`] / [`ExecutionRepo::load_all_results`]
    /// when a newer writer stored a record the current binary does not understand.
    /// Surface to operators as a resume failure — never fall back silently
    /// (ADR-0008 §2, PRODUCT_CANON §4.5).
    #[error("unknown node-result schema version: {version} (max supported: {max_supported})")]
    UnknownSchemaVersion {
        /// Schema version found in the persisted row.
        version: u32,
        /// Highest version this binary knows how to decode.
        max_supported: u32,
    },
}

impl ExecutionRepoError {
    /// Builds a [`ExecutionRepoError::NotFound`].
    pub fn not_found(entity: impl Into<String>, id: impl Into<String>) -> Self {
        Self::NotFound {
            entity: entity.into(),
            id: id.into(),
        }
    }

    /// Builds a [`ExecutionRepoError::Conflict`].
    pub fn conflict(
        entity: impl Into<String>,
        id: impl Into<String>,
        expected: u64,
        actual: u64,
    ) -> Self {
        Self::Conflict {
            entity: entity.into(),
            id: id.into(),
            expected_version: expected,
            actual_version: actual,
        }
    }

    /// Builds a [`ExecutionRepoError::Timeout`].
    pub fn timeout(operation: impl Into<String>, duration: Duration) -> Self {
        Self::Timeout {
            operation: operation.into(),
            duration,
        }
    }
}

/// Persistence and coordination interface for workflow executions.
#[async_trait]
pub trait ExecutionRepo: Send + Sync {
    /// Returns current state snapshot and CAS version for an execution.
    async fn get_state(
        &self,
        id: ExecutionId,
    ) -> Result<Option<(u64, serde_json::Value)>, ExecutionRepoError>;

    /// Applies a state transition if `expected_version` matches current version.
    ///
    /// Returns `Ok(true)` when transition is committed, `Ok(false)` on CAS mismatch.
    async fn transition(
        &self,
        id: ExecutionId,
        expected_version: u64,
        new_state: serde_json::Value,
    ) -> Result<bool, ExecutionRepoError>;

    /// Returns full journal entries for an execution.
    async fn get_journal(
        &self,
        id: ExecutionId,
    ) -> Result<Vec<serde_json::Value>, ExecutionRepoError>;

    /// Appends a single journal entry for an execution.
    ///
    /// Implementations must reject unknown `id` (no persisted execution row), matching
    /// `execution_journal.execution_id` foreign-key semantics on Postgres.
    async fn append_journal(
        &self,
        id: ExecutionId,
        entry: serde_json::Value,
    ) -> Result<(), ExecutionRepoError>;

    /// Attempts to acquire execution lease for a holder and TTL.
    ///
    /// Returns `Ok(true)` when acquired, `Ok(false)` when already held.
    async fn acquire_lease(
        &self,
        id: ExecutionId,
        holder: String,
        ttl: Duration,
    ) -> Result<bool, ExecutionRepoError>;

    /// Renews an existing lease held by `holder`.
    ///
    /// Returns `Ok(true)` when renewed, `Ok(false)` when holder mismatches or lease is absent.
    async fn renew_lease(
        &self,
        id: ExecutionId,
        holder: &str,
        ttl: Duration,
    ) -> Result<bool, ExecutionRepoError>;

    /// Releases lease if currently owned by `holder`.
    ///
    /// Returns `Ok(true)` when released, `Ok(false)` otherwise.
    async fn release_lease(
        &self,
        id: ExecutionId,
        holder: &str,
    ) -> Result<bool, ExecutionRepoError>;

    /// Inserts a new execution. Fails if the execution ID already exists.
    async fn create(
        &self,
        id: ExecutionId,
        workflow_id: WorkflowId,
        state: serde_json::Value,
    ) -> Result<(), ExecutionRepoError>;

    /// Persists a single node output for a specific attempt.
    async fn save_node_output(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        attempt: u32,
        output: serde_json::Value,
    ) -> Result<(), ExecutionRepoError>;

    /// Loads the latest output for a node (highest attempt number).
    async fn load_node_output(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
    ) -> Result<Option<serde_json::Value>, ExecutionRepoError>;

    /// Loads all node outputs for an execution (latest attempt per node).
    async fn load_all_outputs(
        &self,
        execution_id: ExecutionId,
    ) -> Result<HashMap<NodeKey, serde_json::Value>, ExecutionRepoError>;

    // ── Workflow input (ADR-0008 §3, issue #311 foundation) ────────────────
    //
    // The workflow trigger / start input is persisted alongside the execution
    // row so that resume can replay entry nodes with the original payload.
    // Defaults return `ExecutionRepoError::Internal("not implemented")` so
    // backends that do not yet support the new schema still compile —
    // matching the stateful-checkpoint pattern. B2 (resume consumer chip)
    // wires the read side; B1 only exposes the seam.

    /// Persist the workflow input payload for an execution.
    ///
    /// Idempotent: calling twice overwrites (workflows only set input once
    /// at start in practice). Resume expects a single canonical value.
    async fn set_workflow_input(
        &self,
        execution_id: ExecutionId,
        input: serde_json::Value,
    ) -> Result<(), ExecutionRepoError> {
        let _ = (execution_id, input);
        Err(ExecutionRepoError::Internal(
            "set_workflow_input not implemented for this backend".to_owned(),
        ))
    }

    /// Load the persisted workflow input for an execution.
    ///
    /// Returns `Ok(None)` when no input has been persisted yet — the caller
    /// (engine resume path, B2) converts `None` into a typed resume failure
    /// so missing input is loud, not a silent `Value::Null` default
    /// (ADR-0008 §3, PRODUCT_CANON §4.5).
    async fn get_workflow_input(
        &self,
        execution_id: ExecutionId,
    ) -> Result<Option<serde_json::Value>, ExecutionRepoError> {
        let _ = execution_id;
        Err(ExecutionRepoError::Internal(
            "get_workflow_input not implemented for this backend".to_owned(),
        ))
    }

    // ── Node results (ADR-0008 §1, issue #299 foundation) ──────────────────
    //
    // Persists the full `ActionResult<Value>` variant per node attempt so
    // that resume can replay edge decisions through the engine's own
    // `evaluate_edge` path. B3 (resume writer) and B4 (resume reader) land
    // the consumers; B1 only exposes the seam.

    /// Persist a full node-result record for a specific attempt.
    ///
    /// Overwrites on duplicate `(execution_id, node_key, attempt)` — the
    /// latest write wins. Callers must ensure they do not attempt to persist
    /// a record whose `schema_version` exceeds
    /// [`MAX_SUPPORTED_RESULT_SCHEMA_VERSION`]; the repo stores whatever it is
    /// given.
    async fn save_node_result(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        attempt: u32,
        record: NodeResultRecord,
    ) -> Result<(), ExecutionRepoError> {
        let _ = (execution_id, node_key, attempt, record);
        Err(ExecutionRepoError::Internal(
            "save_node_result not implemented for this backend".to_owned(),
        ))
    }

    /// Load the latest node-result record for a node (highest attempt).
    ///
    /// Returns `Ok(None)` when no record exists — either the node has not
    /// been dispatched yet, or the row predates the new schema. Returns
    /// [`ExecutionRepoError::UnknownSchemaVersion`] when a persisted row
    /// carries a `result_schema_version` greater than
    /// [`MAX_SUPPORTED_RESULT_SCHEMA_VERSION`]; the caller surfaces this as
    /// a resume failure rather than falling back (ADR-0008 §2).
    async fn load_node_result(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
    ) -> Result<Option<NodeResultRecord>, ExecutionRepoError> {
        let _ = (execution_id, node_key);
        Err(ExecutionRepoError::Internal(
            "load_node_result not implemented for this backend".to_owned(),
        ))
    }

    /// Load all latest-attempt node-result records for an execution.
    ///
    /// Same error discipline as [`load_node_result`]: unknown schema
    /// versions surface as [`ExecutionRepoError::UnknownSchemaVersion`],
    /// never silently fall back. Missing rows (legacy or not yet
    /// dispatched) are simply absent from the returned map.
    ///
    /// [`load_node_result`]: ExecutionRepo::load_node_result
    async fn load_all_results(
        &self,
        execution_id: ExecutionId,
    ) -> Result<HashMap<NodeKey, NodeResultRecord>, ExecutionRepoError> {
        let _ = execution_id;
        Err(ExecutionRepoError::Internal(
            "load_all_results not implemented for this backend".to_owned(),
        ))
    }

    /// Lists execution IDs in non-terminal states.
    async fn list_running(&self) -> Result<Vec<ExecutionId>, ExecutionRepoError>;

    /// Counts executions, optionally filtered by workflow_id.
    async fn count(&self, workflow_id: Option<WorkflowId>) -> Result<u64, ExecutionRepoError>;

    /// Returns true if this idempotency key has been recorded.
    async fn check_idempotency(&self, key: &str) -> Result<bool, ExecutionRepoError>;

    /// Records an idempotency key. No-op if key already exists.
    async fn mark_idempotent(
        &self,
        key: &str,
        execution_id: ExecutionId,
        node_key: NodeKey,
    ) -> Result<(), ExecutionRepoError>;

    // ── Stateful action checkpoints (#308) ──────────────────────────────────
    //
    // The runtime persists `(iteration, state)` at every iteration boundary
    // of a stateful action so that a process restart can resume from the
    // last completed iteration instead of re-running from `init_state`.
    //
    // Defaults return `ExecutionRepoError::Internal("not implemented")` so
    // backends that do not yet support stateful resumption (the stubbed
    // Postgres backend today) still compile. The runtime surfaces load
    // errors as WARN and falls back to `init_state` — no silent swallowing.

    /// Persist a stateful iteration checkpoint.
    ///
    /// The key is `(execution_id, node_key, attempt)`. Calling twice with
    /// the same key overwrites the prior checkpoint — only the latest
    /// iteration boundary is persisted.
    async fn save_stateful_checkpoint(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        attempt: u32,
        iteration: u32,
        state: serde_json::Value,
    ) -> Result<(), ExecutionRepoError> {
        let _ = (execution_id, node_key, attempt, iteration, state);
        Err(ExecutionRepoError::Internal(
            "save_stateful_checkpoint not implemented for this backend".to_owned(),
        ))
    }

    /// Load the latest stateful checkpoint for a `(execution, node, attempt)`.
    ///
    /// Returns `Ok(None)` when no checkpoint exists yet (fresh dispatch).
    /// Returns `Err` when the backend cannot be queried (connection,
    /// serialization, etc.) — the runtime logs WARN with full
    /// `(action_key, execution_id, node_key)` context and falls through to
    /// `init_state` so iteration progress loss is visible, never swallowed.
    async fn load_stateful_checkpoint(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        attempt: u32,
    ) -> Result<Option<StatefulCheckpointRecord>, ExecutionRepoError> {
        let _ = (execution_id, node_key, attempt);
        Err(ExecutionRepoError::Internal(
            "load_stateful_checkpoint not implemented for this backend".to_owned(),
        ))
    }

    /// Delete the stateful checkpoint for `(execution, node, attempt)`.
    ///
    /// Called when the stateful action reaches a terminal iteration
    /// (`Break`, `Success`, `Skip`, etc.). Implementations should treat a
    /// missing row as success — the caller only needs "the row is gone".
    async fn delete_stateful_checkpoint(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        attempt: u32,
    ) -> Result<(), ExecutionRepoError> {
        let _ = (execution_id, node_key, attempt);
        Err(ExecutionRepoError::Internal(
            "delete_stateful_checkpoint not implemented for this backend".to_owned(),
        ))
    }
}

/// Highest node-result schema version this binary can decode.
///
/// Bumped whenever a change to `ActionResult` (or the [`NodeResultRecord`]
/// shape) could make an older binary fail to decode a record written by a
/// newer one — new variants, new required fields, changed field semantics
/// (ADR-0008 §2). Records with a higher `schema_version` cause
/// [`ExecutionRepoError::UnknownSchemaVersion`] on load, never a silent
/// fall-back.
pub const MAX_SUPPORTED_RESULT_SCHEMA_VERSION: u32 = 1;

/// A persisted node-result record carrying the full `ActionResult<Value>`
/// variant for a node attempt.
///
/// Storage-side mirror of `nebula_action::ActionResult<Value>` — the repo
/// crate does not depend on `nebula-action`, so the variant lives in a
/// neutral JSON blob plus a `kind` tag for SQL-side filtering and a
/// `schema_version` for forward-compat guarding.
///
/// ADR-0008 §1 pins this as the canonical persistence shape for issue #299
/// (resume reconstructs `ActionResult::Branch` / `Route` / `MultiOutput` /
/// `Skip` / `Wait` through the engine's own `evaluate_edge` path, not via
/// synthesized `Success`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NodeResultRecord {
    /// Schema version this record was written under.
    ///
    /// `1` is the initial shape; future changes bump per ADR-0008 §2.
    pub schema_version: u32,
    /// Variant tag (`"Success"`, `"Branch"`, `"Route"`, `"MultiOutput"`,
    /// `"Skip"`, `"Wait"`, `"Retry"`, `"Break"`, `"Continue"`, `"Drop"`,
    /// `"Terminate"`). Mirrors the serde tag on `ActionResult`.
    pub kind: String,
    /// Serialized `ActionResult<Value>` as emitted by `serde_json`.
    pub result: serde_json::Value,
}

impl NodeResultRecord {
    /// Build a new record at the current schema version.
    #[must_use]
    pub fn new(kind: impl Into<String>, result: serde_json::Value) -> Self {
        Self {
            schema_version: MAX_SUPPORTED_RESULT_SCHEMA_VERSION,
            kind: kind.into(),
            result,
        }
    }

    /// Build a record at an explicit schema version (for tests / migration
    /// fixtures that need to pin older shapes).
    #[must_use]
    pub fn with_version(
        schema_version: u32,
        kind: impl Into<String>,
        result: serde_json::Value,
    ) -> Self {
        Self {
            schema_version,
            kind: kind.into(),
            result,
        }
    }
}

/// A persisted stateful iteration checkpoint.
///
/// Storage-side mirror of the runtime's `StatefulCheckpoint` — separate
/// types so that the runtime crate does not depend on `nebula-storage`.
/// The engine glue converts between the two shapes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatefulCheckpointRecord {
    /// Number of completed iterations when the checkpoint was written.
    pub iteration: u32,
    /// Serialized handler state (opaque JSON from the handler's point of view).
    pub state: serde_json::Value,
}

impl StatefulCheckpointRecord {
    /// Build a new record.
    #[must_use]
    pub fn new(iteration: u32, state: serde_json::Value) -> Self {
        Self { iteration, state }
    }
}

/// Key for node output storage: `(execution_id, node_key, attempt)`.
type NodeOutputKey = (ExecutionId, NodeKey, u32);

/// Key for stateful checkpoint storage: `(execution_id, node_key, attempt)`.
type StatefulCheckpointKey = (ExecutionId, NodeKey, u32);

/// Lease entry: `(holder, expires_at)`. Expiration is monotonic via
/// [`tokio::time::Instant`] so paused-time tests behave deterministically.
type LeaseEntry = (String, Instant);

/// Normalize lease TTL to the same safe range as Postgres backend.
fn normalized_lease_ttl(ttl: Duration) -> Duration {
    Duration::from_secs_f64(ttl.as_secs_f64().clamp(1.0, 86_400.0))
}

/// Compute lease expiration instant without panicking on overflow.
fn lease_expires_at(now: Instant, ttl: Duration) -> Instant {
    now.checked_add(normalized_lease_ttl(ttl)).unwrap_or(now)
}

/// In-memory execution repository for tests and single-process/health-only mode.
#[derive(Default)]
pub struct InMemoryExecutionRepo {
    state: Arc<RwLock<HashMap<ExecutionId, (u64, serde_json::Value)>>>,
    journal: Arc<RwLock<HashMap<ExecutionId, Vec<serde_json::Value>>>>,
    leases: Arc<RwLock<HashMap<ExecutionId, LeaseEntry>>>,
    workflows: Arc<RwLock<HashMap<ExecutionId, WorkflowId>>>,
    node_outputs: Arc<RwLock<HashMap<NodeOutputKey, serde_json::Value>>>,
    idempotency: Arc<RwLock<HashSet<String>>>,
    stateful_checkpoints: Arc<RwLock<HashMap<StatefulCheckpointKey, StatefulCheckpointRecord>>>,
    workflow_inputs: Arc<RwLock<HashMap<ExecutionId, serde_json::Value>>>,
    node_results: Arc<RwLock<HashMap<NodeOutputKey, NodeResultRecord>>>,
}

impl InMemoryExecutionRepo {
    /// Creates empty in-memory repository.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ExecutionRepo for InMemoryExecutionRepo {
    async fn get_state(
        &self,
        id: ExecutionId,
    ) -> Result<Option<(u64, serde_json::Value)>, ExecutionRepoError> {
        let state = self.state.read().await;
        Ok(state.get(&id).cloned())
    }

    async fn transition(
        &self,
        id: ExecutionId,
        expected_version: u64,
        new_state: serde_json::Value,
    ) -> Result<bool, ExecutionRepoError> {
        let mut state = self.state.write().await;
        // Keep parity with Postgres UPDATE ... WHERE id = $1 AND version = $2:
        // unknown execution IDs are treated as "no row updated" (false),
        // not implicitly created at version 1.
        let Some((current, _)) = state.get(&id) else {
            return Ok(false);
        };
        if *current != expected_version {
            return Ok(false);
        }
        state.insert(id, (expected_version + 1, new_state));
        Ok(true)
    }

    async fn get_journal(
        &self,
        id: ExecutionId,
    ) -> Result<Vec<serde_json::Value>, ExecutionRepoError> {
        let journal = self.journal.read().await;
        Ok(journal.get(&id).cloned().unwrap_or_default())
    }

    async fn append_journal(
        &self,
        id: ExecutionId,
        entry: serde_json::Value,
    ) -> Result<(), ExecutionRepoError> {
        let state = self.state.read().await;
        let workflows = self.workflows.read().await;
        if !state.contains_key(&id) && !workflows.contains_key(&id) {
            return Err(ExecutionRepoError::not_found("execution", id.to_string()));
        }
        drop((state, workflows));
        let mut journal = self.journal.write().await;
        journal.entry(id).or_default().push(entry);
        Ok(())
    }

    async fn acquire_lease(
        &self,
        id: ExecutionId,
        holder: String,
        ttl: Duration,
    ) -> Result<bool, ExecutionRepoError> {
        let mut leases = self.leases.write().await;
        let now = Instant::now();
        if let Some((_, expires_at)) = leases.get(&id)
            && *expires_at >= now
        {
            return Ok(false);
        }
        leases.insert(id, (holder, lease_expires_at(now, ttl)));
        Ok(true)
    }

    async fn renew_lease(
        &self,
        id: ExecutionId,
        holder: &str,
        ttl: Duration,
    ) -> Result<bool, ExecutionRepoError> {
        let mut leases = self.leases.write().await;
        let now = Instant::now();
        match leases.get_mut(&id) {
            Some((current, expires_at)) if current == holder => {
                *expires_at = lease_expires_at(now, ttl);
                Ok(true)
            },
            _ => Ok(false),
        }
    }

    async fn release_lease(
        &self,
        id: ExecutionId,
        holder: &str,
    ) -> Result<bool, ExecutionRepoError> {
        let mut leases = self.leases.write().await;
        if let Some((current, _)) = leases.get(&id)
            && current == holder
        {
            leases.remove(&id);
            return Ok(true);
        }
        Ok(false)
    }

    async fn create(
        &self,
        id: ExecutionId,
        workflow_id: WorkflowId,
        state: serde_json::Value,
    ) -> Result<(), ExecutionRepoError> {
        let mut store = self.state.write().await;
        if let Some((actual_version, _)) = store.get(&id) {
            return Err(ExecutionRepoError::Conflict {
                entity: "execution".into(),
                id: id.to_string(),
                expected_version: 0,
                actual_version: *actual_version,
            });
        }
        store.insert(id, (1, state));
        drop(store);
        self.workflows.write().await.insert(id, workflow_id);
        Ok(())
    }

    async fn save_node_output(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        attempt: u32,
        output: serde_json::Value,
    ) -> Result<(), ExecutionRepoError> {
        self.node_outputs
            .write()
            .await
            .insert((execution_id, node_key, attempt), output);
        Ok(())
    }

    async fn load_node_output(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
    ) -> Result<Option<serde_json::Value>, ExecutionRepoError> {
        let outputs = self.node_outputs.read().await;
        let best = outputs
            .iter()
            .filter(|((eid, nid, _), _)| *eid == execution_id && *nid == node_key)
            .max_by_key(|((_, _, attempt), _)| *attempt)
            .map(|(_, v)| v.clone());
        Ok(best)
    }

    async fn load_all_outputs(
        &self,
        execution_id: ExecutionId,
    ) -> Result<HashMap<NodeKey, serde_json::Value>, ExecutionRepoError> {
        let outputs = self.node_outputs.read().await;
        let mut best: HashMap<NodeKey, (u32, serde_json::Value)> = HashMap::new();
        for ((eid, nid, attempt), val) in outputs.iter() {
            if *eid != execution_id {
                continue;
            }
            let entry = best.entry(nid.clone()).or_insert((*attempt, val.clone()));
            if *attempt > entry.0 {
                *entry = (*attempt, val.clone());
            }
        }
        Ok(best.into_iter().map(|(nid, (_, v))| (nid, v)).collect())
    }

    async fn list_running(&self) -> Result<Vec<ExecutionId>, ExecutionRepoError> {
        let state = self.state.read().await;
        let running = state
            .iter()
            .filter(|(_, (_, val))| {
                matches!(
                    val.get("status").and_then(|s| s.as_str()),
                    Some("created" | "running" | "paused" | "cancelling")
                )
            })
            .map(|(id, _)| *id)
            .collect();
        Ok(running)
    }

    async fn count(&self, workflow_id: Option<WorkflowId>) -> Result<u64, ExecutionRepoError> {
        let Some(wid) = workflow_id else {
            let state = self.state.read().await;
            return Ok(state.len() as u64);
        };
        let workflows = self.workflows.read().await;
        let n = workflows.values().filter(|v| **v == wid).count() as u64;
        Ok(n)
    }

    async fn check_idempotency(&self, key: &str) -> Result<bool, ExecutionRepoError> {
        let set = self.idempotency.read().await;
        Ok(set.contains(key))
    }

    async fn mark_idempotent(
        &self,
        key: &str,
        execution_id: ExecutionId,
        _node_id: NodeKey,
    ) -> Result<(), ExecutionRepoError> {
        let state = self.state.read().await;
        let workflows = self.workflows.read().await;
        if !state.contains_key(&execution_id) && !workflows.contains_key(&execution_id) {
            return Err(ExecutionRepoError::not_found(
                "execution",
                execution_id.to_string(),
            ));
        }
        drop((state, workflows));
        self.idempotency.write().await.insert(key.to_owned());
        Ok(())
    }

    async fn set_workflow_input(
        &self,
        execution_id: ExecutionId,
        input: serde_json::Value,
    ) -> Result<(), ExecutionRepoError> {
        let state = self.state.read().await;
        let workflows = self.workflows.read().await;
        if !state.contains_key(&execution_id) && !workflows.contains_key(&execution_id) {
            return Err(ExecutionRepoError::not_found(
                "execution",
                execution_id.to_string(),
            ));
        }
        drop((state, workflows));
        self.workflow_inputs
            .write()
            .await
            .insert(execution_id, input);
        Ok(())
    }

    async fn get_workflow_input(
        &self,
        execution_id: ExecutionId,
    ) -> Result<Option<serde_json::Value>, ExecutionRepoError> {
        Ok(self
            .workflow_inputs
            .read()
            .await
            .get(&execution_id)
            .cloned())
    }

    async fn save_node_result(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        attempt: u32,
        record: NodeResultRecord,
    ) -> Result<(), ExecutionRepoError> {
        self.node_results
            .write()
            .await
            .insert((execution_id, node_key, attempt), record);
        Ok(())
    }

    async fn load_node_result(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
    ) -> Result<Option<NodeResultRecord>, ExecutionRepoError> {
        let results = self.node_results.read().await;
        let best = results
            .iter()
            .filter(|((eid, nid, _), _)| *eid == execution_id && *nid == node_key)
            .max_by_key(|((_, _, attempt), _)| *attempt)
            .map(|(_, record)| record.clone());
        if let Some(record) = &best
            && record.schema_version > MAX_SUPPORTED_RESULT_SCHEMA_VERSION
        {
            return Err(ExecutionRepoError::UnknownSchemaVersion {
                version: record.schema_version,
                max_supported: MAX_SUPPORTED_RESULT_SCHEMA_VERSION,
            });
        }
        Ok(best)
    }

    async fn load_all_results(
        &self,
        execution_id: ExecutionId,
    ) -> Result<HashMap<NodeKey, NodeResultRecord>, ExecutionRepoError> {
        let results = self.node_results.read().await;
        // Pick highest-attempt record per node first — matching
        // Postgres's `DISTINCT ON (node_id) ... ORDER BY attempt DESC`.
        // Schema-version validation runs only against the chosen finalists
        // so that an older (non-latest) attempt with a future version does
        // not block a load whose latest attempt is well-formed.
        let mut best: HashMap<NodeKey, (u32, NodeResultRecord)> = HashMap::new();
        for ((eid, nid, attempt), record) in results.iter() {
            if *eid != execution_id {
                continue;
            }
            let entry = best
                .entry(nid.clone())
                .or_insert((*attempt, record.clone()));
            if *attempt > entry.0 {
                *entry = (*attempt, record.clone());
            }
        }
        let mut out = HashMap::with_capacity(best.len());
        for (nid, (_, record)) in best {
            if record.schema_version > MAX_SUPPORTED_RESULT_SCHEMA_VERSION {
                return Err(ExecutionRepoError::UnknownSchemaVersion {
                    version: record.schema_version,
                    max_supported: MAX_SUPPORTED_RESULT_SCHEMA_VERSION,
                });
            }
            out.insert(nid, record);
        }
        Ok(out)
    }

    async fn save_stateful_checkpoint(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        attempt: u32,
        iteration: u32,
        state: serde_json::Value,
    ) -> Result<(), ExecutionRepoError> {
        let mut cps = self.stateful_checkpoints.write().await;
        cps.insert(
            (execution_id, node_key, attempt),
            StatefulCheckpointRecord::new(iteration, state),
        );
        Ok(())
    }

    async fn load_stateful_checkpoint(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        attempt: u32,
    ) -> Result<Option<StatefulCheckpointRecord>, ExecutionRepoError> {
        let cps = self.stateful_checkpoints.read().await;
        Ok(cps.get(&(execution_id, node_key, attempt)).cloned())
    }

    async fn delete_stateful_checkpoint(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        attempt: u32,
    ) -> Result<(), ExecutionRepoError> {
        let mut cps = self.stateful_checkpoints.write().await;
        cps.remove(&(execution_id, node_key, attempt));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::node_key;

    use super::*;

    #[tokio::test]
    async fn append_journal_unknown_execution_returns_not_found() {
        let repo = InMemoryExecutionRepo::default();
        let missing = ExecutionId::new();
        let err = repo
            .append_journal(missing, serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ExecutionRepoError::NotFound { ref entity, ref id }
                if entity == "execution" && id == &missing.to_string()
        ));
    }

    #[tokio::test]
    async fn append_journal_succeeds_after_create() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let wid = WorkflowId::new();
        repo.create(eid, wid, serde_json::json!({"status": "created"}))
            .await
            .unwrap();
        let entry = serde_json::json!({"kind": "test"});
        repo.append_journal(eid, entry.clone()).await.unwrap();
        let journal = repo.get_journal(eid).await.unwrap();
        assert_eq!(journal, vec![entry]);
    }

    #[tokio::test]
    async fn create_and_get_state() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let wid = WorkflowId::new();
        let state = serde_json::json!({"status": "created"});
        repo.create(eid, wid, state.clone()).await.unwrap();
        let (version, loaded) = repo.get_state(eid).await.unwrap().unwrap();
        assert_eq!(version, 1);
        assert_eq!(loaded, state);
    }

    #[tokio::test]
    async fn transition_unknown_execution_returns_false_without_creating_row() {
        let repo = InMemoryExecutionRepo::default();
        let missing = ExecutionId::new();

        let updated = repo
            .transition(missing, 0, serde_json::json!({"status": "ghost"}))
            .await
            .expect("transition should not error");
        assert!(
            !updated,
            "unknown execution transition must behave like zero rows affected"
        );
        assert_eq!(
            repo.get_state(missing).await.unwrap(),
            None,
            "transition must not create missing execution rows"
        );
    }

    #[tokio::test]
    async fn create_duplicate_returns_conflict() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let wid = WorkflowId::new();
        let state = serde_json::json!({"status": "created"});
        repo.create(eid, wid, state.clone()).await.unwrap();
        let err = repo.create(eid, wid, state).await.unwrap_err();
        assert!(matches!(err, ExecutionRepoError::Conflict { .. }));
    }

    #[tokio::test]
    async fn node_output_save_and_load() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let nid = node_key!("nid");
        let output = serde_json::json!({"result": 42});
        repo.save_node_output(eid, nid.clone(), 1, output.clone())
            .await
            .unwrap();
        let loaded = repo.load_node_output(eid, nid).await.unwrap();
        assert_eq!(loaded, Some(output));
    }

    #[tokio::test]
    async fn node_output_returns_latest_attempt() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let nid = node_key!("nid");
        repo.save_node_output(eid, nid.clone(), 1, serde_json::json!("first"))
            .await
            .unwrap();
        repo.save_node_output(eid, nid.clone(), 2, serde_json::json!("second"))
            .await
            .unwrap();
        let loaded = repo.load_node_output(eid, nid).await.unwrap();
        assert_eq!(loaded, Some(serde_json::json!("second")));
    }

    #[tokio::test]
    async fn load_all_outputs_returns_latest_per_node() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let n1 = node_key!("n1");
        let n2 = node_key!("n2");
        repo.save_node_output(eid, n1.clone(), 1, serde_json::json!("n1_v1"))
            .await
            .unwrap();
        repo.save_node_output(eid, n1.clone(), 2, serde_json::json!("n1_v2"))
            .await
            .unwrap();
        repo.save_node_output(eid, n2.clone(), 1, serde_json::json!("n2_v1"))
            .await
            .unwrap();
        let all = repo.load_all_outputs(eid).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[&n1], serde_json::json!("n1_v2"));
        assert_eq!(all[&n2], serde_json::json!("n2_v1"));
    }

    #[tokio::test]
    async fn idempotency_check_and_mark() {
        let repo = InMemoryExecutionRepo::default();
        let key = "exec1:node1:1";
        let eid = ExecutionId::new();
        let wid = WorkflowId::new();
        repo.create(eid, wid, serde_json::json!({"status": "created"}))
            .await
            .unwrap();
        assert!(!repo.check_idempotency(key).await.unwrap());
        repo.mark_idempotent(key, eid, node_key!("test"))
            .await
            .unwrap();
        assert!(repo.check_idempotency(key).await.unwrap());
    }

    #[tokio::test]
    async fn mark_idempotent_unknown_execution_returns_not_found() {
        let repo = InMemoryExecutionRepo::default();
        let missing = ExecutionId::new();
        let err = repo
            .mark_idempotent("k", missing, node_key!("test"))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ExecutionRepoError::NotFound { ref entity, ref id }
                if entity == "execution" && id == &missing.to_string()
        ));
    }

    // ── #308 regression: stateful checkpoint round-trips ───────────────────

    #[tokio::test]
    async fn stateful_checkpoint_save_load_round_trip() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let nid = node_key!("nid");
        let state = serde_json::json!({"cursor": "page-2", "count": 42u32});

        // Empty before first save.
        assert_eq!(
            repo.load_stateful_checkpoint(eid, nid.clone(), 0)
                .await
                .unwrap(),
            None
        );

        repo.save_stateful_checkpoint(eid, nid.clone(), 0, 5, state.clone())
            .await
            .unwrap();
        let loaded = repo
            .load_stateful_checkpoint(eid, nid, 0)
            .await
            .unwrap()
            .expect("checkpoint must exist after save");
        assert_eq!(loaded.iteration, 5);
        assert_eq!(loaded.state, state);
    }

    #[tokio::test]
    async fn stateful_checkpoint_delete_removes_row() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let nid = node_key!("nid");

        repo.save_stateful_checkpoint(eid, nid.clone(), 2, 1, serde_json::json!({"k": 1}))
            .await
            .unwrap();
        assert!(
            repo.load_stateful_checkpoint(eid, nid.clone(), 2)
                .await
                .unwrap()
                .is_some()
        );
        repo.delete_stateful_checkpoint(eid, nid.clone(), 2)
            .await
            .unwrap();
        assert_eq!(
            repo.load_stateful_checkpoint(eid, nid.clone(), 2)
                .await
                .unwrap(),
            None
        );
        // Double-delete is idempotent.
        repo.delete_stateful_checkpoint(eid, nid, 2).await.unwrap();
    }

    #[tokio::test]
    async fn stateful_checkpoint_is_scoped_by_attempt() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let nid = node_key!("nid");

        repo.save_stateful_checkpoint(eid, nid.clone(), 0, 1, serde_json::json!("attempt-0"))
            .await
            .unwrap();
        repo.save_stateful_checkpoint(eid, nid.clone(), 1, 1, serde_json::json!("attempt-1"))
            .await
            .unwrap();

        let a0 = repo
            .load_stateful_checkpoint(eid, nid.clone(), 0)
            .await
            .unwrap()
            .unwrap();
        let a1 = repo
            .load_stateful_checkpoint(eid, nid.clone(), 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(a0.state, serde_json::json!("attempt-0"));
        assert_eq!(a1.state, serde_json::json!("attempt-1"));

        // Deleting one attempt does not touch the other.
        repo.delete_stateful_checkpoint(eid, nid.clone(), 0)
            .await
            .unwrap();
        assert_eq!(
            repo.load_stateful_checkpoint(eid, nid.clone(), 0)
                .await
                .unwrap(),
            None
        );
        assert!(
            repo.load_stateful_checkpoint(eid, nid, 1)
                .await
                .unwrap()
                .is_some()
        );
    }

    // ── #303 lease TTL semantics (batch 5C) ───────────────────────────────

    #[tokio::test(start_paused = true)]
    async fn lease_acquire_blocks_other_holder_until_expiry() {
        let repo = InMemoryExecutionRepo::default();
        let id = ExecutionId::new();
        assert!(
            repo.acquire_lease(id, "A".into(), Duration::from_secs(5))
                .await
                .expect("acquire A")
        );
        // While A is still valid, B cannot steal.
        assert!(
            !repo
                .acquire_lease(id, "B".into(), Duration::from_secs(5))
                .await
                .expect("acquire B during A")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn lease_expires_after_ttl_allowing_reacquire() {
        let repo = InMemoryExecutionRepo::default();
        let id = ExecutionId::new();
        assert!(
            repo.acquire_lease(id, "A".into(), Duration::from_secs(5))
                .await
                .expect("acquire A")
        );
        tokio::time::advance(Duration::from_secs(6)).await;
        // A's stale lease must not block B.
        assert!(
            repo.acquire_lease(id, "B".into(), Duration::from_secs(5))
                .await
                .expect("reacquire by B")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn lease_expiring_now_is_still_live_until_time_advances() {
        let repo = InMemoryExecutionRepo::default();
        let id = ExecutionId::new();
        assert!(
            repo.acquire_lease(id, "A".into(), Duration::from_secs(5))
                .await
                .expect("acquire A")
        );
        tokio::time::advance(Duration::from_secs(5)).await;
        assert!(
            !repo
                .acquire_lease(id, "B".into(), Duration::from_secs(5))
                .await
                .expect("B at exact expiry boundary")
        );
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(
            repo.acquire_lease(id, "B".into(), Duration::from_secs(5))
                .await
                .expect("B after boundary")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn lease_renew_extends_expiry() {
        let repo = InMemoryExecutionRepo::default();
        let id = ExecutionId::new();
        assert!(
            repo.acquire_lease(id, "A".into(), Duration::from_secs(5))
                .await
                .expect("acquire")
        );
        tokio::time::advance(Duration::from_secs(3)).await;
        assert!(
            repo.renew_lease(id, "A", Duration::from_secs(5))
                .await
                .expect("renew")
        );
        tokio::time::advance(Duration::from_secs(3)).await;
        // 6s elapsed since acquire, but renew at 3s pushed expiry to 8s.
        // B must still see A's lease as live.
        assert!(
            !repo
                .acquire_lease(id, "B".into(), Duration::from_secs(5))
                .await
                .expect("steal attempt")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn lease_renew_rejects_wrong_holder() {
        let repo = InMemoryExecutionRepo::default();
        let id = ExecutionId::new();
        assert!(
            repo.acquire_lease(id, "A".into(), Duration::from_secs(5))
                .await
                .expect("acquire")
        );
        assert!(
            !repo
                .renew_lease(id, "B", Duration::from_secs(5))
                .await
                .expect("renew by B")
        );
        // A's lease must remain effective.
        assert!(
            !repo
                .acquire_lease(id, "C".into(), Duration::from_secs(5))
                .await
                .expect("acquire by C")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn lease_renew_allows_expired_lease_for_same_holder() {
        let repo = InMemoryExecutionRepo::default();
        let id = ExecutionId::new();
        assert!(
            repo.acquire_lease(id, "A".into(), Duration::from_secs(5))
                .await
                .expect("acquire A")
        );
        tokio::time::advance(Duration::from_secs(6)).await;
        assert!(
            repo.renew_lease(id, "A", Duration::from_secs(5))
                .await
                .expect("renew after expiry by same holder")
        );
        assert!(
            !repo
                .acquire_lease(id, "B".into(), Duration::from_secs(5))
                .await
                .expect("B after A renew")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn lease_ttl_is_clamped_to_at_least_one_second() {
        let repo = InMemoryExecutionRepo::default();
        let id = ExecutionId::new();
        assert!(
            repo.acquire_lease(id, "A".into(), Duration::ZERO)
                .await
                .expect("acquire A")
        );
        assert!(
            !repo
                .acquire_lease(id, "B".into(), Duration::from_secs(5))
                .await
                .expect("B immediately")
        );
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(
            !repo
                .acquire_lease(id, "B".into(), Duration::from_secs(5))
                .await
                .expect("B at one-second boundary")
        );
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(
            repo.acquire_lease(id, "B".into(), Duration::from_secs(5))
                .await
                .expect("B after one-second minimum ttl")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn lease_release_validates_holder() {
        let repo = InMemoryExecutionRepo::default();
        let id = ExecutionId::new();
        assert!(
            repo.acquire_lease(id, "A".into(), Duration::from_secs(5))
                .await
                .expect("acquire")
        );
        // Wrong holder cannot release.
        assert!(!repo.release_lease(id, "B").await.expect("release by B"));
        // Correct holder can release.
        assert!(repo.release_lease(id, "A").await.expect("release by A"));
        // After release, anyone may acquire.
        assert!(
            repo.acquire_lease(id, "C".into(), Duration::from_secs(5))
                .await
                .expect("reacquire by C")
        );
    }

    // ── ADR-0008 B1: workflow input + node-result persistence ─────────────

    /// Fixture: a sample JSON shape for each `ActionResult` variant as
    /// `serde_json` would emit it. The storage layer treats these as opaque
    /// blobs; the round-trip test just asserts byte-equivalence.
    fn variant_fixtures() -> Vec<(&'static str, serde_json::Value)> {
        vec![
            (
                "Success",
                serde_json::json!({
                    "type": "Success",
                    "output": {"Value": {"answer": 42}},
                }),
            ),
            (
                "Skip",
                serde_json::json!({
                    "type": "Skip",
                    "reason": "filtered",
                    "output": {"Value": {"id": "abc"}},
                }),
            ),
            (
                "Drop",
                serde_json::json!({
                    "type": "Drop",
                    "reason": "rate limited",
                }),
            ),
            (
                "Continue",
                serde_json::json!({
                    "type": "Continue",
                    "output": {"Value": {"page": 2}},
                    "progress": 0.5,
                    "delay": 1000,
                }),
            ),
            (
                "Break",
                serde_json::json!({
                    "type": "Break",
                    "output": {"Value": {"total": 100}},
                    "reason": "Completed",
                }),
            ),
            (
                "Branch",
                serde_json::json!({
                    "type": "Branch",
                    "selected": "true",
                    "output": {"Value": {"matched": true}},
                    "alternatives": {
                        "false": {"Value": {"matched": false}},
                    },
                }),
            ),
            (
                "Route",
                serde_json::json!({
                    "type": "Route",
                    "port": "error",
                    "data": {"Value": {"code": "E_BAD"}},
                }),
            ),
            (
                "MultiOutput",
                serde_json::json!({
                    "type": "MultiOutput",
                    "outputs": {
                        "main": {"Value": 1},
                        "audit": {"Value": 2},
                    },
                    "main_output": {"Value": 1},
                }),
            ),
            (
                "Wait",
                serde_json::json!({
                    "type": "Wait",
                    "condition": {
                        "type": "Duration",
                        "duration": 60000,
                    },
                    "timeout": 300000,
                    "partial_output": null,
                }),
            ),
            // Retry is slated for removal in chip E1; its JSON shape lives
            // under the same `type`-tagged schema and must round-trip
            // until the variant is gone. Drop from this fixture list when
            // E1 lands and the schema version bumps to 2.
            (
                "Retry",
                serde_json::json!({
                    "type": "Retry",
                    "after": 5000,
                    "reason": "rate-limited",
                }),
            ),
            (
                "Terminate",
                serde_json::json!({
                    "type": "Terminate",
                    "reason": {"type": "Success", "note": "done early"},
                }),
            ),
        ]
    }

    #[tokio::test]
    async fn node_result_round_trips_every_action_result_variant() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let wid = WorkflowId::new();
        repo.create(eid, wid, serde_json::json!({"status": "created"}))
            .await
            .unwrap();

        for (idx, (kind, result_json)) in variant_fixtures().into_iter().enumerate() {
            let node = node_key!("n");
            let record = NodeResultRecord::new(kind, result_json.clone());
            let attempt = u32::try_from(idx).unwrap();

            repo.save_node_result(eid, node.clone(), attempt, record.clone())
                .await
                .unwrap();

            let loaded = repo
                .load_node_result(eid, node)
                .await
                .unwrap()
                .expect("record must exist after save");
            assert_eq!(
                loaded.schema_version, MAX_SUPPORTED_RESULT_SCHEMA_VERSION,
                "{kind}: schema_version must be the current default",
            );
            assert_eq!(loaded.kind, kind, "{kind}: kind must round-trip");
            assert_eq!(
                loaded.result, result_json,
                "{kind}: result JSON must round-trip byte-equivalent",
            );
        }
    }

    #[tokio::test]
    async fn load_node_result_returns_latest_attempt() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let node = node_key!("n");

        repo.save_node_result(
            eid,
            node.clone(),
            0,
            NodeResultRecord::new("Success", serde_json::json!({"v": 0})),
        )
        .await
        .unwrap();
        repo.save_node_result(
            eid,
            node.clone(),
            1,
            NodeResultRecord::new("Success", serde_json::json!({"v": 1})),
        )
        .await
        .unwrap();

        let loaded = repo.load_node_result(eid, node).await.unwrap().unwrap();
        assert_eq!(loaded.result, serde_json::json!({"v": 1}));
    }

    #[tokio::test]
    async fn load_all_results_returns_latest_per_node() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let n1 = node_key!("n1");
        let n2 = node_key!("n2");

        repo.save_node_result(
            eid,
            n1.clone(),
            0,
            NodeResultRecord::new("Success", serde_json::json!("n1_v0")),
        )
        .await
        .unwrap();
        repo.save_node_result(
            eid,
            n1.clone(),
            1,
            NodeResultRecord::new("Branch", serde_json::json!("n1_v1")),
        )
        .await
        .unwrap();
        repo.save_node_result(
            eid,
            n2.clone(),
            0,
            NodeResultRecord::new("Skip", serde_json::json!("n2_v0")),
        )
        .await
        .unwrap();

        let all = repo.load_all_results(eid).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[&n1].kind, "Branch");
        assert_eq!(all[&n1].result, serde_json::json!("n1_v1"));
        assert_eq!(all[&n2].kind, "Skip");
    }

    #[tokio::test]
    async fn load_node_result_surfaces_unknown_schema_version_as_typed_error() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let node = node_key!("future");

        let future_record = NodeResultRecord::with_version(
            MAX_SUPPORTED_RESULT_SCHEMA_VERSION + 1,
            "FutureVariant",
            serde_json::json!({"type": "FutureVariant"}),
        );
        repo.save_node_result(eid, node.clone(), 0, future_record)
            .await
            .unwrap();

        let err = repo
            .load_node_result(eid, node)
            .await
            .expect_err("unknown schema version must not fall back");
        match err {
            ExecutionRepoError::UnknownSchemaVersion {
                version,
                max_supported,
            } => {
                assert_eq!(version, MAX_SUPPORTED_RESULT_SCHEMA_VERSION + 1);
                assert_eq!(max_supported, MAX_SUPPORTED_RESULT_SCHEMA_VERSION);
            },
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn load_all_results_surfaces_unknown_schema_version_as_typed_error() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();

        repo.save_node_result(
            eid,
            node_key!("ok"),
            0,
            NodeResultRecord::new("Success", serde_json::json!({})),
        )
        .await
        .unwrap();
        repo.save_node_result(
            eid,
            node_key!("future"),
            0,
            NodeResultRecord::with_version(
                MAX_SUPPORTED_RESULT_SCHEMA_VERSION + 5,
                "Far",
                serde_json::json!({}),
            ),
        )
        .await
        .unwrap();

        let err = repo
            .load_all_results(eid)
            .await
            .expect_err("mixed batch with unknown version must error");
        assert!(matches!(
            err,
            ExecutionRepoError::UnknownSchemaVersion { .. }
        ));
    }

    #[tokio::test]
    async fn load_all_results_ignores_future_version_on_non_latest_attempt() {
        // A node retried after a rollback: attempt 0 was written by a newer
        // binary (unknown schema_version), attempt 1 is fresh and valid.
        // `load_all_results` must surface only the latest attempt per node,
        // so the future-version attempt 0 is not reachable and must not
        // poison the whole batch — matching `load_node_result` and
        // `PgExecutionRepo::load_all_results` (`DISTINCT ON ... ORDER BY
        // attempt DESC`).
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let nid = node_key!("n");

        repo.save_node_result(
            eid,
            nid.clone(),
            0,
            NodeResultRecord::with_version(
                MAX_SUPPORTED_RESULT_SCHEMA_VERSION + 7,
                "FutureStale",
                serde_json::json!({}),
            ),
        )
        .await
        .unwrap();
        repo.save_node_result(
            eid,
            nid.clone(),
            1,
            NodeResultRecord::new("Success", serde_json::json!({"ok": true})),
        )
        .await
        .unwrap();

        let all = repo.load_all_results(eid).await.expect(
            "latest attempt is decodable; earlier future-version attempt must be invisible",
        );
        assert_eq!(all.len(), 1);
        assert_eq!(all[&nid].kind, "Success");
    }

    #[tokio::test]
    async fn load_node_result_none_when_no_record() {
        let repo = InMemoryExecutionRepo::default();
        let loaded = repo
            .load_node_result(ExecutionId::new(), node_key!("n"))
            .await
            .unwrap();
        assert_eq!(loaded, None);
    }

    #[tokio::test]
    async fn workflow_input_round_trip() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let wid = WorkflowId::new();
        repo.create(eid, wid, serde_json::json!({"status": "created"}))
            .await
            .unwrap();

        assert_eq!(
            repo.get_workflow_input(eid).await.unwrap(),
            None,
            "unset input must be None, never a synthesized Null"
        );

        let input = serde_json::json!({"trigger": "http", "payload": {"x": 1}});
        repo.set_workflow_input(eid, input.clone()).await.unwrap();

        assert_eq!(repo.get_workflow_input(eid).await.unwrap(), Some(input));
    }

    #[tokio::test]
    async fn set_workflow_input_rejects_unknown_execution() {
        let repo = InMemoryExecutionRepo::default();
        let missing = ExecutionId::new();
        let err = repo
            .set_workflow_input(missing, serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ExecutionRepoError::NotFound { ref entity, ref id }
                if entity == "execution" && id == &missing.to_string()
        ));
    }

    #[tokio::test]
    async fn get_workflow_input_returns_none_for_unknown_execution() {
        let repo = InMemoryExecutionRepo::default();
        let got = repo.get_workflow_input(ExecutionId::new()).await.unwrap();
        assert_eq!(
            got, None,
            "get_workflow_input is a read seam: unknown id is None, not NotFound; \
             resume caller decides whether missing = error",
        );
    }

    #[tokio::test]
    async fn set_workflow_input_overwrites() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let wid = WorkflowId::new();
        repo.create(eid, wid, serde_json::json!({"status": "created"}))
            .await
            .unwrap();

        repo.set_workflow_input(eid, serde_json::json!({"v": 1}))
            .await
            .unwrap();
        repo.set_workflow_input(eid, serde_json::json!({"v": 2}))
            .await
            .unwrap();

        assert_eq!(
            repo.get_workflow_input(eid).await.unwrap(),
            Some(serde_json::json!({"v": 2})),
        );
    }

    #[tokio::test]
    async fn list_running_filters_by_status() {
        let repo = InMemoryExecutionRepo::default();
        let e1 = ExecutionId::new();
        let e2 = ExecutionId::new();
        let e3 = ExecutionId::new();
        let wid = WorkflowId::new();
        repo.create(e1, wid, serde_json::json!({"status": "running"}))
            .await
            .unwrap();
        repo.create(e2, wid, serde_json::json!({"status": "completed"}))
            .await
            .unwrap();
        repo.create(e3, wid, serde_json::json!({"status": "cancelling"}))
            .await
            .unwrap();
        let running = repo.list_running().await.unwrap();
        assert_eq!(running.len(), 2);
        assert!(running.contains(&e1));
        assert!(running.contains(&e3));
    }
}
