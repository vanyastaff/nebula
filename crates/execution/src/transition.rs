//! State machine transition validation for execution and node states.

use nebula_workflow::NodeState;

use crate::error::ExecutionError;
use crate::status::ExecutionStatus;

/// Returns `true` if the execution-level transition from `from` to `to` is valid.
#[must_use]
pub fn can_transition_execution(from: ExecutionStatus, to: ExecutionStatus) -> bool {
    matches!(
        (from, to),
        (ExecutionStatus::Created, ExecutionStatus::Running)
            | (ExecutionStatus::Running, ExecutionStatus::Paused)
            | (ExecutionStatus::Running, ExecutionStatus::Cancelling)
            | (ExecutionStatus::Running, ExecutionStatus::Completed)
            | (ExecutionStatus::Running, ExecutionStatus::Failed)
            | (ExecutionStatus::Running, ExecutionStatus::TimedOut)
            | (ExecutionStatus::Paused, ExecutionStatus::Running)
            | (ExecutionStatus::Paused, ExecutionStatus::Cancelling)
            | (ExecutionStatus::Cancelling, ExecutionStatus::Cancelled)
            | (ExecutionStatus::Cancelling, ExecutionStatus::Failed)
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
        (NodeState::Pending, NodeState::Ready)
            | (NodeState::Pending, NodeState::Skipped)
            | (NodeState::Pending, NodeState::Cancelled)
            | (NodeState::Ready, NodeState::Running)
            | (NodeState::Ready, NodeState::Skipped)
            | (NodeState::Ready, NodeState::Cancelled)
            | (NodeState::Running, NodeState::Completed)
            | (NodeState::Running, NodeState::Failed)
            | (NodeState::Running, NodeState::Cancelled)
            | (NodeState::Failed, NodeState::Retrying)
            | (NodeState::Failed, NodeState::Cancelled)
            | (NodeState::Retrying, NodeState::Running)
            | (NodeState::Retrying, NodeState::Failed)
            | (NodeState::Retrying, NodeState::Cancelled)
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
