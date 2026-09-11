//! Engine error types.

use nebula_action::ActionError;
use nebula_core::{NodeKey, PortKey, id::ExecutionId};
use nebula_workflow::NodeState;

/// Errors from the engine layer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EngineError {
    /// The command's acceptance could not be confirmed by its execution owner.
    #[error("execution control handoff could not be established")]
    ControlTurnHandoff {
        /// Typed storage cause, excluded from the boundary message.
        #[source]
        source: nebula_storage_port::StorageError,
    },
    /// Execution changed while the command was prepared.
    #[error("execution control version changed during preflight")]
    ControlTurnVersionConflict {
        /// Validated version.
        expected: u64,
        /// Version observed by the execution owner.
        actual: u64,
    },
    /// Stop a turn after its control checkpoint or acknowledgement was lost.
    #[error("execution control acceptance was interrupted")]
    ControlTurnInterrupted,
    /// The acknowledged lease could not be renewed before action dispatch.
    #[error("execution lease adoption could not be confirmed")]
    LeaseAdoption {
        /// Typed storage diagnosis, excluded from the boundary message.
        #[source]
        source: nebula_storage_port::StorageError,
    },
    /// The validated recovery snapshot changed before the owner grant.
    #[error("execution recovery version changed during preflight")]
    RecoveryVersionConflict {
        /// Validated execution version.
        expected: u64,
        /// Execution version observed by its owner.
        actual: u64,
    },
    /// The execution owner could not confirm recovery authority.
    #[error("execution recovery handoff could not be established")]
    RecoveryHandoff {
        /// Typed storage cause, excluded from the boundary message.
        #[source]
        source: nebula_storage_port::StorageError,
    },
    /// The execution owner's snapshot could not be read.
    #[error("persisted execution could not be read")]
    ExecutionRead {
        /// Typed storage cause, excluded from the boundary message.
        #[source]
        source: nebula_storage_port::StorageError,
    },
    /// Atomic control delivery handoff could not be established.
    #[error("control Start handoff could not be established")]
    ControlStartHandoff {
        /// Typed storage diagnosis; boundary messages remain payload-free.
        #[source]
        source: nebula_storage_port::StorageError,
    },
    /// The validated execution snapshot changed before ownership transfer.
    #[error("control Start execution version changed during preflight")]
    ControlStartVersionConflict {
        /// Version validated before handoff.
        expected: u64,
        /// Version observed atomically by the execution owner.
        actual: u64,
    },
    /// Durable remote-effect protocol failure, with no provider payload in diagnostics.
    #[error(transparent)]
    Effect(#[from] crate::EffectExecutionError),
    /// A durable turn lacks a paired catalog and frozen factory snapshot.
    #[error("exact durable runtime is not configured")]
    MissingExactRuntime,
    /// Both exact plan and worker flavor pins are mandatory for durable turns.
    #[error("durable execution is missing exact revision pins")]
    MissingRevisionPins,
    /// Exact catalog integrity or registry compatibility failed.
    #[error("exact durable revision could not be loaded: {source}")]
    ExactRevision {
        /// Redacted catalog/compatibility diagnosis.
        #[source]
        source: Box<crate::revision_catalog::PlanFlavorRevisionBridgeError>,
    },
    /// The recorded plan could not be represented by the scheduler.
    #[error("exact graph projection failed: {source}")]
    ExactGraphProjection {
        /// Typed, value-free projection error.
        #[source]
        source: nebula_plugin::ExecutionGraphProjectionError,
    },
    /// Recorded state does not match the pinned graph.
    #[error("persisted execution does not match its exact graph")]
    InvalidRecordedExecution,
    /// Durable execution has no execution-owned immutable contract.
    #[error("persisted execution contract is missing")]
    MissingContractBundle,
    /// Stored contract identities disagree with the execution or exact plan.
    #[error("persisted execution contract is invalid")]
    InvalidRecordedContract,
    /// Replay evidence is missing, malformed, or inconsistent with recorded state.
    #[error("persisted node checkpoint is invalid")]
    InvalidRecordedCheckpoint,
    /// Full retained replay evidence exceeds the admitted output limit.
    #[error("node checkpoint exceeds the admitted output limit")]
    CheckpointPayloadLimit,
    /// Recorded contract fingerprint or wire protocol integrity failed.
    #[error("persisted execution contract integrity failed")]
    ContractBundleIntegrity {
        /// Typed structural diagnosis, containing no workflow payload.
        #[source]
        source: nebula_execution::ExecutionContractBundleIntegrityError,
    },
    /// The execution owner's contract could not be read.
    #[error("persisted execution contract could not be read")]
    ContractBundleRead {
        /// Storage diagnosis retained for the error chain.
        #[source]
        source: nebula_storage_port::StorageError,
    },
    /// An exact graph factory or version is unavailable.
    #[error("exact graph action factory is unavailable")]
    ExactFactoryUnavailable,
    /// Graph bindings require owner-authenticated resolution.
    #[error("exact graph requires authenticated binding admission")]
    UnresolvedPlanBindings,
    /// Recorded semantics are not implemented by this runtime.
    #[error("exact graph requests unsupported runtime semantics")]
    UnsupportedRecordedSemantics,
    /// Durable execution limits are absent or invalid.
    #[error("persisted execution budget is missing or invalid")]
    InvalidRecordedBudget,
    /// A fresh durable execution must enter through atomic start acceptance.
    #[error("persistent execution requires durable start acceptance")]
    PersistentStartRequiresAcceptance,
    /// A referenced node was not found in the workflow.
    #[error("node not found: {node_key}")]
    NodeNotFound {
        /// The missing node ID.
        node_key: NodeKey,
    },

    /// Execution planning failed.
    #[error("planning failed: {0}")]
    PlanningFailed(String),

    /// A node failed during execution.
    #[error("node {node_key} failed: {error}")]
    NodeFailed {
        /// The node that failed.
        node_key: NodeKey,
        /// The error message.
        error: String,
    },

    /// The execution was cancelled.
    #[error("execution cancelled")]
    Cancelled,

    /// Parameter resolution failed (expression eval, reference lookup, etc.)
    ///
    /// Schema admission and expression failures retain a typed, redacted
    /// [`nebula_schema::ValidationError`] source. Missing predecessor references
    /// have no upstream cause and carry `None`.
    #[error("parameter resolution failed for node {node_key}, param '{param_key}': {error}")]
    ParameterResolution {
        /// The node whose parameter could not be resolved.
        node_key: NodeKey,
        /// The parameter key that failed.
        param_key: String,
        /// Human-readable description of the failure.
        error: String,
        /// Typed schema-bound failure with private payloads and evaluator causes
        /// redacted throughout its public error chain. `None` for references
        /// that have no upstream error.
        #[source]
        source: Option<Box<nebula_schema::ValidationError>>,
    },

    /// Parameter validation failed against the action's schema.
    #[error("parameter validation failed for node {node_key}: {errors}")]
    ParameterValidation {
        /// The node whose parameters failed validation.
        node_key: NodeKey,
        /// Combined validation error messages.
        errors: String,
    },

    /// Edge condition evaluation failed.
    #[error("edge evaluation failed from {from_node} to {to_node}: {error}")]
    EdgeEvaluationFailed {
        /// Source node of the edge.
        from_node: NodeKey,
        /// Target node of the edge.
        to_node: NodeKey,
        /// The underlying error.
        error: String,
    },

    /// A connection routes to an output port the source action never declared.
    ///
    /// Raised by the fresh-execution pre-flight (`validate_declared_output_ports`)
    /// before any node dispatches, so a mistyped/undeclared `from_port` fails
    /// the execution loudly instead of silently never firing. Fail-open by
    /// design when the source action is unregistered or declares a dynamic
    /// port — see the pre-flight's doc comment for why.
    #[error(
        "node {from_node} routes to {to_node} on undeclared output port \
         {port}; source action declares [{}]", declared.join(", ")
    )]
    UndeclaredOutputPort {
        /// Source node of the offending connection.
        from_node: NodeKey,
        /// Target node of the offending connection.
        to_node: NodeKey,
        /// The connection's effective source port, not declared by the
        /// source action (and not `"error"`).
        port: PortKey,
        /// The source action's actually-declared output-port keys, for
        /// operator diagnostics.
        declared: Vec<PortKey>,
    },

    /// A named target port is not a declared support binding. Root data flow
    /// requires the default input connection with no explicit target port.
    #[error(
        "node {from_node} routes to {to_node} on unsupported input port {port}; root flow requires the default input"
    )]
    UnsupportedInputPort {
        /// Source node of the offending connection.
        from_node: NodeKey,
        /// Target node of the offending connection.
        to_node: NodeKey,
        /// Named target port that cannot receive this binding.
        port: PortKey,
    },

    /// A required support port has no enabled incoming connection.
    #[error("node {to_node} requires support input port {port}")]
    MissingRequiredSupportInput {
        /// Target node declaring the required port.
        to_node: NodeKey,
        /// Required support port.
        port: PortKey,
    },

    /// A single-valued support port has more than one enabled connection.
    #[error("node {to_node} support input port {port} accepts one connection, got {actual}")]
    SupportInputMultiplicity {
        /// Target node declaring the port.
        to_node: NodeKey,
        /// Single-valued support port.
        port: PortKey,
        /// Number of enabled incoming connections.
        actual: usize,
    },

    /// A support-port connection does not satisfy its source filter.
    #[error("node {from_node} is not allowed on {to_node} support input port {port}")]
    SupportInputFiltered {
        /// Rejected source node.
        from_node: NodeKey,
        /// Target node declaring the filter.
        to_node: NodeKey,
        /// Filtered support port.
        port: PortKey,
    },

    /// A budget limit was exceeded.
    #[error("budget exceeded: {0}")]
    BudgetExceeded(String),

    /// Error from the runtime layer.
    #[error("runtime error: {0}")]
    Runtime(#[from] crate::runtime::RuntimeError),

    /// In-process metric registry rejected a primitive registration (name
    /// collision across kinds, histogram bucket mismatch, etc.).
    #[error("telemetry metrics error: {0}")]
    Telemetry(#[from] nebula_metrics::MetricsError),

    /// Error from the execution state layer.
    #[error("execution error: {0}")]
    Execution(#[from] nebula_execution::ExecutionError),

    /// A task panicked during execution.
    #[error("task panicked: {0}")]
    TaskPanicked(String),

    /// A typed [`ActionError`] bubbled up from the action/dispatch layer.
    ///
    /// Used by the engine's pre-dispatch pipeline (e.g. proactive
    /// credential refresh) to surface typed errors through the normal
    /// `ErrorStrategy` decision path instead of logging-and-continuing.
    /// Downstream consumers can match on the inner variant to distinguish
    /// `CredentialRefreshFailed` from other failure modes.
    #[error("action failed: {0}")]
    Action(#[from] ActionError),

    /// The frontier loop exited while one or more nodes were still in a
    /// non-terminal state (e.g. `Pending` / `Ready` / `Running`).
    ///
    /// Per `docs/PRODUCT_CANON.md` , the engine must be the single source
    /// of truth for execution status and must not silently report `Completed`
    /// on inconsistent state. This variant is produced when the frontier
    /// drains without `failed_node` or cancellation, yet `all_nodes_terminal`
    /// is false — almost always a scheduler bookkeeping bug.
    #[error(
        "frontier integrity violation: execution {execution_id} exited with \
         {} non-terminal node(s)",
        non_terminal_nodes.len()
    )]
    FrontierIntegrity {
        /// The execution whose frontier loop produced the inconsistent state.
        execution_id: ExecutionId,
        /// Nodes that were still non-terminal at the time the frontier
        /// loop exited, paired with their observed `NodeState`.
        non_terminal_nodes: Vec<(NodeKey, NodeState)>,
    },

    /// The engine could not persist a node-level checkpoint.
    ///
    /// Surfaced so that `run_frontier` aborts the node's progression instead
    /// of continuing on undurable state: per `docs/PRODUCT_CANON.md` /// (durability precedes visibility) and (no silent log-and-continue
    /// on state-transition failures), an unpersisted transition must never
    /// leak to observers or the frontier.
    #[error("checkpoint persist failed for node {node_key}: {reason}")]
    CheckpointFailed {
        /// The node whose checkpoint could not be committed.
        node_key: NodeKey,
        /// Underlying storage failure reason.
        reason: String,
    },

    /// The engine detected a persisted state transition driven by another
    /// actor (API cancel, sibling runner, admin mutation) that the local
    /// in-memory state cannot reconcile.
    ///
    /// Surfaced instead of silently overwriting the concurrent update
    /// (issue #333). Per `docs/PRODUCT_CANON.md` / , the engine
    /// may not report a successful completion when its final CAS write
    /// collided with an authoritative external transition that the
    /// engine could not honor (e.g. the row is still active-non-terminal
    /// at a newer version the engine did not produce).
    #[error(
        "state CAS conflict on execution {execution_id}: \
         expected version {expected_version}, observed {observed_version} \
         (external status: {observed_status:?})"
    )]
    CasConflict {
        /// Execution whose row moved beneath the engine.
        execution_id: ExecutionId,
        /// Version the engine believed was current before the write.
        expected_version: u64,
        /// Version actually present in the repo on CAS failure.
        observed_version: u64,
        /// Status the persisted state carried at `observed_version`.
        /// Rendered for operator diagnostics; not used for control flow.
        observed_status: String,
    },

    /// The runtime was asked to stop while this execution was still
    /// running, and the frontier did not finish inside the shutdown grace.
    ///
    /// The runner abandoned the step *deliberately*: it persisted no final
    /// state and released its execution lease, so a successor can take the
    /// execution over immediately rather than waiting out the lease TTL. The
    /// dispatch row stays unacknowledged and is redelivered by the next
    /// runner's reclaim sweep.
    ///
    /// This is not a failure of the workflow — nothing about the execution is
    /// known to be wrong — so it must never be mapped to a terminal status.
    #[error("execution {execution_id} abandoned: runtime shut down before the frontier finished")]
    ShutdownInterrupted {
        /// The execution whose step was abandoned mid-flight.
        execution_id: ExecutionId,
    },

    /// Another engine instance currently holds the execution lease.
    ///
    /// Surfaced by [`WorkflowEngine::execute_workflow`] and
    /// [`WorkflowEngine::resume_execution`] when `acquire_lease` fails
    /// because a live (non-expired) lease with a different holder is
    /// already recorded in storage. Per ADR 0008 and
    /// `docs/PRODUCT_CANON.md` , exactly one runner may dispatch
    /// nodes for an execution at a time; the second caller must back off
    /// rather than run in parallel.
    ///
    /// The caller (API handler, scheduler) is responsible for deciding
    /// how to react — the engine does not sleep-and-retry.
    ///
    /// [`WorkflowEngine::execute_workflow`]: crate::WorkflowEngine::execute_workflow
    /// [`WorkflowEngine::resume_execution`]: crate::WorkflowEngine::resume_execution
    #[error("execution {execution_id} is leased by another runner: {holder}")]
    Leased {
        /// The execution whose lease is already held.
        execution_id: ExecutionId,
        /// Holder string recorded in storage — surfaced for operator
        /// diagnostics ("which instance is running execution X right now").
        holder: String,
    },
}

