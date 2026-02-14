//! Node execution state tracking.

use serde::{Deserialize, Serialize};

/// The execution state of a single node within a workflow run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    /// Not yet evaluated; waiting for predecessors.
    Pending,
    /// All predecessors completed; eligible for execution.
    Ready,
    /// Currently executing.
    Running,
    /// Finished successfully.
    Completed,
    /// Finished with an error.
    Failed,
    /// Skipped due to an unmet edge condition.
    Skipped,
    /// Failed but a retry attempt is in progress.
    Retrying,
    /// Cancelled by the user or by a shutdown signal.
    Cancelled,
}

impl NodeState {
    /// Returns `true` if the node has reached a final state and will not transition again.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Skipped | Self::Cancelled
        )
    }

    /// Returns `true` if the node is currently doing work.
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Running | Self::Retrying)
    }

    /// Returns `true` if the node completed successfully.
    #[must_use]
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Completed)
    }

    /// Returns `true` if the node ended in a failure state.
    #[must_use]
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failed)
    }
}

impl std::fmt::Display for NodeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Ready => write!(f, "ready"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Skipped => write!(f, "skipped"),
            Self::Retrying => write!(f, "retrying"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_states() {
        assert!(NodeState::Completed.is_terminal());
        assert!(NodeState::Failed.is_terminal());
        assert!(NodeState::Skipped.is_terminal());
        assert!(NodeState::Cancelled.is_terminal());

        assert!(!NodeState::Pending.is_terminal());
        assert!(!NodeState::Ready.is_terminal());
        assert!(!NodeState::Running.is_terminal());
        assert!(!NodeState::Retrying.is_terminal());
    }

    #[test]
    fn active_states() {
        assert!(NodeState::Running.is_active());
        assert!(NodeState::Retrying.is_active());

        assert!(!NodeState::Pending.is_active());
        assert!(!NodeState::Ready.is_active());
        assert!(!NodeState::Completed.is_active());
        assert!(!NodeState::Failed.is_active());
        assert!(!NodeState::Skipped.is_active());
        assert!(!NodeState::Cancelled.is_active());
    }

    #[test]
    fn success_state() {
        assert!(NodeState::Completed.is_success());

        assert!(!NodeState::Failed.is_success());
        assert!(!NodeState::Running.is_success());
        assert!(!NodeState::Pending.is_success());
    }

    #[test]
    fn failure_state() {
        assert!(NodeState::Failed.is_failure());

        assert!(!NodeState::Completed.is_failure());
        assert!(!NodeState::Running.is_failure());
        assert!(!NodeState::Cancelled.is_failure());
    }

    #[test]
    fn display_formatting() {
        assert_eq!(NodeState::Pending.to_string(), "pending");
        assert_eq!(NodeState::Ready.to_string(), "ready");
        assert_eq!(NodeState::Running.to_string(), "running");
        assert_eq!(NodeState::Completed.to_string(), "completed");
        assert_eq!(NodeState::Failed.to_string(), "failed");
        assert_eq!(NodeState::Skipped.to_string(), "skipped");
        assert_eq!(NodeState::Retrying.to_string(), "retrying");
        assert_eq!(NodeState::Cancelled.to_string(), "cancelled");
    }

    #[test]
    fn serde_roundtrip() {
        let states = [
            NodeState::Pending,
            NodeState::Ready,
            NodeState::Running,
            NodeState::Completed,
            NodeState::Failed,
            NodeState::Skipped,
            NodeState::Retrying,
            NodeState::Cancelled,
        ];

        for state in &states {
            let json = serde_json::to_string(state).unwrap();
            let back: NodeState = serde_json::from_str(&json).unwrap();
            assert_eq!(*state, back, "roundtrip failed for {state}");
        }
    }

    #[test]
    fn serde_rename_snake_case() {
        let json = serde_json::to_string(&NodeState::Pending).unwrap();
        assert_eq!(json, "\"pending\"");

        let json = serde_json::to_string(&NodeState::Retrying).unwrap();
        assert_eq!(json, "\"retrying\"");
    }

    #[test]
    fn copy_semantics() {
        let a = NodeState::Running;
        let b = a;
        assert_eq!(a, b); // both usable after copy
    }

    #[test]
    fn hash_in_set() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(NodeState::Pending);
        set.insert(NodeState::Running);
        set.insert(NodeState::Pending); // duplicate
        assert_eq!(set.len(), 2);
    }
}
