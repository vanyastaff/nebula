//! Execution state tracking for workflows and individual nodes.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use nebula_core::{ExecutionId, NodeId, WorkflowId};
use nebula_workflow::NodeState;
use serde::{Deserialize, Serialize};

use crate::{
    attempt::NodeAttempt,
    error::ExecutionError,
    output::NodeOutput,
    status::ExecutionStatus,
    transition::{validate_execution_transition, validate_node_transition},
};

/// The execution state of a single node within a running workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeExecutionState {
    /// Current state of the node.
    pub state: NodeState,
    /// All attempts made to execute this node.
    pub attempts: Vec<NodeAttempt>,
    /// The current output, if any.
    #[serde(default)]
    pub current_output: Option<NodeOutput>,
    /// When this node was first scheduled.
    #[serde(default)]
    pub scheduled_at: Option<DateTime<Utc>>,
    /// When this node started its first attempt.
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    /// When this node reached a terminal state.
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
    /// Error message if the node failed.
    #[serde(default)]
    pub error_message: Option<String>,
}

impl NodeExecutionState {
    /// Create a new node execution state in the Pending state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: NodeState::Pending,
            attempts: Vec::new(),
            current_output: None,
            scheduled_at: None,
            started_at: None,
            completed_at: None,
            error_message: None,
        }
    }

    /// Number of attempts made so far.
    #[must_use]
    pub fn attempt_count(&self) -> usize {
        self.attempts.len()
    }

    /// Get the latest attempt, if any.
    #[must_use]
    pub fn latest_attempt(&self) -> Option<&NodeAttempt> {
        self.attempts.last()
    }

    /// Transition to a new state, validating the transition.
    pub fn transition_to(&mut self, new_state: NodeState) -> Result<(), ExecutionError> {
        validate_node_transition(self.state, new_state)?;
        self.state = new_state;

        if new_state == NodeState::Ready {
            self.scheduled_at = Some(Utc::now());
        }
        if new_state == NodeState::Running && self.started_at.is_none() {
            self.started_at = Some(Utc::now());
        }
        if new_state.is_terminal() {
            self.completed_at = Some(Utc::now());
        }

        Ok(())
    }
}

impl Default for NodeExecutionState {
    fn default() -> Self {
        Self::new()
    }
}

/// The complete execution state of a running workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionState {
    /// Unique identifier for this execution.
    pub execution_id: ExecutionId,
    /// The workflow being executed.
    pub workflow_id: WorkflowId,
    /// Current execution status.
    pub status: ExecutionStatus,
    /// Per-node execution states.
    pub node_states: HashMap<NodeId, NodeExecutionState>,
    /// Optimistic concurrency version (bumped on each state change).
    pub version: u64,
    /// When the execution was created.
    pub created_at: DateTime<Utc>,
    /// When the execution was last updated.
    pub updated_at: DateTime<Utc>,
    /// When the execution started running.
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    /// When the execution completed.
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
    /// Total retry attempts across all nodes.
    pub total_retries: u32,
    /// Total output bytes across all nodes.
    pub total_output_bytes: u64,
    /// Execution-level variables.
    #[serde(default)]
    pub variables: serde_json::Map<String, serde_json::Value>,
}

impl ExecutionState {
    /// Create a new execution state.
    #[must_use]
    pub fn new(execution_id: ExecutionId, workflow_id: WorkflowId, node_ids: &[NodeId]) -> Self {
        let now = Utc::now();
        let mut node_states = HashMap::new();
        for &nid in node_ids {
            node_states.insert(nid, NodeExecutionState::new());
        }

        Self {
            execution_id,
            workflow_id,
            status: ExecutionStatus::Created,
            node_states,
            version: 0,
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
            total_retries: 0,
            total_output_bytes: 0,
            variables: serde_json::Map::new(),
        }
    }

    /// Get a node's execution state.
    #[must_use]
    pub fn node_state(&self, node_id: NodeId) -> Option<&NodeExecutionState> {
        self.node_states.get(&node_id)
    }

    /// Set a node's execution state directly.
    ///
    /// **This bypasses transition validation and the parent version
    /// bump.** It exists for building initial state from storage and
    /// for tests. Engine code MUST use [`transition_node`] — a direct
    /// `set_node_state` (or `get_mut(...).transition_to(...)`) does
    /// not invalidate any optimistic-concurrency reader that was
    /// tracking the parent [`ExecutionState::version`].
    ///
    /// [`transition_node`]: Self::transition_node
    pub fn set_node_state(&mut self, node_id: NodeId, state: NodeExecutionState) {
        self.node_states.insert(node_id, state);
    }

