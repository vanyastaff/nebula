//! Execution state tracking for workflows and individual nodes.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use nebula_core::{
    ExecutablePlanRevisionId, ExecutionId, NodeKey, WorkerFlavorRevisionId, WorkflowId,
};
use nebula_workflow::NodeState;
use serde::{Deserialize, Serialize};

use crate::{
    attempt::NodeAttempt,
    context::ExecutionBudget,
    error::ExecutionError,
    error_envelope::ErrorEnvelope,
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
    /// Carries the failure record for the audit log.
    Failure {
        /// Failure record for the failed attempt. Typed and bounded — the
        /// staging shape handed to [`NodeAttempt::complete_failure`].
        error: ErrorEnvelope,
    },
}

/// How a parked [`NodeState::Waiting`] node's timer wake should be
/// interpreted when it fires.
///
/// A `Waiting` node carries an optional `next_attempt_at` timer (see
/// [`NodeExecutionState::next_attempt_at`]). The timer alone cannot say
/// *what the wake means*: a timer-driven wait (`Until` / `Duration`) and a
/// satisfied signal wait both wake to **complete** the node, whereas a
/// signal wait that was parked with an explicit `timeout` wakes to **fail**
/// the node (the external signal never arrived in time). This enum is that
/// missing discriminator, persisted alongside the timer so the wake's
/// meaning survives a crash + recovery.
///
/// Extensible by design (not a `bool`): future wait-bearing kinds
/// (Interactive / Delay / Agent-HITL) reuse the same park/resume machinery
/// and may introduce further wake semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WaitWake {
    /// The timer wake completes the node (`Waiting → Completed`) and
    /// activates its `main`-port downstream edges. Used by timer-driven
    /// waits (`Until` / `Duration`) and by a signal wait that an explicit
    /// Resume has armed for completion.
    Completion,
    /// The timer wake fails the node (`Waiting → Failed` with
    /// `RuntimeError::WaitTimedOut`) and routes its outgoing edges through
    /// the failure path. Used by a signal wait parked with an explicit
    /// `timeout` whose deadline elapsed before a Resume arrived.
    Timeout,
}

