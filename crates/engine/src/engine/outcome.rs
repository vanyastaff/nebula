//! Node failure-outcome decisions.
//!
//! The stateless decision layer for a node that failed or needs retrying:
//! resolve the effective retry policy, decide retry-vs-terminal, compute the
//! next attempt time, classify the failure against the workflow's error
//! strategy, apply recovery, and route the failure to its error-handler edges.
//! Split out of `engine.rs` as part of the god-module decomposition (audit
//! 🔴-1). These are free functions (no `self`); they reach the engine's other
//! private helpers and types via `use super::*`.

use super::*;

/// Resolve the effective per-node retry policy per T4.
///
/// Resolution order (more specific wins):
/// 1. `NodeDefinition.retry_policy` — operator-declared per-node policy.
/// 2. `workflow_default` — `WorkflowConfig.retry_policy`, the workflow-wide default applied to
///    nodes that do not declare their own.
/// 3. `None` — no engine-level retry; the failure flows straight to the existing
///    classify+route+checkpoint path.
pub(super) fn effective_retry_policy<'a>(
    node_def: &'a nebula_workflow::NodeDefinition,
    workflow_default: Option<&'a nebula_workflow::RetryConfig>,
) -> Option<&'a nebula_workflow::RetryConfig> {
    node_def.retry_policy.as_ref().or(workflow_default)
}

/// The retry decision for a just-failed node attempt.
///
/// Pure: depends only on the per-node attempt count, the resolved
/// policy, and the execution-level budget. Does not mutate any state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RetryDecision {
    /// No retry — fall through to the existing
    /// classify+route+checkpoint path.
    Finalize,
    /// Schedule a retry after `delay`. The frontier loop transitions
    /// the node to `WaitingRetry`, stamps `next_attempt_at = now() + delay`,
    /// increments the global counter, and parks the node on the
    /// retry-pending heap.
    Retry { delay: Duration },
}

/// Decide whether the just-failed dispatch of `node_key` should be
/// retried per T4 acceptance.
///
/// Ordering of checks (whichever caps first wins):
///
/// 0. **Fatal-error short-circuit** — if the just-recorded attempt's typed
///    [`ActionError`] is fatal ([`ActionError::is_fatal`]), finalize
///    immediately, *before* any policy/budget check. Retry is otherwise a
///    pure attempts/budget/backoff policy that never consulted error
///    fatality at all — so a `Fatal` action error (or a runner
///    after-send close, which maps to fatal) used to be re-dispatched
///    under policy. This early-return makes "bytes reached the plugin ⇒
///    never re-dispatch" structural for *all* actions and closes that
///    pre-existing, dispatch-independent gap.
/// 1. **Global budget cap** — `ExecutionState::has_exhausted_retry_budget` consults
///    `ExecutionBudget.max_total_retries`. A `Some(0)` cap disables retry entirely; a `None` cap
///    leaves the per-node policy as the only gate.
/// 2. **Per-node policy presence** — no policy → no retry.
/// 3. **Per-node policy max_attempts** — once `attempts.len()` (the number of completed attempts at
///    the time of this decision) has reached `policy.max_attempts`, the retry budget is exhausted.
/// 4. **Backoff calc** — `policy.delay_for_attempt(attempt_count - 1)` where `attempt_count` is the
///    just-finished attempt number (1-indexed). This yields the same wait the
///    `nebula-resilience::retry` crate would for the same `RetryConfig`.
///
/// `recorded_error` is the typed error of the attempt just pushed to
/// history (the runtime-failure path supplies it; the setup-failure path
/// passes `None` because a param-resolution failure has no `ActionError`
/// and stays retry-eligible — "the action never started").
/// Whether a just-recorded node failure is **terminal** — never re-dispatched,
/// whatever the retry policy says.
///
/// A fatal [`ActionError`] (direct, or wrapped in `RuntimeError::ActionError`)
/// is terminal, preserving the long-standing fatal-action short-circuit. A
/// non-`ActionError` runtime condition is terminal unless it is explicitly
/// retryable: the iteration cap, stuck-state, data-limit, agent turn-budget,
/// and unsupported-wait variants finalize, while `AgentTurnTimeout` stays
/// retryable.
///
/// The ActionError case routes through `is_fatal` rather than
/// `EngineError::is_retryable`: the `RuntimeError::ActionError` variant carries
/// `retryable = false` in its own classify metadata, which would otherwise
/// shadow the inner action error's real retryability and stop a legitimately
/// retryable action error from retrying.
pub(super) fn error_is_terminal(err: &EngineError) -> bool {
    match err.as_action_error() {
        Some(action_err) => action_err.is_fatal(),
        None => !nebula_error::Classify::is_retryable(err),
    }
}

