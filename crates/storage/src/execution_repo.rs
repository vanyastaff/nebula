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
use nebula_core::{ExecutionId, NodeId, WorkflowId};
use thiserror::Error;
use tokio::sync::RwLock;

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
        node_id: NodeId,
        attempt: u32,
        output: serde_json::Value,
    ) -> Result<(), ExecutionRepoError>;

    /// Loads the latest output for a node (highest attempt number).
    async fn load_node_output(
        &self,
        execution_id: ExecutionId,
        node_id: NodeId,
    ) -> Result<Option<serde_json::Value>, ExecutionRepoError>;

    /// Loads all node outputs for an execution (latest attempt per node).
    async fn load_all_outputs(
        &self,
        execution_id: ExecutionId,
    ) -> Result<HashMap<NodeId, serde_json::Value>, ExecutionRepoError>;

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
        node_id: NodeId,
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
    /// The key is `(execution_id, node_id, attempt)`. Calling twice with
    /// the same key overwrites the prior checkpoint — only the latest
    /// iteration boundary is persisted.
    async fn save_stateful_checkpoint(
        &self,
        execution_id: ExecutionId,
        node_id: NodeId,
        attempt: u32,
        iteration: u32,
        state: serde_json::Value,
    ) -> Result<(), ExecutionRepoError> {
        let _ = (execution_id, node_id, attempt, iteration, state);
        Err(ExecutionRepoError::Internal(
            "save_stateful_checkpoint not implemented for this backend".to_owned(),
        ))
    }

    /// Load the latest stateful checkpoint for a `(execution, node, attempt)`.
    ///
    /// Returns `Ok(None)` when no checkpoint exists yet (fresh dispatch).
    /// Returns `Err` when the backend cannot be queried (connection,
    /// serialization, etc.) — the runtime logs WARN with full
    /// `(action_key, execution_id, node_id)` context and falls through to
    /// `init_state` so iteration progress loss is visible, never swallowed.
    async fn load_stateful_checkpoint(
        &self,
        execution_id: ExecutionId,
        node_id: NodeId,
        attempt: u32,
    ) -> Result<Option<StatefulCheckpointRecord>, ExecutionRepoError> {
        let _ = (execution_id, node_id, attempt);
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
        node_id: NodeId,
        attempt: u32,
    ) -> Result<(), ExecutionRepoError> {
        let _ = (execution_id, node_id, attempt);
        Err(ExecutionRepoError::Internal(
            "delete_stateful_checkpoint not implemented for this backend".to_owned(),
        ))
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

/// Key for node output storage: `(execution_id, node_id, attempt)`.
type NodeOutputKey = (ExecutionId, NodeId, u32);

/// Key for stateful checkpoint storage: `(execution_id, node_id, attempt)`.
type StatefulCheckpointKey = (ExecutionId, NodeId, u32);

/// In-memory execution repository for tests and single-process/health-only mode.
#[derive(Default)]
pub struct InMemoryExecutionRepo {
    state: Arc<RwLock<HashMap<ExecutionId, (u64, serde_json::Value)>>>,
    journal: Arc<RwLock<HashMap<ExecutionId, Vec<serde_json::Value>>>>,
    leases: Arc<RwLock<HashMap<ExecutionId, String>>>,
    workflows: Arc<RwLock<HashMap<ExecutionId, WorkflowId>>>,
    node_outputs: Arc<RwLock<HashMap<NodeOutputKey, serde_json::Value>>>,
    idempotency: Arc<RwLock<HashSet<String>>>,
    stateful_checkpoints: Arc<RwLock<HashMap<StatefulCheckpointKey, StatefulCheckpointRecord>>>,
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
        let current = state.get(&id).map(|(v, _)| *v).unwrap_or(0);
        if current != expected_version {
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
        let mut journal = self.journal.write().await;
        journal.entry(id).or_default().push(entry);
        Ok(())
    }

    async fn acquire_lease(
        &self,
        id: ExecutionId,
        holder: String,
        _ttl: Duration,
    ) -> Result<bool, ExecutionRepoError> {
        let mut leases = self.leases.write().await;
        if leases.contains_key(&id) {
            return Ok(false);
        }
        leases.insert(id, holder);
        Ok(true)
    }

    async fn renew_lease(
        &self,
        id: ExecutionId,
        holder: &str,
        _ttl: Duration,
    ) -> Result<bool, ExecutionRepoError> {
        let leases = self.leases.read().await;
        Ok(leases.get(&id).map(|h| h.as_str()) == Some(holder))
    }

    async fn release_lease(
        &self,
        id: ExecutionId,
        holder: &str,
    ) -> Result<bool, ExecutionRepoError> {
        let mut leases = self.leases.write().await;
        let ok = leases.get(&id).map(|h| h.as_str()) == Some(holder);
        if ok {
            leases.remove(&id);
        }
        Ok(ok)
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
        node_id: NodeId,
        attempt: u32,
        output: serde_json::Value,
    ) -> Result<(), ExecutionRepoError> {
        self.node_outputs
            .write()
            .await
            .insert((execution_id, node_id, attempt), output);
        Ok(())
    }

    async fn load_node_output(
        &self,
        execution_id: ExecutionId,
        node_id: NodeId,
    ) -> Result<Option<serde_json::Value>, ExecutionRepoError> {
        let outputs = self.node_outputs.read().await;
        let best = outputs
            .iter()
            .filter(|((eid, nid, _), _)| *eid == execution_id && *nid == node_id)
            .max_by_key(|((_, _, attempt), _)| *attempt)
            .map(|(_, v)| v.clone());
        Ok(best)
    }

    async fn load_all_outputs(
        &self,
        execution_id: ExecutionId,
    ) -> Result<HashMap<NodeId, serde_json::Value>, ExecutionRepoError> {
        let outputs = self.node_outputs.read().await;
        let mut best: HashMap<NodeId, (u32, serde_json::Value)> = HashMap::new();
        for ((eid, nid, attempt), val) in outputs.iter() {
            if *eid != execution_id {
                continue;
            }
            let entry = best.entry(*nid).or_insert((*attempt, val.clone()));
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
        _execution_id: ExecutionId,
        _node_id: NodeId,
    ) -> Result<(), ExecutionRepoError> {
        self.idempotency.write().await.insert(key.to_owned());
        Ok(())
    }

    async fn save_stateful_checkpoint(
        &self,
        execution_id: ExecutionId,
        node_id: NodeId,
        attempt: u32,
        iteration: u32,
        state: serde_json::Value,
    ) -> Result<(), ExecutionRepoError> {
        let mut cps = self.stateful_checkpoints.write().await;
        cps.insert(
            (execution_id, node_id, attempt),
            StatefulCheckpointRecord::new(iteration, state),
        );
        Ok(())
    }

    async fn load_stateful_checkpoint(
        &self,
        execution_id: ExecutionId,
        node_id: NodeId,
        attempt: u32,
    ) -> Result<Option<StatefulCheckpointRecord>, ExecutionRepoError> {
        let cps = self.stateful_checkpoints.read().await;
        Ok(cps.get(&(execution_id, node_id, attempt)).cloned())
    }

    async fn delete_stateful_checkpoint(
        &self,
        execution_id: ExecutionId,
        node_id: NodeId,
        attempt: u32,
    ) -> Result<(), ExecutionRepoError> {
        let mut cps = self.stateful_checkpoints.write().await;
        cps.remove(&(execution_id, node_id, attempt));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let nid = NodeId::new();
        let output = serde_json::json!({"result": 42});
        repo.save_node_output(eid, nid, 1, output.clone())
            .await
            .unwrap();
        let loaded = repo.load_node_output(eid, nid).await.unwrap();
        assert_eq!(loaded, Some(output));
    }

    #[tokio::test]
    async fn node_output_returns_latest_attempt() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let nid = NodeId::new();
        repo.save_node_output(eid, nid, 1, serde_json::json!("first"))
            .await
            .unwrap();
        repo.save_node_output(eid, nid, 2, serde_json::json!("second"))
            .await
            .unwrap();
        let loaded = repo.load_node_output(eid, nid).await.unwrap();
        assert_eq!(loaded, Some(serde_json::json!("second")));
    }

    #[tokio::test]
    async fn load_all_outputs_returns_latest_per_node() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let n1 = NodeId::new();
        let n2 = NodeId::new();
        repo.save_node_output(eid, n1, 1, serde_json::json!("n1_v1"))
            .await
            .unwrap();
        repo.save_node_output(eid, n1, 2, serde_json::json!("n1_v2"))
            .await
            .unwrap();
        repo.save_node_output(eid, n2, 1, serde_json::json!("n2_v1"))
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
        assert!(!repo.check_idempotency(key).await.unwrap());
        repo.mark_idempotent(key, ExecutionId::new(), NodeId::new())
            .await
            .unwrap();
        assert!(repo.check_idempotency(key).await.unwrap());
    }

    // ── #308 regression: stateful checkpoint round-trips ───────────────────

    #[tokio::test]
    async fn stateful_checkpoint_save_load_round_trip() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let nid = NodeId::new();
        let state = serde_json::json!({"cursor": "page-2", "count": 42u32});

        // Empty before first save.
        assert_eq!(
            repo.load_stateful_checkpoint(eid, nid, 0).await.unwrap(),
            None
        );

        repo.save_stateful_checkpoint(eid, nid, 0, 5, state.clone())
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
        let nid = NodeId::new();

        repo.save_stateful_checkpoint(eid, nid, 2, 1, serde_json::json!({"k": 1}))
            .await
            .unwrap();
        assert!(
            repo.load_stateful_checkpoint(eid, nid, 2)
                .await
                .unwrap()
                .is_some()
        );
        repo.delete_stateful_checkpoint(eid, nid, 2).await.unwrap();
        assert_eq!(
            repo.load_stateful_checkpoint(eid, nid, 2).await.unwrap(),
            None
        );
        // Double-delete is idempotent.
        repo.delete_stateful_checkpoint(eid, nid, 2).await.unwrap();
    }

    #[tokio::test]
    async fn stateful_checkpoint_is_scoped_by_attempt() {
        let repo = InMemoryExecutionRepo::default();
        let eid = ExecutionId::new();
        let nid = NodeId::new();

        repo.save_stateful_checkpoint(eid, nid, 0, 1, serde_json::json!("attempt-0"))
            .await
            .unwrap();
        repo.save_stateful_checkpoint(eid, nid, 1, 1, serde_json::json!("attempt-1"))
            .await
            .unwrap();

        let a0 = repo
            .load_stateful_checkpoint(eid, nid, 0)
            .await
            .unwrap()
            .unwrap();
        let a1 = repo
            .load_stateful_checkpoint(eid, nid, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(a0.state, serde_json::json!("attempt-0"));
        assert_eq!(a1.state, serde_json::json!("attempt-1"));

        // Deleting one attempt does not touch the other.
        repo.delete_stateful_checkpoint(eid, nid, 0).await.unwrap();
        assert_eq!(
            repo.load_stateful_checkpoint(eid, nid, 0).await.unwrap(),
            None
        );
        assert!(
            repo.load_stateful_checkpoint(eid, nid, 1)
                .await
                .unwrap()
                .is_some()
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
