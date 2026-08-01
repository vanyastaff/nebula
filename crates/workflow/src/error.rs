//! Workflow-specific error types.

use nebula_core::{NodeKey, PortKey};
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

    /// A parameter `Reference` pulls a value from a node it has no connection
    /// edge from. The dependency graph is built from connections only (it never
    /// reads `parameters`), so a reference with no coincident connection leaves
    /// the true data dependency invisible to the scheduler — the consumer can be
    /// ordered before the producer and read stale or absent output. Adding the
    /// connection makes the dependency visible (and type-checkable).
    #[classify(
        category = "validation",
        code = "WORKFLOW:REFERENCE_WITHOUT_CONNECTION"
    )]
    #[error(
        "node {node_key} references output of {source_node_key} via a parameter but has no \
         connection edge from it — add a connection from {source_node_key} so the dependency is \
         visible to the scheduler"
    )]
    ReferenceWithoutConnection {
        /// The node containing the reference.
        node_key: NodeKey,
        /// The referenced producer node that is not connected.
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

    /// A `ParamValue::Reference`'s `output_path` provably fails to resolve
    /// through the producer node's output schema, on a path that walked
    /// through only **closed** (fully-typed) nodes right up to the failure
    /// point (ADR-0100 TypeDAG, W0 U5 — correctness only; see
    /// `crate::validate::check_reference_edges`).
    ///
    /// Emitted only for [`nebula_schema::PathResolveError::NonIndexOnList`] /
    /// [`nebula_schema::PathResolveError::DescendPastLeaf`] — a missing
    /// `Object` key, or any opaque node encountered along the way, fails open
    /// instead (never this variant). Fires in **both**
    /// [`SchemaCheckMode`](crate::validate::SchemaCheckMode)s: it is a provable
    /// structural mistake, not an undecidable verdict.
    ///
    /// This is a **correctness-only** check: a `Reference` into a
    /// `Field::Secret` producer field is not distinguished from any other
    /// reference here (see the W0 U5 plan's framing — secret exfiltration via
    /// parameter references is a separate, filed, not-yet-built initiative).
    ///
    /// The payload is `Box`ed for the same `clippy::result_large_err` reason as
    /// [`Self::PortSchemaIncompatible`].
    #[classify(category = "validation", code = "WORKFLOW:REFERENCE_PATH_UNRESOLVED")]
    #[error("reference path unresolved: {0}")]
    ReferencePathUnresolved(Box<ReferencePathUnresolvedDetails>),

    /// A `ParamValue::Reference`'s `output_path` resolves through the
    /// producer's output schema (a fully-closed path —
    /// [`nebula_schema::PathWalk::Resolved`]), but the resolved leaf field is
    /// provably **not assignable** to the consumer parameter's expected field
    /// ([`nebula_schema::Assignability::No`]). Classified consistently with
    /// [`Self::PortSchemaIncompatible`] (always a hard error, both modes).
    ///
    /// The payload is `Box`ed for the same `clippy::result_large_err` reason as
    /// [`Self::PortSchemaIncompatible`].
    #[classify(category = "validation", code = "WORKFLOW:REFERENCE_TYPE_INCOMPATIBLE")]
    #[error("reference type incompatible: {0}")]
    ReferenceTypeIncompatible(Box<ReferenceTypeIncompatDetails>),

    /// Like [`Self::ReferenceTypeIncompatible`], but the assignability verdict
    /// was [`nebula_schema::Assignability::Unknown`] — classified consistently
    /// with [`Self::PortSchemaUndecidable`]: blocked only under
    /// [`SchemaCheckMode::Strict`](crate::validate::SchemaCheckMode::Strict);
    /// `Gradual` warns-and-passes.
    #[classify(category = "validation", code = "WORKFLOW:REFERENCE_TYPE_UNDECIDABLE")]
    #[error("reference type undecidable: {0}")]
    ReferenceTypeUndecidable(Box<ReferenceTypeUndecidableDetails>),
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
    /// The source output port, if named (`None` = default `"out"`).
    pub from_port: Option<PortKey>,
    /// The target input port, if named (`None` = default flow input).
    pub to_port: Option<PortKey>,
    /// Every incompatibility found on this edge (depth-first, consumer-field
    /// order), structured for programmatic inspection. The `Display` impl joins
    /// their [`nebula_schema::SchemaIncompat`] descriptions with `"; "`.
    pub incompatibilities: Vec<nebula_schema::SchemaIncompat>,
}

