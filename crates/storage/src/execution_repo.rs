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
        execution_id: ExecutionId,
        _node_id: NodeId,
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
        let eid = ExecutionId::new();
        let wid = WorkflowId::new();
        repo.create(eid, wid, serde_json::json!({"status": "created"}))
            .await
            .unwrap();
        assert!(!repo.check_idempotency(key).await.unwrap());
        repo.mark_idempotent(key, eid, NodeId::new()).await.unwrap();
        assert!(repo.check_idempotency(key).await.unwrap());
    }

    #[tokio::test]
    async fn mark_idempotent_unknown_execution_returns_not_found() {
        let repo = InMemoryExecutionRepo::default();
        let missing = ExecutionId::new();
        let err = repo
            .mark_idempotent("k", missing, NodeId::new())
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