impl EngineError {
    /// The typed [`ActionError`] this engine error carries, if any.
    ///
    /// An in-flight action failure surfaces either as the bare
    /// [`EngineError::Action`] variant or, when it travelled through the
    /// runtime dispatcher, wrapped inside [`EngineError::Runtime`] as a
    /// [`crate::runtime::RuntimeError::ActionError`]. The frontier loop
    /// consults this so [`ActionError::is_fatal`] on the just-recorded
    /// attempt can finalize the node *before* the retry policy runs — a
    /// fatal action error must never be re-dispatched by attempts/budget
    /// policy.
    #[must_use]
    pub fn as_action_error(&self) -> Option<&ActionError> {
        match self {
            Self::Action(e) => Some(e),
            Self::Runtime(e) => e.as_action_error(),
            _ => None,
        }
    }
}

impl nebula_error::Classify for EngineError {
    fn category(&self) -> nebula_error::ErrorCategory {
        match self {
            Self::Effect(error) if error.is_deferred() => nebula_error::ErrorCategory::Unavailable,
            Self::Effect(_) => nebula_error::ErrorCategory::External,
            Self::MissingExactRuntime
            | Self::ControlTurnHandoff { .. }
            | Self::ControlTurnInterrupted
            | Self::LeaseAdoption { .. }
            | Self::RecoveryHandoff { .. }
            | Self::ExecutionRead { .. }
            | Self::ControlStartHandoff { .. }
            | Self::ExactFactoryUnavailable
            | Self::ContractBundleRead { .. }
            | Self::ExactRevision { .. } => nebula_error::ErrorCategory::Unavailable,
            Self::MissingRevisionPins
            | Self::ExactGraphProjection { .. }
            | Self::InvalidRecordedExecution
            | Self::MissingContractBundle
            | Self::InvalidRecordedContract
            | Self::InvalidRecordedCheckpoint
            | Self::ContractBundleIntegrity { .. }
            | Self::UnresolvedPlanBindings
            | Self::UnsupportedRecordedSemantics
            | Self::InvalidRecordedBudget
            | Self::PersistentStartRequiresAcceptance => nebula_error::ErrorCategory::Validation,
            Self::NodeNotFound { .. } => nebula_error::ErrorCategory::NotFound,
            Self::PlanningFailed(_)
            | Self::ParameterResolution { .. }
            | Self::ParameterValidation { .. }
            | Self::EdgeEvaluationFailed { .. }
            | Self::UndeclaredOutputPort { .. }
            | Self::UnsupportedInputPort { .. }
            | Self::MissingRequiredSupportInput { .. }
            | Self::SupportInputMultiplicity { .. }
            | Self::SupportInputFiltered { .. } => nebula_error::ErrorCategory::Validation,
            Self::NodeFailed { .. }
            | Self::TaskPanicked(_)
            | Self::FrontierIntegrity { .. }
            | Self::CheckpointFailed { .. }
            | Self::CasConflict { .. } => nebula_error::ErrorCategory::Internal,
            Self::Cancelled => nebula_error::ErrorCategory::Cancelled,
            Self::BudgetExceeded(_) | Self::CheckpointPayloadLimit => {
                nebula_error::ErrorCategory::Exhausted
            },
            // Leased is a transient coordination conflict — a second
            // runner saw the execution already in flight. Conflict
            // matches HTTP 409 at the API edge.
            Self::Leased { .. }
            | Self::ControlTurnVersionConflict { .. }
            | Self::ControlStartVersionConflict { .. }
            | Self::RecoveryVersionConflict { .. } => nebula_error::ErrorCategory::Conflict,
            // Nothing is wrong with the execution — this runner simply stopped
            // first. Unavailable, so the caller retries against a live runner
            // rather than reporting a workflow fault.
            Self::ShutdownInterrupted { .. } => nebula_error::ErrorCategory::Unavailable,
            Self::Telemetry(_) => nebula_error::ErrorCategory::Internal,
            Self::Runtime(e) => nebula_error::Classify::category(e),
            Self::Execution(e) => nebula_error::Classify::category(e),
            Self::Action(e) => nebula_error::Classify::category(e),
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        nebula_error::ErrorCode::new(match self {
            Self::Effect(error) => error.code(),
            Self::ControlTurnHandoff { .. } => "ENGINE:CONTROL_TURN_HANDOFF",
            Self::ControlTurnVersionConflict { .. } => "ENGINE:CONTROL_TURN_VERSION_CONFLICT",
            Self::ControlTurnInterrupted => "ENGINE:CONTROL_TURN_INTERRUPTED",
            Self::LeaseAdoption { .. } => "ENGINE:LEASE_ADOPTION",
            Self::RecoveryHandoff { .. } => "ENGINE:RECOVERY_HANDOFF",
            Self::RecoveryVersionConflict { .. } => "ENGINE:RECOVERY_VERSION_CONFLICT",
            Self::ExecutionRead { .. } => "ENGINE:EXECUTION_READ",
            Self::ControlStartHandoff { .. } => "ENGINE:CONTROL_START_HANDOFF",
            Self::ControlStartVersionConflict { .. } => "ENGINE:CONTROL_START_VERSION_CONFLICT",
            Self::MissingExactRuntime => "ENGINE:MISSING_EXACT_RUNTIME",
            Self::MissingRevisionPins => "ENGINE:MISSING_REVISION_PINS",
            Self::ExactRevision { .. } => "ENGINE:EXACT_REVISION",
            Self::ExactGraphProjection { .. } => "ENGINE:EXACT_GRAPH_PROJECTION",
            Self::InvalidRecordedExecution => "ENGINE:INVALID_RECORDED_EXECUTION",
            Self::MissingContractBundle => "ENGINE:MISSING_CONTRACT_BUNDLE",
            Self::InvalidRecordedContract => "ENGINE:INVALID_RECORDED_CONTRACT",
            Self::InvalidRecordedCheckpoint => "ENGINE:INVALID_RECORDED_CHECKPOINT",
            Self::CheckpointPayloadLimit => "ENGINE:CHECKPOINT_PAYLOAD_LIMIT",
            Self::ContractBundleIntegrity { .. } => "ENGINE:CONTRACT_BUNDLE_INTEGRITY",
            Self::ContractBundleRead { .. } => "ENGINE:CONTRACT_BUNDLE_READ",
            Self::ExactFactoryUnavailable => "ENGINE:EXACT_FACTORY_UNAVAILABLE",
            Self::UnresolvedPlanBindings => "ENGINE:UNRESOLVED_PLAN_BINDINGS",
            Self::UnsupportedRecordedSemantics => "ENGINE:UNSUPPORTED_RECORDED_SEMANTICS",
            Self::InvalidRecordedBudget => "ENGINE:INVALID_RECORDED_BUDGET",
            Self::PersistentStartRequiresAcceptance => {
                "ENGINE:PERSISTENT_START_REQUIRES_ACCEPTANCE"
            },
            Self::NodeNotFound { .. } => "ENGINE:NODE_NOT_FOUND",
            Self::PlanningFailed(_) => "ENGINE:PLANNING_FAILED",
            Self::NodeFailed { .. } => "ENGINE:NODE_FAILED",
            Self::Cancelled => "ENGINE:CANCELLED",
            Self::ParameterResolution { .. } => "ENGINE:PARAM_RESOLUTION",
            Self::ParameterValidation { .. } => "ENGINE:PARAM_VALIDATION",
            Self::EdgeEvaluationFailed { .. } => "ENGINE:EDGE_EVAL",
            Self::UndeclaredOutputPort { .. } => "ENGINE:UNDECLARED_OUTPUT_PORT",
            Self::UnsupportedInputPort { .. } => "ENGINE:UNSUPPORTED_INPUT_PORT",
            Self::MissingRequiredSupportInput { .. } => "ENGINE:MISSING_SUPPORT_INPUT",
            Self::SupportInputMultiplicity { .. } => "ENGINE:SUPPORT_INPUT_MULTIPLICITY",
            Self::SupportInputFiltered { .. } => "ENGINE:SUPPORT_INPUT_FILTERED",
            Self::BudgetExceeded(_) => "ENGINE:BUDGET_EXCEEDED",
            Self::Runtime(e) => return nebula_error::Classify::code(e),
            Self::Execution(e) => return nebula_error::Classify::code(e),
            Self::Action(e) => return nebula_error::Classify::code(e),
            Self::TaskPanicked(_) => "ENGINE:TASK_PANICKED",
            Self::FrontierIntegrity { .. } => "ENGINE:FRONTIER_INTEGRITY",
            Self::CheckpointFailed { .. } => "ENGINE:CHECKPOINT_FAILED",
            Self::CasConflict { .. } => "ENGINE:CAS_CONFLICT",
            Self::Leased { .. } => "ENGINE:LEASED",
            Self::ShutdownInterrupted { .. } => "ENGINE:SHUTDOWN_INTERRUPTED",
            Self::Telemetry(_) => "ENGINE:TELEMETRY",
        })
    }

    fn is_retryable(&self) -> bool {
        match self {
            Self::Telemetry(_) => false,
            Self::Runtime(e) => nebula_error::Classify::is_retryable(e),
            Self::Execution(e) => nebula_error::Classify::is_retryable(e),
            Self::Action(e) => nebula_error::Classify::is_retryable(e),
            _ => self.category().is_default_retryable(),
        }
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::node_key;

    use super::*;

    #[test]
    fn planning_failed_display() {
        let err = EngineError::PlanningFailed("no nodes".into());
        assert_eq!(err.to_string(), "planning failed: no nodes");
    }

    #[test]
    fn cancelled_display() {
        let err = EngineError::Cancelled;
        assert_eq!(err.to_string(), "execution cancelled");
    }

    #[test]
    fn budget_exceeded_display() {
        let err = EngineError::BudgetExceeded("max retries".into());
        assert_eq!(err.to_string(), "budget exceeded: max retries");
    }

    #[test]
    fn node_failed_display() {
        let node_key = node_key!("test_node");
        let err = EngineError::NodeFailed {
            node_key,
            error: "timeout".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("timeout"));
        assert!(msg.contains("failed"));
    }

    #[test]
    fn leased_display_and_classification() {
        use nebula_core::id::ExecutionId;
        use nebula_error::{Classify, ErrorCategory};

        let exec_id = ExecutionId::new();
        let err = EngineError::Leased {
            execution_id: exec_id,
            holder: "nbl_01HZABC".into(),
        };

        let msg = err.to_string();
        assert!(msg.contains("leased"));
        assert!(msg.contains("nbl_01HZABC"));
        assert!(msg.contains(&exec_id.to_string()));

        // Leased maps to Conflict — HTTP 409 at the API edge, client-
        // side error because the caller should back off rather than
        // treat it as retryable-server-error (ADR 0008).
        assert_eq!(Classify::category(&err), ErrorCategory::Conflict);
        assert_eq!(Classify::code(&err).as_str(), "ENGINE:LEASED");
        assert!(!Classify::is_retryable(&err));
    }

    #[test]
    fn action_variant_returns_some() {
        // The bare `EngineError::Action` wrapper must expose its inner
        // `ActionError` so the frontier loop can consult `is_fatal` on the
        // just-recorded attempt before retry policy runs.
        let err = EngineError::Action(ActionError::fatal("bad schema"));
        let inner = err
            .as_action_error()
            .expect("Action variant must yield Some(&ActionError)");
        assert!(
            matches!(inner, ActionError::Fatal { .. }),
            "the wrapped ActionError must round-trip unchanged, got {inner:?}"
        );
    }

    #[test]
    fn runtime_action_error_returns_some() {
        // An action failure that travelled through the runtime dispatcher
        // surfaces as `EngineError::Runtime(RuntimeError::ActionError(..))`
        // — `as_action_error` must see through the Runtime wrapper too.
        let err = EngineError::Runtime(crate::runtime::RuntimeError::ActionError(
            ActionError::retryable("transient"),
        ));
        let inner = err
            .as_action_error()
            .expect("Runtime-wrapped ActionError must yield Some(&ActionError)");
        assert!(
            matches!(inner, ActionError::Retryable { .. }),
            "the action error must surface through EngineError::Runtime, got {inner:?}"
        );
    }

    #[test]
    fn non_action_variant_returns_none() {
        // A non-action engine error has no inner `ActionError`.
        assert!(
            EngineError::Cancelled.as_action_error().is_none(),
            "a non-action variant must yield None"
        );
    }

    #[test]
    fn frontier_integrity_display_and_classification() {
        use nebula_core::id::ExecutionId;
        use nebula_error::{Classify, ErrorCategory};

        let exec_id = ExecutionId::new();
        let err = EngineError::FrontierIntegrity {
            execution_id: exec_id,
            non_terminal_nodes: vec![
                (node_key!("a"), NodeState::Pending),
                (node_key!("b"), NodeState::Running),
            ],
        };

        let msg = err.to_string();
        assert!(msg.contains("frontier integrity violation"));
        assert!(msg.contains("2 non-terminal"));
        assert!(msg.contains(&exec_id.to_string()));

        assert_eq!(Classify::category(&err), ErrorCategory::Internal);
        assert_eq!(
            Classify::code(&err).as_str(),
            "ENGINE:FRONTIER_INTEGRITY",
            "stable error code for operators / dashboards"
        );
    }
}