impl std::fmt::Display for PortSchemaIncompatDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let from_port = self
            .from_port
            .as_ref()
            .map(PortKey::as_str)
            .unwrap_or("out");
        let to_port = self
            .to_port
            .as_ref()
            .map(PortKey::as_str)
            .unwrap_or("default");
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
    /// The source output port, if named (`None` = default `"out"`).
    pub from_port: Option<PortKey>,
    /// The target input port, if named (`None` = default flow input).
    pub to_port: Option<PortKey>,
    /// Every reason the edge is undecidable, structured so a policy can route on
    /// them (e.g. suppress [`OpaqueProducer`](nebula_schema::UnknownReason::OpaqueProducer)
    /// while blocking [`DynamicLoaderBacked`](nebula_schema::UnknownReason::DynamicLoaderBacked))
    /// without string-parsing. The `Display` impl joins their descriptions with `"; "`.
    pub reasons: Vec<nebula_schema::UnknownReason>,
}

impl std::fmt::Display for PortSchemaUndecidableDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let from_port = self
            .from_port
            .as_ref()
            .map(PortKey::as_str)
            .unwrap_or("out");
        let to_port = self
            .to_port
            .as_ref()
            .map(PortKey::as_str)
            .unwrap_or("default");
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

/// Payload for [`WorkflowError::ReferencePathUnresolved`].
///
/// Kept separate and `Box`ed on the enum to satisfy `clippy::result_large_err`,
/// mirroring [`PortSchemaIncompatDetails`].
#[derive(Debug)]
#[non_exhaustive]
pub struct ReferencePathUnresolvedDetails {
    /// The node whose parameter carries the unresolved reference.
    pub consumer_node: NodeKey,
    /// The parameter key on `consumer_node`.
    pub param_key: String,
    /// The referenced producer node.
    pub producer_node: NodeKey,
    /// The authored `output_path` that failed to resolve.
    pub output_path: String,
    /// The rendered [`nebula_schema::PathResolveError`].
    pub reason: String,
}

impl std::fmt::Display for ReferencePathUnresolvedDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "node {} parameter `{}` references {}.{}, which does not resolve: {}",
            self.consumer_node, self.param_key, self.producer_node, self.output_path, self.reason
        )
    }
}

/// Payload for [`WorkflowError::ReferenceTypeIncompatible`].
///
/// Kept separate and `Box`ed on the enum to satisfy `clippy::result_large_err`,
/// mirroring [`PortSchemaIncompatDetails`].
#[derive(Debug)]
#[non_exhaustive]
pub struct ReferenceTypeIncompatDetails {
    /// The node whose parameter carries the reference.
    pub consumer_node: NodeKey,
    /// The parameter key on `consumer_node`.
    pub param_key: String,
    /// The referenced producer node.
    pub producer_node: NodeKey,
    /// The authored `output_path` the reference resolved through.
    pub output_path: String,
    /// Every incompatibility found between the resolved producer leaf field and
    /// the consumer's expected field, structured for programmatic inspection.
    pub incompatibilities: Vec<nebula_schema::SchemaIncompat>,
}

impl std::fmt::Display for ReferenceTypeIncompatDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}.{} \u{2190} {}.{}: {}",
            self.consumer_node,
            self.param_key,
            self.producer_node,
            self.output_path,
            join_display(&self.incompatibilities)
        )
    }
}