/// The external signal a parked [`NodeState::Waiting`] node is waiting **for**
/// — the persisted resume-identity of a signal-driven wait.
///
/// A signal wait (`Webhook` / `Approval` / `Execution`) is satisfied by an
/// explicit Resume, not a timer. Before W-S3a the parked node recorded only
/// *that* it was waiting (via [`next_attempt_at`](NodeExecutionState::next_attempt_at)
/// / [`wait_wake`](NodeExecutionState::wait_wake)), never *what for*: the
/// `WaitCondition`'s identity was destructured away at park. That made every
/// Resume untargeted — it armed every signal-`Waiting` node — and let a Resume
/// for one wait satisfy an unrelated sibling, or a webhook Resume satisfy an
/// approval gate (two confused-deputy bugs). `WaitSignal` persists the minimum
/// identity needed to target the arm: the `callback_id` of a webhook, the
/// `approver` of an approval gate, or the `execution_id` of an execution wait.
///
/// Only the **identity** is persisted — never the approval `message`, a webhook
/// body, or any payload/schema. Carrying the inbound payload is a later slice
/// (W-S4); persisting it here would widen the durable surface without a
/// targeting need.
///
/// `WaitSignal` is the kind-aware peer of [`WaitWake`]: `wait_wake` records how
/// a *timer* wake is read, `wait_signal` records which *external signal* a
/// non-timer wait awaits. A node parked with a `timeout` carries both; a
/// signal-only park carries `wait_signal` with no timer; a timer-driven wait
/// (`Until` / `Duration`) carries neither — see
/// [`park_node`](ExecutionState::park_node)'s invariant.
///
/// Extensible by design (`#[non_exhaustive]`): future wait-bearing kinds may
/// introduce further signal identities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum WaitSignal {
    /// Awaiting an inbound HTTP callback identified by `callback_id`.
    Webhook {
        /// The author-declared callback label an inbound webhook Resume must
        /// match. Mirrors `WaitCondition::Webhook::callback_id`.
        callback_id: String,
    },
    /// Awaiting human approval from `approver`.
    Approval {
        /// Identifier of the person whose approval Resume must match. Mirrors
        /// `WaitCondition::Approval::approver`. The approval `message` shown to
        /// the approver is deliberately NOT persisted here — only the identity
        /// needed for targeting.
        approver: String,
    },
    /// Awaiting another execution to complete.
    Execution {
        /// The execution this wait is gated on. Mirrors
        /// `WaitCondition::Execution::execution_id`.
        execution_id: ExecutionId,
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
    /// Failure record if the node failed.
    ///
    /// Durable carrier: this rides inside the persisted `ExecutionState`, so it
    /// holds a typed [`ErrorEnvelope`] rather than the failing action's own text.
    /// A row written before the envelope existed fails to decode instead of
    /// resurrecting that text — see [`ErrorEnvelope`].
    #[serde(default)]
    pub error_message: Option<ErrorEnvelope>,
    /// Wall-clock instant at which the engine should dispatch the next
    /// retry attempt for this node.
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
    /// How this node's parked-wait timer wake should be interpreted when it
    /// fires — see [`WaitWake`].
    ///
    /// Paired with [`next_attempt_at`](Self::next_attempt_at): `wait_wake`
    /// is `Some(_)` exactly when the node is parked with a timer wake
    /// (`next_attempt_at.is_some()`), and `None` for a signal-only park (no
    /// timer) or any non-`Waiting` state. [`park_node`] enforces the
    /// `wait_wake.is_some() == wake_at.is_some()` invariant.
    ///
    /// `Completion` (or legacy `None` on an armed timer wait) drives the
    /// wake through the completion path; `Timeout` drives it through the
    /// failure path (a signal wait whose `timeout` elapsed).
    ///
    /// Forward-compat: legacy persisted states that predate this field
    /// deserialize as `None`. A `None` on a still-`Waiting{Some}` timer node
    /// is read as `Completion` — preserving W-S1 timer-wake semantics for
    /// rows written before W-S2b.
    ///
    /// [`park_node`]: ExecutionState::park_node
    #[serde(default)]
    pub wait_wake: Option<WaitWake>,
    /// The external signal this parked node is waiting **for** — the persisted
    /// resume-identity of a signal-driven wait. See [`WaitSignal`].
    ///
    /// `Some(_)` exactly when this node is parked on a SIGNAL `WaitCondition`
    /// (`Webhook` / `Approval` / `Execution`), independent of
    /// [`wait_wake`](Self::wait_wake) / [`next_attempt_at`](Self::next_attempt_at):
    /// a signal-only park carries `wait_signal: Some, wake_at: None,
    /// wait_wake: None`; a signal+timeout park (W-S2b) carries `wait_signal:
    /// Some, wake_at: Some, wait_wake: Some(Timeout)`; a timer-driven wait
    /// (`Until` / `Duration`) carries `wait_signal: None`.
    /// [`park_node`](ExecutionState::park_node) enforces this signal/timer
    /// classification as a typed runtime guard.
    ///
    /// A targeted Resume arms only the node whose `wait_signal` matches the
    /// resume target (a webhook target matches only a `Webhook` signal whose
    /// `callback_id` is equal — never an `Approval` / `Execution`); an
    /// untargeted Resume keeps the legacy all-signal-waits behavior.
    ///
    /// Forward-compat: legacy persisted states that predate this field
    /// deserialize as `None`. A `None` on a still-`Waiting` signal node is read
    /// as "untargetable by identity" — only an untargeted Resume arms it, which
    /// preserves W-S2b behavior for rows written before W-S3a.
    #[serde(default)]
    pub wait_signal: Option<WaitSignal>,
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
            wait_wake: None,
            wait_signal: None,
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
    /// `WaitingRetry → Ready → Running` for a scheduled retry;
    /// `Ready → Running` when the engine has already
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

    /// Arm a parked signal wait for **completion**: stamp the timer wake at
    /// `when` and record [`WaitWake::Completion`] so the next Phase-0b drain
    /// transitions the node `Waiting → Completed` on its main port.
    ///
    /// Writes the (`next_attempt_at`, `wait_wake`) pair together so the
    /// `next_attempt_at.is_some() == wait_wake.is_some()` invariant cannot
    /// drift — the same invariant [`ExecutionState::park_node`] asserts on the
    /// park path. The node state and version are NOT touched here; the caller
    /// leaves the node `Waiting` and bumps the execution version through the
    /// checkpoint that commits the arm.
    ///
    /// The two fields cannot be merged into one to make desync structurally
    /// impossible because `next_attempt_at` is also the retry-wake instant for
    /// a `WaitingRetry` node (where no `wait_wake` applies). Pairing every write
    /// behind this method is the next-best guard.
    ///
    /// [`WaitWake::Completion`]: WaitWake::Completion
    /// [`ExecutionState::park_node`]: ExecutionState::park_node
    pub fn arm_wait_completion(&mut self, when: DateTime<Utc>) {
        self.next_attempt_at = Some(when);
        self.wait_wake = Some(WaitWake::Completion);
        debug_assert_eq!(
            self.next_attempt_at.is_some(),
            self.wait_wake.is_some(),
            "arm_wait_completion must leave next_attempt_at and wait_wake both Some"
        );
    }

    /// Clear a parked wait's metadata after the wake has been resolved (the
    /// node completed or timed out): drop `next_attempt_at`, `wait_wake`, and
    /// the persisted `wait_signal` resume-identity so a later terminal state
    /// carries no contradictory wait metadata. Once a wait has resolved, the
    /// node is no longer arm-targetable, so its `wait_signal` is dropped too.
    ///
    /// Apply this only on the wait-cleanup paths (a resolved `Waiting` node).
    /// It must NOT be used on a `WaitingRetry` node: there `next_attempt_at` is
    /// the retry-wake instant and `wait_wake` is legitimately `None`, so
    /// clearing the pair here would assert and would erase a live retry timer.
    pub fn clear_wait_timer(&mut self) {
        self.next_attempt_at = None;
        self.wait_wake = None;
        self.wait_signal = None;
        debug_assert_eq!(
            self.next_attempt_at.is_some(),
            self.wait_wake.is_some(),
            "clear_wait_timer must leave next_attempt_at and wait_wake both None"
        );
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
    /// Owner-processed replay evidence, committed atomically with this state.
    /// Missing legacy evidence is distinct from a supported empty checkpoint.
    #[serde(default)]
    pub checkpoint: Option<crate::ExecutionCheckpoint>,
    /// Unique identifier for this execution.
    pub execution_id: ExecutionId,
    /// The workflow being executed.
    pub workflow_id: WorkflowId,
    /// Published workflow version number this execution started under.
    ///
    /// Historical publication number. Durable turns execute the exact recorded
    /// plan selected by the revision pins below, not an authoring definition
    /// selected by this number. Legacy absence does not permit latest-version
    /// fallback.
    #[serde(default)]
    pub workflow_version_number: Option<u32>,
    /// Exact executable-plan revision this execution is pinned to (#974).
    ///
    /// Persisted so resume/re-drive loads the same plan even if the
    /// registry is replaced. Legacy rows that predate this field deserialize
    /// as `None`; durable turns reject missing pins before instantiating actions.
    #[serde(default)]
    pub executable_plan_revision_id: Option<ExecutablePlanRevisionId>,
    /// Exact worker-flavor revision this execution is pinned to (#974).
    ///
    /// Persisted so resume/re-drive validates the same flavor even if the
    /// registry is replaced. Legacy rows that predate this field
    /// deserialize as `None`.
    #[serde(default)]
    pub worker_flavor_revision_id: Option<WorkerFlavorRevisionId>,
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
    /// the eventual final status (; ROADMAP §M0.3).
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
    /// (post-decision, pre-checkpoint).
    ///
    /// Paired with [`ExecutionBudget::max_total_retries`] as a global
    /// cap that complements per-node `RetryConfig::max_attempts`. The
    /// engine consults both on every failure; whichever caps first
    /// wins. A `None` budget cap means the global
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
            checkpoint: Some(crate::ExecutionCheckpoint::empty_v1()),
            workflow_version_number: None,
            executable_plan_revision_id: None,
            worker_flavor_revision_id: None,
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

    /// Attach the exact plan/flavor revision pair this execution is pinned to.
    ///
    /// Call once at start time, before the execution is persisted (#974).
    pub fn set_revision_ids(
        &mut self,
        plan_id: ExecutablePlanRevisionId,
        flavor_id: WorkerFlavorRevisionId,
    ) {
        self.executable_plan_revision_id = Some(plan_id);
        self.worker_flavor_revision_id = Some(flavor_id);
    }

    /// Attach the published workflow version number this execution starts under.
    pub fn set_workflow_version_number(&mut self, number: u32) {
        self.workflow_version_number = Some(number);
    }

    /// Record a scheduled retry attempt at the execution level.
    ///
    /// Called by the engine on every successful retry decision (per
    /// so [`ExecutionBudget::max_total_retries`]
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
    ///    `map_termination_reason(node_key.clone(),...)`, so a mismatch indicates a programming
    ///    error in a non-engine caller (or a refactor regression).
    /// 3. **First-write-wins.** Only the first signal is durable; subsequent signals are
    ///    debug-logged and dropped so the post-mortem audit log has a single authoritative source
    ///    per execution. The frontier loop holds `&mut ExecutionState` while it
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
                 with matching by_node (durable lifecycle honesty; ROADMAP §M0.3)"
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
    /// The engine pushes a [`NodeAttempt`] into
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
    /// attempt's persisted output (issue #266, ).
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
    /// (next dispatch) and the retry decision (.1
    /// T4).
    ///
    /// The idempotency key is **derived internally** from the just-
    /// finished attempt number (`attempts.len() + 1`) so a stale
    /// caller cannot persist `attempt_number = N` against an
    /// `attempt-(N-1)` key — that mismatch would silently corrupt
    /// the retry/idempotency audit trail this API is supposed to
    /// own (engine retry path).
    ///
    /// Returns the recorded attempt number (1-indexed). Returns
    /// [`ExecutionError::NodeNotFound`] if `node_key` is unknown.
    pub fn record_node_attempt(
        &mut self,
        node_key: NodeKey,
        outcome: AttemptOutcome,
    ) -> Result<u32, ExecutionError> {
        let execution_id = self.execution_id;
        let ns = self
            .node_states
            .get_mut(&node_key)
            .ok_or_else(|| ExecutionError::NodeNotFound(node_key.clone()))?;
        let attempt_number = (ns.attempts.len() as u32).saturating_add(1);
        // Single source of truth for the attempt-N key — engine code
        // must NOT pass its own pre-computed key here, otherwise the
        // attempt history and the idempotency-key store can drift.
        let idempotency_key =
            IdempotencyKey::for_attempt(execution_id, node_key.clone(), attempt_number);
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

    /// Schedule the next retry attempt for a node.1
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
        // `Failed → WaitingRetry` puts the node back in flight. Stale
        // failure-only metadata from the just-finished attempt
        // (`error_message`, `completed_at`) must be cleared so a later
        // successful attempt does not leave contradictory persisted
        // state — e.g., `state == Completed` paired with the
        // pre-retry `error_message`. Per-attempt failure history
        // lives on `attempts` (the failure record was pushed before
        // this call).
        let ns = self
            .node_states
            .get_mut(&node_key)
            .ok_or(ExecutionError::NodeNotFound(node_key))?;
        ns.next_attempt_at = Some(next_attempt_at);
        ns.error_message = None;
        ns.completed_at = None;
        // total_retries bump: separate version step is acceptable —
        // both bumps land on the same `checkpoint_node` write.
        self.total_retries = self.total_retries.saturating_add(1);
        self.version += 1;
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Park a node that returned `ActionResult::Wait`.
    ///
    /// Promotes `Running → Waiting`, stamps `next_attempt_at` with the
    /// timer wake instant (or clears it when no timer applies), records
    /// the wake discriminator [`wait_wake`](NodeExecutionState::wait_wake)
    /// and the resume-identity [`wait_signal`](NodeExecutionState::wait_signal),
    /// and bumps the execution version. Mirrors [`schedule_node_retry`] for
    /// the wait-park path.
    ///
    /// # Invariants
    ///
    /// 1. **Wake pairing** — `wait_wake.is_some() == wake_at.is_some()`. A timer
    ///    wake (`wake_at` `Some`) must declare what the wake means
    ///    ([`WaitWake::Completion`] for a timer-driven or armed-signal wait,
    ///    [`WaitWake::Timeout`] for a signal wait whose `timeout` is the wake),
    ///    and a wait with no timer must carry `None` (it is satisfied by a
    ///    Resume, not a timer).
    /// 2. **Signal/timer classification** — `wait_signal.is_some()` iff the wait
    ///    is signal-driven, INDEPENDENT of the timer. Concretely a node is a
    ///    signal wait when `wait_wake != Some(WaitWake::Completion)` (i.e. a
    ///    signal-only park, `wait_wake == None`, or a signal+timeout park,
    ///    `wait_wake == Some(Timeout)`), and a timer wait when
    ///    `wait_wake == Some(WaitWake::Completion)`. So `wait_signal` MUST be
    ///    `Some` for the former and `None` for the latter. This makes the three
    ///    legal shapes the only representable ones:
    ///    - signal-only: `wake_at None, wait_wake None, wait_signal Some`
    ///    - signal+timeout: `wake_at Some, wait_wake Some(Timeout), wait_signal Some`
    ///    - timer: `wake_at Some, wait_wake Some(Completion), wait_signal None`
    ///
    /// `park_node` is the sole caller-facing writer of the (`next_attempt_at`,
    /// `wait_wake`, `wait_signal`) triple, and it takes all three from the
    /// caller — so both invariants are enforced as fallible runtime guards (not
    /// `debug_assert!`): a `Some(wake_at), None` desync would turn a timeout
    /// into a completion, and a signal wait with no persisted identity (or a
    /// timer wait with one) would mis-target a Resume — both load-bearing rather
    /// than debug-only checks.
    ///
    /// The caller is responsible for:
    /// - Persisting the node's `partial_output` through the normal
    ///   outputs map before calling this method (so `checkpoint_node`
    ///   commits both the output and the `Waiting` state atomically).
    /// - Pushing `(wake_at, node_key)` onto the engine's `wait_heap`
    ///   when `wake_at` is `Some` — that is the source of truth for
    ///   timer-driven wakes.
    ///
    /// On success the per-node `Running → Waiting` transition, the
    /// `next_attempt_at` stamp, the `wait_wake` discriminator, and the
    /// `wait_signal` resume-identity are all reflected in `version`. On `Err`
    /// the state is left untouched.
    ///
    /// # Errors
    /// - [`ExecutionError::InvalidTransition`] if the (`wake_at`, `wait_wake`)
    ///   pair is not both-`Some` or both-`None`, or if the signal/timer
    ///   classification of (`wait_wake`, `wait_signal`) is inconsistent — both
    ///   checked before any mutation, so a desync leaves the state untouched.
    /// - [`ExecutionError::NodeNotFound`] if `node_key` is unknown.
    /// - [`ExecutionError::InvalidTransition`] if the node is not in `Running`
    ///   (the engine may only call this immediately after dispatching the action).
    ///
    /// [`schedule_node_retry`]: Self::schedule_node_retry
    pub fn park_node(
        &mut self,
        node_key: NodeKey,
        wake_at: Option<DateTime<Utc>>,
        wait_wake: Option<WaitWake>,
        wait_signal: Option<WaitSignal>,
    ) -> Result<(), ExecutionError> {
        // Enforce the wake pairing BEFORE any mutation. A `Some(wake_at),
        // None` (or the inverse) desync would persist a timer wake with no
        // declared meaning — silently turning a timeout into a completion (or
        // a completion into a timer with no deadline). The caller supplies
        // both fields, so this is a real input-validation guard, not a
        // surrounding-code invariant a `debug_assert!` may elide in release.
        if wait_wake.is_some() != wake_at.is_some() {
            return Err(ExecutionError::InvalidTransition {
                from: self
                    .node_states
                    .get(&node_key)
                    .map_or_else(|| NodeState::Running.to_string(), |ns| ns.state.to_string()),
                to: "Waiting (wait_wake/wake_at must be paired)".to_owned(),
            });
        }
        // Enforce the signal/timer classification BEFORE any mutation. A signal
        // wait (`wait_wake != Some(Completion)`) MUST carry a persisted
        // `wait_signal` identity so a targeted Resume can match it; a timer wait
        // (`wait_wake == Some(Completion)`) MUST NOT — a persisted signal on a
        // pure timer would let a webhook/approval Resume mis-satisfy it. Same
        // release-safe typed-error pattern as the wake-pairing guard above.
        let is_timer_wait = wait_wake == Some(WaitWake::Completion);
        if is_timer_wait == wait_signal.is_some() {
            return Err(ExecutionError::InvalidTransition {
                from: self
                    .node_states
                    .get(&node_key)
                    .map_or_else(|| NodeState::Running.to_string(), |ns| ns.state.to_string()),
                to: "Waiting (wait_signal must be Some iff a signal wait)".to_owned(),
            });
        }
        self.transition_node(node_key.clone(), NodeState::Waiting)?;
        // Stamp or clear the wake instant. `Waiting` with `wake_at ==
        // None` means the park is signal-driven (webhook/approval/
        // execution) with no timeout: only an explicit Resume will satisfy
        // the condition. `wait_wake` records how a timer wake (if any) is to
        // be read when it fires; `wait_signal` records the resume-identity a
        // targeted Resume matches. Stale failure metadata is cleared for the
        // same reason `schedule_node_retry` clears it — a later `Completed`
        // transition must not carry contradictory persisted fields.
        let ns = self
            .node_states
            .get_mut(&node_key)
            .ok_or(ExecutionError::NodeNotFound(node_key))?;
        ns.next_attempt_at = wake_at;
        ns.wait_wake = wait_wake;
        ns.wait_signal = wait_signal;
        ns.error_message = None;
        ns.completed_at = None;
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
    /// `node_states.get_mut(...).state =...` assignment.
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
    /// resolution, missing node definition, etc.) and record the failure
    /// record. Handles both first-dispatch Pending-state failures and
    /// retry-path failures where the node is already Failed or
    /// Retrying.
    ///
    /// The caller supplies a typed [`ErrorEnvelope`], not a message string:
    /// this value is persisted, so it must not carry the failing action's own
    /// text (see [`ErrorEnvelope`]).
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
        error: ErrorEnvelope,
    ) -> Result<(), ExecutionError> {
        self.override_node_state(node_key.clone(), NodeState::Failed)?;
        if let Some(ns) = self.node_states.get_mut(&node_key) {
            ns.error_message = Some(error);
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

    /// Terminalize every non-terminal node as `Cancelled` (clearing any
    /// `next_attempt_at`), returning how many nodes were transitioned.
    ///
    /// Used when an execution is cancelled with **no live frontier** to tear
    /// down: the in-loop teardown (`drain_pending_to_cancelled`) only sees nodes
    /// on its heaps/queues, so a signal-`Waiting{next_attempt_at: None}` node
    /// (never heap-tracked) — or any node parked while the frontier was alive
    /// and then exited (a `Paused` execution) — would otherwise remain
    /// non-terminal under a `Cancelled` execution, violating the
    /// terminal-execution ⇒ all-nodes-terminal invariant.
    ///
    /// Every non-terminal node state has a valid `→ Cancelled` edge (see
    /// `can_transition_node`), so each transition goes through the checked
    /// `transition_node` (not a silent field write) and a transition error
    /// would be a transition-table regression, surfaced not swallowed.
    ///
    /// # Errors
    ///
    /// Propagates any [`ExecutionError`] from [`Self::transition_node`] (a
    /// missing node or an unexpected illegal `→ Cancelled` edge).
    pub fn cancel_nonterminal_nodes(&mut self) -> Result<usize, ExecutionError> {
        let targets: Vec<NodeKey> = self
            .node_states
            .iter()
            .filter(|(_, ns)| !ns.state.is_terminal())
            .map(|(id, _)| id.clone())
            .collect();
        let count = targets.len();
        for node_key in targets {
            self.transition_node(node_key.clone(), NodeState::Cancelled)?;
            if let Some(ns) = self.node_states.get_mut(&node_key) {
                ns.next_attempt_at = None;
                ns.wait_wake = None;
                ns.wait_signal = None;
            }
        }
        Ok(count)
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
#[path = "state_tests.rs"]
mod tests;