    /// Override a node's raw state without running transition
    /// validation, but still bump the parent execution version.
    ///
    /// This is the escape hatch for the engine's recovery paths — the
    /// `resume_execution` reset (Running → Pending after a crash) and
    /// the `IgnoreErrors` strategy (Failed → Completed) both need to
    /// move a node into a state that is not reachable from the
    /// current one via the forward state machine. They still MUST
    /// bump the parent version so CAS readers observe the change
    /// (issue #255); use this method instead of a direct
    /// `node_states.get_mut(...).state = ...` assignment.
    ///
    /// Application code that is NOT in a recovery path should use
    /// [`transition_node`](Self::transition_node) instead — it
    /// enforces the transition rules.
    ///
    /// Returns an error only if `node_id` is unknown.
    pub fn override_node_state(
        &mut self,
        node_id: NodeId,
        new_state: NodeState,
    ) -> Result<(), ExecutionError> {
        let ns = self
            .node_states
            .get_mut(&node_id)
            .ok_or(ExecutionError::NodeNotFound(node_id))?;
        ns.state = new_state;
        self.version += 1;
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Transition a node through the validated state machine and bump
    /// the parent execution version.
    ///
    /// This is the ONLY correct way to mutate a node's state from
    /// engine code. Direct mutation via
    /// `node_states.get_mut(&id).unwrap().transition_to(...)`
    /// validates the per-node transition but silently leaves
    /// `ExecutionState::version` and `ExecutionState::updated_at`
    /// behind, which breaks any optimistic-concurrency reader that
    /// keyed its CAS on the parent version — it will happily accept a
    /// stale snapshot because the version never moved.
    ///
    /// # Errors
    ///
    /// - [`ExecutionError::NodeNotFound`] if `node_id` is not in this
    ///   execution's node map.
    /// - Any error returned by [`NodeExecutionState::transition_to`]
    ///   for invalid transitions — in which case the version is NOT
    ///   bumped (the state did not actually change).
    pub fn transition_node(
        &mut self,
        node_id: NodeId,
        new_state: NodeState,
    ) -> Result<(), ExecutionError> {
        let ns = self
            .node_states
            .get_mut(&node_id)
            .ok_or(ExecutionError::NodeNotFound(node_id))?;
        ns.transition_to(new_state)?;
        self.version += 1;
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Returns `true` if all nodes are in terminal states.
    #[must_use]
    pub fn all_nodes_terminal(&self) -> bool {
        self.node_states.values().all(|ns| ns.state.is_terminal())
    }

    /// Get the IDs of all currently active (running/retrying) nodes.
    #[must_use]
    pub fn active_node_ids(&self) -> Vec<NodeId> {
        self.node_states
            .iter()
            .filter(|(_, ns)| ns.state.is_active())
            .map(|(&id, _)| id)
            .collect()
    }

    /// Get the IDs of all completed nodes.
    #[must_use]
    pub fn completed_node_ids(&self) -> Vec<NodeId> {
        self.node_states
            .iter()
            .filter(|(_, ns)| ns.state == NodeState::Completed)
            .map(|(&id, _)| id)
            .collect()
    }

    /// Get the IDs of all failed nodes.
    #[must_use]
    pub fn failed_node_ids(&self) -> Vec<NodeId> {
        self.node_states
            .iter()
            .filter(|(_, ns)| ns.state == NodeState::Failed)
            .map(|(&id, _)| id)
            .collect()
    }

    /// Transition the execution status, validating the transition and bumping the version.
    pub fn transition_status(&mut self, new_status: ExecutionStatus) -> Result<(), ExecutionError> {
        validate_execution_transition(self.status, new_status)?;
        self.status = new_status;
        self.version += 1;
        self.updated_at = Utc::now();

        if new_status == ExecutionStatus::Running && self.started_at.is_none() {
            self.started_at = Some(Utc::now());
        }
        if new_status.is_terminal() {
            self.completed_at = Some(Utc::now());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state() -> (ExecutionState, NodeId, NodeId) {
        let n1 = NodeId::new();
        let n2 = NodeId::new();
        let state = ExecutionState::new(ExecutionId::new(), WorkflowId::new(), &[n1, n2]);
        (state, n1, n2)
    }

    #[test]
    fn new_execution_state() {
        let (state, n1, _n2) = make_state();
        assert_eq!(state.status, ExecutionStatus::Created);
        assert_eq!(state.version, 0);
        assert_eq!(state.node_states.len(), 2);
        assert_eq!(state.node_state(n1).unwrap().state, NodeState::Pending);
    }

    #[test]
    fn node_execution_state_default() {
        let nes = NodeExecutionState::new();
        assert_eq!(nes.state, NodeState::Pending);
        assert_eq!(nes.attempt_count(), 0);
        assert!(nes.latest_attempt().is_none());
        assert!(nes.scheduled_at.is_none());
    }

    #[test]
    fn node_state_transition() {
        let mut nes = NodeExecutionState::new();
        assert!(nes.transition_to(NodeState::Ready).is_ok());
        assert_eq!(nes.state, NodeState::Ready);
        assert!(nes.scheduled_at.is_some());

        assert!(nes.transition_to(NodeState::Running).is_ok());
        assert_eq!(nes.state, NodeState::Running);
        assert!(nes.started_at.is_some());

        assert!(nes.transition_to(NodeState::Completed).is_ok());
        assert!(nes.completed_at.is_some());
    }

    #[test]
    fn node_state_invalid_transition() {
        let mut nes = NodeExecutionState::new();
        let err = nes.transition_to(NodeState::Completed).unwrap_err();
        assert!(err.to_string().contains("invalid transition"));
    }

    #[test]
    fn all_nodes_terminal() {
        let (mut state, n1, n2) = make_state();
        assert!(!state.all_nodes_terminal());

        state.node_states.get_mut(&n1).unwrap().state = NodeState::Completed;
        state.node_states.get_mut(&n2).unwrap().state = NodeState::Failed;
        assert!(state.all_nodes_terminal());
    }

    #[test]
    fn active_node_ids() {
        let (mut state, n1, _n2) = make_state();
        state.node_states.get_mut(&n1).unwrap().state = NodeState::Running;
        let active = state.active_node_ids();
        assert_eq!(active.len(), 1);
        assert!(active.contains(&n1));
    }

    #[test]
    fn completed_and_failed_node_ids() {
        let (mut state, n1, n2) = make_state();
        state.node_states.get_mut(&n1).unwrap().state = NodeState::Completed;
        state.node_states.get_mut(&n2).unwrap().state = NodeState::Failed;

        assert_eq!(state.completed_node_ids(), vec![n1]);
        assert_eq!(state.failed_node_ids(), vec![n2]);
    }

    #[test]
    fn transition_status_valid() {
        let (mut state, _n1, _n2) = make_state();
        assert!(state.transition_status(ExecutionStatus::Running).is_ok());
        assert_eq!(state.status, ExecutionStatus::Running);
        assert_eq!(state.version, 1);
        assert!(state.started_at.is_some());
    }

    #[test]
    fn transition_status_invalid() {
        let (mut state, _n1, _n2) = make_state();
        let err = state
            .transition_status(ExecutionStatus::Completed)
            .unwrap_err();
        assert!(err.to_string().contains("invalid transition"));
        assert_eq!(state.version, 0); // version not bumped
    }

    #[test]
    fn transition_status_terminal_sets_completed_at() {
        let (mut state, _n1, _n2) = make_state();
        state.transition_status(ExecutionStatus::Running).unwrap();
        state.transition_status(ExecutionStatus::Completed).unwrap();
        assert!(state.completed_at.is_some());
    }

    #[test]
    fn set_node_state() {
        let (mut state, _n1, _n2) = make_state();
        let new_node = NodeId::new();
        state.set_node_state(new_node, NodeExecutionState::new());
        assert!(state.node_state(new_node).is_some());
    }

    /// Regression for issue #255: every node-state transition must
    /// bump the parent `ExecutionState::version` so optimistic
    /// concurrency readers can detect the change. The old engine
    /// pattern `state.node_states.get_mut(&id).unwrap().transition_to(...)`
    /// silently skipped the bump — the `transition_node` method closes
    /// that hole.
    #[test]
    fn transition_node_bumps_parent_version_and_touches_updated_at() {
        let (mut state, n1, _n2) = make_state();
        let v0 = state.version;
        let t0 = state.updated_at;

        state
            .transition_node(n1, NodeState::Ready)
            .expect("valid transition");
        assert_eq!(state.node_state(n1).unwrap().state, NodeState::Ready);
        assert_eq!(state.version, v0 + 1, "version must be bumped");
        assert!(state.updated_at >= t0, "updated_at must move forward");

        // Chained transitions each bump the version once.
        state.transition_node(n1, NodeState::Running).unwrap();
        assert_eq!(state.version, v0 + 2);
        state.transition_node(n1, NodeState::Completed).unwrap();
        assert_eq!(state.version, v0 + 3);
        assert!(state.node_state(n1).unwrap().state.is_terminal());
    }

    #[test]
    fn transition_node_invalid_transition_does_not_bump_version() {
        let (mut state, n1, _n2) = make_state();
        let v0 = state.version;
        // Pending -> Completed is invalid (must pass through Ready/Running).
        let err = state
            .transition_node(n1, NodeState::Completed)
            .expect_err("invalid transition must error");
        assert!(err.to_string().contains("invalid transition"));
        // Version must NOT move on a rejected transition — if it did,
        // optimistic-concurrency readers would see a phantom change.
        assert_eq!(state.version, v0);
        // And the node stayed Pending.
        assert_eq!(state.node_state(n1).unwrap().state, NodeState::Pending);
    }

    #[test]
    fn transition_node_unknown_node_is_error() {
        let (mut state, _n1, _n2) = make_state();
        let ghost = NodeId::new();
        let err = state
            .transition_node(ghost, NodeState::Ready)
            .expect_err("unknown node id");
        assert!(matches!(err, ExecutionError::NodeNotFound(_)));
        // Version unchanged.
        assert_eq!(state.version, 0);
    }

    #[test]
    fn serde_roundtrip() {
        let (state, _n1, _n2) = make_state();
        let json = serde_json::to_string(&state).unwrap();
        let back: ExecutionState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.execution_id, state.execution_id);
        assert_eq!(back.workflow_id, state.workflow_id);
        assert_eq!(back.status, state.status);
        assert_eq!(back.node_states.len(), state.node_states.len());
    }
}