/// Payload for [`WorkflowError::ReferenceTypeUndecidable`].
///
/// Kept separate and `Box`ed on the enum for the same `clippy::result_large_err`
/// reason as [`ReferenceTypeIncompatDetails`].
#[derive(Debug)]
#[non_exhaustive]
pub struct ReferenceTypeUndecidableDetails {
    /// The node whose parameter carries the reference.
    pub consumer_node: NodeKey,
    /// The parameter key on `consumer_node`.
    pub param_key: String,
    /// The referenced producer node.
    pub producer_node: NodeKey,
    /// The authored `output_path` the reference resolved through.
    pub output_path: String,
    /// Every reason the reference's assignability is undecidable, structured so
    /// a policy can route on them without string-parsing.
    pub reasons: Vec<nebula_schema::UnknownReason>,
}

impl std::fmt::Display for ReferenceTypeUndecidableDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}.{} \u{2190} {}.{}: {}",
            self.consumer_node,
            self.param_key,
            self.producer_node,
            self.output_path,
            join_display(&self.reasons)
        )
    }
}

/// Render a detail list, or a sentinel when it is empty.
///
/// An empty list makes `actual` blank, which the diagnostic contract refuses —
/// and the whole diagnostic then falls back to a coarse `/workflow` path,
/// losing the very element the author has to change. A sentinel keeps the
/// specific code and path while still saying honestly that no detail was
/// reported.
fn join_or_sentinel<T: std::fmt::Display>(items: &[T]) -> String {
    if items.is_empty() {
        return "<no detail reported>".to_owned();
    }
    join_display(items)
}

/// Build one diagnostic, falling back to a complete but coarse one.
///
/// Every call site supplies five non-empty values, so construction cannot
/// legitimately fail. The fallback exists because a rejection that reports
/// nothing is worse than one that reports a coarse code: a caller must never
/// receive an empty diagnostic list.
fn diagnostic(
    code: &str,
    path: String,
    expected: String,
    actual: String,
    remediation: &str,
) -> nebula_error::ActivationDiagnostic {
    nebula_error::ActivationDiagnostic::new(code, &path, expected, actual, remediation)
        .or_else(|| {
            nebula_error::ActivationDiagnostic::new(
                code,
                "/workflow",
                "<contract>",
                "<unavailable>",
                "repair the reported workflow element",
            )
        })
        .unwrap_or_else(|| unreachable!("the fallback diagnostic uses non-empty constants"))
}

/// Path to one node's parameter, or to the node when the parameter is unknown.
fn parameter_path(node: &NodeKey, param_key: &str) -> String {
    format!("/nodes/{node}/parameters/{param_key}")
}

/// Path to one connection, named by the endpoints it wires.
fn connection_path(from: &NodeKey, to: &NodeKey) -> String {
    format!("/connections/{from}->{to}")
}

