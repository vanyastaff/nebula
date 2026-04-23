//! Execution state tracking for workflows and individual nodes.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use nebula_core::{ExecutionId, NodeKey, WorkflowId};
use nebula_workflow::NodeState;
use serde::{Deserialize, Serialize};

use crate::{
    attempt::NodeAttempt,
    context::ExecutionBudget,
    error::ExecutionError,
    idempotency::IdempotencyKey,
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

    /// Drive a node to `Running` for a fresh attempt, covering both the
    /// first dispatch (`Pending → Ready → Running`) and retry paths
    /// (`Failed → Retrying → Running`, `Retrying → Running`). Any other
    /// source state is an invalid transition and returned as such — the
    /// engine must route the node through the setup-failure path
    /// instead of silently spawning a task on stale state (issue #300).
    pub fn start_attempt(&mut self) -> Result<(), ExecutionError> {
        match self.state {
            NodeState::Pending => {
                self.transition_to(NodeState::Ready)?;
                self.transition_to(NodeState::Running)
            },
            NodeState::Failed => {
                self.transition_to(NodeState::Retrying)?;
                self.transition_to(NodeState::Running)
            },
            NodeState::Retrying => self.transition_to(NodeState::Running),
            from => Err(ExecutionError::InvalidTransition {
                from: from.to_string(),
                to: NodeState::Running.to_string(),
            }),
        }
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
    pub node_states: HashMap<NodeKey, NodeExecutionState>,
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
    /// The original workflow-level input (trigger payload) for this
    /// execution. Persisted so that `resume_execution` can feed entry
    /// nodes the same payload the original run saw, rather than
    /// silently substituting `Null` (issue #311).
    ///
    /// Legacy persisted states that predate this field deserialize as
    /// `None` and the engine falls back to `Null` with a warning log.
    #[serde(default)]
    pub workflow_input: Option<serde_json::Value>,
    /// The [`ExecutionBudget`] the execution was started with.
    ///
    /// Persisted so that `resume_execution` enforces the same
    /// concurrency, retry, and timeout limits the original run was
    /// configured with, rather than silently falling back to
    /// [`ExecutionBudget::default()`] on recovery (issue #289).
    ///
    /// Legacy persisted states that predate this field deserialize as
    /// `None`; the engine falls back to the default budget with a
    /// warning log so the degradation is visible.
    #[serde(default)]
    pub budget: Option<ExecutionBudget>,
}

impl ExecutionState {
    /// Create a new execution state.
    #[must_use]
    pub fn new(execution_id: ExecutionId, workflow_id: WorkflowId, node_ids: &[NodeKey]) -> Self {
        let now = Utc::now();
        let mut node_states = HashMap::new();
        for nid in node_ids {
            node_states.insert(nid.clone(), NodeExecutionState::new());
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
            workflow_input: None,
            budget: None,
        }
    }

    /// Attach the original workflow-level input to this execution.
    ///
    /// Called by the engine at execution start so that
    /// `resume_execution` can feed entry nodes the same payload the
    /// original run saw (issue #311).
    pub fn set_workflow_input(&mut self, input: serde_json::Value) {
        self.workflow_input = Some(input);
    }

    /// Attach the [`ExecutionBudget`] the execution was configured
    /// with.
    ///
    /// Called by the engine at execution start so that
    /// `resume_execution` can restore the same concurrency, retry, and
    /// timeout limits the original run was configured with, rather
    /// than silently falling back to [`ExecutionBudget::default()`] on
    /// recovery (issue #289).
    pub fn set_budget(&mut self, budget: ExecutionBudget) {
        self.budget = Some(budget);
    }

    /// Get a node's execution state.
    #[must_use]
    pub fn node_state(&self, node_key: NodeKey) -> Option<&NodeExecutionState> {
        self.node_states.get(&node_key)
    }

    /// Build the idempotency key for a node at its current attempt
    /// count. This is the single source of truth the engine uses on
    /// both the check and mark sides of the canonical
    /// (`check_idempotency` → act → `mark_idempotent`) flow, so that a
    /// retried or restart-replayed attempt does not collide with a
    /// previous attempt's persisted output (issue #266, canon §11.3).
    ///
    /// The execution id is taken from `self` — callers cannot pass a
    /// mismatched id by accident.
    ///
    /// When the node's `attempts` is empty (the common case while
    /// engine-level retry accounting is still `planned` per §11.2), the
    /// key uses attempt number `1` — matching what `save_node_output`
    /// records via `attempt_count().max(1)`. When the retry scheduler
    /// lands and begins pushing [`NodeAttempt`]s into `attempts`, this
    /// helper automatically starts differentiating attempts without any
    /// engine change.
    ///
    /// If `node_key` is not present in `node_states` (a programming
    /// error in practice — the engine only generates keys for nodes it
    /// has dispatched), the helper also defaults to attempt number `1`,
    /// mirroring the `.unwrap_or(1)` fallback `save_node_output` uses
    /// for the same input.
    #[must_use]
    pub fn idempotency_key_for_node(&self, node_key: NodeKey) -> IdempotencyKey {
        let attempt = self
            .node_states
            .get(&node_key)
            .map_or(1, |ns| ns.attempt_count().max(1) as u32);
        IdempotencyKey::for_attempt(self.execution_id, node_key, attempt)
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
    pub fn set_node_state(&mut self, node_key: NodeKey, state: NodeExecutionState) {
        self.node_states.insert(node_key, state);
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
    /// Returns an error only if `node_key` is unknown.
    pub fn override_node_state(
        &mut self,
        node_key: NodeKey,
        new_state: NodeState,
    ) -> Result<(), ExecutionError> {
        let ns = self
            .node_states
            .get_mut(&node_key)
            .ok_or(ExecutionError::NodeNotFound(node_key))?;
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
    /// - [`ExecutionError::NodeNotFound`] if `node_key` is not in this execution's node map.
    /// - Any error returned by [`NodeExecutionState::transition_to`] for invalid transitions — in
    ///   which case the version is NOT bumped (the state did not actually change).
    pub fn transition_node(
        &mut self,
        node_key: NodeKey,
        new_state: NodeState,
    ) -> Result<(), ExecutionError> {
        let ns = self
            .node_states
            .get_mut(&node_key)
            .ok_or(ExecutionError::NodeNotFound(node_key))?;
        ns.transition_to(new_state)?;
        self.version += 1;
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Drive a node to `Running` for a fresh attempt (first dispatch
    /// or retry). Delegates to
    /// [`NodeExecutionState::start_attempt`] and bumps the parent
    /// version on success so CAS readers observe the transition.
    ///
    /// # Errors
    ///
    /// - [`ExecutionError::NodeNotFound`] if `node_key` is unknown.
    /// - [`ExecutionError::InvalidTransition`] if the node is not in a state from which a fresh
    ///   attempt is legal. Callers must route the node through the setup-failure path on `Err` —
    ///   they must NOT silently spawn a task on stale state (issue #300).
    pub fn start_node_attempt(&mut self, node_key: NodeKey) -> Result<(), ExecutionError> {
        let ns = self
            .node_states
            .get_mut(&node_key)
            .ok_or(ExecutionError::NodeNotFound(node_key))?;
        let before_version = self.version;
        // `start_attempt` may bump through two per-node transitions;
        // count the parent version by one logical "attempt start".
        ns.start_attempt()?;
        self.version = before_version + 1;
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Move a node to `Failed` for a setup-time failure (parameter
    /// resolution, missing node definition, etc.) and record the error
    /// message. Handles both first-dispatch Pending-state failures and
    /// retry-path failures where the node is already Failed or
    /// Retrying.
    ///
    /// Uses `override_node_state` because Pending → Failed is not a
    /// valid forward transition — setup fails before the node has
    /// reached Running — but the version is still bumped so CAS
    /// readers observe the change (issue #255, #300).
    ///
    /// # Errors
    ///
    /// - [`ExecutionError::NodeNotFound`] if `node_key` is unknown.
    pub fn mark_setup_failed(
        &mut self,
        node_key: NodeKey,
        error_message: impl Into<String>,
    ) -> Result<(), ExecutionError> {
        self.override_node_state(node_key.clone(), NodeState::Failed)?;
        if let Some(ns) = self.node_states.get_mut(&node_key) {
            ns.error_message = Some(error_message.into());
        }
        Ok(())
    }

    /// Returns `true` if all nodes are in terminal states.
    #[must_use]
    pub fn all_nodes_terminal(&self) -> bool {
        self.node_states.values().all(|ns| ns.state.is_terminal())
    }

    /// Get the IDs of all currently active (running/retrying) nodes.
    #[must_use]
    pub fn active_node_ids(&self) -> Vec<NodeKey> {
        self.node_states
            .iter()
            .filter(|(_, ns)| ns.state.is_active())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Get the IDs of all completed nodes.
    #[must_use]
    pub fn completed_node_ids(&self) -> Vec<NodeKey> {
        self.node_states
            .iter()
            .filter(|(_, ns)| ns.state == NodeState::Completed)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Get the IDs of all failed nodes.
    #[must_use]
    pub fn failed_node_ids(&self) -> Vec<NodeKey> {
        self.node_states
            .iter()
            .filter(|(_, ns)| ns.state == NodeState::Failed)
            .map(|(id, _)| id.clone())
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
    use nebula_core::node_key;

    use super::*;

    fn make_state() -> (ExecutionState, NodeKey, NodeKey) {
        let n1 = node_key!("n1");
        let n2 = node_key!("n2");
        let state = ExecutionState::new(
            ExecutionId::new(),
            WorkflowId::new(),
            &[n1.clone(), n2.clone()],
        );
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
        let new_node = node_key!("new_node");
        state.set_node_state(new_node.clone(), NodeExecutionState::new());
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
            .transition_node(n1.clone(), NodeState::Ready)
            .expect("valid transition");
        assert_eq!(
            state.node_state(n1.clone()).unwrap().state,
            NodeState::Ready
        );
        assert_eq!(state.version, v0 + 1, "version must be bumped");
        assert!(state.updated_at >= t0, "updated_at must move forward");

        // Chained transitions each bump the version once.
        state
            .transition_node(n1.clone(), NodeState::Running)
            .unwrap();
        assert_eq!(state.version, v0 + 2);
        state
            .transition_node(n1.clone(), NodeState::Completed)
            .unwrap();
        assert_eq!(state.version, v0 + 3);
        assert!(state.node_state(n1).unwrap().state.is_terminal());
    }

    #[test]
    fn transition_node_invalid_transition_does_not_bump_version() {
        let (mut state, n1, _n2) = make_state();
        let v0 = state.version;
        // Pending -> Completed is invalid (must pass through Ready/Running).
        let err = state
            .transition_node(n1.clone(), NodeState::Completed)
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
        let ghost = node_key!("ghost");
        let err = state
            .transition_node(ghost, NodeState::Ready)
            .expect_err("unknown node id");
        assert!(matches!(err, ExecutionError::NodeNotFound(_)));
        // Version unchanged.
        assert_eq!(state.version, 0);
    }

    #[test]
    fn start_attempt_pending_path() {
        let mut ns = NodeExecutionState::new();
        ns.start_attempt()
            .expect("pending -> running should be legal");
        assert_eq!(ns.state, NodeState::Running);
        assert!(ns.scheduled_at.is_some());
        assert!(ns.started_at.is_some());
    }

    #[test]
    fn start_attempt_retry_path() {
        let mut ns = NodeExecutionState::new();
        // Drive to Failed via the legal transition chain.
        ns.transition_to(NodeState::Ready).unwrap();
        ns.transition_to(NodeState::Running).unwrap();
        ns.transition_to(NodeState::Failed).unwrap();
        ns.start_attempt()
            .expect("failed -> running via retrying should be legal");
        assert_eq!(ns.state, NodeState::Running);
    }

    #[test]
    fn start_attempt_rejects_completed() {
        let mut ns = NodeExecutionState::new();
        ns.transition_to(NodeState::Ready).unwrap();
        ns.transition_to(NodeState::Running).unwrap();
        ns.transition_to(NodeState::Completed).unwrap();
        let err = ns
            .start_attempt()
            .expect_err("completed nodes cannot start a fresh attempt");
        assert!(matches!(err, ExecutionError::InvalidTransition { .. }));
        assert_eq!(
            ns.state,
            NodeState::Completed,
            "state must not move on error"
        );
    }

    #[test]
    fn execution_state_start_node_attempt_bumps_version() {
        let (mut state, n1, _n2) = make_state();
        let v0 = state.version;
        state.start_node_attempt(n1.clone()).unwrap();
        assert_eq!(state.node_state(n1).unwrap().state, NodeState::Running);
        assert_eq!(state.version, v0 + 1);
    }

    #[test]
    fn mark_setup_failed_records_error_and_bumps_version() {
        let (mut state, n1, _n2) = make_state();
        let v0 = state.version;
        state
            .mark_setup_failed(n1.clone(), "param resolution: missing credential")
            .unwrap();
        let ns = state.node_state(n1).unwrap();
        assert_eq!(ns.state, NodeState::Failed);
        assert_eq!(
            ns.error_message.as_deref(),
            Some("param resolution: missing credential")
        );
        assert_eq!(state.version, v0 + 1);
    }

    #[test]
    fn workflow_input_roundtrip_via_serde() {
        let (mut state, _n1, _n2) = make_state();
        assert!(state.workflow_input.is_none());
        state.set_workflow_input(serde_json::json!({"trigger": "webhook"}));
        let json = serde_json::to_string(&state).unwrap();
        let back: ExecutionState = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.workflow_input,
            Some(serde_json::json!({"trigger": "webhook"}))
        );
    }

    /// Issue #289 — `ExecutionBudget` must round-trip through serde
    /// so `resume_execution` can restore the original run's
    /// concurrency / retry / timeout limits instead of silently
    /// falling back to [`ExecutionBudget::default()`].
    #[test]
    fn budget_roundtrip_via_serde() {
        use std::time::Duration;

        let (mut state, _n1, _n2) = make_state();
        assert!(state.budget.is_none());

        let budget = ExecutionBudget::default()
            .with_max_concurrent_nodes(4)
            .with_max_duration(Duration::from_mins(2))
            .with_max_output_bytes(4 * 1024 * 1024)
            .with_max_total_retries(7);
        state.set_budget(budget.clone());

        let json = serde_json::to_string(&state).unwrap();
        let back: ExecutionState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.budget, Some(budget));
    }

    /// Issue #289 — legacy states that predate `budget` must still
    /// deserialize as `None` so the engine can fall back to
    /// `ExecutionBudget::default()` with a warning.
    #[test]
    fn budget_missing_field_deserializes_as_none() {
        let legacy = serde_json::json!({
            "execution_id": ExecutionId::new(),
            "workflow_id": WorkflowId::new(),
            "status": "created",
            "node_states": {},
            "version": 0,
            "created_at": Utc::now(),
            "updated_at": Utc::now(),
            "total_retries": 0,
            "total_output_bytes": 0,
        });
        let state: ExecutionState = serde_json::from_value(legacy).unwrap();
        assert!(state.budget.is_none());
    }

    #[test]
    fn workflow_input_missing_field_deserializes_as_none() {
        // Legacy stored states that predate `workflow_input` must
        // still deserialize — we rely on `#[serde(default)]`.
        let legacy = serde_json::json!({
            "execution_id": ExecutionId::new(),
            "workflow_id": WorkflowId::new(),
            "status": "created",
            "node_states": {},
            "version": 0,
            "created_at": Utc::now(),
            "updated_at": Utc::now(),
            "total_retries": 0,
            "total_output_bytes": 0,
        });
        let state: ExecutionState = serde_json::from_value(legacy).unwrap();
        assert!(state.workflow_input.is_none());
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

    // Regression for #266: idempotency key must reflect the node's real
    // attempt count, not a hardcoded ":1". The engine calls this helper on
    // both check and record paths so that cross-restart replay does not
    // collapse all attempts into one key.
    #[test]
    fn idempotency_key_for_node_uses_attempt_count() {
        use crate::{attempt::NodeAttempt, idempotency::IdempotencyKey};

        let (mut state, n1, _n2) = make_state();
        let eid = state.execution_id;

        let fresh = state.idempotency_key_for_node(n1.clone());
        assert_eq!(
            fresh,
            IdempotencyKey::for_attempt(eid, n1.clone(), 1),
            "a node with no attempts should key on attempt=1 (first dispatch)"
        );

        let ns = state.node_states.get_mut(&n1).unwrap();
        let seed_key = IdempotencyKey::for_attempt(eid, n1.clone(), 1);
        ns.attempts.push(NodeAttempt::new(0, seed_key));
        ns.attempts.push(NodeAttempt::new(
            1,
            IdempotencyKey::for_attempt(eid, n1.clone(), 2),
        ));

        let after_two = state.idempotency_key_for_node(n1.clone());
        assert_eq!(
            after_two,
            IdempotencyKey::for_attempt(eid, n1, 2),
            "a node with two prior attempts should key on attempt=2"
        );
    }

    #[test]
    fn idempotency_key_for_node_unknown_node_defaults_to_one() {
        use crate::idempotency::IdempotencyKey;

        let (state, _n1, _n2) = make_state();
        let phantom = node_key!("not_in_state");
        let eid = state.execution_id;

        let key = state.idempotency_key_for_node(phantom.clone());
        assert_eq!(key, IdempotencyKey::for_attempt(eid, phantom, 1));
    }
}
