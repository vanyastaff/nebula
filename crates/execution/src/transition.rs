//! State machine transition validation for execution and node states.

use nebula_workflow::NodeState;

use crate::{error::ExecutionError, status::ExecutionStatus};

/// Returns `true` if the execution-level transition from `from` to `to` is valid.
///
/// `Created → Cancelled`, `Paused → Failed`, and `Paused → TimedOut` are
/// reachable under normal operation (pre-start cancel, out-of-band failure
/// on a paused run, global deadline firing on a paused run) and must not
/// force a phantom `Running` bridge into the audit trail (issue #273).
#[must_use]
pub fn can_transition_execution(from: ExecutionStatus, to: ExecutionStatus) -> bool {
    matches!(
        (from, to),
        (
            ExecutionStatus::Created | ExecutionStatus::Paused,
            ExecutionStatus::Running
        ) | (
            ExecutionStatus::Created | ExecutionStatus::Cancelling,
            ExecutionStatus::Cancelled
        ) | (
            ExecutionStatus::Running,
            ExecutionStatus::Paused
                | ExecutionStatus::Cancelling
                | ExecutionStatus::Completed
                | ExecutionStatus::Failed
                | ExecutionStatus::TimedOut
        ) | (
            ExecutionStatus::Paused,
            ExecutionStatus::Cancelling | ExecutionStatus::Failed | ExecutionStatus::TimedOut
        ) | (
            ExecutionStatus::Cancelling,
            ExecutionStatus::Failed | ExecutionStatus::Completed | ExecutionStatus::TimedOut
        )
    )
}

/// Validate an execution-level transition, returning an error if invalid.
pub fn validate_execution_transition(
    from: ExecutionStatus,
    to: ExecutionStatus,
) -> Result<(), ExecutionError> {
    if can_transition_execution(from, to) {
        Ok(())
    } else {
        Err(ExecutionError::invalid_execution_transition(from, to))
    }
}

/// Returns `true` if the node-level transition from `from` to `to` is valid.
#[must_use]
pub fn can_transition_node(from: NodeState, to: NodeState) -> bool {
    matches!(
        (from, to),
        (
            NodeState::Pending,
            NodeState::Ready | NodeState::Skipped | NodeState::Cancelled
        ) | (NodeState::Ready | NodeState::Retrying, NodeState::Running)
            | (NodeState::Ready, NodeState::Skipped | NodeState::Cancelled)
            | (
                NodeState::Running,
                NodeState::Completed | NodeState::Failed | NodeState::Cancelled
            )
            | (
                NodeState::Failed,
                NodeState::Retrying | NodeState::Cancelled
            )
            | (
                NodeState::Retrying,
                NodeState::Failed | NodeState::Cancelled
            )
    )
}

