//! Node execution state tracking.

use serde::{Deserialize, Serialize};

/// The execution state of a single node within a workflow run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
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
    /// Cancelled by the user or by a shutdown signal.
    Cancelled,
    /// Failed but a retry is scheduled — `NodeExecutionState::next_attempt_at`
    /// holds the wake-up time. Transient retry state in the
    /// `Failed → WaitingRetry → Ready → Running` path for engine-level retry
    /// per ADR-0042. Not terminal: a `WaitingRetry` node will eventually
    /// transition back to `Ready`/`Running` (retry attempt) or to
    /// `Cancelled` (shutdown / explicit cancel during the wait).
    WaitingRetry,
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
        matches!(self, Self::Running)
    }

    /// Returns `true` if the node completed successfully.
    #[must_use]
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Completed)
    }

    /// Returns `true` if the node ended in a failure state.
    ///
    /// `WaitingRetry` is *not* a failure — the retry has not yet been
    /// exhausted. Only the final post-retry `Failed` state counts.
    #[must_use]
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failed)
    }

    /// Returns `true` if the node is awaiting a scheduled retry attempt.
    ///
    /// A `WaitingRetry` node is held in the engine's retry-pending heap
    /// until `NodeExecutionState::next_attempt_at` arrives, at which
    /// point the engine transitions it back to `Ready` and re-dispatches.
    #[must_use]
    pub fn is_waiting_retry(&self) -> bool {
        matches!(self, Self::WaitingRetry)
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
            Self::Cancelled => write!(f, "cancelled"),
            Self::WaitingRetry => write!(f, "waiting_retry"),
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
        assert!(
            !NodeState::WaitingRetry.is_terminal(),
            "WaitingRetry must be non-terminal — engine flips it back \
             to Ready when next_attempt_at fires (ADR-0042)"
        );
    }

    #[test]
    fn active_states() {
        assert!(NodeState::Running.is_active());

        assert!(!NodeState::Pending.is_active());
        assert!(!NodeState::Ready.is_active());
        assert!(!NodeState::Completed.is_active());
        assert!(!NodeState::Failed.is_active());
        assert!(!NodeState::Skipped.is_active());
        assert!(!NodeState::Cancelled.is_active());
        assert!(
            !NodeState::WaitingRetry.is_active(),
            "WaitingRetry is parked between attempts; the work is not \
             running — frontier loop's max_concurrent guard must not \
             count it (canon §11.1)"
        );
    }

    /// `WaitingRetry` is the parked-between-attempts state — distinct
    /// from a final `Failed`. `is_failure()` must return `false` so
    /// `failed_node_ids()` and `determine_final_status` only count
    /// nodes whose retry budget is fully exhausted (ADR-0042).
    #[test]
    fn waiting_retry_is_not_failure() {
        assert!(!NodeState::WaitingRetry.is_failure());
        assert!(NodeState::WaitingRetry.is_waiting_retry());
        assert!(!NodeState::Failed.is_waiting_retry());
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
        assert_eq!(NodeState::Cancelled.to_string(), "cancelled");
        assert_eq!(NodeState::WaitingRetry.to_string(), "waiting_retry");
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
            NodeState::Cancelled,
            NodeState::WaitingRetry,
        ];

        for state in &states {
            let json = serde_json::to_string(state).unwrap();
            let back: NodeState = serde_json::from_str(&json).unwrap();
            assert_eq!(*state, back, "roundtrip failed for {state}");
        }
    }

    #[test]
    fn serde_waiting_retry_uses_snake_case() {
        // ADR-0042 wire format: rename_all = "snake_case" so the new
        // variant serializes as `"waiting_retry"`, matching the
        // existing `"failed"` / `"completed"` style. Out-of-tree
        // consumers (UI, API clients, audit log readers) must see this
        // exact tag.
        let json = serde_json::to_string(&NodeState::WaitingRetry).unwrap();
        assert_eq!(json, "\"waiting_retry\"");
        let back: NodeState = serde_json::from_str("\"waiting_retry\"").unwrap();
        assert_eq!(back, NodeState::WaitingRetry);
    }

    #[test]
    fn serde_rename_snake_case() {
        let json = serde_json::to_string(&NodeState::Pending).unwrap();
        assert_eq!(json, "\"pending\"");

        let json = serde_json::to_string(&NodeState::Failed).unwrap();
        assert_eq!(json, "\"failed\"");
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