impl nebula_error::ActivationDiagnostics for WorkflowError {
    fn activation_diagnostics(&self) -> Vec<nebula_error::ActivationDiagnostic> {
        let single = match self {
            Self::EmptyName => diagnostic(
                "WORKFLOW:EMPTY_NAME",
                "/name".to_owned(),
                "a non-empty workflow name".to_owned(),
                "an empty name".to_owned(),
                "give the workflow a name",
            ),
            Self::NoNodes => diagnostic(
                "WORKFLOW:NO_NODES",
                "/nodes".to_owned(),
                "at least one node".to_owned(),
                "no nodes".to_owned(),
                "add at least one node to the workflow",
            ),
            Self::DuplicateNodeKey(key) => diagnostic(
                "WORKFLOW:DUPLICATE_NODE_KEY",
                format!("/nodes/{key}"),
                "one node per key".to_owned(),
                "two or more nodes sharing this key".to_owned(),
                "rename one of the nodes so every key is unique",
            ),
            Self::UnknownNode(key) => diagnostic(
                "WORKFLOW:UNKNOWN_NODE",
                "/connections".to_owned(),
                "a connection endpoint naming a declared node".to_owned(),
                key.to_string(),
                "declare the node, or remove the connection that names it",
            ),
            Self::SelfLoop(key) => diagnostic(
                "WORKFLOW:SELF_LOOP",
                "/connections".to_owned(),
                "a connection between two different nodes".to_owned(),
                format!("{key}->{key}"),
                "remove the self-referencing connection",
            ),
            Self::CycleDetected => diagnostic(
                "WORKFLOW:CYCLE_DETECTED",
                "/connections".to_owned(),
                "an acyclic graph".to_owned(),
                "a graph containing a cycle".to_owned(),
                "break the cycle: execution order cannot be derived from a cyclic graph",
            ),
            Self::NoEntryNodes => diagnostic(
                "WORKFLOW:NO_ENTRY_NODES",
                "/nodes".to_owned(),
                "at least one node with no incoming edge".to_owned(),
                "every node has an incoming edge".to_owned(),
                "remove an incoming edge so execution has somewhere to start",
            ),
            Self::InvalidParameterReference {
                node_key,
                source_node_key,
            } => diagnostic(
                "WORKFLOW:INVALID_PARAM_REF",
                format!("/nodes/{node_key}/parameters"),
                "a reference to a declared node".to_owned(),
                source_node_key.to_string(),
                "declare the referenced node, or point the parameter at an existing one",
            ),
            Self::ReferenceWithoutConnection {
                node_key,
                source_node_key,
            } => diagnostic(
                "WORKFLOW:REFERENCE_WITHOUT_CONNECTION",
                format!("/nodes/{node_key}/parameters"),
                format!("a connection from {source_node_key}"),
                "a reference with no connection edge behind it".to_owned(),
                "add the connection so the scheduler can see the dependency and order the nodes",
            ),
            Self::InvalidActionKey { key, reason } => diagnostic(
                "WORKFLOW:INVALID_ACTION_KEY",
                "/nodes/action_key".to_owned(),
                "a well-formed action key".to_owned(),
                key.clone(),
                reason,
            ),
            Self::InvalidPluginKey { key, reason } => diagnostic(
                "WORKFLOW:INVALID_PLUGIN_KEY",
                "/nodes/plugin_key".to_owned(),
                "a well-formed plugin key".to_owned(),
                key.clone(),
                reason,
            ),
            Self::InvalidTrigger { reason } => diagnostic(
                "WORKFLOW:INVALID_TRIGGER",
                "/triggers".to_owned(),
                "a well-formed trigger".to_owned(),
                reason.clone(),
                "correct the trigger configuration",
            ),
            Self::UnsupportedSchema { version, max } => diagnostic(
                "WORKFLOW:UNSUPPORTED_SCHEMA",
                "/schema_version".to_owned(),
                format!("at most {max}"),
                version.to_string(),
                "re-export the workflow from a version this runtime supports",
            ),
            Self::InvalidOwnerId => diagnostic(
                "WORKFLOW:INVALID_OWNER_ID",
                "/owner_id".to_owned(),
                "a non-blank owner id".to_owned(),
                "an empty or blank owner id".to_owned(),
                "set an owner id on the workflow",
            ),
            Self::GraphError(reason) => diagnostic(
                "WORKFLOW:GRAPH_ERROR",
                "/connections".to_owned(),
                "a constructible dependency graph".to_owned(),
                reason.clone(),
                "repair the connections so a dependency graph can be built",
            ),
            Self::DuplicateConnection { from, to } => diagnostic(
                "WORKFLOW:DUPLICATE_CONNECTION",
                connection_path(from, to),
                "one connection per (source, target, port) triple".to_owned(),
                "two or more identical connections".to_owned(),
                "remove the redundant connection: duplicates confuse incoming-edge counting",
            ),
            Self::PortSchemaIncompatible(details) => diagnostic(
                "WORKFLOW:PORT_SCHEMA_INCOMPATIBLE",
                connection_path(&details.from_node, &details.to_node),
                "an output schema assignable to the consumer's input schema".to_owned(),
                join_or_sentinel(&details.incompatibilities),
                "align the producer output with the consumer input, or insert a mapping node",
            ),
            Self::PortSchemaUndecidable(details) => diagnostic(
                "WORKFLOW:PORT_SCHEMA_UNDECIDABLE",
                connection_path(&details.from_node, &details.to_node),
                "a statically decidable edge".to_owned(),
                join_or_sentinel(&details.reasons),
                "type the dynamic or opaque field, or validate in Gradual mode",
            ),
            Self::InvalidRetryConfig { node, reason } => diagnostic(
                "WORKFLOW:INVALID_RETRY_CONFIG",
                node.as_ref().map_or_else(
                    || "/config/retry_policy".to_owned(),
                    |key| format!("/nodes/{key}/retry_policy"),
                ),
                "a retry policy the scheduler can honour".to_owned(),
                reason.clone(),
                "correct the retry policy, or remove it to disable retries",
            ),
            Self::ReferencePathUnresolved(details) => diagnostic(
                "WORKFLOW:REFERENCE_PATH_UNRESOLVED",
                parameter_path(&details.consumer_node, &details.param_key),
                format!(
                    "a path resolvable through the output schema of {}",
                    details.producer_node
                ),
                format!("{}: {}", details.output_path, details.reason),
                "correct the output path, or widen the producer's output schema",
            ),
            Self::ReferenceTypeIncompatible(details) => diagnostic(
                "WORKFLOW:REFERENCE_TYPE_INCOMPATIBLE",
                parameter_path(&details.consumer_node, &details.param_key),
                format!(
                    "a leaf at {} assignable to this parameter",
                    details.output_path
                ),
                join_or_sentinel(&details.incompatibilities),
                "reference a compatible field, or change the parameter's expected type",
            ),
            Self::ReferenceTypeUndecidable(details) => diagnostic(
                "WORKFLOW:REFERENCE_TYPE_UNDECIDABLE",
                parameter_path(&details.consumer_node, &details.param_key),
                format!("a statically decidable leaf at {}", details.output_path),
                join_or_sentinel(&details.reasons),
                "type the dynamic or opaque field, or validate in Gradual mode",
            ),
        };
        vec![single]
    }
}