pub(super) fn compute_retry_decision(
    node_key: &NodeKey,
    exec_state: &ExecutionState,
    retry_policy: Option<&nebula_workflow::RetryConfig>,
    recorded_error_is_terminal: bool,
) -> RetryDecision {
    if recorded_error_is_terminal {
        tracing::debug!(
            target = "engine::retry",
            execution_id = %exec_state.execution_id,
            %node_key,
            "retry skipped: just-recorded attempt error is terminal \
             (fatal action error or non-retryable runtime condition) — no re-dispatch"
        );
        return RetryDecision::Finalize;
    }

    if exec_state.has_exhausted_retry_budget() {
        tracing::debug!(
            target = "engine::retry",
            execution_id = %exec_state.execution_id,
            %node_key,
            total_retries = exec_state.total_retries,
            "retry skipped: ExecutionBudget.max_total_retries cap reached"
        );
        return RetryDecision::Finalize;
    }

    let Some(policy) = retry_policy else {
        return RetryDecision::Finalize;
    };

    // Missing node state means the engine has no history we can
    // base a retry decision on (programming error: only nodes the
    // engine has dispatched are eligible). Refuse the retry rather
    // than fabricating an `attempts_used = 0` for a stranger node —
    // that would let a programming bug schedule retries on
    // unbounded state (hot-path safety).
    let Some(ns) = exec_state.node_states.get(node_key) else {
        tracing::warn!(
            target = "engine::retry",
            execution_id = %exec_state.execution_id,
            %node_key,
            "retry skipped: node state missing for retry decision"
        );
        return RetryDecision::Finalize;
    };
    // `attempts.len()` is the count of *completed* attempts at the
    // moment of decision (post-push of the just-failed attempt). Once
    // it reaches `max_attempts`, the budget is spent.
    let attempts_used = ns.attempts.len() as u32;

    if attempts_used >= policy.max_attempts {
        tracing::debug!(
            target = "engine::retry",
            execution_id = %exec_state.execution_id,
            %node_key,
            attempts_used,
            max_attempts = policy.max_attempts,
            "retry skipped: per-node max_attempts reached"
        );
        return RetryDecision::Finalize;
    }

    // `delay_for_attempt(0)` = initial delay (after attempt #1 fails);
    // `delay_for_attempt(1)` = after attempt #2 fails; etc.
    // The just-finished attempt index is `attempts_used - 1` (0-based).
    let delay = policy.delay_for_attempt(attempts_used.saturating_sub(1));
    RetryDecision::Retry { delay }
}

/// Translate a Tokio-style retry delay to a chrono wall-clock
/// timestamp without panicking on extreme inputs.
///
/// `chrono::Duration::from_std` rejects values larger than
/// `i64::MAX` milliseconds (~292 million years). A naive
/// `.unwrap_or_default()` would silently substitute zero — turning
/// a misconfigured huge backoff into a hot-loop retry. Instead, we
/// log + clamp to `DateTime::<Utc>::MAX_UTC` so the next attempt is
/// effectively never scheduled, but the engine remains responsive
/// to cancel/wall-clock teardown signals. Using the absolute ceiling
/// avoids the `now + chrono::Duration::MAX` overflow that occurs for
/// any `now` value beyond the epoch.
///
/// `now` is supplied by the caller from an injectable [`Clock`] so
/// retry deadlines are deterministic and testable without real wall time.
pub(super) fn next_retry_at(
    execution_id: ExecutionId,
    node_key: &NodeKey,
    delay: Duration,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    match chrono::Duration::from_std(delay) {
        Ok(d) => now + d,
        Err(e) => {
            tracing::warn!(
                target = "engine::retry",
                %execution_id,
                %node_key,
                error = %e,
                delay_ms = delay.as_millis() as u64,
                "retry delay is not representable by chrono; clamping retry deadline to MAX_UTC"
            );
            // `now + chrono::Duration::MAX` overflows for any `now` near the epoch.
            // Use the absolute ceiling of `DateTime<Utc>` so the node is effectively
            // never retried while the engine remains responsive to cancel/shutdown.
            DateTime::<Utc>::MAX_UTC
        },
    }
}

/// Classification of a node failure against the workflow's error strategy.
///
/// Pure function of the strategy: splits the outcome from the state
/// mutation + edge routing that used to live together in the old
/// `handle_node_failure`. Split lets `run_frontier` order `state-mutation
/// → persist → emit → route` per / #297 — routing outgoing edges
/// may push successors into `ready_queue`, which must be a deterministic
/// function of the persisted state, not of an in-memory decision that
/// a crash can lose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FailureOutcome {
    /// `IgnoreErrors`: the node is recovered to `Completed` with a null
    /// output. No `NodeFailed` event is emitted; downstream edges activate
    /// as if the node had returned `ActionResult::success(null)`.
    Recover,
    /// `FailFast` or `ContinueOnError`: the node stays `Failed`. The
    /// caller emits `NodeFailed` and then routes failure edges, which
    /// may activate an OnError handler, resolve-without-activate for
    /// ContinueOnError, or request abort for FailFast.
    Fail,
}

