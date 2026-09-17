//! Frontier heaps — the `run_frontier` loop's two timer-heap drain phases.
//!
//! [`WorkflowEngine::drain_due_retries`] is Phase 0 of the loop: it promotes
//! due `WaitingRetry` entries to `Ready` and enqueues them on the ready queue.
//! [`WorkflowEngine::drain_due_wait_wakes`] is Phase 0b: it drains due
//! `wait_heap` wakes into either the completion path (`Waiting → Completed` +
//! `TimerCompleted` checkpoint + downstream edges) or the timeout failure
//! path (`Waiting → Failed` + failure-edge routing). Both act only on heap
//! entries whose deadline has passed; all post-drain control flow stays in
//! the loop body in `super`, which awaits Phase 0b inline — it is never
//! spawned or raced.

use super::*;

impl WorkflowEngine {
    /// Drain due retries from `ctx.retry_heap` into `ctx.ready_queue`.
    ///
    /// Phase 0 of the [`WorkflowEngine::run_frontier`] loop: promotes
    /// `WaitingRetry → Ready` and clears `next_attempt_at` so any subsequent
    /// cancel/terminate teardown sees a `Ready` node — `Ready → Cancelled` is
    /// a valid transition while a stranded `WaitingRetry` node would trip the
    /// frontier integrity (CAS on version) check. Phase 1's `spawn_node` then
    /// performs `Ready → Running` via `start_node_attempt`.
    ///
    /// Synchronous: it has no awaits, so it runs inline in the loop body and
    /// falls through when the heap's next entry is not yet due. No early
    /// returns — every drained entry either re-dispatches, is skipped
    /// defensively, or leaves the heap exhausted.
    pub(super) fn drain_due_retries(&self, ctx: &mut FrontierCtx<'_>, execution_id: ExecutionId) {
        let now_drain = self.clock.now();
        while let Some(Reverse((when, _))) = ctx.retry_heap.peek() {
            if *when > now_drain {
                break;
            }
            let Some(Reverse((_, node_key))) = ctx.retry_heap.pop() else {
                // Unreachable: peek-then-pop on a single-threaded
                // owner cannot lose the entry. Surface defensively
                // rather than panic so a future refactor can't
                // crash the frontier loop (hot-path safety).
                tracing::warn!(
                    target = "engine::retry",
                    %execution_id,
                    "retry heap became empty after peek; aborting retry drain"
                );
                break;
            };
            let still_parked = ctx
                .exec_state
                .node_state(node_key.clone())
                .is_some_and(|ns| ns.state == NodeState::WaitingRetry);
            if still_parked {
                match ctx
                    .exec_state
                    .transition_node(node_key.clone(), NodeState::Ready)
                {
                    Ok(()) => {
                        if let Some(ns) = ctx.exec_state.node_states.get_mut(&node_key) {
                            ns.next_attempt_at = None;
                        }
                        ctx.ready_queue.push_back(node_key.clone());
                        tracing::debug!(
                            target = "engine::frontier",
                            %execution_id,
                            %node_key,
                            "retry attempt re-dispatched after backoff"
                        );
                    },
                    Err(err) => {
                        // WaitingRetry → Ready is in the canonical
                        // table; this is unreachable in practice
                        // but we surface defensively rather than
                        // panic (hot-path safety).
                        tracing::warn!(
                            target = "engine::retry",
                            %execution_id,
                            %node_key,
                            %err,
                            "retry promotion to Ready rejected; skipping"
                        );
                    },
                }
            } else {
                tracing::debug!(
                    target = "engine::frontier",
                    %execution_id,
                    %node_key,
                    "retry drained but node no longer in WaitingRetry; skipping"
                );
            }
        }
    }

