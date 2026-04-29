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
    output::{ExecutionOutput, NodeOutput},
    status::{ExecutionStatus, ExecutionTerminationReason},
    transition::{validate_execution_transition, validate_node_transition},
};

/// Outcome of a single node dispatch, recorded into
/// `NodeExecutionState::attempts` by [`ExecutionState::record_node_attempt`].
///
/// The split is exhaustive — every dispatch either produced an
/// `ActionResult` (success path) or surfaced an `EngineError` (failure
/// path). Cancel-during-wait does **not** record an attempt: the
/// previous failure that scheduled the retry is already captured;
/// the cancel terminates the wait, not a fresh attempt.
#[derive(Debug, Clone)]
pub enum AttemptOutcome {
    /// The action returned an `ActionResult` (any variant). Carries
    /// the inline output value the engine staged into `outputs[node_key]`
    /// and the byte size used for budget accounting.
    Success {
        /// Output payload of the attempt.
        output: ExecutionOutput,
        /// Output size in bytes (used for budget accounting and
        /// post-mortem audit).
        output_bytes: u64,
    },
    /// The action surfaced an error before producing a result.
    /// Carries the error message for the audit log.
    Failure {
        /// Error string captured from the failing attempt
        /// (`EngineError::to_string()`).
        error: String,
    },
}

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
    /// Wall-clock instant at which the engine should dispatch the next
    /// retry attempt for this node (ADR-0042 §Decision Layer 2).
    ///
    /// `Some(_)` is paired with `state == NodeState::WaitingRetry`: the
    /// engine sets it when the retry policy still has budget after a
    /// `Running → Failed` transition, then parks the node in
    /// `WaitingRetry` and waits until this timestamp before re-driving
    /// it through `Ready → Running`. The engine clears the field once
    /// the retry is promoted out of `WaitingRetry` for re-dispatch, or
    /// when retry waiting is torn down by cancel / wall-clock teardown
    /// — a stale `Some(_)` on a non-`WaitingRetry` node would mislead
    /// resume seeding and audit tooling. Per-attempt history lives on
    /// [`NodeExecutionState::attempts`] (push of [`AttemptOutcome::Failure`]
    /// happens *before* `schedule_node_retry`), so post-mortem readers
    /// keep the failure record without relying on this field.
    ///
    /// Forward-compat: legacy persisted states that predate this field
    /// deserialize as `None` (engine treats those nodes as not having
    /// a pending retry — same as a freshly failed node with a
    /// retry-exhausted policy).
    #[serde(default)]
    pub next_attempt_at: Option<DateTime<Utc>>,
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
            next_attempt_at: None,
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

    /// Drive a node to `Running` for a fresh dispatch
    /// (`Pending → Ready → Running` for the first attempt;
    /// `WaitingRetry → Ready → Running` for a scheduled retry per
    /// ADR-0042; `Ready → Running` when the engine has already
    /// promoted the node to `Ready` in a prior phase). Any other
    /// source state is an invalid transition and returned as such —
    /// the engine must route the node through the setup-failure path
    /// instead of silently spawning a task on stale state (issue
    /// #300).
    pub fn start_attempt(&mut self) -> Result<(), ExecutionError> {
        match self.state {
            NodeState::Pending => {
                self.transition_to(NodeState::Ready)?;
                self.transition_to(NodeState::Running)
            },
            NodeState::WaitingRetry => {
                self.transition_to(NodeState::Ready)?;
                self.transition_to(NodeState::Running)
            },
            NodeState::Ready => self.transition_to(NodeState::Running),
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
    /// concurrency, timeout, and output-size limits the original run was
    /// configured with, rather than silently falling back to
    /// [`ExecutionBudget::default()`] on recovery (issue #289).
    ///
    /// Legacy persisted states that predate this field deserialize as
    /// `None`; the engine falls back to the default budget with a
    /// warning log so the degradation is visible.
    #[serde(default)]
    pub budget: Option<ExecutionBudget>,
    /// First explicit termination signal observed during this
    /// execution. `Some((node_key, reason))` means the named node
    /// returned `ActionResult::Terminate` and its
    /// `ExecutionTerminationReason` is the authoritative source of
    /// the eventual final status (canon §4.5; ROADMAP §M0.3).
    /// First-write-wins: subsequent terminate signals from racing
    /// siblings are dropped at `set_terminated_by`.
    ///
    /// Legacy persisted states that predate this field deserialize
    /// as `None`; the engine treats those executions as not having
    /// received an explicit termination.
    #[serde(default)]
    pub terminated_by: Option<(NodeKey, ExecutionTerminationReason)>,
    /// Total number of retry attempts dispatched across all nodes in
    /// this execution. Bumped exactly once per scheduled retry
    /// (post-decision, pre-checkpoint) per ADR-0042 §M2.1 T4.
    ///
    /// Paired with [`ExecutionBudget::max_total_retries`] as a global
    /// cap that complements per-node `RetryConfig::max_attempts`. The
    /// engine consults both on every failure; whichever caps first
    /// wins (canon §11.2). A `None` budget cap means the global
    /// counter is informational only — the engine still increments
    /// it for observability.
    ///
    /// Forward-compat: legacy persisted states that predate this
    /// field deserialize as `0` (engine treats the resumed execution
    /// as having no prior retries on the books — slightly generous
    /// vs the original run, but the per-node `attempt_count` on
    /// `NodeExecutionState::attempts` keeps re-dispatch idempotency
    /// honest).
    #[serde(default)]
    pub total_retries: u32,
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
            total_output_bytes: 0,
            variables: serde_json::Map::new(),
            workflow_input: None,
            budget: None,
            terminated_by: None,
            total_retries: 0,
        }
    }

    /// Record a scheduled retry attempt at the execution level.
    ///
    /// Called by the engine on every successful retry decision (per
    /// ADR-0042 §M2.1 T4) so [`ExecutionBudget::max_total_retries`]
    /// can be enforced as a global cap across all nodes. Bumps the
    /// parent version so optimistic-concurrency readers observe the
    /// state change (issue #255).
    ///
    /// The increment is always-on — even when the budget cap is
    /// `None` — so the counter remains a faithful audit number
    /// regardless of policy.
    pub fn increment_total_retries(&mut self) {
        self.total_retries = self.total_retries.saturating_add(1);
        self.version += 1;
        self.updated_at = Utc::now();
    }

    /// Returns `true` if this execution has hit its global retry cap
    /// from [`ExecutionBudget::max_total_retries`].
    ///
    /// Returns `false` when the budget is absent (`None` cap) or when
    /// the budget itself was never set on the execution — the engine
    /// then defers solely to per-node `RetryConfig::max_attempts`.
    #[must_use]
    pub fn has_exhausted_retry_budget(&self) -> bool {
        self.budget
            .as_ref()
            .and_then(|b| b.max_total_retries)
            .is_some_and(|cap| self.total_retries >= cap)
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
    /// `resume_execution` can restore the same concurrency, timeout, and
    /// output-size limits the original run was configured with, rather
    /// than silently falling back to [`ExecutionBudget::default()`] on
    /// recovery (issue #289).
    pub fn set_budget(&mut self, budget: ExecutionBudget) {
        self.budget = Some(budget);
    }

    /// Record an explicit termination signal from a node returning
    /// `ActionResult::Terminate`.
    ///
    /// # Invariants enforced
    ///
    /// 1. **Reason kind is explicit.** Only [`ExecutionTerminationReason::ExplicitStop`] and
    ///    [`ExecutionTerminationReason::ExplicitFail`] are accepted. `NaturalCompletion`,
    ///    `Cancelled`, and `SystemError` are engine-attributed in `nebula-engine`'s
    ///    `determine_final_status` via other priority-ladder branches and must not be recorded in
    ///    `terminated_by` directly — passing them returns `false` with a `tracing::warn!` and no
    ///    mutation.
    /// 2. **`by_node` matches `node_key`.** The variant's inner `by_node` field MUST equal the
    ///    `node_key` argument. Mismatched identity returns `false` with a `tracing::warn!` and no
    ///    mutation. Engine wiring constructs the reason via
    ///    `map_termination_reason(node_key.clone(), ...)`, so a mismatch indicates a programming
    ///    error in a non-engine caller (or a refactor regression).
    /// 3. **First-write-wins.** Only the first signal is durable; subsequent signals are
    ///    debug-logged and dropped so the post-mortem audit log has a single authoritative source
    ///    per execution (canon §4.5). The frontier loop holds `&mut ExecutionState` while it
    ///    consumes node results, so no two writers race here at the language level.
    ///
    /// On a successful set this method bumps the parent
    /// [`ExecutionState::version`] and `updated_at` so any
    /// optimistic-concurrency reader observes the change (issue
    /// #255). On a rejected call (any of the cases above) it is a
    /// no-op.
    ///
    /// Returns `true` when the signal was recorded, `false` when
    /// rejected. The return value is load-bearing — the
    /// `nebula-engine` crate uses it to decide whether to signal the
    /// `cancel_token` (only on first successful set).
    pub fn set_terminated_by(
        &mut self,
        node_key: NodeKey,
        reason: ExecutionTerminationReason,
    ) -> bool {
        // Invariants 1 + 2: reason must be an explicit variant, and
        // its inner `by_node` must match the caller's `node_key`.
        let kind_consistent = match &reason {
            ExecutionTerminationReason::ExplicitStop { by_node, .. }
            | ExecutionTerminationReason::ExplicitFail { by_node, .. } => by_node == &node_key,
            // NaturalCompletion / Cancelled / SystemError are engine-
            // attributed via determine_final_status priority ladder
            // and must not be stored as `terminated_by`.
            _ => false,
        };
        if !kind_consistent {
            tracing::warn!(
                target = "execution::state",
                execution_id = %self.execution_id,
                attempted_by = %node_key,
                attempted_reason = ?reason,
                "set_terminated_by rejected — reason must be ExplicitStop/ExplicitFail \
                 with matching by_node (canon §4.5; ROADMAP §M0.3)"
            );
            return false;
        }

        // Invariant 3: first-write-wins.
        if self.terminated_by.is_some() {
            tracing::debug!(
                target = "execution::state",
                execution_id = %self.execution_id,
                already_set_by = ?self.terminated_by.as_ref().map(|(nk, _)| nk),
                attempted_by = %node_key,
                attempted_reason = ?reason,
                "set_terminated_by skipped — already set (first-write-wins)"
            );
            return false;
        }
        tracing::trace!(
            target = "execution::state",
            execution_id = %self.execution_id,
            %node_key,
            ?reason,
            "set_terminated_by"
        );
        self.terminated_by = Some((node_key, reason));
        self.version += 1;
        self.updated_at = Utc::now();
        true
    }

    /// Drop a previously recorded explicit termination signal **without**
    /// bumping the parent version.
    ///
    /// Recovery escape hatch for the engine's durability path: when a
    /// `set_terminated_by` succeeded in-memory but the next
    /// `checkpoint_node` returned `Err` (CAS conflict, storage failure,
    /// etc.), the signal never reached disk. Leaving the in-memory
    /// `terminated_by` set would let `determine_final_status` report a
    /// durable-looking `termination_reason` on `ExecutionResult` /
    /// `ExecutionEvent::ExecutionFinished` while the persisted state
    /// row contains `None` — a semantic divergence between the event
    /// stream and the audit-of-record.
    ///
    /// `clear_terminated_by` undoes the in-memory record so the engine
    /// reports the honest system-driven outcome (e.g. `(Failed, None)`
    /// from `failed_node` priority). Version is **not** bumped because
    /// the matching set's bump never made it to disk either — readers
    /// keying on `version` should never have observed the intermediate
    /// state.
    ///
    /// Returns `true` when there was a signal to clear, `false`
    /// otherwise. The return value is informational; the engine uses
    /// it only for log fidelity.
    pub fn clear_terminated_by(&mut self) -> bool {
        if let Some((node_key, reason)) = self.terminated_by.take() {
            tracing::warn!(
                target = "execution::state",
                execution_id = %self.execution_id,
                cleared_by = %node_key,
                ?reason,
                "clear_terminated_by — recovery path; signal was not durable"
            );
            true
        } else {
            false
        }
    }

    /// Get a node's execution state.
    #[must_use]
    pub fn node_state(&self, node_key: NodeKey) -> Option<&NodeExecutionState> {
        self.node_states.get(&node_key)
    }

    /// Build the idempotency key for the **next** dispatch of a node.
    ///
    /// Per ADR-0042 §M2.1 T4 the engine pushes a [`NodeAttempt`] into
    /// `node_states[*].attempts` after each finished attempt
    /// (success or failure). The key for the next dispatch is therefore
    /// `attempts.len() + 1`:
    ///
    /// - First dispatch (no prior attempts): `attempt = 1`.
    /// - First retry (one prior failure pushed): `attempt = 2`.
    /// - Second retry: `attempt = 3`. And so on.
    ///
    /// This is the single source of truth the engine uses on both the
    /// check and mark sides of the canonical (`check_idempotency` →
    /// act → `mark_idempotent`) flow, so that a retried or
    /// restart-replayed attempt does not collide with a previous
    /// attempt's persisted output (issue #266, canon §11.3).
    ///
    /// The execution id is taken from `self` — callers cannot pass a
    /// mismatched id by accident. If `node_key` is not present in
    /// `node_states` (a programming error in practice — the engine
    /// only generates keys for nodes it has dispatched), the helper
    /// defaults to attempt number `1`.
    #[must_use]
    pub fn idempotency_key_for_node(&self, node_key: NodeKey) -> IdempotencyKey {
        let attempt = self
            .node_states
            .get(&node_key)
            .map_or(1, |ns| (ns.attempt_count() as u32).saturating_add(1));
        IdempotencyKey::for_attempt(self.execution_id, node_key, attempt)
    }

    /// Push a [`NodeAttempt`] outcome onto the node's history and
    /// bump the parent version (issue #255).
    ///
    /// Called by the engine's frontier loop **after** the action's
    /// dispatch resolves — once on success, once on failure — so the
    /// canonical attempt count drives both `idempotency_key_for_node`
    /// (next dispatch) and the retry decision (per ADR-0042 §M2.1
    /// T4). The attempt number is derived from the pre-push
    /// `attempts.len()` so it stays in lockstep with the
    /// just-dispatched key.
    ///
    /// Returns the recorded attempt number (1-indexed). Returns
    /// [`ExecutionError::NodeNotFound`] if `node_key` is unknown.
    pub fn record_node_attempt(
        &mut self,
        node_key: NodeKey,
        idempotency_key: IdempotencyKey,
        outcome: AttemptOutcome,
    ) -> Result<u32, ExecutionError> {
        let ns = self
            .node_states
            .get_mut(&node_key)
            .ok_or_else(|| ExecutionError::NodeNotFound(node_key.clone()))?;
        let attempt_number = (ns.attempts.len() as u32).saturating_add(1);
        let mut attempt = NodeAttempt::new(attempt_number, idempotency_key);
        match outcome {
            AttemptOutcome::Success {
                output,
                output_bytes,
            } => {
                attempt.complete_success(output, output_bytes);
            },
            AttemptOutcome::Failure { error } => {
                attempt.complete_failure(error);
            },
        }
        ns.attempts.push(attempt);
        self.version += 1;
        self.updated_at = Utc::now();
        Ok(attempt_number)
    }

    /// Schedule the next retry attempt for a node per ADR-0042 §M2.1
    /// T4.
    ///
    /// Promotes a `Failed` node to `WaitingRetry`, stamps the wall-clock
    /// `next_attempt_at`, and increments the global retry counter. The
    /// caller is responsible for the budget + per-node policy decision
    /// — this helper is a pure mutation primitive; it does not
    /// re-evaluate whether the retry is allowed.
    ///
    /// On success, both the per-node transition (`Failed → WaitingRetry`)
    /// and the global counter bump are reflected in `version`. On
    /// `Err`, the state is left untouched so the caller can route
    /// through the regular failure path without leaking an in-memory
    /// half-applied retry.
    ///
    /// # Errors
    /// - [`ExecutionError::NodeNotFound`] if `node_key` is unknown.
    /// - [`ExecutionError::InvalidTransition`] if the node is not in `Failed` (the engine must only
    ///   call this after `mark_node_failed`).
    pub fn schedule_node_retry(
        &mut self,
        node_key: NodeKey,
        next_attempt_at: DateTime<Utc>,
    ) -> Result<(), ExecutionError> {
        self.transition_node(node_key.clone(), NodeState::WaitingRetry)?;
        if let Some(ns) = self.node_states.get_mut(&node_key) {
            ns.next_attempt_at = Some(next_attempt_at);
        }
        // total_retries bump: separate version step is acceptable —
        // both bumps land on the same `checkpoint_node` write.
        self.total_retries = self.total_retries.saturating_add(1);
        self.version += 1;
        self.updated_at = Utc::now();
        Ok(())
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

    /// ADR-0042 — engine retries via the `Failed → WaitingRetry` edge,
    /// not directly from `Failed`. A `start_attempt` on `Failed` is
    /// still rejected (the engine must first promote the node to
    /// `WaitingRetry` via the retry-decision path); but
    /// `WaitingRetry` itself is now a legal source.
    #[test]
    fn start_attempt_rejects_failed() {
        let mut ns = NodeExecutionState::new();
        ns.transition_to(NodeState::Ready).unwrap();
        ns.transition_to(NodeState::Running).unwrap();
        ns.transition_to(NodeState::Failed).unwrap();
        let err = ns
            .start_attempt()
            .expect_err("Failed must promote to WaitingRetry before re-dispatch");
        assert!(matches!(err, ExecutionError::InvalidTransition { .. }));
        assert_eq!(ns.state, NodeState::Failed, "state must not move on error");
    }

    /// ADR-0042 — `WaitingRetry → Ready → Running` is the retry
    /// re-dispatch path. `start_attempt` honors it.
    #[test]
    fn start_attempt_promotes_waiting_retry() {
        let mut ns = NodeExecutionState::new();
        ns.transition_to(NodeState::Ready).unwrap();
        ns.transition_to(NodeState::Running).unwrap();
        ns.transition_to(NodeState::Failed).unwrap();
        ns.transition_to(NodeState::WaitingRetry).unwrap();

        ns.start_attempt()
            .expect("WaitingRetry must be a legal start_attempt source per ADR-0042");
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
    /// so `resume_execution` can restore the original run's concurrency,
    /// timeout, and output-size limits instead of silently falling back
    /// to [`ExecutionBudget::default()`].
    #[test]
    fn budget_roundtrip_via_serde() {
        use std::time::Duration;

        let (mut state, _n1, _n2) = make_state();
        assert!(state.budget.is_none());

        let budget = ExecutionBudget::default()
            .with_max_concurrent_nodes(4)
            .with_max_duration(Duration::from_mins(2))
            .with_max_output_bytes(4 * 1024 * 1024);
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

    // Regression for #266 + ADR-0042 §M2.1 T4: the idempotency key is
    // for the **next** dispatch — `attempts.len() + 1`. Push-on-result
    // semantics in the engine guarantees that a retried or
    // restart-replayed attempt does not collide with a previous
    // attempt's persisted output.
    #[test]
    fn idempotency_key_for_node_uses_attempt_count() {
        use crate::{attempt::NodeAttempt, idempotency::IdempotencyKey};

        let (mut state, n1, _n2) = make_state();
        let eid = state.execution_id;

        let fresh = state.idempotency_key_for_node(n1.clone());
        assert_eq!(
            fresh,
            IdempotencyKey::for_attempt(eid, n1.clone(), 1),
            "first dispatch (no prior attempts) keys on attempt=1"
        );

        let ns = state.node_states.get_mut(&n1).unwrap();
        let seed_key = IdempotencyKey::for_attempt(eid, n1.clone(), 1);
        ns.attempts.push(NodeAttempt::new(1, seed_key));

        let after_one = state.idempotency_key_for_node(n1.clone());
        assert_eq!(
            after_one,
            IdempotencyKey::for_attempt(eid, n1.clone(), 2),
            "after one prior attempt the next dispatch keys on attempt=2"
        );

        let ns = state.node_states.get_mut(&n1).unwrap();
        ns.attempts.push(NodeAttempt::new(
            2,
            IdempotencyKey::for_attempt(eid, n1.clone(), 2),
        ));

        let after_two = state.idempotency_key_for_node(n1.clone());
        assert_eq!(
            after_two,
            IdempotencyKey::for_attempt(eid, n1, 3),
            "after two prior attempts the next dispatch keys on attempt=3"
        );
    }

    /// ADR-0042 §M2.1 T4 — `record_node_attempt` pushes a sequential
    /// attempt with the right number, captures the outcome, and bumps
    /// the parent version (issue #255).
    #[test]
    fn record_node_attempt_appends_with_sequential_number() {
        use crate::output::ExecutionOutput;

        let (mut state, n1, _n2) = make_state();
        let eid = state.execution_id;
        let v0 = state.version;

        let key1 = IdempotencyKey::for_attempt(eid, n1.clone(), 1);
        let n = state
            .record_node_attempt(
                n1.clone(),
                key1,
                AttemptOutcome::Failure {
                    error: "boom".to_owned(),
                },
            )
            .unwrap();
        assert_eq!(n, 1, "first attempt is numbered 1");
        assert_eq!(state.version, v0 + 1);

        let ns = state.node_state(n1.clone()).unwrap();
        assert_eq!(ns.attempts.len(), 1);
        assert_eq!(ns.attempts[0].attempt_number, 1);
        assert!(ns.attempts[0].is_failure());

        let key2 = IdempotencyKey::for_attempt(eid, n1.clone(), 2);
        let n = state
            .record_node_attempt(
                n1.clone(),
                key2,
                AttemptOutcome::Success {
                    output: ExecutionOutput::inline(serde_json::json!({"ok": true})),
                    output_bytes: 12,
                },
            )
            .unwrap();
        assert_eq!(n, 2, "second attempt is numbered 2");
        assert_eq!(state.version, v0 + 2);

        let ns = state.node_state(n1).unwrap();
        assert_eq!(ns.attempts.len(), 2);
        assert!(ns.attempts[1].is_success());
    }

    /// `record_node_attempt` rejects unknown nodes — the engine must
    /// surface the programming error rather than silently lose the
    /// attempt record.
    #[test]
    fn record_node_attempt_unknown_node_is_error() {
        let (mut state, _n1, _n2) = make_state();
        let eid = state.execution_id;
        let ghost = node_key!("ghost");
        let key = IdempotencyKey::for_attempt(eid, ghost.clone(), 1);
        let err = state
            .record_node_attempt(
                ghost,
                key,
                AttemptOutcome::Failure {
                    error: "boom".to_owned(),
                },
            )
            .expect_err("unknown node must error");
        assert!(matches!(err, ExecutionError::NodeNotFound(_)));
    }

    /// ADR-0042 §M2.1 T4 — `schedule_node_retry` promotes Failed →
    /// WaitingRetry, stamps `next_attempt_at`, and increments
    /// `total_retries`. All three observable effects move atomically
    /// (single `checkpoint_node` covers the version bumps).
    #[test]
    fn schedule_node_retry_promotes_failed_and_increments_total_retries() {
        let (mut state, n1, _n2) = make_state();

        // Drive n1 to Failed via the real path.
        state.start_node_attempt(n1.clone()).unwrap();
        state
            .transition_node(n1.clone(), NodeState::Failed)
            .unwrap();
        let v_before = state.version;
        let total_before = state.total_retries;
        let when = Utc::now() + chrono::Duration::milliseconds(500);

        state.schedule_node_retry(n1.clone(), when).unwrap();

        let ns = state.node_state(n1).unwrap();
        assert_eq!(ns.state, NodeState::WaitingRetry);
        assert_eq!(ns.next_attempt_at, Some(when));
        assert_eq!(state.total_retries, total_before + 1);
        // transition_node + total_retries bump = 2 version moves on
        // the same `checkpoint_node` write.
        assert_eq!(state.version, v_before + 2);
    }

    /// `schedule_node_retry` rejects nodes that are not in `Failed` —
    /// e.g. `Running` (race between failure and a stale call).
    #[test]
    fn schedule_node_retry_rejects_non_failed() {
        let (mut state, n1, _n2) = make_state();
        state.start_node_attempt(n1.clone()).unwrap();
        // n1 is now Running, not Failed.
        let when = Utc::now();
        let err = state
            .schedule_node_retry(n1.clone(), when)
            .expect_err("Running → WaitingRetry must be rejected");
        assert!(matches!(err, ExecutionError::InvalidTransition { .. }));
        // State unchanged.
        assert_eq!(state.node_state(n1).unwrap().state, NodeState::Running);
        assert_eq!(state.total_retries, 0);
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

    /// ROADMAP §M0.3 — `terminated_by` must round-trip through serde
    /// so a resumed execution sees the same authoritative termination
    /// signal the original run recorded. Pairs with the runtime
    /// guarantee that the engine persists `ExecutionState` (including
    /// this field) via `checkpoint_node` immediately after
    /// `set_terminated_by`.
    #[test]
    fn terminated_by_roundtrip_via_serde() {
        let (mut state, n1, _n2) = make_state();
        assert!(state.terminated_by.is_none());

        let was_first = state.set_terminated_by(
            n1.clone(),
            ExecutionTerminationReason::ExplicitStop {
                by_node: n1.clone(),
                note: Some("done".to_owned()),
            },
        );
        assert!(was_first, "first set_terminated_by must return true");

        let json = serde_json::to_string(&state).unwrap();
        let back: ExecutionState = serde_json::from_str(&json).unwrap();
        match back.terminated_by {
            Some((nk, ExecutionTerminationReason::ExplicitStop { by_node, note })) => {
                assert_eq!(nk, n1);
                assert_eq!(by_node, n1);
                assert_eq!(note.as_deref(), Some("done"));
            },
            other => panic!("unexpected terminated_by after roundtrip: {other:?}"),
        }
    }

    /// ROADMAP §M0.3 — legacy persisted states that predate
    /// `terminated_by` must still deserialize so a resumed legacy
    /// execution does not crash on missing field. Engine then treats
    /// those as never-explicitly-terminated.
    #[test]
    fn terminated_by_missing_field_deserializes_as_none() {
        let legacy = serde_json::json!({
            "execution_id": ExecutionId::new(),
            "workflow_id": WorkflowId::new(),
            "status": "created",
            "node_states": {},
            "version": 0,
            "created_at": Utc::now(),
            "updated_at": Utc::now(),
            "total_output_bytes": 0,
        });
        let state: ExecutionState = serde_json::from_value(legacy).unwrap();
        assert!(state.terminated_by.is_none());
    }

    /// ROADMAP §M0.3 — first-write-wins. The engine relies on this
    /// return value to decide whether to signal `cancel_token` (only
    /// on `true`) so the second signal must NOT replace the first.
    #[test]
    fn set_terminated_by_is_first_write_wins() {
        let (mut state, n1, n2) = make_state();

        let first = state.set_terminated_by(
            n1.clone(),
            ExecutionTerminationReason::ExplicitStop {
                by_node: n1.clone(),
                note: None,
            },
        );
        assert!(first, "first set must succeed");
        let v_after_first = state.version;

        let second = state.set_terminated_by(
            n2.clone(),
            ExecutionTerminationReason::ExplicitFail {
                by_node: n2,
                code: crate::status::ExecutionTerminationCode::new("E_FAIL"),
                message: "should be ignored".to_owned(),
            },
        );
        assert!(!second, "second set must return false (idempotent)");

        // The original signal must still be the recorded one.
        match state.terminated_by.as_ref() {
            Some((
                nk,
                ExecutionTerminationReason::ExplicitStop {
                    by_node,
                    note: None,
                },
            )) => {
                assert_eq!(nk, &n1);
                assert_eq!(by_node, &n1);
            },
            other => panic!("first signal must remain in place: {other:?}"),
        }
        // And the version must NOT have moved on the duplicate path.
        assert_eq!(
            state.version, v_after_first,
            "version must not bump on duplicate set_terminated_by"
        );
    }

    /// ROADMAP §M0.3 invariant 1 — `set_terminated_by` must reject
    /// non-explicit reason variants. `NaturalCompletion`, `Cancelled`,
    /// and `SystemError` are engine-attributed via
    /// `determine_final_status` priority-ladder branches and must not
    /// be recorded directly in `terminated_by`.
    #[test]
    fn set_terminated_by_rejects_non_explicit_reason() {
        let (mut state, n1, _n2) = make_state();
        let v0 = state.version;

        for reason in [
            ExecutionTerminationReason::NaturalCompletion,
            ExecutionTerminationReason::Cancelled,
            ExecutionTerminationReason::SystemError,
        ] {
            assert!(
                !state.set_terminated_by(n1.clone(), reason),
                "non-explicit variant must be rejected"
            );
            assert!(
                state.terminated_by.is_none(),
                "rejected call must not mutate terminated_by"
            );
            assert_eq!(state.version, v0, "rejected call must not bump version");
        }
    }

    /// ROADMAP §M0.3 invariant 2 — `set_terminated_by` must reject a
    /// reason whose inner `by_node` does not match the `node_key`
    /// argument. Engine wiring constructs the reason via
    /// `map_termination_reason(node_key.clone(), ...)` so a mismatch
    /// indicates a programming error (or a refactor regression) and
    /// must surface as `false` rather than store inconsistent data.
    #[test]
    fn set_terminated_by_rejects_mismatched_by_node() {
        let (mut state, n1, n2) = make_state();
        let v0 = state.version;

        // Outer key = n1, inner by_node = n2 — identity mismatch.
        let mismatched = ExecutionTerminationReason::ExplicitStop {
            by_node: n2,
            note: None,
        };
        assert!(
            !state.set_terminated_by(n1, mismatched),
            "mismatched by_node must be rejected"
        );
        assert!(state.terminated_by.is_none());
        assert_eq!(state.version, v0);
    }

    /// ROADMAP §M0.3 review M1 — recovery escape hatch:
    /// `clear_terminated_by` removes an in-memory signal that never
    /// made it to disk via `checkpoint_node`. Returns `true` when
    /// there was a signal to clear, `false` otherwise. Does NOT bump
    /// `version` (the matching set's bump never reached disk either).
    #[test]
    fn clear_terminated_by_undoes_in_memory_set() {
        let (mut state, n1, _n2) = make_state();

        // No signal to clear initially.
        assert!(
            !state.clear_terminated_by(),
            "clear on empty must return false"
        );

        // Set, then clear.
        let was_first = state.set_terminated_by(
            n1.clone(),
            ExecutionTerminationReason::ExplicitStop {
                by_node: n1,
                note: None,
            },
        );
        assert!(was_first);
        let v_after_set = state.version;

        let cleared = state.clear_terminated_by();
        assert!(cleared, "clear on Some(_) must return true");
        assert!(state.terminated_by.is_none());
        assert_eq!(
            state.version, v_after_set,
            "clear_terminated_by must NOT bump version — readers keying on \
             the set's bump should never have observed the intermediate state"
        );
    }

    /// ADR-0042 §M2.1 T2 — `next_attempt_at` must round-trip via
    /// serde so a resumed engine picks up scheduled retries at their
    /// declared time.
    #[test]
    fn next_attempt_at_roundtrip_via_serde() {
        let mut ns = NodeExecutionState::new();
        let when = Utc::now();
        ns.next_attempt_at = Some(when);
        let json = serde_json::to_string(&ns).unwrap();
        let back: NodeExecutionState = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.next_attempt_at,
            Some(when),
            "next_attempt_at must survive serde roundtrip"
        );
    }

    /// Forward-compat: legacy `NodeExecutionState` JSON that predates
    /// `next_attempt_at` deserializes as `None`. Engine then treats
    /// the node as not having a pending retry.
    #[test]
    fn next_attempt_at_missing_field_deserializes_as_none() {
        let legacy = serde_json::json!({
            "state": "pending",
            "attempts": [],
        });
        let ns: NodeExecutionState = serde_json::from_value(legacy).unwrap();
        assert!(ns.next_attempt_at.is_none());
    }

    /// ADR-0042 §M2.1 T2 — `total_retries` round-trips and starts
    /// at zero.
    #[test]
    fn total_retries_roundtrip_and_default() {
        let (state, _n1, _n2) = make_state();
        assert_eq!(state.total_retries, 0);

        let json = serde_json::to_string(&state).unwrap();
        let back: ExecutionState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.total_retries, 0);
    }

    /// Forward-compat: legacy `ExecutionState` JSON that predates
    /// `total_retries` deserializes as `0`.
    #[test]
    fn total_retries_missing_field_deserializes_as_zero() {
        let legacy = serde_json::json!({
            "execution_id": ExecutionId::new(),
            "workflow_id": WorkflowId::new(),
            "status": "created",
            "node_states": {},
            "version": 0,
            "created_at": Utc::now(),
            "updated_at": Utc::now(),
            "total_output_bytes": 0,
        });
        let state: ExecutionState = serde_json::from_value(legacy).unwrap();
        assert_eq!(state.total_retries, 0);
    }

    /// ADR-0042 §M2.1 T4 — `increment_total_retries` bumps both the
    /// counter and the parent execution version (issue #255).
    #[test]
    fn increment_total_retries_bumps_version() {
        let (mut state, _n1, _n2) = make_state();
        let v0 = state.version;
        state.increment_total_retries();
        assert_eq!(state.total_retries, 1);
        assert_eq!(state.version, v0 + 1);
        state.increment_total_retries();
        assert_eq!(state.total_retries, 2);
        assert_eq!(state.version, v0 + 2);
    }

    /// `has_exhausted_retry_budget` reflects the cap when set, and
    /// returns `false` when no cap is configured.
    #[test]
    fn has_exhausted_retry_budget_respects_cap() {
        let (mut state, _n1, _n2) = make_state();

        // No budget set — never exhausted.
        assert!(!state.has_exhausted_retry_budget());

        // Budget without cap — still not exhausted.
        state.set_budget(ExecutionBudget::default());
        assert!(!state.has_exhausted_retry_budget());

        // Cap = 2: counter 0 and 1 are under cap; 2 is exhausted.
        state.set_budget(ExecutionBudget::default().with_max_total_retries(2));
        assert!(!state.has_exhausted_retry_budget());
        state.increment_total_retries();
        assert!(!state.has_exhausted_retry_budget());
        state.increment_total_retries();
        assert!(state.has_exhausted_retry_budget());

        // Cap = 0 disables retry entirely from the start.
        let mut zero_cap =
            ExecutionState::new(ExecutionId::new(), WorkflowId::new(), &[node_key!("only")]);
        zero_cap.set_budget(ExecutionBudget::default().with_max_total_retries(0));
        assert!(zero_cap.has_exhausted_retry_budget());
    }

    /// ROADMAP §M0.3 — successful set bumps `version` and
    /// `updated_at` so optimistic-concurrency readers observe the
    /// change (issue #255).
    #[test]
    fn set_terminated_by_bumps_version_and_updated_at() {
        let (mut state, n1, _n2) = make_state();
        let v0 = state.version;
        let t0 = state.updated_at;

        let was_first = state.set_terminated_by(
            n1.clone(),
            ExecutionTerminationReason::ExplicitStop {
                by_node: n1,
                note: None,
            },
        );
        assert!(was_first);
        assert_eq!(state.version, v0 + 1, "version must be bumped on first set");
        assert!(state.updated_at >= t0, "updated_at must move forward");
    }
}
