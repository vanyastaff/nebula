//! Runtime error types.

use nebula_core::{ActionKey, NodeKey};

/// Errors from the runtime layer.
#[derive(Debug, thiserror::Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum RuntimeError {
    /// Action not found in the registry.
    #[classify(
        category = "not_found",
        code = "RUNTIME:ACTION_NOT_FOUND",
        retryable = false
    )]
    #[error("action not found: {key}")]
    ActionNotFound {
        /// The action key that was looked up.
        key: String,
    },

    /// The action key string failed to parse into a valid `ActionKey`.
    #[classify(
        category = "validation",
        code = "RUNTIME:INVALID_ACTION_KEY",
        retryable = false
    )]
    #[error("invalid action key '{key}': {reason}")]
    InvalidActionKey {
        /// The raw key string that failed to parse.
        key: String,
        /// The parse error reason.
        reason: String,
    },

    /// Action execution failed.
    #[classify(
        category = "external",
        code = "RUNTIME:ACTION_ERROR",
        retryable = false
    )]
    #[error("action error: {0}")]
    ActionError(#[from] nebula_action::ActionError),

    /// Data limit exceeded.
    #[classify(category = "exhausted", code = "RUNTIME:DATA_LIMIT", retryable = false)]
    #[error("data limit exceeded: {actual_bytes} bytes > {limit_bytes} bytes")]
    DataLimitExceeded {
        /// Maximum allowed output size.
        limit_bytes: u64,
        /// Actual output size.
        actual_bytes: u64,
    },

    /// A `StatefulAction` returned `Continue` without mutating its state —
    /// the author's iteration is stuck (forgot to advance a cursor, reset
    /// an accumulator to the same value, etc.). The runtime converts this
    /// infinite loop into an explicit fatal so retry / error routing can
    /// handle it like any other terminal failure.
    ///
    /// Spec 28 : engine-managed stuck-state detection is part of the
    /// stateful contract, not a generic `ActionError::Fatal`.
    #[classify(
        category = "internal",
        code = "RUNTIME:STATEFUL_STUCK",
        retryable = false
    )]
    #[error(
        "stateful action '{action_key}' returned Continue without mutating state at iteration \
         {iteration} (node {node_key:?}) — refusing to loop indefinitely"
    )]
    StatefulStuck {
        /// The action key whose iteration is stuck.
        action_key: ActionKey,
        /// The node the iteration is running under.
        node_key: NodeKey,
        /// The iteration count at which the stall was detected (1-based —
        /// the handler had just returned `Continue` for this iteration).
        iteration: u32,
    },

    /// The stateful action exceeded the runtime's hard iteration cap
    /// (`MAX_ITERATIONS`). Separate from [`StatefulStuck`](Self::StatefulStuck)
    /// because the state IS changing — the handler just never terminates.
    #[classify(
        category = "exhausted",
        code = "RUNTIME:ITERATION_CAP",
        retryable = false
    )]
    #[error("stateful action '{action_key}' exceeded max iterations ({cap}) at node {node_key:?}")]
    IterationCapExceeded {
        /// The action key that hit the cap.
        action_key: ActionKey,
        /// The node the iteration is running under.
        node_key: NodeKey,
        /// The cap that was tripped.
        cap: u32,
    },

    /// The action key resolves to a trigger, which has its own start/stop
    /// lifecycle and is not executable via `ActionRuntime::execute_action`.
    /// Triggers run via the trigger runtime (separate from action execution).
    #[classify(
        category = "unsupported",
        code = "RUNTIME:TRIGGER_NOT_EXECUTABLE",
        retryable = false
    )]
    #[error("trigger '{key}' is not executable via ActionRuntime — use the trigger runtime")]
    TriggerNotExecutable {
        /// The action key that was looked up.
        key: String,
    },

    /// The action key resolves to a resource, which has its own
    /// configure/cleanup lifecycle scoped to a downstream subtree.
    /// Resources are not executable via `ActionRuntime::execute_action`.
    #[classify(
        category = "unsupported",
        code = "RUNTIME:RESOURCE_NOT_EXECUTABLE",
        retryable = false
    )]
    #[error("resource '{key}' is not executable via ActionRuntime — use the resource graph")]
    ResourceNotExecutable {
        /// The action key that was looked up.
        key: String,
    },

    /// The out-of-process plugin pool was configured with a zero per-key
    /// capacity. A `0`-permit semaphore makes every `acquire` block
    /// forever, so the supervisor refuses to construct rather than wedge
    /// every dispatch silently at runtime.
    #[classify(
        category = "validation",
        code = "RUNTIME:INVALID_POOL_CAPACITY",
        retryable = false
    )]
    #[error(
        "out-of-process plugin pool capacity must be >= 1 (got {requested}); \
         a zero-permit semaphore would block every acquire forever"
    )]
    InvalidPoolCapacity {
        /// The rejected per-key capacity (always `0`).
        requested: usize,
    },

    /// An agent action exceeded its `max_turns()` budget without returning a
    /// terminal result. Separate from
    /// [`IterationCapExceeded`](Self::IterationCapExceeded): the stateful cap
    /// is a hard 10 000-iteration global guard; this cap is the author-declared
    /// per-agent turn budget (default 25), enforced per-handle.
    #[classify(
        category = "exhausted",
        code = "RUNTIME:AGENT_BUDGET_EXCEEDED",
        retryable = false
    )]
    #[error(
        "agent action '{key}' exceeded its turn budget of {max_turns} turns \
         without returning a terminal result"
    )]
    AgentBudgetExceeded {
        /// The action key whose loop ran out of turns.
        key: String,
        /// The author-declared turn budget that was tripped.
        max_turns: u32,
    },

    /// A single turn of an agent action exceeded its per-turn wall-clock timeout.
    ///
    /// Returned when `AgentHandle::turn_timeout()` returns `Some(d)` and the
    /// `step` future did not resolve within that deadline. Prevents a hung
    /// provider call from pinning a worker indefinitely.
    #[classify(
        category = "exhausted",
        code = "RUNTIME:AGENT_TURN_TIMEOUT",
        retryable = true
    )]
    #[error("agent action '{key}' turn {turn} exceeded per-turn timeout of {timeout:?}")]
    AgentTurnTimeout {
        /// The action key whose turn timed out.
        key: String,
        /// The zero-based turn index that timed out.
        turn: u32,
        /// The per-turn timeout that was exceeded.
        timeout: std::time::Duration,
    },

    /// An agent action returned `ActionResult::Wait` in a phase where the
    /// wait-state engine is not yet wired.
    ///
    /// The `Wait` arm of the agent loop requires the durable park/resume
    /// machinery. Until that ships, returning `Wait` from an agent step is an
    /// explicit boundary — the engine surfaces this error rather than silently
    /// dropping the result.
    #[classify(
        category = "unsupported",
        code = "RUNTIME:AGENT_WAIT_NOT_SUPPORTED",
        retryable = false
    )]
    #[error(
        "agent action '{key}' returned ActionResult::Wait, which is not yet wired \
         in the engine's agent loop — the wait-state engine must ship first"
    )]
    AgentWaitNotSupported {
        /// The action key that tried to park.
        key: String,
    },

    /// An action returned `ActionResult::Wait` with a signal-driven condition
    /// (`Webhook`, `Approval`, or `Execution`) that requires an explicit Resume
    /// signal to satisfy. The W-S1 slice only wires timer-based conditions
    /// (`Until` / `Duration`). Signal-driven resume is the W-S2 work item.
    ///
    /// The engine surfaces this error rather than parking the node with
    /// `wake_at = None` and no timer entry — which would permanently stall
    /// the execution until a Resume signal arrived through an unimplemented
    /// path.
    #[classify(
        category = "unsupported",
        code = "RUNTIME:WAIT_CONDITION_NOT_SUPPORTED",
        retryable = false
    )]
    #[error(
        "ActionResult::Wait with condition '{condition_kind}' is not yet supported — \
         only 'Until' and 'Duration' (timer-based) conditions are wired in W-S1; \
         signal-driven resume (Webhook/Approval/Execution) ships in W-S2"
    )]
    WaitConditionNotSupported {
        /// The `WaitCondition` variant name that was rejected.
        condition_kind: String,
    },

    /// A signal-driven `ActionResult::Wait` (`Webhook` / `Approval` /
    /// `Execution`) was parked with an explicit `timeout` and that deadline
    /// elapsed before a Resume arrived. The engine fails the node and routes
    /// its outgoing edges through the failure path (OnError / Skip /
    /// FailFast) — the timeout is the declared maximum the author asked the
    /// engine to enforce (ADR-0099 W-S2b).
    ///
    /// Terminal and **not** retryable: a wait timeout is a deliberate
    /// deadline, not a transient fault. The engine bypasses the retry
    /// decision entirely for this error (it does not count against the
    /// per-node or per-execution retry budget).
    #[classify(
        category = "exhausted",
        code = "RUNTIME:WAIT_TIMED_OUT",
        retryable = false
    )]
    #[error("ActionResult::Wait signal condition timed out after {timeout_ms}ms without a Resume")]
    WaitTimedOut {
        /// The parked signal-wait kind. Currently always the literal
        /// `"signal"`: the parked node does not persist the exact
        /// `WaitCondition` variant (`Webhook` / `Approval` / `Execution`), so
        /// after the timer fires — and especially after a crash + recovery —
        /// only the generic discriminator is recoverable. Per-variant detail
        /// will arrive with persisted resume targeting. The field is retained
        /// for that forward-compatible use and is emitted on the
        /// `NodeWaitTimedOut` event; it is intentionally not interpolated into
        /// the `Display` message while it is a constant.
        condition_kind: String,
        /// Timeout duration reported for observability, in milliseconds.
        /// Reconstructed from the persisted wait deadline and node
        /// `started_at`, so it may slightly exceed the author-declared timeout.
        timeout_ms: u64,
    },

    /// Internal runtime error.
    #[classify(category = "internal", code = "RUNTIME:INTERNAL")]
    #[error("runtime error: {0}")]
    Internal(String),
}

