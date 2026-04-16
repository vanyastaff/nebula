//! Workflow-specific error types.

use nebula_core::NodeKey;
use thiserror::Error;

/// Errors that can occur during workflow definition, validation, or graph construction.
#[derive(Debug, Error, nebula_error::Classify)]
pub enum WorkflowError {
    /// Workflow name must not be empty.
    #[classify(category = "validation", code = "WORKFLOW:EMPTY_NAME")]
    #[error("workflow name must not be empty")]
    EmptyName,

    /// Workflow must have at least one node.
    #[classify(category = "validation", code = "WORKFLOW:NO_NODES")]
    #[error("workflow must have at least one node")]
    NoNodes,

    /// Duplicate node id found.
    #[classify(category = "validation", code = "WORKFLOW:DUPLICATE_NODE_ID")]
    #[error("duplicate node id: {0}")]
    DuplicateNodeId(NodeKey),

    /// Connection references a node that does not exist.
    #[classify(category = "validation", code = "WORKFLOW:UNKNOWN_NODE")]
    #[error("connection references unknown node: {0}")]
    UnknownNode(NodeKey),

    /// A connection has the same source and target node.
    #[classify(category = "validation", code = "WORKFLOW:SELF_LOOP")]
    #[error("self-loop detected on node: {0}")]
    SelfLoop(NodeKey),

    /// The workflow graph contains a cycle and is not a DAG.
    #[classify(category = "validation", code = "WORKFLOW:CYCLE_DETECTED")]
    #[error("cycle detected in workflow graph")]
    CycleDetected,

    /// Every node has incoming edges, so there is no place to start execution.
    #[classify(category = "validation", code = "WORKFLOW:NO_ENTRY_NODES")]
    #[error("workflow has no entry nodes (all nodes have incoming edges)")]
    NoEntryNodes,

    /// A parameter reference points to a node that does not exist.
    #[classify(category = "validation", code = "WORKFLOW:INVALID_PARAM_REF")]
    #[error("node {node_key} references unknown parameter source node: {source_node_key}")]
    InvalidParameterReference {
        /// The node containing the bad reference.
        node_key: NodeKey,
        /// The referenced node that does not exist.
        source_node_key: NodeKey,
    },

    /// Invalid action key format.
    #[classify(category = "validation", code = "WORKFLOW:INVALID_ACTION_KEY")]
    #[error("invalid action key `{key}`: {reason}")]
    InvalidActionKey {
        /// The invalid key string.
        key: String,
        /// Why it's invalid.
        reason: String,
    },

    /// Invalid trigger configuration.
    #[classify(category = "validation", code = "WORKFLOW:INVALID_TRIGGER")]
    #[error("invalid trigger: {reason}")]
    InvalidTrigger {
        /// What's wrong with the trigger.
        reason: String,
    },

    /// Workflow schema version not supported.
    #[classify(category = "validation", code = "WORKFLOW:UNSUPPORTED_SCHEMA")]
    #[error("unsupported schema version {version}, max supported: {max}")]
    UnsupportedSchema {
        /// The version found in the definition.
        version: u32,
        /// Maximum supported version.
        max: u32,
    },

    /// Owner ID must not be empty or blank.
    #[classify(category = "validation", code = "WORKFLOW:INVALID_OWNER_ID")]
    #[error("owner_id must not be empty or blank")]
    InvalidOwnerId,

    /// Generic graph construction error.
    #[classify(category = "validation", code = "WORKFLOW:GRAPH_ERROR")]
    #[error("graph error: {0}")]
    GraphError(String),

    /// Two or more connections in the workflow are identical.
    ///
    /// Duplicate connections (same source node, target node, source port, target
    /// port, and edge condition) are always redundant and usually indicate a
    /// modelling error. They also confuse the engine's edge-resolution bookkeeping
    /// which counts incoming edges and compares to a required total.
    #[classify(category = "validation", code = "WORKFLOW:DUPLICATE_CONNECTION")]
    #[error("duplicate connection from {from} to {to}")]
    DuplicateConnection {
        /// Source node of the duplicated connection.
        from: NodeKey,
        /// Target node of the duplicated connection.
        to: NodeKey,
    },
}