/// Classify a failure outcome. Pure — does not touch `exec_state`.
pub(super) fn classify_failure(error_strategy: nebula_workflow::ErrorStrategy) -> FailureOutcome {
    match error_strategy {
        nebula_workflow::ErrorStrategy::IgnoreErrors => FailureOutcome::Recover,
        _ => FailureOutcome::Fail,
    }
}

/// Apply the IgnoreErrors in-memory recovery before routing + checkpoint.
///
/// For `FailureOutcome::Recover` (IgnoreErrors): overrides the state to
/// `Completed`, clears `error_message`, inserts a `null` output. Mirrors
/// the old `handle_node_failure` IgnoreErrors path. The override bumps
/// the version per #255 so CAS readers see the recovery.
///
/// For `FailureOutcome::Fail`: no-op. The failed state was set by the
/// caller's `mark_node_failed` (or `spawn_node`'s override); the OnError
/// input payload (if any edge matches) is written by
/// `route_failure_edges` and captured by the following checkpoint.
///
/// Returns `Err(EngineError::Execution)` if `override_node_state`
/// cannot find the node — the caller MUST abort the node's progression
/// rather than leave state + outputs half-applied . Pre-review
/// (PR #436 / Copilot) this function discarded the `Result` via
/// `let _ = ...`, silently masking a real consistency error.
pub(super) fn apply_failure_recovery(
    outcome: FailureOutcome,
    node_key: NodeKey,
    exec_state: &mut ExecutionState,
    outputs: &Arc<DashMap<NodeKey, serde_json::Value>>,
) -> Result<(), EngineError> {
    if outcome == FailureOutcome::Recover {
        exec_state.override_node_state(node_key.clone(), NodeState::Completed)?;
        if let Some(ns) = exec_state.node_states.get_mut(&node_key) {
            ns.error_message = None;
        }
        outputs.insert(node_key, serde_json::json!(null));
    }
    Ok(())
}

/// Route outgoing edges. MUST be called BEFORE `checkpoint_node` so
/// the OnError input payload this function writes into
/// `outputs[node_key]` is captured by the following checkpoint — that
/// is what `resume_execution`'s `load_all_outputs` reads when a
/// crashed OnError handler is replayed.
///
/// Successors pushed into `ready_queue` are invisible to external
/// observers until the next `Phase 1` dispatch, which runs strictly
/// after the outer match arm's `checkpoint_node`. If the following
/// checkpoint returns `Err`, the caller aborts the frontier (cancel
/// token + early return); the discarded `ready_queue` mutations never
/// surface — invariant holds.
///
/// Returns `Some(error_message)` if the frontier must abort — FailFast
/// strategy with no OnError handler took the failure. Returns `None`
/// when routing completed cleanly (OnError handled, ContinueOnError
/// resolved, or IgnoreErrors routed-as-success).
#[expect(clippy::too_many_arguments)]
pub(super) fn route_failure_edges(
    outcome: FailureOutcome,
    node_key: NodeKey,
    error_msg: &str,
    error_strategy: nebula_workflow::ErrorStrategy,
    graph: &DependencyGraph,
    outputs: &Arc<DashMap<NodeKey, serde_json::Value>>,
    activated_edges: &mut HashMap<NodeKey, HashSet<NodeKey>>,
    resolved_edges: &mut HashMap<NodeKey, usize>,
    required_count: &HashMap<NodeKey, usize>,
    ready_queue: &mut VecDeque<NodeKey>,
    exec_state: &mut ExecutionState,
) -> Option<String> {
    match outcome {
        FailureOutcome::Recover => {
            process_outgoing_edges(
                node_key,
                Some(&ActionResult::success(serde_json::json!(null))),
                None,
                graph,
                activated_edges,
                resolved_edges,
                required_count,
                ready_queue,
                exec_state,
            );
            None
        },
        FailureOutcome::Fail => {
            // Evaluate outgoing edges as a failure: OnError handlers,
            // if any, are activated; otherwise edges are resolved
            // without activation so dependents get Skipped.
            let error_handled = process_outgoing_edges(
                node_key.clone(),
                None,
                Some(error_msg),
                graph,
                activated_edges,
                resolved_edges,
                required_count,
                ready_queue,
                exec_state,
            );

            if error_handled {
                // Stage OnError handler input into outputs BEFORE the
                // checkpoint that will run next — guarantees the
                // payload is durably captured so a resumed OnError
                // successor can read it from persisted state via
                // `load_all_outputs` (#297 review / Copilot).
                outputs.insert(
                    node_key.clone(),
                    serde_json::json!({
                        "error": error_msg,
                        "node_id": node_key.to_string(),
                    }),
                );
                return None;
            }

            match error_strategy {
                nebula_workflow::ErrorStrategy::ContinueOnError => {
                    // Edges resolved (not activated) — dependents will be
                    // Skipped; unaffected branches continue.
                    None
                },
                // FailFast and future variants
                _ => Some(error_msg.to_owned()),
            }
        },
    }
}
