//! In-memory `NodeResultStore`.
//!
//! This backs the engine's resume seam (node outputs/results + workflow
//! input). Each slot is a `parking_lot::Mutex`-guarded map keyed
//! by `(scope, execution_id, node_id)` so a cross-tenant read can never
//! observe another tenant's payload — the same isolation predicate the SQL
//! backends enforce with `WHERE workspace_id = ? AND org_id = ?`.
//!
//! Node-output vs node-result are distinct slots: `save_node_output` stores
//! the raw payload a node emitted; `save_node_result` stores the full typed
//! result record so idempotent replay can reconstruct routing semantics.
//! The schema version on every saved and loaded record is checked against
//! `MAX_SUPPORTED_RESULT_SCHEMA_VERSION` and a newer record fails closed
//! with [`StorageError::UnknownSchemaVersion`] rather than being silently
//! misinterpreted (unknown schema version fails closed). The bulk loads
//! answer in node-id byte order, as the SQL backends do.
//!
//! The SQL backends also refuse a write for an execution they do not hold;
//! this reference model is composed independently of the execution store and
//! does not check that reference.

use std::collections::HashMap;
use std::sync::Arc;

use nebula_storage_port::dto::NodeResultRecord;
use nebula_storage_port::store::NodeResultStore;
use nebula_storage_port::{Scope, StorageError};
use parking_lot::Mutex;

use crate::node_result::guard_schema;

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
        out.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
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
        out.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
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