impl RuntimeError {
    /// Whether this error is retryable.
    ///
    /// Returns `true` when the error is a transient condition that a retry
    /// policy may safely re-attempt:
    /// - An [`ActionError`](Self::ActionError) whose inner error reports itself
    ///   retryable.
    /// - [`AgentTurnTimeout`](Self::AgentTurnTimeout): a single turn exceeded
    ///   its per-turn wall-clock deadline; retrying from the last checkpoint is
    ///   the intended recovery path.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::ActionError(e) => e.is_retryable(),
            Self::AgentTurnTimeout { .. } => true,
            _ => false,
        }
    }

    /// The wrapped `ActionError`, if this runtime error is one. Lets the
    /// engine consult `ActionError::is_fatal` on the just-recorded attempt
    /// so a fatal action error is never re-dispatched by retry policy.
    #[must_use]
    pub fn as_action_error(&self) -> Option<&nebula_action::ActionError> {
        match self {
            Self::ActionError(e) => Some(e),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use nebula_action::ActionError;

    use super::*;

    #[test]
    fn action_not_found_display() {
        let err = RuntimeError::ActionNotFound {
            key: "http.request".into(),
        };
        assert_eq!(err.to_string(), "action not found: http.request");
    }

    #[test]
    fn retryable_propagation() {
        let err = RuntimeError::ActionError(ActionError::retryable("timeout"));
        assert!(err.is_retryable());

        let err = RuntimeError::ActionError(ActionError::fatal("bad schema"));
        assert!(!err.is_retryable());
    }

    #[test]
    fn data_limit_not_retryable() {
        let err = RuntimeError::DataLimitExceeded {
            limit_bytes: 1_000,
            actual_bytes: 5_000,
        };
        assert!(!err.is_retryable());
    }

    #[test]
    fn agent_turn_timeout_is_retryable() {
        // `AgentTurnTimeout` carries `retryable = true` in its `#[classify]`
        // attribute; `is_retryable()` must agree so retry policies that call
        // the method (rather than reading classify metadata) honour it.
        let err = RuntimeError::AgentTurnTimeout {
            key: "my.agent".into(),
            turn: 0,
            timeout: std::time::Duration::from_millis(10),
        };
        assert!(err.is_retryable(), "AgentTurnTimeout must be retryable");
    }

    #[test]
    fn agent_budget_exceeded_not_retryable() {
        let err = RuntimeError::AgentBudgetExceeded {
            key: "my.agent".into(),
            max_turns: 3,
        };
        assert!(!err.is_retryable());
    }

    #[test]
    fn wait_timed_out_not_retryable() {
        // A wait timeout is a deliberate deadline, not a transient fault:
        // `is_retryable()` must agree with the `retryable = false` classify
        // attribute so retry policies never re-park a timed-out wait.
        let err = RuntimeError::WaitTimedOut {
            condition_kind: "Webhook".into(),
            timeout_ms: 5_000,
        };
        assert!(!err.is_retryable(), "WaitTimedOut must not be retryable");
    }

    #[test]
    fn wait_timed_out_display_carries_timeout_but_not_the_kind() {
        // `condition_kind` is currently always the constant `"signal"`, so the
        // Display deliberately does NOT interpolate it (that would render the
        // tautology "signal condition 'signal' timed out"). The message must
        // still carry the actionable timeout, and reference the signal
        // condition in prose.
        let err = RuntimeError::WaitTimedOut {
            condition_kind: "signal".into(),
            timeout_ms: 1_500,
        };
        let msg = err.to_string();
        assert!(
            msg.contains("1500"),
            "message must carry the timeout ms: {msg}"
        );
        assert!(
            msg.contains("signal condition"),
            "message must reference the signal condition: {msg}"
        );
        // The raw field value must NOT be echoed as a quoted token — a constant
        // kind interpolated into the message reads as a tautology.
        assert!(
            !msg.contains("'signal'"),
            "the constant condition_kind must not be interpolated into Display: {msg}"
        );
    }
}
