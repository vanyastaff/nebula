//! Execution-level status tracking.

use std::sync::Arc;

use nebula_core::NodeKey;
use serde::{Deserialize, Serialize};

/// The overall status of a workflow execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    /// Created but not yet started.
    Created,
    /// Actively running nodes.
    Running,
    /// Temporarily paused by the user or system.
    Paused,
    /// Cancellation has been requested; waiting for active nodes to drain.
    Cancelling,
    /// All nodes completed successfully.
    Completed,
    /// At least one node failed and the execution could not continue.
    Failed,
    /// Cancelled after a cancellation request was fully processed.
    Cancelled,
    /// The execution exceeded its wall-clock time budget.
    TimedOut,
}

impl ExecutionStatus {
    /// Returns `true` if the execution has reached a final state.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::TimedOut
        )
    }

    /// Returns `true` if the execution is currently doing work.
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Running | Self::Cancelling)
    }

    /// Returns `true` if the execution completed successfully.
    #[must_use]
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Completed)
    }

    /// Returns `true` if the execution ended in a failure state.
    #[must_use]
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failed | Self::TimedOut)
    }
}

impl std::fmt::Display for ExecutionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Created => write!(f, "created"),
            Self::Running => write!(f, "running"),
            Self::Paused => write!(f, "paused"),
            Self::Cancelling => write!(f, "cancelling"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::TimedOut => write!(f, "timed_out"),
        }
    }
}

/// Why a workflow execution reached its terminal state.
///
/// Attached to [`crate::ExecutionResult`] so that audit logs and UI can
/// distinguish intentional terminations (a `StopAction` or `FailAction`
/// node requested the end) from system-level terminations (crashes,
/// timeouts, cancellations) and from natural completion (all nodes
/// finished normally).
///
/// This does **not** replace [`ExecutionStatus`]. Status describes *what*
/// terminal state was reached; this enum describes *why*.
///
/// # v1 wiring status
///
/// The `ExplicitStop` / `ExplicitFail` variants are defined and serde-
/// round-trippable, but the engine is **not yet** populating them off
/// `ActionResult::Terminate`. Today a node returning `Terminate` only
/// causes its own downstream edges to be gated off in `evaluate_edge`;
/// the scheduler still drives `determine_final_status` from aggregate
/// node state, so executions that reach a `Terminate` end up with
/// `termination_reason == None` on the result. Full propagation is
/// tracked as Phase 3 of the ControlAction plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
#[non_exhaustive]
pub enum ExecutionTerminationReason {
    /// Execution completed naturally — all reachable nodes ran and the
    /// frontier drained without an explicit terminate signal.
    NaturalCompletion,

    /// A node explicitly requested successful termination via
    /// `ActionResult::Terminate { TerminationReason::Success }` (typically
    /// a `StopAction`).
    ///
    /// Parallel branches still running at the time are cancelled cleanly.
    ExplicitStop {
        /// The node that requested termination.
        by_node: NodeKey,
        /// Optional note describing why the node chose to stop early.
        note: Option<String>,
    },

    /// A node explicitly requested failed termination via
    /// `ActionResult::Terminate { TerminationReason::Failure }` (typically
    /// a `FailAction`).
    ///
    /// Parallel branches still running at the time are cancelled cleanly.
    ExplicitFail {
        /// The node that requested termination.
        by_node: NodeKey,
        /// Opaque error code identifier.
        ///
        /// See [`ExecutionTerminationCode`] — a thin newtype over
        /// `Arc<str>` that will be swapped to the structured `ErrorCode`
        /// in Phase 10 of the action-v2 roadmap without changing this
        /// public shape or wire format.
        code: ExecutionTerminationCode,
        /// Human-readable error message.
        message: String,
    },

    /// Execution was cancelled by the user, the API, or the engine shutting
    /// down — not by a node inside the workflow.
    Cancelled,

    /// Execution ended due to a system-level error: crash, panic, storage
    /// failure, engine timeout. Distinct from `ExplicitFail`, which is a
    /// deliberate in-workflow decision.
    SystemError,
}

/// Opaque identifier for an execution-level termination error.
///
/// Parallel to `nebula_action::TerminationCode` — but defined locally
/// here so that `nebula-execution` does not pick up a dependency on
/// `nebula-action` (wrong-direction layer edge). Currently backed by
/// `Arc<str>` and serialised as a bare JSON string via
/// `#[serde(transparent)]`; Phase 10 of the action-v2 roadmap will swap
/// the inner representation to a structured `ErrorCode` without changing
/// this public API or the wire format.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExecutionTerminationCode(Arc<str>);

impl ExecutionTerminationCode {
    /// Construct from anything convertible to `Arc<str>`.
    #[must_use]
    pub fn new(code: impl Into<Arc<str>>) -> Self {
        Self(code.into())
    }

    /// Borrow the underlying string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ExecutionTerminationCode {
    fn from(s: &str) -> Self {
        Self(Arc::from(s))
    }
}

impl From<String> for ExecutionTerminationCode {
    fn from(s: String) -> Self {
        Self(Arc::from(s))
    }
}

impl From<Arc<str>> for ExecutionTerminationCode {
    fn from(a: Arc<str>) -> Self {
        Self(a)
    }
}

