//! Node-output / node-result store trait (ADR-0009).
use crate::dto::NodeResultRecord;
use crate::error::StorageError;
use crate::scope::Scope;

/// Per-node output + result persistence.
///
/// Records are port-local [`NodeResultRecord`]s — the port does not depend
/// on `ActionResult`. An unknown `schema_version` fails closed with
/// [`StorageError::UnknownSchemaVersion`] rather than being misinterpreted.
#[async_trait::async_trait]
pub trait NodeResultStore: Send + Sync + std::fmt::Debug {
    /// Persist a node's raw output payload.
    async fn save_node_output(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
        output: NodeResultRecord,
    ) -> Result<(), StorageError>;

    /// Load a node's raw output payload.
    async fn load_node_output(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
    ) -> Result<Option<NodeResultRecord>, StorageError>;

    /// Persist a node's typed result record.
    async fn save_node_result(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
        result: NodeResultRecord,
    ) -> Result<(), StorageError>;

    /// Load a node's typed result record.
    async fn load_node_result(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
    ) -> Result<Option<NodeResultRecord>, StorageError>;

    /// Load all node results for an execution.
    async fn load_all_results(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<Vec<(String, NodeResultRecord)>, StorageError>;

    /// Load all raw node *outputs* for an execution (the
    /// `save_node_output` slot, not the typed-result slot). The resume
    /// path repopulates the in-memory output map from this so a
    /// crash-resumed run feeds successors the same raw payloads a
    /// non-crashed run produced — the typed-result slot carries the
    /// serialized `ActionResult` envelope and is not interchangeable.
    async fn load_all_node_outputs(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<Vec<(String, NodeResultRecord)>, StorageError>;

    /// Persist the workflow-level input record.
    async fn set_workflow_input(
        &self,
        scope: &Scope,
        execution_id: &str,
        input: NodeResultRecord,
    ) -> Result<(), StorageError>;

    /// Load the workflow-level input record.
    async fn get_workflow_input(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<Option<NodeResultRecord>, StorageError>;
}
