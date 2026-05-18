//! In-memory `NodeResultStore` + `CheckpointStore`.
//!
//! These back the engine's resume seam (ADR-0009 node outputs/results +
//! workflow input) and the stateful-action checkpoint optimisation
//! (spec-16 §11.5). Each is its own `parking_lot::Mutex`-guarded map keyed
//! by `(scope, execution_id, node_id)` so a cross-tenant read can never
//! observe another tenant's payload — the same isolation predicate the SQL
//! backends enforce with `WHERE workspace_id = ? AND org_id = ?`.
//!
//! Node-output vs node-result are distinct slots: `save_node_output` stores
//! the raw payload a node emitted; `save_node_result` stores the full typed
//! result record so idempotent replay can reconstruct routing semantics.
//! The schema version on every loaded record is checked against
//! [`MAX_SUPPORTED_RESULT_SCHEMA_VERSION`] and a newer record fails closed
//! with [`StorageError::UnknownSchemaVersion`] rather than being silently
//! misinterpreted (ADR-0009 §2).

use std::collections::HashMap;
use std::sync::Arc;

use nebula_storage_port::dto::{MAX_SUPPORTED_RESULT_SCHEMA_VERSION, NodeResultRecord};
use nebula_storage_port::store::{CheckpointStore, NodeResultStore};
use nebula_storage_port::{Scope, StorageError};
use parking_lot::Mutex;

/// Per-node slot key: `(workspace_id, org_id, execution_id, node_id)`.
/// Folding the scope into the key makes a cross-tenant collision
/// structurally impossible (a probe for tenant B's node cannot hit
/// tenant A's slot).
type NodeKey = (String, String, String, String);

/// Per-execution workflow-input key: `(workspace_id, org_id,
/// execution_id)`.
type InputKey = (String, String, String);

fn node_key(scope: &Scope, execution_id: &str, node_id: &str) -> NodeKey {
    (
        scope.workspace_id.clone(),
        scope.org_id.clone(),
        execution_id.to_string(),
        node_id.to_string(),
    )
}

fn input_key(scope: &Scope, execution_id: &str) -> InputKey {
    (
        scope.workspace_id.clone(),
        scope.org_id.clone(),
        execution_id.to_string(),
    )
}

/// Reject a record whose schema version this binary cannot decode.
///
/// A newer writer may have stored a shape this process does not
/// understand; surfacing it as a typed error keeps resume loud instead of
/// silently reconstructing a node from a misread payload (ADR-0009 §2,
/// PRODUCT_CANON §4.5).
fn guard_schema(record: &NodeResultRecord) -> Result<(), StorageError> {
    if record.schema_version > MAX_SUPPORTED_RESULT_SCHEMA_VERSION {
        return Err(StorageError::UnknownSchemaVersion {
            found: record.schema_version,
            max: MAX_SUPPORTED_RESULT_SCHEMA_VERSION,
        });
    }
    Ok(())
}

#[derive(Debug, Default)]
struct NodeResultState {
    /// Raw per-node outputs (latest write wins per key).
    outputs: HashMap<NodeKey, NodeResultRecord>,
    /// Full typed per-node result records (latest write wins per key).
    results: HashMap<NodeKey, NodeResultRecord>,
    /// Per-execution workflow input record.
    inputs: HashMap<InputKey, NodeResultRecord>,
}

/// In-memory node-output / node-result store.
#[derive(Debug, Default, Clone)]
pub struct InMemoryNodeResultStore {
    inner: Arc<Mutex<NodeResultState>>,
}

impl InMemoryNodeResultStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl NodeResultStore for InMemoryNodeResultStore {
    async fn save_node_output(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
        output: NodeResultRecord,
    ) -> Result<(), StorageError> {
        guard_schema(&output)?;
        self.inner
            .lock()
            .outputs
            .insert(node_key(scope, execution_id, node_id), output);
        tracing::debug!(
            target: "nebula_storage::inmem",
            execution_id,
            node_id,
            "node output persisted"
        );
        Ok(())
    }

    async fn load_node_output(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
    ) -> Result<Option<NodeResultRecord>, StorageError> {
        let st = self.inner.lock();
        match st.outputs.get(&node_key(scope, execution_id, node_id)) {
            Some(r) => {
                guard_schema(r)?;
                Ok(Some(r.clone()))
            },
            None => Ok(None),
        }
    }

    async fn save_node_result(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
        result: NodeResultRecord,
    ) -> Result<(), StorageError> {
        guard_schema(&result)?;
        self.inner
            .lock()
            .results
            .insert(node_key(scope, execution_id, node_id), result);
        tracing::debug!(
            target: "nebula_storage::inmem",
            execution_id,
            node_id,
            "node result persisted"
        );
        Ok(())
    }

    async fn load_node_result(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
    ) -> Result<Option<NodeResultRecord>, StorageError> {
        let st = self.inner.lock();
        match st.results.get(&node_key(scope, execution_id, node_id)) {
            Some(r) => {
                guard_schema(r)?;
                Ok(Some(r.clone()))
            },
            None => Ok(None),
        }
    }

    async fn load_all_results(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<Vec<(String, NodeResultRecord)>, StorageError> {
        let st = self.inner.lock();
        let mut out = Vec::new();
        for ((ws, org, exec, node), rec) in &st.results {
            if ws == &scope.workspace_id && org == &scope.org_id && exec == execution_id {
                guard_schema(rec)?;
                out.push((node.clone(), rec.clone()));
            }
        }
        Ok(out)
    }

    async fn load_all_node_outputs(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<Vec<(String, NodeResultRecord)>, StorageError> {
        let st = self.inner.lock();
        let mut out = Vec::new();
        for ((ws, org, exec, node), rec) in &st.outputs {
            if ws == &scope.workspace_id && org == &scope.org_id && exec == execution_id {
                guard_schema(rec)?;
                out.push((node.clone(), rec.clone()));
            }
        }
        Ok(out)
    }

    async fn set_workflow_input(
        &self,
        scope: &Scope,
        execution_id: &str,
        input: NodeResultRecord,
    ) -> Result<(), StorageError> {
        guard_schema(&input)?;
        // Idempotent: a workflow sets its input once at start; the latest
        // canonical value wins.
        self.inner
            .lock()
            .inputs
            .insert(input_key(scope, execution_id), input);
        Ok(())
    }

    async fn get_workflow_input(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<Option<NodeResultRecord>, StorageError> {
        let st = self.inner.lock();
        match st.inputs.get(&input_key(scope, execution_id)) {
            Some(r) => {
                guard_schema(r)?;
                Ok(Some(r.clone()))
            },
            None => Ok(None),
        }
    }
}

/// In-memory stateful-action checkpoint store.
///
/// Best-effort by contract: a missing checkpoint means "replay from the
/// last committed state", never data loss — the authoritative state is the
/// execution row written through the transition batch.
#[derive(Debug, Default, Clone)]
pub struct InMemoryCheckpointStore {
    inner: Arc<Mutex<HashMap<NodeKey, serde_json::Value>>>,
}

impl InMemoryCheckpointStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl CheckpointStore for InMemoryCheckpointStore {
    async fn save_stateful_checkpoint(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
        checkpoint: serde_json::Value,
    ) -> Result<(), StorageError> {
        self.inner
            .lock()
            .insert(node_key(scope, execution_id, node_id), checkpoint);
        Ok(())
    }

    async fn load_stateful_checkpoint(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
    ) -> Result<Option<serde_json::Value>, StorageError> {
        Ok(self
            .inner
            .lock()
            .get(&node_key(scope, execution_id, node_id))
            .cloned())
    }
}
