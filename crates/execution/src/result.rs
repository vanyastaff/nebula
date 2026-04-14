//! Execution result summary.

use std::{collections::HashMap, time::Duration};

use chrono::{DateTime, Utc};
use nebula_core::{ExecutionId, NodeId};
use serde::{Deserialize, Serialize};

use crate::status::{ExecutionStatus, ExecutionTerminationReason};

/// Summary of a completed workflow execution.
///
/// Captures timing, node counts, and terminal-node outputs after an
/// execution reaches a terminal [`ExecutionStatus`].
///
/// # Examples
///
/// ```
/// use nebula_core::ExecutionId;
/// use nebula_execution::{ExecutionResult, ExecutionStatus};
///
/// let result = ExecutionResult::new(ExecutionId::new(), ExecutionStatus::Completed);
/// assert!(result.status.is_terminal());
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// The execution ID.
    pub execution_id: ExecutionId,
    /// Final status.
    pub status: ExecutionStatus,
    /// When the execution started.
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    /// When the execution finished.
    #[serde(default)]
    pub finished_at: Option<DateTime<Utc>>,
    /// Total wall-clock duration.
    #[serde(default, with = "crate::serde_duration_opt")]
    pub duration: Option<Duration>,
    /// Number of nodes that ran.
    #[serde(default)]
    pub nodes_executed: usize,
    /// Number of nodes that failed.
    #[serde(default)]
    pub nodes_failed: usize,
    /// Number of nodes skipped.
    #[serde(default)]
    pub nodes_skipped: usize,
    /// Collected outputs from terminal nodes (nodes with no outgoing edges).
    #[serde(default)]
    pub outputs: HashMap<NodeId, serde_json::Value>,
    /// Why this execution reached its terminal state.
    ///
    /// `None` for in-flight executions and for results serialised before
    /// this field existed. When `None` on a *terminal* status, callers
    /// should interpret it as:
    ///
    /// - [`ExecutionStatus::Completed`] → [`ExecutionTerminationReason::NaturalCompletion`]
    /// - [`ExecutionStatus::Cancelled`] → [`ExecutionTerminationReason::Cancelled`] (legacy
    ///   executions cancelled before this field existed landed here legitimately)
    /// - [`ExecutionStatus::Failed`] or [`ExecutionStatus::TimedOut`] → unknown cause; prefer
    ///   [`ExecutionTerminationReason::SystemError`] rather than collapsing to another category
    /// - any other status → should not occur for a terminal result; treat as
    ///   [`ExecutionTerminationReason::SystemError`]
    #[serde(default)]
    pub termination_reason: Option<ExecutionTerminationReason>,
}

impl ExecutionResult {
    /// Create a new result with the given ID and status.
    ///
    /// All other fields are zeroed / empty. Use builder methods to fill them.
    #[must_use]
    pub fn new(execution_id: ExecutionId, status: ExecutionStatus) -> Self {
        Self {
            execution_id,
            status,
            started_at: None,
            finished_at: None,
            duration: None,
            nodes_executed: 0,
            nodes_failed: 0,
            nodes_skipped: 0,
            outputs: HashMap::new(),
            termination_reason: None,
        }
    }