    /// Drain due timer-wakes from `ctx.wait_heap` (Phase 0b of the
    /// [`WorkflowEngine::run_frontier`] loop).
    ///
    /// A `Waiting` node whose `next_attempt_at` has passed has its
    /// timer wake due. What the wake MEANS is read from the persisted
    /// `wait_wake` discriminator (W-S2b), re-read here under the
    /// single-threaded loop owner AFTER the pop — never acted on from
    /// the popped heap tuple alone, so a Resume-then-timeout race
    /// (which self-arms the node `Completion`) cannot be double-routed:
    ///   - `Completion` / legacy `None`: complete the node
    ///     (`Waiting → Completed`), activate `main`-port downstream.
    ///     The `partial_output` was committed at park time.
    ///   - `Timeout`: the signal wait's deadline elapsed with no
    ///     Resume — FAIL the node (`Waiting → Failed` with
    ///     `RuntimeError::WaitTimedOut`) and route its outgoing edges
    ///     through the failure path (OnError / Skip / FailFast).
    ///
    /// Awaits inline (checkpoint persistence) and is awaited by the loop body
    /// — never spawned or raced. The inner `continue`s below are
    /// inner-`while` continues: they skip to the next heap entry, not out of
    /// the drain.
    ///
    /// Returns:
    ///   - `Ok(None)` — the drain finished without a frontier exit; the loop
    ///     body proceeds to Phase 1.
    ///   - `Ok(Some((node_key, error)))` — the timeout fail-path's FailFast
    ///     abort (no OnError handler): `cancel_token` is cancelled first so
    ///     `determine_final_status` priority-2 marks the execution `Failed`.
    ///   - `Err(e)` — a checkpoint failure propagated out of the drain. The
    ///     timeout and completion checkpoint sites cancel `cancel_token`
    ///     before returning, exactly as in the inline path; the
    ///     `TimerCompleted` partial-output projection propagates WITHOUT a
    ///     cancel, also as in the inline path. Both stay `Failed`
    ///     (`determine_final_status` keys `Cancelled` on
    ///     `cancel_token.is_cancelled`, so these paths must never surface as
    ///     an `Ok` result).
    #[expect(
        clippy::too_many_arguments,
        reason = "the drain mirrors the inline phase's parameter list; bundling the \
                  loop-carried scope/graph/strategy/fencing into a struct would \
                  re-shape the Phase 1..3 call sites for no behavioral gain"
    )]
    pub(super) async fn drain_due_wait_wakes(
        &self,
        ctx: &mut FrontierCtx<'_>,
        scope: &Scope,
        graph: &DependencyGraph,
        execution_id: ExecutionId,
        error_strategy: nebula_workflow::ErrorStrategy,
        fencing: Option<nebula_storage_port::FencingToken>,
        cancel_token: &CancellationToken,
    ) -> Result<Option<(NodeKey, String)>, EngineError> {
        let now_wait_drain = self.clock.now();
        while let Some(Reverse((when, _))) = ctx.wait_heap.peek() {
            if *when > now_wait_drain {
                break;
            }
            // Capture the deadline from the POPPED tuple (owned), not the
            // peek reference, so we may mutate `wait_heap` again below
            // while still using it for the timeout-ms reconstruction.
            let Some(Reverse((deadline, node_key))) = ctx.wait_heap.pop() else {
                // Unreachable: peek-then-pop on a single-threaded
                // owner cannot lose the entry. Surface defensively
                // rather than panic (hot-path safety).
                tracing::warn!(
                    target = "engine::wait",
                    %execution_id,
                    "wait heap became empty after peek; aborting wait drain"
                );
                break;
            };
            // Race-safe re-read (R2): both the state AND the wake
            // discriminator come from the live `exec_state` AFTER the pop,
            // so a stale `(deadline, key)` entry for a node a Resume
            // already re-armed `Completion` is read as a completion, not a
            // timeout — never double-routed.
            let parked_wake = ctx
                .exec_state
                .node_state(node_key.clone())
                .filter(|ns| ns.state == NodeState::Waiting)
                .map(|ns| ns.wait_wake);
            let Some(wait_wake) = parked_wake else {
                tracing::debug!(
                    target = "engine::wait",
                    %execution_id,
                    %node_key,
                    "wait heap drained but node no longer in Waiting; skipping"
                );
                continue;
            };
            // Legacy `None` on an armed timer wait reads as `Completion`
            // (preserves W-S1 timer-wake semantics for pre-W-S2b rows).
            if matches!(wait_wake, Some(WaitWake::Timeout)) {
                // ── Timeout fail path ──
                //
                // The `WaitCondition` variant is not persisted on the node
                // (only `next_attempt_at` + `wait_wake` are) — so the exact
                // signal kind (`Webhook` / `Approval` / `Execution`) is not
                // recoverable here, especially after a crash + recovery.
                // Report the honest discriminator we DO have: a signal
                // wait. (Per-variant detail returns with W-S3's persisted
                // resume targeting.)
                let condition_kind = "signal".to_owned();
                // Best-effort declared-timeout reconstruction: the absolute
                // deadline `when` (== `next_attempt_at`) minus the node's
                // `started_at` (stamped just before it parked) approximates
                // the original `timeout` duration. It survives crash +
                // recovery (both fields are persisted) and is an
                // observability value, not a control input — a small
                // over-estimate (the node's pre-park run time) is acceptable.
                let timeout_ms = ctx
                    .exec_state
                    .node_state(node_key.clone())
                    .and_then(|ns| ns.started_at)
                    .map(|started| {
                        deadline
                            .signed_duration_since(started)
                            .num_milliseconds()
                            .max(0) as u64
                    })
                    .unwrap_or(0);
                let timed_out = crate::runtime::error::RuntimeError::WaitTimedOut {
                    condition_kind: condition_kind.clone(),
                    timeout_ms,
                };
                let engine_err = EngineError::Runtime(timed_out);
                // The failure text below is handed to `route_failure_edges`,
                // whose OnError input payload the following checkpoint
                // captures durably — so it takes its text from the same
                // bounded, control-character-escaped envelope seam as the
                // node's own failure record (`mark_node_failed`), instead of
                // a raw `Display` that bypassed it.
                //
                // The projection is the envelope's full `Display` (`code:
                // message`), matching the action-failure branch below, so an
                // OnError handler parses ONE payload shape regardless of
                // which failure fired — not the message half on one path and
                // the code-prefixed form on the other.
                // `Waiting → Failed` is the W-S2b timeout edge. A
                // WaitTimedOut is terminal and bypasses the retry decision
                // entirely (it never counts against the retry budget).
                // `mark_node_failed` builds the envelope once; reuse its
                // return for the projection below instead of a second
                // `durable_error_envelope` build over the same message.
                let err_str =
                    mark_node_failed(ctx.exec_state, node_key.clone(), &engine_err).to_string();
                if let Some(ns) = ctx.exec_state.node_states.get_mut(&node_key) {
                    // Resolved wait: drop the timer pair so the now-`Failed`
                    // node carries no stale wake metadata.
                    ns.clear_wait_timer();
                }
                // Route outgoing edges through the failure path BEFORE the
                // checkpoint so an OnError handler's input payload is
                // durably captured (reuse — same contract as the Phase-3
                // finalize path). `Fail` (not `Recover`): OnError handlers,
                // if wired, activate; otherwise dependents are Skipped and
                // FailFast aborts the frontier.
                //
                // `Fail` is deliberate even under an `ErrorStrategy::
                // IgnoreErrors` node strategy: a wait timeout is a real
                // negative outcome (a missed approval/signal), not a
                // swallowable transient fault. It must surface as a failure
                // rather than be coerced to `Completed` with a null output.
                let abort = route_failure_edges(
                    FailureOutcome::Fail,
                    node_key.clone(),
                    &err_str,
                    error_strategy,
                    graph,
                    ctx.outputs,
                    &mut ctx.activated_edges,
                    &mut ctx.resolved_edges,
                    &ctx.required_count,
                    &mut ctx.ready_queue,
                    ctx.exec_state,
                );
                // Durably commit the `Failed` transition (+ any OnError
                // payload routing already staged) before any observer sees
                // the timeout.
                if let Err(e) = self
                    .checkpoint_node(
                        scope,
                        execution_id,
                        node_key.clone(),
                        Some(checkpoint::failure_checkpoint(
                            FailureOutcome::Fail,
                            ctx.outputs,
                            &node_key,
                        )),
                        ctx.outputs,
                        ctx.exec_state,
                        ctx.repo_version,
                        fencing,
                        vec![],
                    )
                    .await
                {
                    cancel_token.cancel();
                    return Err(e);
                }
                self.emit_event(ExecutionEvent::NodeWaitTimedOut {
                    execution_id,
                    node_key: node_key.clone(),
                    condition_kind,
                    timeout_ms,
                });
                tracing::info!(
                    target = "engine::wait",
                    %execution_id,
                    %node_key,
                    timeout_ms,
                    "signal wait timed out; node failed and failure edges routed"
                );
                if let Some(err_msg) = abort {
                    // FailFast (no OnError handler): abort the frontier and
                    // surface the timeout as the `failed_node` so
                    // `determine_final_status` priority-2 marks the
                    // execution `Failed`.
                    cancel_token.cancel();
                    return Ok(Some((node_key.clone(), err_msg)));
                }
                // OnError-handled / ContinueOnError: the failure was routed
                // to the error branch / dependents Skipped — the loop
                // continues so the error subtree runs.
                continue;
            }
            // ── Completion path (Completion / legacy None) ──
            match ctx
                .exec_state
                .transition_node(node_key.clone(), NodeState::Completed)
            {
                Ok(()) => {
                    if let Some(ns) = ctx.exec_state.node_states.get_mut(&node_key) {
                        // Resolved wait: drop the timer pair on the now-
                        // `Completed` node.
                        ns.clear_wait_timer();
                        ns.completed_at = Some(self.clock.now());
                    }
                    // Persist the `Completed` transition and the cleared
                    // timer fields atomically before activating downstream —
                    // durability precedes visibility.
                    if let Err(e) = self
                        .checkpoint_node(
                            scope,
                            execution_id,
                            node_key.clone(),
                            Some(nebula_execution::NodeCheckpoint::TimerCompleted {
                                partial_output: ctx
                                    .exec_state
                                    .checkpoint
                                    .as_ref()
                                    .and_then(|checkpoint| checkpoint.nodes().get(&node_key))
                                    .map(checkpoint::checkpoint_output)
                                    .transpose()?
                                    .flatten(),
                            }),
                            ctx.outputs,
                            ctx.exec_state,
                            ctx.repo_version,
                            fencing,
                            vec![],
                        )
                        .await
                    {
                        cancel_token.cancel();
                        return Err(e);
                    }
                    self.emit_event(ExecutionEvent::NodeWaitCompleted {
                        execution_id,
                        node_key: node_key.clone(),
                    });
                    tracing::info!(
                        target = "engine::wait",
                        %execution_id,
                        %node_key,
                        "wait condition satisfied (timer); node completed"
                    );
                    // Activate downstream edges now that the node is
                    // `Completed` — this is the point at which the
                    // downstream gate lifts.
                    process_outgoing_edges(
                        node_key.clone(),
                        None, // no live `ActionResult` for this synthetic completion
                        None,
                        graph,
                        &mut ctx.activated_edges,
                        &mut ctx.resolved_edges,
                        &ctx.required_count,
                        &mut ctx.ready_queue,
                        ctx.exec_state,
                    );
                },
                Err(err) => {
                    // `Waiting → Completed` is in the canonical table;
                    // this branch fires only if the node was concurrently
                    // cancelled. Surface defensively.
                    tracing::warn!(
                        target = "engine::wait",
                        %execution_id,
                        %node_key,
                        %err,
                        "wait-heap wake: Waiting→Completed rejected; skipping"
                    );
                },
            }
        }
        Ok(None)
    }
}