#[cfg(test)]
mod activation_diagnostic_tests {
    use nebula_error::ActivationDiagnostics;

    use super::*;

    fn node(name: &'static str) -> NodeKey {
        name.parse().expect("the fixture node key is well-formed")
    }

    /// One instance of every rejection this crate can produce.
    ///
    /// `activation_diagnostics` matches the enum without a wildcard, so a new
    /// variant fails to compile there before it can reach a caller. This list
    /// is the runtime half of that guarantee: it proves each variant actually
    /// fills all five fields rather than merely having an arm.
    fn every_rejection() -> Vec<WorkflowError> {
        let incompat = || {
            Box::new(PortSchemaIncompatDetails {
                from_node: node("a"),
                to_node: node("b"),
                from_port: None,
                to_port: None,
                incompatibilities: Vec::new(),
            })
        };
        vec![
            WorkflowError::EmptyName,
            WorkflowError::NoNodes,
            WorkflowError::DuplicateNodeKey(node("a")),
            WorkflowError::UnknownNode(node("a")),
            WorkflowError::SelfLoop(node("a")),
            WorkflowError::CycleDetected,
            WorkflowError::NoEntryNodes,
            WorkflowError::InvalidParameterReference {
                node_key: node("a"),
                source_node_key: node("b"),
            },
            WorkflowError::ReferenceWithoutConnection {
                node_key: node("a"),
                source_node_key: node("b"),
            },
            WorkflowError::InvalidActionKey {
                key: "bad key".to_owned(),
                reason: "keys must be dotted".to_owned(),
            },
            WorkflowError::InvalidPluginKey {
                key: "bad key".to_owned(),
                reason: "keys must be lowercase".to_owned(),
            },
            WorkflowError::InvalidTrigger {
                reason: "cron expression is empty".to_owned(),
            },
            WorkflowError::UnsupportedSchema { version: 9, max: 1 },
            WorkflowError::InvalidOwnerId,
            WorkflowError::GraphError("edge resolution failed".to_owned()),
            WorkflowError::DuplicateConnection {
                from: node("a"),
                to: node("b"),
            },
            WorkflowError::PortSchemaIncompatible(incompat()),
            WorkflowError::PortSchemaUndecidable(Box::new(PortSchemaUndecidableDetails {
                from_node: node("a"),
                to_node: node("b"),
                from_port: None,
                to_port: None,
                reasons: Vec::new(),
            })),
            WorkflowError::InvalidRetryConfig {
                node: Some(node("a")),
                reason: "max_attempts must be >= 1".to_owned(),
            },
            WorkflowError::InvalidRetryConfig {
                node: None,
                reason: "max_attempts must be >= 1".to_owned(),
            },
            WorkflowError::ReferencePathUnresolved(Box::new(ReferencePathUnresolvedDetails {
                consumer_node: node("b"),
                param_key: "input".to_owned(),
                producer_node: node("a"),
                output_path: "$.items[0]".to_owned(),
                reason: "descend past leaf".to_owned(),
            })),
            WorkflowError::ReferenceTypeIncompatible(Box::new(ReferenceTypeIncompatDetails {
                consumer_node: node("b"),
                param_key: "input".to_owned(),
                producer_node: node("a"),
                output_path: "$.count".to_owned(),
                incompatibilities: Vec::new(),
            })),
            WorkflowError::ReferenceTypeUndecidable(Box::new(ReferenceTypeUndecidableDetails {
                consumer_node: node("b"),
                param_key: "input".to_owned(),
                producer_node: node("a"),
                output_path: "$.count".to_owned(),
                reasons: Vec::new(),
            })),
        ]
    }