    /// Set the termination reason for this execution.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_termination_reason(mut self, reason: ExecutionTerminationReason) -> Self {
        self.termination_reason = Some(reason);
        self
    }

    /// Set timing information.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_timing(mut self, started_at: DateTime<Utc>, finished_at: DateTime<Utc>) -> Self {
        let dur = (finished_at - started_at).to_std().unwrap_or_default();
        self.started_at = Some(started_at);
        self.finished_at = Some(finished_at);
        self.duration = Some(dur);
        self
    }

    /// Set node execution counts.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_node_counts(mut self, executed: usize, failed: usize, skipped: usize) -> Self {
        self.nodes_executed = executed;
        self.nodes_failed = failed;
        self.nodes_skipped = skipped;
        self
    }

    /// Set terminal-node outputs.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_outputs(mut self, outputs: HashMap<NodeId, serde_json::Value>) -> Self {
        self.outputs = outputs;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_empty_result() {
        let id = ExecutionId::new();
        let result = ExecutionResult::new(id, ExecutionStatus::Completed);

        assert_eq!(result.execution_id, id);
        assert_eq!(result.status, ExecutionStatus::Completed);
        assert!(result.started_at.is_none());
        assert!(result.finished_at.is_none());
        assert!(result.duration.is_none());
        assert_eq!(result.nodes_executed, 0);
        assert_eq!(result.nodes_failed, 0);
        assert_eq!(result.nodes_skipped, 0);
        assert!(result.outputs.is_empty());
    }

    #[test]
    fn builder_sets_node_counts() {
        let result = ExecutionResult::new(ExecutionId::new(), ExecutionStatus::Failed)
            .with_node_counts(5, 2, 1);

        assert_eq!(result.nodes_executed, 5);
        assert_eq!(result.nodes_failed, 2);
        assert_eq!(result.nodes_skipped, 1);
    }

    #[test]
    fn builder_sets_timing() {
        let start = Utc::now();
        let end = start + chrono::Duration::seconds(10);
        let result = ExecutionResult::new(ExecutionId::new(), ExecutionStatus::Completed)
            .with_timing(start, end);

        assert_eq!(result.started_at, Some(start));
        assert_eq!(result.finished_at, Some(end));
        assert_eq!(result.duration, Some(Duration::from_secs(10)));
    }

    #[test]
    fn builder_sets_outputs() {
        let node_id = NodeId::new();
        let mut outputs = HashMap::new();
        outputs.insert(node_id, serde_json::json!({"key": "value"}));

        let result = ExecutionResult::new(ExecutionId::new(), ExecutionStatus::Completed)
            .with_outputs(outputs.clone());

        assert_eq!(result.outputs.len(), 1);
        assert_eq!(
            result.outputs[&node_id],
            serde_json::json!({"key": "value"})
        );
    }

    #[test]
    fn serde_roundtrip() {
        let node_id = NodeId::new();
        let start = Utc::now();
        let end = start + chrono::Duration::seconds(42);

        let mut outputs = HashMap::new();
        outputs.insert(node_id, serde_json::json!(42));

        let original = ExecutionResult::new(ExecutionId::new(), ExecutionStatus::Failed)
            .with_timing(start, end)
            .with_node_counts(10, 3, 2)
            .with_outputs(outputs);

        let json = serde_json::to_string(&original).unwrap();
        let restored: ExecutionResult = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.execution_id, original.execution_id);
        assert_eq!(restored.status, original.status);
        assert_eq!(restored.nodes_executed, 10);
        assert_eq!(restored.nodes_failed, 3);
        assert_eq!(restored.nodes_skipped, 2);
        assert_eq!(restored.duration, Some(Duration::from_secs(42)));
        assert_eq!(restored.outputs.len(), 1);
    }

    #[test]
    fn termination_reason_defaults_to_none() {
        let result = ExecutionResult::new(ExecutionId::new(), ExecutionStatus::Completed);
        assert!(result.termination_reason.is_none());
    }

    #[test]
    fn with_termination_reason_builder() {
        let node_id = NodeId::new();
        let result = ExecutionResult::new(ExecutionId::new(), ExecutionStatus::Completed)
            .with_termination_reason(ExecutionTerminationReason::ExplicitStop {
                by_node: node_id,
                note: Some("done early".into()),
            });
        match result.termination_reason {
            Some(ExecutionTerminationReason::ExplicitStop { by_node, note }) => {
                assert_eq!(by_node, node_id);
                assert_eq!(note.as_deref(), Some("done early"));
            },
            _ => panic!("expected ExplicitStop"),
        }
    }

    #[test]
    fn termination_reason_serde_roundtrip() {
        let original = ExecutionResult::new(ExecutionId::new(), ExecutionStatus::Failed)
            .with_termination_reason(ExecutionTerminationReason::ExplicitFail {
                by_node: NodeId::new(),
                code: "E_BAD".into(),
                message: "broken".into(),
            });
        let json = serde_json::to_string(&original).unwrap();
        let back: ExecutionResult = serde_json::from_str(&json).unwrap();
        match back.termination_reason {
            Some(ExecutionTerminationReason::ExplicitFail { code, message, .. }) => {
                assert_eq!(code.as_str(), "E_BAD");
                assert_eq!(message, "broken");
            },
            _ => panic!("expected ExplicitFail"),
        }
    }

    #[test]
    fn termination_reason_backward_compat_deserialize_without_field() {
        // Legacy payloads serialized before termination_reason existed
        // must deserialize with termination_reason == None.
        let id = ExecutionId::new();
        let json = format!(r#"{{"execution_id":"{}","status":"completed"}}"#, id);
        let result: ExecutionResult = serde_json::from_str(&json).unwrap();
        assert!(result.termination_reason.is_none());
    }

    #[test]
    fn deserialize_minimal_json() {
        let id = ExecutionId::new();
        let json = format!(r#"{{"execution_id":"{}","status":"completed"}}"#, id);
        let result: ExecutionResult = serde_json::from_str(&json).unwrap();

        assert_eq!(result.execution_id, id);
        assert_eq!(result.status, ExecutionStatus::Completed);
        assert_eq!(result.nodes_executed, 0);
        assert!(result.outputs.is_empty());
    }
}