impl std::fmt::Display for ExecutionTerminationCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::node_key;

    use super::*;

    #[test]
    fn terminal_states() {
        assert!(ExecutionStatus::Completed.is_terminal());
        assert!(ExecutionStatus::Failed.is_terminal());
        assert!(ExecutionStatus::Cancelled.is_terminal());
        assert!(ExecutionStatus::TimedOut.is_terminal());

        assert!(!ExecutionStatus::Created.is_terminal());
        assert!(!ExecutionStatus::Running.is_terminal());
        assert!(!ExecutionStatus::Paused.is_terminal());
        assert!(!ExecutionStatus::Cancelling.is_terminal());
    }

    #[test]
    fn active_states() {
        assert!(ExecutionStatus::Running.is_active());
        assert!(ExecutionStatus::Cancelling.is_active());

        assert!(!ExecutionStatus::Created.is_active());
        assert!(!ExecutionStatus::Paused.is_active());
        assert!(!ExecutionStatus::Completed.is_active());
    }

    #[test]
    fn success_state() {
        assert!(ExecutionStatus::Completed.is_success());
        assert!(!ExecutionStatus::Failed.is_success());
        assert!(!ExecutionStatus::Running.is_success());
    }

    #[test]
    fn failure_states() {
        assert!(ExecutionStatus::Failed.is_failure());
        assert!(ExecutionStatus::TimedOut.is_failure());
        assert!(!ExecutionStatus::Completed.is_failure());
        assert!(!ExecutionStatus::Cancelled.is_failure());
    }

    #[test]
    fn display_formatting() {
        assert_eq!(ExecutionStatus::Created.to_string(), "created");
        assert_eq!(ExecutionStatus::Running.to_string(), "running");
        assert_eq!(ExecutionStatus::Paused.to_string(), "paused");
        assert_eq!(ExecutionStatus::Cancelling.to_string(), "cancelling");
        assert_eq!(ExecutionStatus::Completed.to_string(), "completed");
        assert_eq!(ExecutionStatus::Failed.to_string(), "failed");
        assert_eq!(ExecutionStatus::Cancelled.to_string(), "cancelled");
        assert_eq!(ExecutionStatus::TimedOut.to_string(), "timed_out");
    }

    #[test]
    fn serde_roundtrip() {
        let statuses = [
            ExecutionStatus::Created,
            ExecutionStatus::Running,
            ExecutionStatus::Paused,
            ExecutionStatus::Cancelling,
            ExecutionStatus::Completed,
            ExecutionStatus::Failed,
            ExecutionStatus::Cancelled,
            ExecutionStatus::TimedOut,
        ];

        for status in &statuses {
            let json = serde_json::to_string(status).unwrap();
            let back: ExecutionStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(*status, back, "roundtrip failed for {status}");
        }
    }

    #[test]
    fn serde_rename_snake_case() {
        let json = serde_json::to_string(&ExecutionStatus::TimedOut).unwrap();
        assert_eq!(json, "\"timed_out\"");

        let json = serde_json::to_string(&ExecutionStatus::Created).unwrap();
        assert_eq!(json, "\"created\"");
    }

    #[test]
    fn copy_semantics() {
        let a = ExecutionStatus::Running;
        let b = a;
        assert_eq!(a, b);
    }

    // ── ExecutionTerminationReason ──────────────────────────────────

    #[test]
    fn termination_reason_natural_completion_roundtrip() {
        let reason = ExecutionTerminationReason::NaturalCompletion;
        let json = serde_json::to_string(&reason).unwrap();
        let back: ExecutionTerminationReason = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            ExecutionTerminationReason::NaturalCompletion
        ));
    }

    #[test]
    fn termination_reason_explicit_stop_roundtrip() {
        let reason = ExecutionTerminationReason::ExplicitStop {
            by_node: node_key!("test"),
            note: Some("duplicate detected".into()),
        };
        let json = serde_json::to_string(&reason).unwrap();
        let back: ExecutionTerminationReason = serde_json::from_str(&json).unwrap();
        match back {
            ExecutionTerminationReason::ExplicitStop { note, .. } => {
                assert_eq!(note.as_deref(), Some("duplicate detected"));
            },
            _ => panic!("expected ExplicitStop"),
        }
    }

    #[test]
    fn termination_reason_explicit_fail_roundtrip() {
        let reason = ExecutionTerminationReason::ExplicitFail {
            by_node: node_key!("test"),
            code: "E_VALIDATION".into(),
            message: "field `name` is required".into(),
        };
        let json = serde_json::to_string(&reason).unwrap();
        let back: ExecutionTerminationReason = serde_json::from_str(&json).unwrap();
        match back {
            ExecutionTerminationReason::ExplicitFail { code, message, .. } => {
                assert_eq!(code.as_str(), "E_VALIDATION");
                assert_eq!(message, "field `name` is required");
            },
            _ => panic!("expected ExplicitFail"),
        }
    }

    #[test]
    fn termination_reason_cancelled_roundtrip() {
        let reason = ExecutionTerminationReason::Cancelled;
        let json = serde_json::to_string(&reason).unwrap();
        let back: ExecutionTerminationReason = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, ExecutionTerminationReason::Cancelled));
    }

    #[test]
    fn termination_reason_system_error_roundtrip() {
        let reason = ExecutionTerminationReason::SystemError;
        let json = serde_json::to_string(&reason).unwrap();
        let back: ExecutionTerminationReason = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, ExecutionTerminationReason::SystemError));
    }
}
