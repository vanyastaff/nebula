//! Scope-enforcing [`NodeResultStore`] decorator.

use std::sync::Arc;

use nebula_storage_port::dto::NodeResultRecord;
use nebula_storage_port::store::NodeResultStore;
use nebula_storage_port::{Scope, StorageError};

/// Wraps a [`NodeResultStore`] and forces every call into the bound
/// [`Scope`]. The caller-supplied `scope` argument is *ignored*, so a
/// handler cannot read or write another tenant's node outputs/results
/// even with a forged scope (§6.1 confused-deputy, closed by
/// construction — same substitution rule as [`ScopedExecutionStore`]).
///
/// [`ScopedExecutionStore`]: super::ScopedExecutionStore
#[derive(Clone)]
pub struct ScopedNodeResultStore {
    inner: Arc<dyn NodeResultStore>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedNodeResultStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedNodeResultStore")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedNodeResultStore {
    /// Bind `inner` to `scope`. Constructed at the composition root from
    /// the request principal via a `ScopeResolver`.
    #[must_use]
    pub fn new(inner: Arc<dyn NodeResultStore>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }
}

#[async_trait::async_trait]
impl NodeResultStore for ScopedNodeResultStore {
    async fn save_node_output(
        &self,
        _scope: &Scope,
        execution_id: &str,
        node_id: &str,
        output: NodeResultRecord,
    ) -> Result<(), StorageError> {
        self.inner
            .save_node_output(&self.bound, execution_id, node_id, output)
            .await
    }

    async fn load_node_output(
        &self,
        _scope: &Scope,
        execution_id: &str,
        node_id: &str,
    ) -> Result<Option<NodeResultRecord>, StorageError> {
        self.inner
            .load_node_output(&self.bound, execution_id, node_id)
            .await
    }

    async fn save_node_result(
        &self,
        _scope: &Scope,
        execution_id: &str,
        node_id: &str,
        result: NodeResultRecord,
    ) -> Result<(), StorageError> {
        self.inner
            .save_node_result(&self.bound, execution_id, node_id, result)
            .await
    }

    async fn load_node_result(
        &self,
        _scope: &Scope,
        execution_id: &str,
        node_id: &str,
    ) -> Result<Option<NodeResultRecord>, StorageError> {
        self.inner
            .load_node_result(&self.bound, execution_id, node_id)
            .await
    }

    async fn load_all_results(
        &self,
        _scope: &Scope,
        execution_id: &str,
    ) -> Result<Vec<(String, NodeResultRecord)>, StorageError> {
        self.inner.load_all_results(&self.bound, execution_id).await
    }

    async fn load_all_node_outputs(
        &self,
        _scope: &Scope,
        execution_id: &str,
    ) -> Result<Vec<(String, NodeResultRecord)>, StorageError> {
        self.inner
            .load_all_node_outputs(&self.bound, execution_id)
            .await
    }

    async fn set_workflow_input(
        &self,
        _scope: &Scope,
        execution_id: &str,
        input: NodeResultRecord,
    ) -> Result<(), StorageError> {
        self.inner
            .set_workflow_input(&self.bound, execution_id, input)
            .await
    }

    async fn get_workflow_input(
        &self,
        _scope: &Scope,
        execution_id: &str,
    ) -> Result<Option<NodeResultRecord>, StorageError> {
        self.inner
            .get_workflow_input(&self.bound, execution_id)
            .await
    }
}
