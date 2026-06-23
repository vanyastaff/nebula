//! Workflow-specific error types.

use nebula_core::NodeKey;
use thiserror::Error;

/// Errors that can occur during workflow definition, validation, or graph construction.
#[derive(Debug, Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum WorkflowError {
    /// Workflow name must not be empty.
    #[classify(category = "validation", code = "WORKFLOW:EMPTY_NAME")]
    #[error("workflow name must not be empty")]
    EmptyName,

    /// Workflow must have at least one node.
    #[classify(category = "validation", code = "WORKFLOW:NO_NODES")]
    #[error("workflow must have at least one node")]
    NoNodes,

    /// Duplicate node key found.
    #[classify(category = "validation", code = "WORKFLOW:DUPLICATE_NODE_KEY")]
    #[error("duplicate node key: {0}")]
    DuplicateNodeKey(NodeKey),

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

    /// Invalid plugin key format.
    #[classify(category = "validation", code = "WORKFLOW:INVALID_PLUGIN_KEY")]
    #[error("invalid plugin key `{key}`: {reason}")]
    InvalidPluginKey {
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

    /// The producer node's output schema is not assignable to the consumer
    /// node's input schema on this connection (ADR-0100 TypeDAG T3).
    ///
    /// Emitted by [`crate::validate::validate_workflow_with_resolver`] when both
    /// endpoints resolve and `nebula_schema::explain_assignable` returns
    /// [`Assignability::No`](nebula_schema::Assignability::No) — a provable
    /// incompatibility. (An undecidable [`Unknown`](nebula_schema::Assignability::Unknown)
    /// verdict is [`Self::PortSchemaUndecidable`] in Strict mode, not this.)
    /// Structural errors (unknown nodes, cycles, …) are reported first; this
    /// error only fires when both nodes are structurally valid and both schemas
    /// are resolvable from the catalog.
    ///
    /// The payload is `Box`ed to keep the `WorkflowError` enum small enough to
    /// satisfy `clippy::result_large_err`.
    #[classify(category = "validation", code = "WORKFLOW:PORT_SCHEMA_INCOMPATIBLE")]
    #[error("port schema incompatible: {0}")]
    PortSchemaIncompatible(Box<PortSchemaIncompatDetails>),

    /// The producer→consumer edge is **not statically decidable** under
    /// [`SchemaCheckMode::Strict`](crate::validate::SchemaCheckMode) (ADR-0100
    /// TypeDAG): the assignability verdict was
    /// [`nebula_schema::Assignability::Unknown`] — a loader-backed `Dynamic`
    /// field, an opaque `Any` producer, `Mode` sum-type variance, or a float→int
    /// narrowing — so compatibility could be neither proven nor refuted.
    ///
    /// Never emitted under
    /// [`SchemaCheckMode::Gradual`](crate::validate::SchemaCheckMode), which
    /// passes undecidable edges (the default, preserving untyped
    /// `serde_json::Value` workflows). Boxed for the same `result_large_err`
    /// reason as [`Self::PortSchemaIncompatible`].
    #[classify(category = "validation", code = "WORKFLOW:PORT_SCHEMA_UNDECIDABLE")]
    #[error("port schema undecidable: {0}")]
    PortSchemaUndecidable(Box<PortSchemaUndecidableDetails>),

    /// A `RetryConfig` (per-node or workflow-default) violates the validity
    /// rules: `max_attempts == 0`, `max_delay_ms < initial_delay_ms`,
    /// `backoff_multiplier <= 0` or non-finite, or `initial_delay_ms == 0`
    /// combined with `max_attempts > 1` (burst retry without backoff).
    /// Per ROADMAP §M2.1 + the engine relies on these constraints —
    /// shift-left rejection at activation prevents nonsensical configs from
    /// reaching the runtime scheduler.
    #[classify(category = "validation", code = "WORKFLOW:INVALID_RETRY_CONFIG")]
    #[error(
        "invalid retry_policy{}: {reason}",
        node.as_ref().map_or(String::new(), |n| format!(" on node {n}"))
    )]
    InvalidRetryConfig {
        /// The node carrying the bad config, or `None` for workflow-default
        /// (`WorkflowConfig.retry_policy`).
        node: Option<NodeKey>,
        /// Why the config is invalid.
        reason: String,
    },
}

/// Join a slice of `Display` items with `"; "` (shared by the two payload
/// `Display` impls below).
fn join_display<T: std::fmt::Display>(items: &[T]) -> String {
    items
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Payload for [`WorkflowError::PortSchemaIncompatible`].
///
/// Kept separate and `Box`ed on the enum to satisfy `clippy::result_large_err`.
#[derive(Debug)]
#[non_exhaustive]
pub struct PortSchemaIncompatDetails {
    /// The producer (source) node key.
    pub from_node: NodeKey,
    /// The consumer (target) node key.
    pub to_node: NodeKey,
    /// The source output port, if named (`None` = default `"main"`).
    pub from_port: Option<String>,
    /// The target input port, if named (`None` = default flow input).
    pub to_port: Option<String>,
    /// Every incompatibility found on this edge (depth-first, consumer-field
    /// order), structured for programmatic inspection. The `Display` impl joins
    /// their [`nebula_schema::SchemaIncompat`] descriptions with `"; "`.
    pub incompatibilities: Vec<nebula_schema::SchemaIncompat>,
}

impl std::fmt::Display for PortSchemaIncompatDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let from_port = self.from_port.as_deref().unwrap_or("main");
        let to_port = self.to_port.as_deref().unwrap_or("default");
        write!(
            f,
            "{}.{} \u{2192} {}.{}: {}",
            self.from_node,
            from_port,
            self.to_node,
            to_port,
            join_display(&self.incompatibilities)
        )
    }
}

/// Payload for [`WorkflowError::PortSchemaUndecidable`].
///
/// Kept separate and `Box`ed on the enum for the same `clippy::result_large_err`
/// reason as [`PortSchemaIncompatDetails`].
#[derive(Debug)]
#[non_exhaustive]
pub struct PortSchemaUndecidableDetails {
    /// The producer (source) node key.
    pub from_node: NodeKey,
    /// The consumer (target) node key.
    pub to_node: NodeKey,
    /// The source output port, if named (`None` = default `"main"`).
    pub from_port: Option<String>,
    /// The target input port, if named (`None` = default flow input).
    pub to_port: Option<String>,
    /// Every reason the edge is undecidable, structured so a policy can route on
    /// them (e.g. suppress [`OpaqueProducer`](nebula_schema::UnknownReason::OpaqueProducer)
    /// while blocking [`ModeVariance`](nebula_schema::UnknownReason::ModeVariance))
    /// without string-parsing. The `Display` impl joins their descriptions with `"; "`.
    pub reasons: Vec<nebula_schema::UnknownReason>,
}

impl std::fmt::Display for PortSchemaUndecidableDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let from_port = self.from_port.as_deref().unwrap_or("main");
        let to_port = self.to_port.as_deref().unwrap_or("default");
        write!(
            f,
            "{}.{} \u{2192} {}.{}: {}",
            self.from_node,
            from_port,
            self.to_node,
            to_port,
            join_display(&self.reasons)
        )
    }
}