/// Validate a node-level transition, returning an error if invalid.
pub fn validate_node_transition(from: NodeState, to: NodeState) -> Result<(), ExecutionError> {
    if can_transition_node(from, to) {
        Ok(())
    } else {
        Err(ExecutionError::InvalidTransition {
            from: from.to_string(),
            to: to.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_execution_transitions() {
        assert!(can_transition_execution(
            ExecutionStatus::Created,
            ExecutionStatus::Running
        ));
        assert!(can_transition_execution(
            ExecutionStatus::Running,
            ExecutionStatus::Completed
        ));
        assert!(can_transition_execution(
            ExecutionStatus::Running,
            ExecutionStatus::Failed
        ));
        assert!(can_transition_execution(
            ExecutionStatus::Running,
            ExecutionStatus::Paused
        ));
        assert!(can_transition_execution(
            ExecutionStatus::Paused,
            ExecutionStatus::Running
        ));
        assert!(can_transition_execution(
            ExecutionStatus::Running,
            ExecutionStatus::Cancelling
        ));
        assert!(can_transition_execution(
            ExecutionStatus::Cancelling,
            ExecutionStatus::Cancelled
        ));
        assert!(can_transition_execution(
            ExecutionStatus::Running,
            ExecutionStatus::TimedOut
        ));
    }

    #[test]
    fn cancelling_can_transition_to_completed() {
        assert!(can_transition_execution(
            ExecutionStatus::Cancelling,
            ExecutionStatus::Completed
        ));
    }

    #[test]
    fn cancelling_can_transition_to_timed_out() {
        assert!(can_transition_execution(
            ExecutionStatus::Cancelling,
            ExecutionStatus::TimedOut
        ));
    }

    #[test]
    fn invalid_execution_transitions() {
        assert!(!can_transition_execution(
            ExecutionStatus::Created,
            ExecutionStatus::Completed
        ));
        assert!(!can_transition_execution(
            ExecutionStatus::Completed,
            ExecutionStatus::Running
        ));
        assert!(!can_transition_execution(
            ExecutionStatus::Cancelled,
            ExecutionStatus::Running
        ));
        assert!(!can_transition_execution(
            ExecutionStatus::Created,
            ExecutionStatus::Created
        ));
    }

    /// Regression for issue #273: out-of-band failures (credential rotation,
    /// downstream outage, supervisor death) can land on a Paused execution;
    /// they must be expressible without a phantom `Paused → Running → Failed`
    /// detour that pollutes the audit trail.
    #[test]
    fn paused_can_transition_to_failed() {
        assert!(can_transition_execution(
            ExecutionStatus::Paused,
            ExecutionStatus::Failed
        ));
    }

    /// Regression for issue #273: global deadline timers fire regardless of
    /// execution status. A paused execution that blows its deadline must
    /// reach TimedOut directly.
    #[test]
    fn paused_can_transition_to_timed_out() {
        assert!(can_transition_execution(
            ExecutionStatus::Paused,
            ExecutionStatus::TimedOut
        ));
    }

    /// Regression for issue #273: cancelling a scheduled execution before
    /// the worker picks it up must be expressible in one step; the previous
    /// table forced `Created → Running → Cancelling → Cancelled`, which lies
    /// in the audit log about the run ever having run.
    #[test]
    fn created_can_transition_to_cancelled() {
        assert!(can_transition_execution(
            ExecutionStatus::Created,
            ExecutionStatus::Cancelled
        ));
    }

    /// Guard: the three new edges must not leak into illegal targets.
    /// `Created → Completed/Failed/TimedOut/Paused` stay invalid — only
    /// pre-start cancellation is allowed from `Created`.
    #[test]
    fn created_still_cannot_reach_other_terminals_directly() {
        assert!(!can_transition_execution(
            ExecutionStatus::Created,
            ExecutionStatus::Completed
        ));
        assert!(!can_transition_execution(
            ExecutionStatus::Created,
            ExecutionStatus::Failed
        ));
        assert!(!can_transition_execution(
            ExecutionStatus::Created,
            ExecutionStatus::TimedOut
        ));
        assert!(!can_transition_execution(
            ExecutionStatus::Created,
            ExecutionStatus::Paused
        ));
    }

    /// Guard: `Paused → Completed` is deliberately *not* in the table yet —
    /// the semantics of "done from paused" need engine-level agreement
    /// (see issue #273 "worth considering" note). Keep it rejected until
    /// that decision is explicit.
    #[test]
    fn paused_cannot_transition_to_completed_yet() {
        assert!(!can_transition_execution(
            ExecutionStatus::Paused,
            ExecutionStatus::Completed
        ));
    }

    #[test]
    fn validate_execution_transition_ok() {
        assert!(
            validate_execution_transition(ExecutionStatus::Created, ExecutionStatus::Running)
                .is_ok()
        );
    }

    #[test]
    fn validate_execution_transition_err() {
        let err =
            validate_execution_transition(ExecutionStatus::Completed, ExecutionStatus::Running)
                .unwrap_err();
        assert!(err.to_string().contains("invalid transition"));
    }

    #[test]
    fn valid_node_transitions() {
        assert!(can_transition_node(NodeState::Pending, NodeState::Ready));
        assert!(can_transition_node(NodeState::Ready, NodeState::Running));
        assert!(can_transition_node(
            NodeState::Running,
            NodeState::Completed
        ));
        assert!(can_transition_node(NodeState::Running, NodeState::Failed));
        assert!(can_transition_node(NodeState::Failed, NodeState::Retrying));
        assert!(can_transition_node(NodeState::Retrying, NodeState::Running));
        assert!(can_transition_node(
            NodeState::Pending,
            NodeState::Cancelled
        ));
        assert!(can_transition_node(NodeState::Pending, NodeState::Skipped));
    }

    #[test]
    fn invalid_node_transitions() {
        assert!(!can_transition_node(NodeState::Pending, NodeState::Running));
        assert!(!can_transition_node(
            NodeState::Completed,
            NodeState::Running
        ));
        assert!(!can_transition_node(NodeState::Skipped, NodeState::Running));
        assert!(!can_transition_node(
            NodeState::Cancelled,
            NodeState::Running
        ));
    }
}
