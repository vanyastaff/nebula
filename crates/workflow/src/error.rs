//! Workflow-specific error types.

use nebula_core::NodeId;
use thiserror::Error;

/// Errors that can occur during workflow definition, validation, or graph construction.
#[derive(Debug, Error)]
pub enum WorkflowError {
    /// Workflow name must not be empty.
    #[error("workflow name must not be empty")]
    EmptyName,

    /// Workflow must have at least one node.
    #[error("workflow must have at least one node")]
    NoNodes,

    /// Duplicate node id found.
    #[error("duplicate node id: {0}")]
    DuplicateNodeId(NodeId),

    /// Connection references a node that does not exist.
    #[error("connection references unknown node: {0}")]
    UnknownNode(NodeId),

    /// A connection has the same source and target node.
    #[error("self-loop detected on node: {0}")]
    SelfLoop(NodeId),

    /// The workflow graph contains a cycle and is not a DAG.
    #[error("cycle detected in workflow graph")]
    CycleDetected,

    /// Every node has incoming edges, so there is no place to start execution.
    #[error("workflow has no entry nodes (all nodes have incoming edges)")]
    NoEntryNodes,

    /// A parameter reference points to a node that does not exist.
    #[error("node {node_id} references unknown parameter source node: {source_node_id}")]
    InvalidParameterReference {
        /// The node containing the bad reference.
        node_id: NodeId,
        /// The referenced node that does not exist.
        source_node_id: NodeId,
    },

    /// Generic graph construction error.
    #[error("graph error: {0}")]
    GraphError(String),
}

impl nebula_error::Classify for WorkflowError {
    fn category(&self) -> nebula_error::ErrorCategory {
        nebula_error::ErrorCategory::Validation
    }

    fn code(&self) -> nebula_error::ErrorCode {
        nebula_error::ErrorCode::new(match self {
            Self::EmptyName => "WORKFLOW:EMPTY_NAME",
            Self::NoNodes => "WORKFLOW:NO_NODES",
            Self::DuplicateNodeId(_) => "WORKFLOW:DUPLICATE_NODE_ID",
            Self::UnknownNode(_) => "WORKFLOW:UNKNOWN_NODE",
            Self::SelfLoop(_) => "WORKFLOW:SELF_LOOP",
            Self::CycleDetected => "WORKFLOW:CYCLE_DETECTED",
            Self::NoEntryNodes => "WORKFLOW:NO_ENTRY_NODES",
            Self::InvalidParameterReference { .. } => "WORKFLOW:INVALID_PARAM_REF",
            Self::GraphError(_) => "WORKFLOW:GRAPH_ERROR",
        })
    }
}
