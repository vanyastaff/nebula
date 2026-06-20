//! Execution events emitted by the engine during workflow execution.
//!
//! Subscribe via `WorkflowEngine::with_event_sender` to receive real-time
//! updates about node lifecycle transitions. Used by the CLI TUI for live
//! execution monitoring.

use std::time::Duration;

use chrono::{DateTime, Utc};
use nebula_core::{NodeKey, ResourceKey, id::ExecutionId};
use nebula_execution::status::ExecutionTerminationReason;
use nebula_workflow::NodeState;

use crate::scoped_resources::BranchId;

/// Events emitted during workflow execution.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ExecutionEvent {
    /// A node started executing.
    NodeStarted {
        /// Execution this node belongs to.
        execution_id: ExecutionId,
        /// The node that started.
        node_key: NodeKey,
        /// Action key being executed.
        action_key: String,
    },

    /// A node completed successfully.
    NodeCompleted {
        /// Execution this node belongs to.
        execution_id: ExecutionId,
        /// The node that completed.
        node_key: NodeKey,
        /// How long the node took.
        elapsed: Duration,
    },

    /// A node failed.
    NodeFailed {
        /// Execution this node belongs to.
        execution_id: ExecutionId,
        /// The node that failed.
        node_key: NodeKey,
        /// Error message.
        error: String,
    },

    /// A node returned `ActionResult::Wait` and has been durably parked
    /// pending an external condition (timer, webhook, or human approval).
    ///
    /// The node has transitioned to `Waiting`, the worker has been
    /// released, and downstream edges are gated until
    /// [`ExecutionEvent::NodeWaitCompleted`] fires.
    ///
    /// `wake_at` is `Some` for timer-driven conditions (`Until` /
    /// `Duration`) and `None` for signal-driven conditions (`Webhook` /
    /// `Approval` / `Execution`) that have no timeout.
    NodeParked {
        /// Execution this node belongs to.
        execution_id: ExecutionId,
        /// The node that was parked.
        node_key: NodeKey,
        /// Wall-clock instant the engine plans to satisfy the wait, or
        /// `None` for signal-only conditions.
        wake_at: Option<DateTime<Utc>>,
    },

    /// A parked node's wait condition was satisfied and the node has
    /// been transitioned to `Completed`. Downstream edges are now active.
    ///
    /// Subscribers should treat [`ExecutionEvent::NodeParked`] and this
    /// event as a matched pair: `Parked` gates downstream, `WaitCompleted`
    /// unblocks it.
    NodeWaitCompleted {
        /// Execution this node belongs to.
        execution_id: ExecutionId,
        /// The node whose wait condition was satisfied.
        node_key: NodeKey,
    },

    /// A signal-driven parked node (`Webhook` / `Approval` / `Execution`)
    /// was parked with an explicit `timeout` and that deadline elapsed
    /// before a Resume arrived. The engine has transitioned the node
    /// `Waiting → Failed` (with `RuntimeError::WaitTimedOut`) and routed its
    /// outgoing edges through the failure path (OnError / Skip / FailFast).
    ///
    /// The matched-pair partner of [`ExecutionEvent::NodeParked`] on the
    /// timeout branch (where [`ExecutionEvent::NodeWaitCompleted`] is the
    /// success-branch partner): `Parked` gates downstream, `WaitTimedOut`
    /// fails the node and routes the failure edges.
    NodeWaitTimedOut {
        /// Execution this node belongs to.
        execution_id: ExecutionId,
        /// The node whose wait timed out.
        node_key: NodeKey,
        /// The parked signal-wait kind. Currently always the literal
        /// `"signal"`: the parked node does not persist the exact
        /// `WaitCondition` variant (`Webhook` / `Approval` / `Execution`), so
        /// once the timer fires only the generic discriminator is recoverable.
        /// Per-variant detail will arrive with persisted resume targeting.
        condition_kind: String,
        /// The declared `timeout` that elapsed, in milliseconds.
        timeout_ms: u64,
    },

    /// A node attempt failed but the engine scheduled a retry per
    /// (Layer 2 — engine-level retry). The node has
    /// transitioned to `WaitingRetry` and will be re-dispatched at
    /// `next_attempt_at`.
    ///
    /// Subscribers should treat this as **not-final**: the node has
    /// not failed in the canonical sense (`is_failure() == false`
    /// while `WaitingRetry`); only the post-retry-exhausted
    /// [`ExecutionEvent::NodeFailed`] counts as a final failure.
    NodeRetryScheduled {
        /// Execution this node belongs to.
        execution_id: ExecutionId,
        /// The node whose retry is scheduled.
        node_key: NodeKey,
        /// Sequential attempt number that just failed (1-indexed).
        attempt: u32,
        /// Wall-clock instant the engine plans to dispatch the next
        /// attempt at.
        next_attempt_at: DateTime<Utc>,
        /// Error string from the just-failed attempt.
        last_error: String,
    },

    /// A node was skipped (disabled or dependency not met).
    NodeSkipped {
        /// Execution this node belongs to.
        execution_id: ExecutionId,
        /// The node that was skipped.
        node_key: NodeKey,
    },

    /// The frontier loop exited while one or more nodes were still in a
    /// non-terminal state.
    ///
    /// Per `docs/PRODUCT_CANON.md` , the engine must not silently report
    /// `Completed` on inconsistent state. This event is emitted just before
    /// [`ExecutionEvent::ExecutionFinished`] so operators observing the event
    /// stream see the integrity violation rather than only a successful-looking
    /// final event.
    FrontierIntegrityViolation {
        /// The execution whose frontier loop produced the inconsistent state.
        execution_id: ExecutionId,
        /// Nodes that were still non-terminal at the time the frontier loop
        /// exited, paired with their observed `NodeState`.
        non_terminal_nodes: Vec<(NodeKey, NodeState)>,
    },

    /// Workflow execution completed.
    ExecutionFinished {
        /// The execution that finished.
        execution_id: ExecutionId,
        /// Whether it succeeded.
        success: bool,
        /// Total elapsed time.
        elapsed: Duration,
        /// Engine's attribution for *why* the execution reached its
        /// final status (operational honesty).
        ///
        /// `Some(_)` means the engine attributed a concrete
        /// termination reason. `None` is **also intentional**: it
        /// represents a system-driven failure where execution did not
        /// complete successfully but the engine has nothing to add
        /// beyond the failure itself (the failure detail lives on
        /// `ExecutionState::node_states[*].error_message` and
        /// surfaces through the engine's
        /// [`crate::result::ExecutionResult::node_errors`] map).
        /// `determine_final_status` priority 2 (`failed_node` set,
        /// `terminated_by` empty) is the canonical source of `None`.
        ///
        /// Variant guidance:
        ///
        /// - `ExplicitStop` / `ExplicitFail` → a node returned `ActionResult::Terminate {
        ///   TerminationReason::Success | Failure }`.
        /// - `Cancelled` → external cancel (API / admin / shutdown tripped the cancel token
        ///   without a node-driven Terminate).
        /// - `NaturalCompletion` → frontier drained cleanly with no failures and no explicit
        ///   signal.
        /// - `SystemError` → engine attributed the failure to a system-level invariant breach
        ///   (frontier integrity violation, unmapped future `TerminationReason` variant).
        ///
        /// Use `success` for the binary outcome; use
        /// `termination_reason` to distinguish attributed termination
        /// from unattributed system-driven failure (`None`).
        termination_reason: Option<ExecutionTerminationReason>,
    },

    /// A `dispatch_resume` command could not durably complete and is being
    /// redelivered (NOT dropped). Emitted on two paths, both leaving the
    /// control-queue row unacked for at-least-once (B1 reclaim) redelivery:
    ///
    /// 1. **Satisfy did not arm** — `satisfy_signal_waits` could not land
    ///    (the execution lease was held by another runner, or the CAS was
    ///    rejected by a concurrent actor): the signal waits were NOT armed.
    /// 2. **Armed, but the drive deferred** — `satisfy_signal_waits` DID
    ///    durably arm the wait (`next_attempt_at = Some`), but the follow-up
    ///    `drive_armed_resume` could not acquire the lease (e.g. a
    ///    crashed/stalled holder whose TTL has not expired) or hit a CAS
    ///    conflict, so the armed wait is not yet completed.
    ///
    /// In both cases the redelivery converges: a reclaim re-drive re-parks an
    /// un-armed wait (the redelivered Resume re-arms it); an already-armed wait
    /// is completed by the next drive once the lease frees; a wait already
    /// completed makes the redelivery a no-op via the status short-circuit.
    ///
    /// # Observability
    ///
    /// A `tracing::warn!` fires on the same code path immediately before this
    /// event is bus-emitted. Together they allow operators to distinguish a
    /// transient CAS race (expected; low rate) from a systematic drop (unexpected;
    /// high rate suggests a lease or routing bug).
    ResumeDeferred {
        /// Execution whose Resume was deferred due to a CAS conflict.
        execution_id: ExecutionId,
        /// Human-readable reason from the `EngineError` that caused the deferral.
        reason: String,
    },

    /// A scoped resource's `Resource::destroy` future overran its
    /// configured cleanup budget (default
    /// [`crate::scoped_resources::DEFAULT_CLEANUP_TIMEOUT`]).
    ///
    /// Emitted by Phase 7 (M6.2) when the engine drives branch-exit
    /// cleanup and a per-resource timeout fires. The runtime is dropped
    /// without further awaiting; downstream observers (storage writer,
    /// metrics collector, audit writer) use this event to attribute
    /// resource leaks.
    ///
    /// # Observability triple
    ///
    /// - Typed event variant (`thiserror`-free; events are not errors).
    /// - `tracing::warn!` span fires inside the cleanup driver before this event is bus-emitted.
    /// - Engine asserts the invariant `elapsed >= budget` when constructing the event (timeout path
    ///   only).
    ScopedResourceCleanupTimeout {
        /// Execution this branch belongs to.
        execution_id: ExecutionId,
        /// Branch that owned the timed-out resource.
        branch_id: BranchId,
        /// Resource key of the timed-out resource.
        resource_key: ResourceKey,
        /// Budget that elapsed before the future was dropped.
        budget: Duration,
        /// Wall-clock time spent in the cleanup body before the timeout
        /// fired. Always `>= budget` modulo monotonic-clock skew.
        elapsed: Duration,
    },
}