    #[test]
    fn every_workflow_rejection_reports_all_five_fields() {
        for rejection in every_rejection() {
            let diagnostics = rejection.activation_diagnostics();
            assert!(
                !diagnostics.is_empty(),
                "a rejection with nothing to report is not actionable: {rejection:?}"
            );
            for reported in diagnostics {
                for (name, field) in [
                    ("code", reported.code()),
                    ("path", reported.path()),
                    ("expected", reported.expected()),
                    ("actual", reported.actual()),
                    ("remediation", reported.remediation()),
                ] {
                    assert!(
                        !field.trim().is_empty(),
                        "NS14 requires all five fields; `{name}` was blank for {rejection:?}"
                    );
                }
            }
        }
    }

    /// Codes are the machine-readable half of the contract, so two different
    /// rejections must never answer to the same one.
    #[test]
    fn each_rejection_carries_a_distinct_stable_code() {
        let mut codes: Vec<String> = every_rejection()
            .iter()
            .flat_map(|rejection| {
                rejection
                    .activation_diagnostics()
                    .into_iter()
                    .map(|reported| reported.code().to_owned())
            })
            .collect();
        codes.sort();
        codes.dedup();

        // Both `InvalidRetryConfig` fixtures share one code by design; every
        // other variant contributes its own.
        assert_eq!(
            codes.len(),
            every_rejection().len() - 1,
            "two rejections must not answer to the same code"
        );
        assert!(codes.iter().all(|code| code.starts_with("WORKFLOW:")));
    }

    /// The path points at the element the author has to change, so a UI can
    /// highlight it instead of making the author search.
    #[test]
    fn the_path_names_the_offending_element() {
        let reported =
            WorkflowError::ReferenceTypeIncompatible(Box::new(ReferenceTypeIncompatDetails {
                consumer_node: node("consumer"),
                param_key: "input".to_owned(),
                producer_node: node("producer"),
                output_path: "$.count".to_owned(),
                incompatibilities: Vec::new(),
            }))
            .activation_diagnostics();

        let only = reported.first().expect("one diagnostic is reported");
        assert_eq!(only.path(), "/nodes/consumer/parameters/input");

        let workflow_default = WorkflowError::InvalidRetryConfig {
            node: None,
            reason: "max_attempts must be >= 1".to_owned(),
        }
        .activation_diagnostics();
        assert_eq!(
            workflow_default
                .first()
                .expect("one diagnostic is reported")
                .path(),
            "/config/retry_policy",
            "a workflow-default policy points at the config, not at a node"
        );
    }
}
