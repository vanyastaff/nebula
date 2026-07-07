//! Frontier execution — the level-by-level DAG executor.
//!
//! `run_frontier` drives ready nodes to completion with bounded concurrency,
//! and `spawn_node` builds the per-node task. Split out of `engine.rs` as part
//! of the god-module decomposition (audit 🔴-1). These remain `impl
//! WorkflowEngine` methods in a child module, so they keep full access to the
//! engine's private fields, sibling methods, helper free functions, and types
//! through `use super::*`.

use super::*;

impl WorkflowEngine {
    /// Execute all reachable nodes using a frontier-based approach.
    ///
    /// Nodes are spawned as soon as all their incoming edges have been resolved
    /// and at least one edge has been activated. This supports branching, skip
    /// propagation, and error routing.
    ///
    /// `seed_nodes` is the initial set of nodes to place on the ready queue.
    /// For a fresh execution this is the graph's entry nodes; for resumed
    /// executions it is the computed resume frontier.
    ///
    /// `initial_activated` and `initial_resolved` carry the edge-tracking
    /// state derived from already-completed nodes (populated for resume; empty
    /// for fresh executions).
    ///
    /// Returns `Some((node_key, error))` if a node failed without an error handler,
    /// `None` if all reachable nodes completed (or were skipped).
    #[expect(clippy::too_many_arguments)]
    pub(super) async fn run_frontier(
        &self,
        scope: &Scope,
        graph: &DependencyGraph,
        node_map: &HashMap<NodeKey, &nebula_workflow::NodeDefinition>,
        outputs: &Arc<DashMap<NodeKey, serde_json::Value>>,
        semaphore: &Arc<Semaphore>,
        cancel_token: &CancellationToken,
        resume_rx: &mut mpsc::Receiver<ResumeRequest>,
        exec_state: &mut ExecutionState,
        execution_id: ExecutionId,
        workflow_id: WorkflowId,
        input: &serde_json::Value,
        repo_version: &mut u64,
        fencing: Option<nebula_storage_port::FencingToken>,
        budget: &ExecutionBudget,
        started: &Instant,
        error_strategy: nebula_workflow::ErrorStrategy,
        workflow_retry_policy: Option<nebula_workflow::RetryConfig>,
        seed_nodes: Vec<NodeKey>,
        initial_activated: HashMap<NodeKey, HashSet<NodeKey>>,
        initial_resolved: HashMap<NodeKey, usize>,
    ) -> Option<(NodeKey, String)> {
        let total_output_bytes = Arc::new(AtomicU64::new(0));
        // Precompute how many incoming edges each node has
        let required_count: HashMap<NodeKey, usize> = node_map
            .keys()
            .map(|nid| (nid.clone(), graph.incoming_connections(nid.clone()).len()))
            .collect();

        // Track edge resolution state (pre-populated for resume)
        let mut activated_edges = initial_activated;
        let mut resolved_edges = initial_resolved;

        // Queue of nodes ready to execute
        let mut ready_queue: VecDeque<NodeKey> = VecDeque::new();

        // Seed with the provided nodes (entry nodes for fresh; frontier for resume)
        for node_key in seed_nodes {
            ready_queue.push_back(node_key);
        }

        // Min-heap (via `Reverse`) of `(next_attempt_at, NodeKey)` for
        // nodes parked in `WaitingRetry` per T5. The
        // heap is the engine's source of truth for "what to dispatch
        // next when the current frontier is otherwise idle"; cancel /
        // terminate / budget guards run AFTER the timer fires so a
        // cancelled execution does not silently re-dispatch a node.
        let mut retry_heap: BinaryHeap<Reverse<(DateTime<Utc>, NodeKey)>> = BinaryHeap::new();

        // Resume seeding: any node already in `WaitingRetry` from a
        // prior run (its `next_attempt_at` survived via JSONB) needs
        // to land on the heap before the loop starts. Otherwise a
        // resumed retry would silently never re-dispatch.
        for (key, ns) in &exec_state.node_states {
            if ns.state == NodeState::WaitingRetry
                && let Some(when) = ns.next_attempt_at
            {
                retry_heap.push(Reverse((when, key.clone())));
            }
        }

        // Min-heap of `(wake_at, NodeKey)` for nodes parked in `Waiting`
        // with a timer-based condition (`Until` / `Duration`). Mirrors
        // `retry_heap` but drains to `Completed` instead of `Ready` —
        // a satisfied wait condition means the node is done, not
        // restarted. Signal-only parked nodes (webhook/approval/execution
        // with no timeout) are NOT on this heap; they stay parked until
        // a `Resume` signal arrives (built separately).
        let mut wait_heap: BinaryHeap<Reverse<(DateTime<Utc>, NodeKey)>> = BinaryHeap::new();

        // Resume seeding for `Waiting` nodes: a crashed engine may have
        // persisted a node in `Waiting` with a `next_attempt_at` timer;
        // re-seed the heap so the wake fires without requiring a fresh
        // `ActionResult::Wait` dispatch.
        for (key, ns) in &exec_state.node_states {
            if ns.state == NodeState::Waiting
                && let Some(when) = ns.next_attempt_at
            {
                wait_heap.push(Reverse((when, key.clone())));
            }
        }

        // In-flight tasks + a side map from tokio task id → NodeKey so
        // that panics (where the inner future's `(NodeKey, _)` payload
        // is lost) can still be attributed to the real node instead
        // of a synthesized placeholder (issue #301).
        let mut join_set: JoinSet<(
            NodeKey,
            Result<ActionResult<serde_json::Value>, EngineError>,
        )> = JoinSet::new();
        let mut task_nodes: HashMap<tokio::task::Id, NodeKey> = HashMap::new();

        // Disarms the `resume_rx.recv()` select! arm after the first `None`
        // (channel closed). Without this guard the arm would poll `Ready(None)`
        // on every iteration — a busy-spin for the full run duration. This
        // fires immediately on the replay path (the Sender is dropped at the
        // `RunningEntry` construction site) and also defends against any
        // premature drop of the Running registration in the execute path.
        let mut resume_rx_closed = false;

        // Main frontier loop
        loop {
            // Phase 0: Drain due retries from the retry_heap into the
            // ready_queue.
            //
            // Phase 0 promotes `WaitingRetry → Ready` and clears
            // `next_attempt_at` so any subsequent cancel/terminate
            // teardown sees a `Ready` node — `Ready → Cancelled` is a
            // valid transition while a stranded `WaitingRetry` node
            // would trip the frontier integrity (CAS on version) check.
            // Phase 1's `spawn_node` then performs `Ready → Running`
            // via `start_node_attempt`.
            let now_drain = self.clock.now();
            while let Some(Reverse((when, _))) = retry_heap.peek() {
                if *when > now_drain {
                    break;
                }
                let Some(Reverse((_, node_key))) = retry_heap.pop() else {
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
                let still_parked = exec_state
                    .node_state(node_key.clone())
                    .is_some_and(|ns| ns.state == NodeState::WaitingRetry);
                if still_parked {
                    match exec_state.transition_node(node_key.clone(), NodeState::Ready) {
                        Ok(()) => {
                            if let Some(ns) = exec_state.node_states.get_mut(&node_key) {
                                ns.next_attempt_at = None;
                            }
                            ready_queue.push_back(node_key.clone());
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

            // Phase 0b: Drain due timer-wakes from `wait_heap`.
            //
            // A `Waiting` node whose `next_attempt_at` has passed has its
            // timer wake due. What the wake MEANS is read from the persisted
            // `wait_wake` discriminator (W-S2b), re-read here under the
            // single-threaded loop owner AFTER the pop — never acted on from
            // the popped heap tuple alone, so a Resume-then-timeout race
            // (which self-arms the node `Completion`) cannot be double-routed:
            //   - `Completion` / legacy `None`: complete the node
            //     (`Waiting → Completed`), activate `main`-port downstream.
            //     The `partial_output` was committed at park time.
            //   - `Timeout`: the signal wait's deadline elapsed with no
            //     Resume — FAIL the node (`Waiting → Failed` with
            //     `RuntimeError::WaitTimedOut`) and route its outgoing edges
            //     through the failure path (OnError / Skip / FailFast).
            let now_wait_drain = self.clock.now();
            while let Some(Reverse((when, _))) = wait_heap.peek() {
                if *when > now_wait_drain {
                    break;
                }
                // Capture the deadline from the POPPED tuple (owned), not the
                // peek reference, so we may mutate `wait_heap` again below
                // while still using it for the timeout-ms reconstruction.
                let Some(Reverse((deadline, node_key))) = wait_heap.pop() else {
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
                let parked_wake = exec_state
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
                    let timeout_ms = exec_state
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
                    let err_str = engine_err.to_string();
                    // `Waiting → Failed` is the W-S2b timeout edge. A
                    // WaitTimedOut is terminal and bypasses the retry decision
                    // entirely (it never counts against the retry budget).
                    mark_node_failed(exec_state, node_key.clone(), &engine_err);
                    if let Some(ns) = exec_state.node_states.get_mut(&node_key) {
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
                        outputs,
                        &mut activated_edges,
                        &mut resolved_edges,
                        &required_count,
                        &mut ready_queue,
                        exec_state,
                    );
                    // Durably commit the `Failed` transition (+ any OnError
                    // payload routing already staged) before any observer sees
                    // the timeout.
                    if let Err(e) = self
                        .checkpoint_node(
                            scope,
                            execution_id,
                            node_key.clone(),
                            outputs,
                            exec_state,
                            repo_version,
                            fencing,
                            vec![],
                        )
                        .await
                    {
                        cancel_token.cancel();
                        return Some((node_key.clone(), e.to_string()));
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
                        return Some((node_key.clone(), err_msg));
                    }
                    // OnError-handled / ContinueOnError: the failure was routed
                    // to the error branch / dependents Skipped — the loop
                    // continues so the error subtree runs.
                    continue;
                }
                // ── Completion path (Completion / legacy None) ──
                match exec_state.transition_node(node_key.clone(), NodeState::Completed) {
                    Ok(()) => {
                        if let Some(ns) = exec_state.node_states.get_mut(&node_key) {
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
                                outputs,
                                exec_state,
                                repo_version,
                                fencing,
                                vec![],
                            )
                            .await
                        {
                            cancel_token.cancel();
                            return Some((node_key.clone(), e.to_string()));
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
                            &mut activated_edges,
                            &mut resolved_edges,
                            &required_count,
                            &mut ready_queue,
                            exec_state,
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

            // Phase 1: Drain ready queue → spawn into join_set.
            // Cancel-check runs BEFORE `pop_front` so a node that
            // observes the cancel signal mid-iteration stays in the
            // queue and is collected by `drain_pending_to_cancelled`
            // on the cancel/wall-clock teardown branches — popping
            // first would drop the node and strand it as `Ready`,
            // tripping frontier integrity (CAS on version).
            while !cancel_token.is_cancelled()
                && let Some(node_key) = ready_queue.pop_front()
            {
                // Check budget limits before dispatching
                if let Some(violation) = check_budget(budget, started, &total_output_bytes) {
                    cancel_token.cancel();
                    return Some((node_key, violation));
                }

                // Skip disabled nodes: mark as Skipped and activate outgoing edges
                // with null output so successors continue normally.
                if node_map.get(&node_key).is_some_and(|nd| !nd.enabled) {
                    mark_node_skipped(exec_state, node_key.clone());
                    process_outgoing_edges(
                        node_key.clone(),
                        None, // null output — Always edges activate
                        None, // not failed
                        graph,
                        &mut activated_edges,
                        &mut resolved_edges,
                        &required_count,
                        &mut ready_queue,
                        exec_state,
                    );
                    continue;
                }

                // Durable idempotency check: if this node was already executed
                // (e.g., on a previous attempt), load the persisted output and
                // mark it completed without re-dispatching.
                if self
                    .check_and_apply_idempotency(
                        scope,
                        execution_id,
                        node_key.clone(),
                        outputs,
                        exec_state,
                        graph,
                        &mut activated_edges,
                        &mut resolved_edges,
                        &required_count,
                        &mut ready_queue,
                    )
                    .await
                {
                    continue;
                }

                let spawned = self.spawn_node(
                    node_key.clone(),
                    node_map,
                    graph,
                    outputs,
                    semaphore,
                    cancel_token,
                    exec_state,
                    execution_id,
                    workflow_id,
                    input,
                    &activated_edges,
                    &mut join_set,
                    &mut task_nodes,
                );
                if spawned {
                    let action_key = node_map
                        .get(&node_key)
                        .map(|n| n.action_key.to_string())
                        .unwrap_or_default();
                    self.emit_event(ExecutionEvent::NodeStarted {
                        execution_id,
                        node_key: node_key.clone(),
                        action_key,
                    });
                    continue;
                }

                // Node failed during setup (e.g., param resolution).
                // `spawn_node` already marked the node as Failed and stored
                // the typed error message on `NodeExecutionState`.
                //
                // T4 — setup failures are retry-eligible
                // ("the action never started — re-running may succeed,
                // e.g. credential rotation"). Same retry-decision flow
                // as the runtime-failure path.
                //
                // Ordering on the no-retry path (, #297 review):
                // record_attempt → classify → apply recovery → route
                // (stages OnError payload into outputs) → checkpoint
                // (durably commits state + staged payload) → emit.
                let err_msg = exec_state
                    .node_state(node_key.clone())
                    .and_then(|ns| ns.error_message.clone())
                    .unwrap_or_else(|| "parameter resolution failed".to_string());

                // Push the failure attempt record so retry-decision
                // and idempotency_key see the same attempt count.
                // T4: if recording fails (programming
                // error: unknown node), force `Finalize` rather than
                // running `compute_retry_decision` against a stale
                // `attempts.len()` — that would let `max_attempts`
                // be bypassed and risk an idempotency-key collision.
                let setup_attempt_recorded = match exec_state.record_node_attempt(
                    node_key.clone(),
                    AttemptOutcome::Failure {
                        error: err_msg.clone(),
                    },
                ) {
                    Ok(_) => true,
                    Err(e) => {
                        tracing::warn!(
                            target = "engine::frontier",
                            %execution_id,
                            %node_key,
                            error = %e,
                            "record_node_attempt(setup-failure) failed; forcing finalize \
                             so stale attempt history cannot bypass max_attempts or \
                             collide idempotency keys"
                        );
                        false
                    },
                };

                // T4 — retry decision (setup-failure
                // path). Mirror runtime-failure semantics. Skip the
                // decision entirely when attempt history is broken.
                let setup_decision = if setup_attempt_recorded {
                    let setup_retry_policy = node_map
                        .get(&node_key)
                        .and_then(|nd| effective_retry_policy(nd, workflow_retry_policy.as_ref()))
                        .cloned();
                    // Setup failures (param resolution etc.) have no typed
                    // `ActionError` and stay retry-eligible — "the action
                    // never started", so no fatal short-circuit applies.
                    compute_retry_decision(
                        &node_key,
                        exec_state,
                        setup_retry_policy.as_ref(),
                        false,
                    )
                } else {
                    RetryDecision::Finalize
                };

                if let RetryDecision::Retry { delay } = setup_decision {
                    let attempt_number = exec_state
                        .node_states
                        .get(&node_key)
                        .map_or(1, |ns| ns.attempt_count() as u32);
                    let next_at = next_retry_at(execution_id, &node_key, delay, self.clock.now());
                    if exec_state
                        .schedule_node_retry(node_key.clone(), next_at)
                        .is_ok()
                    {
                        if let Err(e) = self
                            .checkpoint_node(
                                scope,
                                execution_id,
                                node_key.clone(),
                                outputs,
                                exec_state,
                                repo_version,
                                fencing,
                                vec![],
                            )
                            .await
                        {
                            cancel_token.cancel();
                            return Some((node_key, e.to_string()));
                        }
                        retry_heap.push(Reverse((next_at, node_key.clone())));
                        tracing::info!(
                            target = "engine::retry",
                            %execution_id,
                            %node_key,
                            attempt = attempt_number,
                            delay_ms = delay.as_millis() as u64,
                            next_attempt_at = %next_at,
                            total_retries = exec_state.total_retries,
                            "retry scheduled (setup-failure path)"
                        );
                        self.emit_event(ExecutionEvent::NodeRetryScheduled {
                            execution_id,
                            node_key: node_key.clone(),
                            attempt: attempt_number,
                            next_attempt_at: next_at,
                            last_error: err_msg.clone(),
                        });
                        continue;
                    }
                }

                let outcome = classify_failure(error_strategy);
                if let Err(e) =
                    apply_failure_recovery(outcome, node_key.clone(), exec_state, outputs)
                {
                    cancel_token.cancel();
                    return Some((node_key, e.to_string()));
                }

                // Route BEFORE checkpoint so the OnError input payload
                // (`outputs[node_key] = {error, node_id}`) written by
                // `route_failure_edges` is captured by the checkpoint.
                // Successors enqueued into `ready_queue` are invisible
                // until Phase 1 of the next loop iteration, which runs
                // strictly after the checkpoint below — nothing external
                // observes the routing before the store commits it.
                let abort = route_failure_edges(
                    outcome,
                    node_key.clone(),
                    &err_msg,
                    error_strategy,
                    graph,
                    outputs,
                    &mut activated_edges,
                    &mut resolved_edges,
                    &required_count,
                    &mut ready_queue,
                    exec_state,
                );

                if let Err(e) = self
                    .checkpoint_node(
                        scope,
                        execution_id,
                        node_key.clone(),
                        outputs,
                        exec_state,
                        repo_version,
                        fencing,
                        vec![],
                    )
                    .await
                {
                    cancel_token.cancel();
                    return Some((node_key, e.to_string()));
                }

                if exec_state
                    .node_state(node_key.clone())
                    .is_some_and(|ns| ns.state == NodeState::Failed)
                {
                    self.emit_event(ExecutionEvent::NodeFailed {
                        execution_id,
                        node_key: node_key.clone(),
                        details: NodeFailedDetails {
                            error_code: "ENGINE:NODE_FAILED".to_owned(),
                            display_message: err_msg.clone(),
                        },
                    });
                }

                if let Some(err_msg) = abort {
                    cancel_token.cancel();
                    return Some((node_key, err_msg));
                }
            }

            // Phase 2: tear-down / exit.
            //
            // The cancel check MUST run BEFORE the empty-heap exit: a
            // signal-parked execution has all heaps empty and an empty
            // join_set, so the empty-heap exit would fire first and skip
            // the cancel teardown — leaving the signal-`Waiting{None}` node
            // non-terminal under a `Cancelled` execution. Checking cancel
            // first routes a cancel-during-signal-park through
            // `drain_pending_to_cancelled` (which also cancels signal waits
            // that are not on any heap). A clean (non-cancelled) finish
            // still exits via the empty-heap break below.
            if cancel_token.is_cancelled() {
                join_set.abort_all();
                while join_set.join_next_with_id().await.is_some() {}
                task_nodes.clear();
                // Tear down parked retries (WaitingRetry → Cancelled),
                // parked wait nodes (Waiting → Cancelled, incl. signal
                // waits not on `wait_heap`), AND the ready_queue
                // (Ready → Cancelled). The previous failure already lives
                // in `NodeAttempt`; the cancel terminates the wait, not the
                // attempt (operational honesty). Without draining
                // `ready_queue`, a node Phase 0 already promoted to `Ready`
                // would stay non-terminal after the loop exits, tripping
                // the frontier integrity check.
                drain_pending_to_cancelled(
                    &mut retry_heap,
                    &mut wait_heap,
                    &mut ready_queue,
                    exec_state,
                    execution_id,
                );
                break;
            }

            // Exit only when join_set, retry_heap, AND wait_heap are
            // all drained — a non-empty heap with an empty join_set
            // is a legal "everything paused for a timer" state.
            if join_set.is_empty() && retry_heap.is_empty() && wait_heap.is_empty() {
                break;
            }

            // Race join_set against the wall-clock deadline so a hung node
            // cannot starve budget enforcement. The Phase 1 check_budget call
            // only fires while ready_queue has work; once everything is in
            // flight, this select is the sole budget guard.
            let wall_clock_remaining: Option<Duration> = budget
                .max_duration
                .map(|max_dur| max_dur.saturating_sub(started.elapsed()));
            let sleep_fut = async {
                if let Some(d) = wall_clock_remaining {
                    tokio::time::sleep(d).await;
                } else {
                    std::future::pending::<()>().await;
                }
            };
            tokio::pin!(sleep_fut);

            // Compute the sleep until the next retry timer fires. If
            // `retry_heap` is empty, sleep forever (the join_set / cancel
            // / wall-clock arms still drive the select).
            let next_retry_in: Option<Duration> = retry_heap.peek().map(|Reverse((when, _))| {
                when.signed_duration_since(self.clock.now())
                    .to_std()
                    .unwrap_or(Duration::ZERO)
            });
            let retry_sleep_fut = async {
                if let Some(d) = next_retry_in {
                    tokio::time::sleep(d).await;
                } else {
                    std::future::pending::<()>().await;
                }
            };
            tokio::pin!(retry_sleep_fut);

            // Compute the sleep until the earliest parked-wait timer fires.
            // This drives Phase 0b drains when `join_set` is otherwise idle.
            let next_wait_in: Option<Duration> = wait_heap.peek().map(|Reverse((when, _))| {
                when.signed_duration_since(self.clock.now())
                    .to_std()
                    .unwrap_or(Duration::ZERO)
            });
            let wait_sleep_fut = async {
                if let Some(d) = next_wait_in {
                    tokio::time::sleep(d).await;
                } else {
                    std::future::pending::<()>().await;
                }
            };
            tokio::pin!(wait_sleep_fut);

            // If join_set is empty but retry_heap or wait_heap has work,
            // we still need to sleep until the timer (or cancel /
            // wall-clock). Pre-pin a boxed future per branch so `select!`
            // never has to enter an `unreachable!()` placeholder — library
            // code must not panic on hot paths (hot-path safety).
            let join_set_empty = join_set.is_empty();

            type JoinedResult = Option<
                Result<
                    (
                        tokio::task::Id,
                        (
                            NodeKey,
                            Result<ActionResult<serde_json::Value>, EngineError>,
                        ),
                    ),
                    tokio::task::JoinError,
                >,
            >;
            // Joined variant is the wide one — the other arms are
            // unit-like timer markers. The size asymmetry is intrinsic
            // to a wake-reason discriminant and acceptable on a path
            // that allocates one value per loop iteration.
            #[expect(clippy::large_enum_variant)]
            enum WakeReason {
                Joined(JoinedResult),
                RetryTimer,
                WaitTimer,
                WallClock,
                Cancel,
                ResumeSignalled(ResumeRequest),
                ResumeChannelClosed,
            }

            let join_next_fut: Pin<Box<dyn Future<Output = JoinedResult> + Send + '_>> =
                if join_set_empty {
                    Box::pin(std::future::pending::<JoinedResult>())
                } else {
                    Box::pin(join_set.join_next_with_id())
                };

            // `mpsc::Receiver::recv()` is cancellation-safe in this `select!`:
            // a `ResumeRequest` that is sent while another arm wins this
            // iteration is NOT consumed — it stays buffered in the channel and
            // the next iteration's fresh `recv()` delivers it. This is strictly
            // better than the prior `Notify` permit-latch: the request (and its
            // `ack` reply channel) is never dropped between iterations, so the
            // caller's ack-await always resolves to a durable outcome.
            //
            // The `if !resume_rx_closed` guard is REQUIRED: once the channel is
            // closed, `recv()` returns `Ready(None)` synchronously on every
            // poll. Without the guard the closed arm would win the select! on
            // every iteration regardless of wall-clock sleep — a busy-spin for
            // the full run duration. After exactly one `None` the flag is set
            // and the arm becomes permanently `Pending` (disabled), letting the
            // other arms run at their natural pace.
            let wake = tokio::select! {
                result = join_next_fut => WakeReason::Joined(result),
                () = &mut retry_sleep_fut, if next_retry_in.is_some() => WakeReason::RetryTimer,
                () = &mut wait_sleep_fut, if next_wait_in.is_some() => WakeReason::WaitTimer,
                () = &mut sleep_fut => WakeReason::WallClock,
                () = cancel_token.cancelled() => WakeReason::Cancel,
                maybe_req = resume_rx.recv(), if !resume_rx_closed => match maybe_req {
                    Some(req) => WakeReason::ResumeSignalled(req),
                    None => WakeReason::ResumeChannelClosed,
                },
            };

            let join_result = match wake {
                WakeReason::Joined(Some(r)) => r,
                WakeReason::Joined(None) => {
                    // join_set drained mid-iteration — loop back so
                    // Phase 0 / Phase 0b / Phase 1 / exit-condition
                    // observe the current heap state.
                    continue;
                },
                WakeReason::RetryTimer => {
                    // Timer fired — loop back so Phase 0 drains due
                    // retries into ready_queue.
                    continue;
                },
                WakeReason::WaitTimer => {
                    // Timer fired — loop back so Phase 0b drains due
                    // wait-wakes into completed + downstream edges.
                    continue;
                },
                WakeReason::ResumeSignalled(req) => {
                    // A `Resume` command targeted this LIVE execution (W-S2b).
                    // The row stayed `Running` (a signal wait was parked with a
                    // `timeout`, so the loop holds the lease on the timeout
                    // timer). The durable satisfy-CAS path cannot be used here —
                    // it acquires the lease this loop already holds (two-writers-
                    // one-row). Instead the live loop is the SOLE writer of its
                    // own row: self-arm each signal-`Waiting{next_attempt_at:
                    // None}` node for completion under the loop's OWN lease, then
                    // loop back so Phase-0b completes it through the main port.
                    //
                    // P1#1 ack-gating: the caller's control-queue ack is gated
                    // on the durable result we send on `req.ack`. The contract
                    // is exactly one `send` per request, and `Armed` is sent
                    // ONLY after the self-arm checkpoint lands `Ok` (strictly
                    // after the version advances). A dropped `ack` Sender (this
                    // loop exits before sending) resolves the caller's receiver
                    // to `Err` → `LoopGone` → Deferred — a free fail-safe.
                    //
                    // Targeted in W-S3a: `req.resume_target` selects which signal
                    // wait(s) to arm — `Some(target)` arms only the kind+identity
                    // match, `None` arms every signal wait (W-S2b behavior). The
                    // shared `arm_signal_waits_under_lease` runs under THIS loop's
                    // own lease (it is the sole writer of its own row), preserving
                    // the own-the-lease-before-RMW invariant.
                    //
                    // A LIVE (`Running`) execution's signal waits are the
                    // timeout-bearing ones: parked with `wait_wake == Timeout`
                    // and a future `next_attempt_at` deadline. (A signal-only
                    // wait, `next_attempt_at == None`, would have driven the row
                    // to `Paused` — case-a, satisfied by the durable CAS, not
                    // this channel.) `arm_wait_completion` re-stamps
                    // `next_attempt_at = now` (overriding any future timeout
                    // deadline so Phase-0b completes immediately) and flips
                    // `wait_wake = Completion` (a Resume completes, never times
                    // out). The stale future `(deadline, key)` heap entry pops
                    // later, finds the node non-`Waiting`, and is skipped — the
                    // race-safety the Phase-0b state re-read guarantees.
                    let now = self.clock.now();
                    let to_arm =
                        arm_signal_waits_under_lease(exec_state, req.resume_target.as_ref(), now);
                    if to_arm.is_empty() {
                        // Spurious, already-armed, or no-match wake — nothing to
                        // do. Ack as `NothingToArm` (the caller may ack the row; a
                        // duplicate or non-matching Resume is idempotent), then
                        // loop back; the exit-condition / timers re-evaluate.
                        let _ = req.ack.send(ResumeOutcome::NothingToArm);
                        tracing::debug!(
                            target = "engine::wait",
                            %execution_id,
                            "resume signalled but no matching signal-Waiting node to arm; ignoring"
                        );
                        continue;
                    }
                    // Bump the version once so the checkpoint CAS advances and
                    // any reader observes the arm. (`set_node_state`/direct field
                    // writes do not bump; mirror the satisfy path's single bump.)
                    exec_state.version += 1;
                    exec_state.updated_at = now;
                    // The single checkpoint below carries ALL armed nodes but is
                    // attributed to `to_arm[0]` (a checkpoint takes one key).
                    // Log the full armed set so a checkpoint failure with N>1
                    // armed nodes is not misread as touching only the first key.
                    tracing::debug!(
                        target = "engine::wait",
                        %execution_id,
                        armed = ?to_arm,
                        armed_count = to_arm.len(),
                        "live-frontier resume: arming signal waits for Phase-0b completion"
                    );
                    // Durably commit the arm under the loop's OWN lease before
                    // Phase-0b acts on it. On checkpoint failure (incl.
                    // FencedOut / CasConflict — the loop lost its lease
                    // mid-iteration), ack `ArmFailed` BEFORE aborting so the
                    // caller defers the Resume; NEVER `Armed` on a failed arm.
                    if let Err(e) = self
                        .checkpoint_node(
                            scope,
                            execution_id,
                            // No single node owns this multi-node arm; reuse the
                            // first armed node as the checkpoint's attribution key.
                            to_arm[0].clone(),
                            outputs,
                            exec_state,
                            repo_version,
                            fencing,
                            vec![],
                        )
                        .await
                    {
                        let _ = req.ack.send(ResumeOutcome::ArmFailed);
                        cancel_token.cancel();
                        return Some((to_arm[0].clone(), e.to_string()));
                    }
                    // The arm is durable: ack `Armed` (the caller may now ack
                    // the control-queue row), strictly after the version
                    // advanced and the checkpoint landed `Ok`.
                    let _ = req.ack.send(ResumeOutcome::Armed {
                        count: to_arm.len(),
                    });
                    // Purge any stale future heap entry for the re-armed nodes
                    // (e.g. a signal+timeout wait's original timeout deadline)
                    // and replace it with a now-due entry, so Phase-0b completes
                    // immediately and the loop does not idle until the old
                    // deadline. `BinaryHeap` has no keyed remove, so rebuild it
                    // without the re-armed keys (the heap is small — one entry
                    // per parked node). Correctness does not depend on this
                    // (the stale entry would pop later and be skipped via the
                    // state re-read); it removes a spurious wait until the old
                    // deadline.
                    let armed: HashSet<&NodeKey> = to_arm.iter().collect();
                    let retained: Vec<_> = std::mem::take(&mut wait_heap)
                        .into_iter()
                        .filter(|Reverse((_, key))| !armed.contains(key))
                        .collect();
                    wait_heap.extend(retained);
                    for node_key in to_arm {
                        wait_heap.push(Reverse((now, node_key.clone())));
                        tracing::info!(
                            target = "engine::wait",
                            %execution_id,
                            %node_key,
                            "live-frontier resume: armed signal wait for Phase-0b completion"
                        );
                    }
                    // Loop back so Phase-0b drains the armed waits → main port.
                    continue;
                },
                WakeReason::ResumeChannelClosed => {
                    // Every `resume_tx` Sender has been dropped (the published
                    // `RunningEntry` was removed / never published, as on the
                    // lease-less replay path). No further Resume can arrive on
                    // this channel. Set the guard flag so the `recv()` arm is
                    // permanently disabled and the select! does not busy-spin on
                    // the `Ready(None)` that a closed channel returns every poll.
                    // Loop back so the other arms keep driving the frontier.
                    // No `ack` to honor — `recv()` returned `None`, not a request.
                    resume_rx_closed = true;
                    tracing::trace!(
                        target = "engine::wait",
                        %execution_id,
                        "resume channel closed; no live Resume producer remains — arm disarmed"
                    );
                    continue;
                },
                WakeReason::WallClock => {
                    cancel_token.cancel();
                    join_set.abort_all();
                    while join_set.join_next_with_id().await.is_some() {}
                    task_nodes.clear();
                    drain_pending_to_cancelled(
                        &mut retry_heap,
                        &mut wait_heap,
                        &mut ready_queue,
                        exec_state,
                        execution_id,
                    );
                    return Some((
                        node_key!("_timeout"),
                        "execution budget exceeded: max_duration".to_string(),
                    ));
                },
                WakeReason::Cancel => {
                    join_set.abort_all();
                    while join_set.join_next_with_id().await.is_some() {}
                    task_nodes.clear();
                    drain_pending_to_cancelled(
                        &mut retry_heap,
                        &mut wait_heap,
                        &mut ready_queue,
                        exec_state,
                        execution_id,
                    );
                    break;
                },
            };

            // Phase 3: Process the completed task
            match join_result {
                Ok((task_id, (node_key, Ok(action_result)))) => {
                    task_nodes.remove(&task_id);

                    // Park path: action returned `ActionResult::Wait`.
                    //
                    // The `partial_output` was already written into `outputs`
                    // by `extract_primary_output` in the dispatch future, so
                    // `checkpoint_node` will commit it alongside the `Waiting`
                    // state in one atomic write. Downstream edges are NOT
                    // activated here — they remain gated until the wait
                    // condition is satisfied.
                    //
                    // Conditions supported by this path:
                    //   Timer: `Until` / `Duration` (timeout:None) — `wake_at` is
                    //     the condition's instant, `wait_wake = Completion`; pushed
                    //     onto `wait_heap`; Phase-0b drains to `Completed`.
                    //   Signal (timeout:None): `Webhook` / `Approval` / `Execution` —
                    //     `wake_at = None`, `wait_wake = None`, no heap entry;
                    //     execution parks at `Paused` until a `Resume` command's
                    //     durable satisfy-CAS arms it for Phase-0b completion (case-a).
                    //   Signal (timeout:Some(dur)): `Webhook` / `Approval` /
                    //     `Execution` with a deadline — `wake_at = now + dur`,
                    //     `wait_wake = Timeout`; pushed onto `wait_heap`; the row
                    //     stays `Running` (a live loop on the timeout timer). A
                    //     `Resume` reaches it through the live-frontier resume channel
                    //     (W-S2b), NOT the Paused satisfy-CAS. If the timer fires
                    //     first, Phase-0b FAILS the node (`WaitTimedOut`).
                    //
                    // Still rejected:
                    //   Timer (`Until` / `Duration`) WITH `timeout:Some(..)`: two
                    //     competing timers is ambiguous; silently honouring one and
                    //     discarding the other is a correctness bug (W-S1 P2). A
                    //     timeout on a timer wait stays an explicit error.
                    if let ActionResult::Wait {
                        ref condition,
                        timeout,
                        ..
                    } = action_result
                    {
                        let now = self.clock.now();
                        // Compute the (wake_at, wait_wake) pair. `wait_wake` records
                        // how a timer wake is to be read when it fires: `Completion`
                        // for a timer-driven wait, `Timeout` for a signal wait whose
                        // declared `timeout` is the wake. A signal-only park carries
                        // neither (satisfied by an explicit Resume, not a timer); its
                        // node stays `Waiting{None}` and `determine_final_status`
                        // priority-4a recognises that as `Paused`, not a frontier bug.
                        let wake_plan: (Option<DateTime<Utc>>, Option<WaitWake>) = match condition {
                            WaitCondition::Until { .. } | WaitCondition::Duration { .. } => {
                                // Timer wait. An explicit `timeout` on a timer is two
                                // competing deadlines — reject (W-S1 P2 invariant).
                                if timeout.is_some() {
                                    let condition_kind = match condition {
                                        WaitCondition::Until { .. } => {
                                            "Until with explicit timeout"
                                        },
                                        _ => "Duration with explicit timeout",
                                    };
                                    let engine_err = EngineError::Runtime(
                                        crate::runtime::error::RuntimeError::WaitConditionNotSupported {
                                            condition_kind: condition_kind.to_owned(),
                                        },
                                    );
                                    tracing::error!(
                                        target = "engine::wait",
                                        %execution_id,
                                        %node_key,
                                        condition_kind,
                                        error = %engine_err,
                                        "explicit timeout on a TIMER WaitCondition is ambiguous \
                                         (two competing deadlines); marking node Failed"
                                    );
                                    mark_node_failed(exec_state, node_key.clone(), &engine_err);
                                    cancel_token.cancel();
                                    return Some((node_key.clone(), engine_err.to_string()));
                                }
                                // FAIL CLOSED on an unrepresentable / overflowing timer
                                // Duration. Mapping the error to `None` would silently
                                // turn a TIMER wait into a signal-driven indefinite park
                                // that a generic `Resume` could satisfy — wrong semantics.
                                let fail_unschedulable =
                                    |exec_state: &mut ExecutionState, reason: String| {
                                        let engine_err = EngineError::Runtime(
                                            crate::runtime::error::RuntimeError::WaitConditionNotSupported {
                                                condition_kind: reason,
                                            },
                                        );
                                        tracing::error!(
                                            target = "engine::wait",
                                            %execution_id,
                                            %node_key,
                                            error = %engine_err,
                                            "timer WaitCondition cannot be scheduled; marking \
                                             node Failed (fail-closed)"
                                        );
                                        mark_node_failed(exec_state, node_key.clone(), &engine_err);
                                        cancel_token.cancel();
                                        engine_err.to_string()
                                    };
                                let when = match condition {
                                    WaitCondition::Until { datetime } => *datetime,
                                    WaitCondition::Duration { duration } => {
                                        let Ok(chrono_dur) = chrono::Duration::from_std(*duration)
                                        else {
                                            let msg = fail_unschedulable(
                                                exec_state,
                                                format!(
                                                    "Duration wait not representable: {duration:?}"
                                                ),
                                            );
                                            return Some((node_key.clone(), msg));
                                        };
                                        let Some(when) = now.checked_add_signed(chrono_dur) else {
                                            let msg = fail_unschedulable(
                                                exec_state,
                                                "Duration wait overflows the scheduler timestamp"
                                                    .to_owned(),
                                            );
                                            return Some((node_key.clone(), msg));
                                        };
                                        when
                                    },
                                    // Unreachable: outer arm is Until|Duration only.
                                    _ => unreachable!("timer arm matched a non-timer condition"),
                                };
                                (Some(when), Some(WaitWake::Completion))
                            },
                            // Signal-driven conditions.
                            WaitCondition::Webhook { .. }
                            | WaitCondition::Approval { .. }
                            | WaitCondition::Execution { .. } => {
                                match timeout {
                                    // Signal + timeout (W-S2b): park with a timeout
                                    // timer, `wait_wake = Timeout`. The row stays
                                    // `Running`; Phase-0b FAILS the node if the timer
                                    // fires before a Resume arrives.
                                    Some(dur) => {
                                        let Ok(chrono_dur) = chrono::Duration::from_std(dur) else {
                                            let engine_err = EngineError::Runtime(
                                                crate::runtime::error::RuntimeError::WaitConditionNotSupported {
                                                    condition_kind: format!(
                                                        "signal wait timeout not representable: \
                                                         {dur:?}"
                                                    ),
                                                },
                                            );
                                            mark_node_failed(
                                                exec_state,
                                                node_key.clone(),
                                                &engine_err,
                                            );
                                            cancel_token.cancel();
                                            return Some((
                                                node_key.clone(),
                                                engine_err.to_string(),
                                            ));
                                        };
                                        let Some(deadline) = now.checked_add_signed(chrono_dur)
                                        else {
                                            let engine_err = EngineError::Runtime(
                                                crate::runtime::error::RuntimeError::WaitConditionNotSupported {
                                                    condition_kind:
                                                        "signal wait timeout overflows the \
                                                         scheduler timestamp"
                                                            .to_owned(),
                                                },
                                            );
                                            mark_node_failed(
                                                exec_state,
                                                node_key.clone(),
                                                &engine_err,
                                            );
                                            cancel_token.cancel();
                                            return Some((
                                                node_key.clone(),
                                                engine_err.to_string(),
                                            ));
                                        };
                                        (Some(deadline), Some(WaitWake::Timeout))
                                    },
                                    // Signal only (case-a): no timer, parks at Paused.
                                    None => (None, None),
                                }
                            },
                            _ => {
                                // Unknown WaitCondition variant — FAIL CLOSED.
                                // Parking with `wake_at = None` would let a generic
                                // execution-level Resume satisfy a wait whose semantics
                                // this engine cannot classify (a signal vs timer vs
                                // something else). Until a variant is explicitly added
                                // to the signal/timer arms above, reject it on the same
                                // `WaitConditionNotSupported` path rather than parking it.
                                let runtime_err =
                                    crate::runtime::error::RuntimeError::WaitConditionNotSupported {
                                        condition_kind: "unrecognised WaitCondition variant"
                                            .to_owned(),
                                    };
                                let engine_err = EngineError::Runtime(runtime_err);
                                tracing::error!(
                                    target = "engine::wait",
                                    %execution_id,
                                    %node_key,
                                    error = %engine_err,
                                    "unrecognised WaitCondition variant; marking node Failed \
                                     (fail-closed — a Resume must not satisfy an unclassified wait)"
                                );
                                mark_node_failed(exec_state, node_key.clone(), &engine_err);
                                cancel_token.cancel();
                                return Some((node_key.clone(), engine_err.to_string()));
                            },
                        };
                        let (wake_at, wait_wake) = wake_plan;

                        // Capture the resume-IDENTITY of a signal wait so a later
                        // targeted Resume can match it (W-S3a). A signal condition
                        // (Webhook / Approval / Execution) persists the minimum
                        // identity needed for targeting — the callback_id /
                        // approver / execution_id — never the Approval `message` or
                        // any inbound payload (that is W-S4). A timer condition
                        // (Until / Duration) carries no identity. This classifies
                        // independently of the timer: `park_node` enforces that
                        // `wait_signal.is_some()` iff the wait is signal-driven.
                        let wait_signal: Option<WaitSignal> = match condition {
                            WaitCondition::Webhook { callback_id } => Some(WaitSignal::Webhook {
                                callback_id: callback_id.clone(),
                            }),
                            WaitCondition::Approval { approver, .. } => {
                                Some(WaitSignal::Approval {
                                    approver: approver.clone(),
                                })
                            },
                            WaitCondition::Execution { execution_id } => {
                                Some(WaitSignal::Execution {
                                    execution_id: *execution_id,
                                })
                            },
                            WaitCondition::Until { .. } | WaitCondition::Duration { .. } => None,
                            // Any unclassified variant fails closed above (the
                            // wake_plan `_` arm marks the node Failed and returns),
                            // so this arm is unreachable at park time. Persisting
                            // `None` here would be wrong (it would not match the
                            // signal classification of an unknown variant), but the
                            // node never reaches `park_node` in that case.
                            _ => None,
                        };

                        // Budget enforcement for the partial output committed at park
                        // time. The normal success path increments `total_output_bytes`
                        // AFTER the node completes; Phase 1's `check_budget` catches
                        // the violation before the next downstream node is dispatched.
                        // The park path's `continue` skips Phase 3 output accounting
                        // entirely — so we must enforce the budget HERE, before park,
                        // or a large `partial_output` bypasses the limit silently.
                        //
                        // If the partial output is over-budget, fail the node (do NOT
                        // park) so the downstream child is never dispatched.
                        let partial_output_bytes: u64 = outputs
                            .get(&node_key)
                            .and_then(|v| serde_json::to_string(v.value()).ok())
                            .map_or(0, |s| s.len() as u64);
                        if partial_output_bytes > 0 {
                            let new_total = total_output_bytes
                                .fetch_add(partial_output_bytes, Ordering::Relaxed)
                                + partial_output_bytes;
                            if let Some(max_bytes) = budget.max_output_bytes
                                && new_total > max_bytes
                            {
                                let budget_err = "execution budget exceeded: max_output_bytes \
                                                  (partial_output at park time)";
                                tracing::error!(
                                    target = "engine::wait",
                                    %execution_id,
                                    %node_key,
                                    partial_output_bytes,
                                    new_total,
                                    max_bytes,
                                    "partial_output at park exceeds max_output_bytes budget; \
                                     failing node instead of parking"
                                );
                                // Restore the counter — the node is being failed, not
                                // committed, so its bytes should not count against the
                                // budget for the remaining nodes.
                                total_output_bytes
                                    .fetch_sub(partial_output_bytes, Ordering::Relaxed);
                                mark_node_failed(
                                    exec_state,
                                    node_key.clone(),
                                    &EngineError::BudgetExceeded(budget_err.to_owned()),
                                );
                                cancel_token.cancel();
                                return Some((node_key.clone(), budget_err.to_owned()));
                            }
                        }

                        // W-S3c: mint a resume token for signal-park conditions
                        // that expect an external caller (Webhook, Approval).
                        // `Execution` waits are internal and must NOT mint.
                        // `#[non_exhaustive]` — unknown future variants get no
                        // token (safe default: caller must use a different path).
                        //
                        // Must be done BEFORE `park_node` moves `wait_signal`
                        // into the execution state.  The `SecretString` bearer
                        // is dropped at the end of the `Ok(())` arm — W-S3d
                        // will route it to the waiting caller when that slice
                        // lands; for now it is deliberately unused.
                        let token_now = self.clock.now();
                        let park_token_result: Option<
                            Result<(ResumeTokenRow, SecretString), EngineError>,
                        > = match &wait_signal {
                            Some(WaitSignal::Webhook { callback_id }) => Some(mint_park_token(
                                scope,
                                execution_id,
                                &node_key,
                                ResumeTokenWaitKind::Webhook,
                                callback_id.clone(),
                                wake_at,
                                token_now,
                            )),
                            Some(WaitSignal::Approval { approver }) => Some(mint_park_token(
                                scope,
                                execution_id,
                                &node_key,
                                ResumeTokenWaitKind::Approval,
                                approver.clone(),
                                wake_at,
                                token_now,
                            )),
                            // Execution waits are internal — no external bearer.
                            Some(WaitSignal::Execution { .. } | _) | None => None,
                        };

                        match exec_state.park_node(
                            node_key.clone(),
                            wake_at,
                            wait_wake,
                            wait_signal,
                        ) {
                            Ok(()) => {
                                // Signal park (no timer) that leaves NO other active
                                // frontier work fully suspends the execution: persist
                                // `Paused` atomically in this park checkpoint batch
                                // (`checkpoint_node` serialises `exec_state`, status
                                // included). Otherwise the row would sit durably
                                // `Running` + `Waiting{next_attempt_at: None}` until the
                                // frontier exit's `persist_final_state` writes `Paused`;
                                // a crash in that window is unrecoverable because
                                // `dispatch_start`/`dispatch_resume` short-circuit on
                                // `Running`. The exit's `transition_status(Paused)` is a
                                // no-op once we set it here (`Paused→Paused` is rejected
                                // and ignored). The general crashed-`Running` recovery
                                // gap (e.g. a sibling completing last) is tracked
                                // separately.
                                //
                                // A signal + timeout park has `wake_at = Some(_)`, so it
                                // is NOT covered here: the row stays `Running` with a
                                // live loop on the timeout timer (W-S2b). Its Resume
                                // arrives through the live-frontier resume channel.
                                if wake_at.is_none()
                                    && join_set.is_empty()
                                    && ready_queue.is_empty()
                                    && retry_heap.is_empty()
                                    && wait_heap.is_empty()
                                {
                                    let _ = exec_state.transition_status(ExecutionStatus::Paused);
                                }

                                // Resolve the minted token (if any) or propagate
                                // the mint error as a checkpoint failure.
                                let (park_token_row, _plaintext_bearer) = match park_token_result {
                                    Some(Ok(pair)) => (Some(pair.0), Some(pair.1)),
                                    Some(Err(mint_err)) => {
                                        cancel_token.cancel();
                                        return Some((node_key.clone(), mint_err.to_string()));
                                    },
                                    None => (None, None),
                                };
                                // `_plaintext_bearer` is dropped here (W-S3c):
                                // the SecretString zeroizes on drop.  W-S3d
                                // will route it to the API caller when that
                                // slice ships.

                                let resume_tokens: Vec<ResumeTokenRow> =
                                    park_token_row.into_iter().collect();

                                // Durably commit the `Waiting` state and the
                                // already-staged `partial_output` before any
                                // observer sees the node is parked. On
                                // checkpoint failure, abort: the task slot
                                // was already removed above and cannot be
                                // re-dispatched, so abort is the honest path.
                                if let Err(e) = self
                                    .checkpoint_node(
                                        scope,
                                        execution_id,
                                        node_key.clone(),
                                        outputs,
                                        exec_state,
                                        repo_version,
                                        fencing,
                                        resume_tokens,
                                    )
                                    .await
                                {
                                    cancel_token.cancel();
                                    return Some((node_key.clone(), e.to_string()));
                                }
                                // Push onto the wait_heap whenever there is a timer
                                // (`wake_at == Some`): a timer-driven completion wait
                                // (`Completion`) OR a signal+timeout wait
                                // (`Timeout`). Signal-only conditions (`wake_at ==
                                // None`) are never pushed; their node stays `Waiting`
                                // until a Resume command's durable satisfy-CAS arms it.
                                if let Some(when) = wake_at {
                                    wait_heap.push(Reverse((when, node_key.clone())));
                                }
                                self.emit_event(ExecutionEvent::NodeParked {
                                    execution_id,
                                    node_key: node_key.clone(),
                                    wake_at,
                                });
                                tracing::info!(
                                    target = "engine::wait",
                                    %execution_id,
                                    %node_key,
                                    ?wake_at,
                                    "node parked for external wait condition"
                                );
                                // Skip the normal completion path — downstream
                                // gate holds until the wait is satisfied.
                                continue;
                            },
                            Err(park_err) => {
                                // park_node rejected the transition; treat as a
                                // system failure rather than silently dropping
                                // the node (the task slot was already removed,
                                // so the engine cannot re-dispatch it). Surface
                                // the error through the frontier abort path.
                                tracing::error!(
                                    target = "engine::wait",
                                    %execution_id,
                                    %node_key,
                                    error = %park_err,
                                    "park_node rejected Running→Waiting; aborting frontier"
                                );
                                cancel_token.cancel();
                                return Some((node_key.clone(), park_err.to_string()));
                            },
                        }
                    }

                    mark_node_completed(exec_state, node_key.clone());

                    // Track output size for budget enforcement.
                    let mut output_bytes: u64 = 0;
                    if let Some(output) = outputs.get(&node_key) {
                        output_bytes =
                            serde_json::to_string(output.value()).map_or(0, |s| s.len() as u64);
                        total_output_bytes.fetch_add(output_bytes, Ordering::Relaxed);
                    }
                    // Capture an explicit-termination signal BEFORE the
                    // checkpoint so that the same CAS-write durably
                    // persists `terminated_by` (termination metadata; ROADMAP
                    // §M0.3). The companion `cancel_token.cancel()` is
                    // deferred until AFTER `Ok` from `checkpoint_node` so
                    // we tear down sibling branches only on a durable
                    // decision.
                    let terminate_was_first_set =
                        if let ActionResult::Terminate { reason } = &action_result {
                            let exec_reason = map_termination_reason(node_key.clone(), reason);
                            let was_first =
                                exec_state.set_terminated_by(node_key.clone(), exec_reason.clone());
                            tracing::info!(
                                target = "engine::frontier",
                                execution_id = %execution_id,
                                node_key = %node_key,
                                ?exec_reason,
                                was_first,
                                "explicit_termination_signal"
                            );
                            was_first
                        } else {
                            false
                        };

                    // Persist node output + execution state, then record the
                    // idempotency key, before any external observer learns the
                    // node is done. This guarantees durability precedes
                    // visibility (, #297). Checkpoint failure aborts the
                    // node's progression so observers never see an
                    // unpersisted transition and the frontier never advances
                    // on an undurable decision.
                    if let Err(e) = self
                        .checkpoint_node(
                            scope,
                            execution_id,
                            node_key.clone(),
                            outputs,
                            exec_state,
                            repo_version,
                            fencing,
                            vec![],
                        )
                        .await
                    {
                        // Durability recovery (ROADMAP §M0.3 review M1):
                        // if the action returned `Terminate` and we
                        // recorded `terminated_by` in-memory above but
                        // `checkpoint_node` failed (CAS conflict /
                        // storage err), the signal never reached disk.
                        // Drop it so `determine_final_status` does not
                        // report a durable-looking `termination_reason`
                        // on the event stream while the audit row stays
                        // `None`. The engine still surfaces the failure
                        // via `failed_node` (system-driven `Failed`),
                        // which is the honest outcome.
                        if terminate_was_first_set {
                            tracing::warn!(
                                target = "engine::frontier",
                                %execution_id,
                                %node_key,
                                checkpoint_error = %e,
                                "explicit_termination_signal lost on \
                                 checkpoint failure; clearing in-memory \
                                 terminated_by to avoid event-vs-audit \
                                 divergence"
                            );
                            exec_state.clear_terminated_by();
                        }
                        cancel_token.cancel();
                        return Some((node_key.clone(), e.to_string()));
                    }
                    self.record_idempotency(scope, exec_state, execution_id, node_key.clone())
                        .await;

                    // Persist the full ActionResult alongside the raw
                    // output so that idempotent replay can reconstruct
                    // the exact routing semantics (issue #299).
                    //
                    // T4 — `attempt_count + 1` is the
                    self.record_node_result(scope, execution_id, node_key.clone(), &action_result)
                        .await;

                    // T4 — push the success attempt
                    // record AFTER record_idempotency / record_node_result
                    // so those helpers see the just-finished attempt's
                    // key (push advances the next-dispatch key). The
                    // attempt's idempotency key is derived inside
                    // `record_node_attempt` from the new attempt
                    // number, so engine code cannot drift the audit
                    // row out of step with the persisted key.
                    let success_payload = outputs
                        .get(&node_key)
                        .map_or_else(|| serde_json::Value::Null, |v| v.value().clone());
                    if let Err(e) = exec_state.record_node_attempt(
                        node_key.clone(),
                        AttemptOutcome::Success {
                            output: ExecutionOutput::inline(success_payload),
                            output_bytes,
                        },
                    ) {
                        tracing::warn!(
                            target = "engine::frontier",
                            %execution_id,
                            %node_key,
                            error = %e,
                            "record_node_attempt(success) failed; continuing without \
                             attempt history (idempotency key may collide on resume)"
                        );
                    }

                    self.emit_event(ExecutionEvent::NodeCompleted {
                        execution_id,
                        node_key: node_key.clone(),
                        elapsed: started.elapsed(),
                    });

                    // Evaluate outgoing edges and update frontier
                    process_outgoing_edges(
                        node_key.clone(),
                        Some(&action_result),
                        None, // not failed
                        graph,
                        &mut activated_edges,
                        &mut resolved_edges,
                        &required_count,
                        &mut ready_queue,
                        exec_state,
                    );

                    // ROADMAP §M0.3: signal `cancel_token` ONLY after the
                    // termination signal is durable AND we've gated the
                    // local downstream edges through `process_outgoing_edges`
                    // (which already treats `Terminate` like `Skip`).
                    // Siblings still in flight observe the cancel and tear
                    // down; the executor's `select!` arm reconciles their
                    // `Cancelled` state on the next loop iteration.
                    if terminate_was_first_set {
                        tracing::trace!(
                            target = "engine::frontier",
                            execution_id = %execution_id,
                            node_key = %node_key,
                            "cancel_token signalled after durable termination"
                        );
                        cancel_token.cancel();
                    }
                },
                Ok((task_id, (node_key, Err(ref err)))) => {
                    task_nodes.remove(&task_id);

                    // Cooperative cancel: the action returned after awaiting the same
                    // `CancellationToken` that control-queue `Cancel` / external cancel trips.
                    // If we route this through `mark_node_failed`, `run_frontier` returns
                    // `Some(failed_node)` and [`determine_final_status`] picks **Failed** over
                    // `cancel_token.is_cancelled` — wrong for A3 / lease_takeover T4.
                    // Mirror [`WakeReason::Cancel`]: mark the node `Cancelled`, drain in-flight
                    // bookkeeping, and exit without a synthetic `failed_node`.
                    //
                    // **Match the runtime-wrapped variant too.** `execute_action_with_node`
                    // returns `Err(e)` which the caller wraps as `EngineError::Runtime(e)`
                    // (see this file's `execute_action` future — `Err(e) => …
                    // Err(EngineError::Runtime(e))`). So an in-flight action that picks up
                    // cancel via the token surfaces here as
                    // `EngineError::Runtime(RuntimeError::ActionError(ActionError::Cancelled))`,
                    // **not** the bare `EngineError::Action(...)` variant. Missing that arm
                    // is what `lease_takeover` T4 catches.
                    if cancel_token.is_cancelled()
                        && matches!(
                            err,
                            EngineError::Cancelled
                                | EngineError::Action(ActionError::Cancelled)
                                | EngineError::Runtime(crate::runtime::RuntimeError::ActionError(
                                    ActionError::Cancelled,
                                ),)
                        )
                    {
                        tracing::debug!(
                            target = "engine::frontier",
                            %execution_id,
                            %node_key,
                            "node returned cooperative cancel under active cancel token; \
                             tearing down frontier (not Failed)"
                        );
                        if exec_state
                            .transition_node(node_key.clone(), NodeState::Cancelled)
                            .is_ok()
                        {
                            join_set.abort_all();
                            while join_set.join_next_with_id().await.is_some() {}
                            task_nodes.clear();
                            drain_pending_to_cancelled(
                                &mut retry_heap,
                                &mut wait_heap,
                                &mut ready_queue,
                                exec_state,
                                execution_id,
                            );
                            break;
                        }
                        tracing::warn!(
                            target = "engine::frontier",
                            %execution_id,
                            %node_key,
                            "transition_node(Cancelled) failed after cooperative cancel; \
                             continuing through normal failure path"
                        );
                    }

                    // Node failed at runtime. Ordering (, #297 PR
                    // review by Copilot — route stages OnError payload
                    // that checkpoint must capture so resume can read
                    // it from `load_all_outputs`):
                    //   1. `mark_node_failed`      — in-memory Failed
                    //   2. `record_node_attempt(failure)` — push the attempt to history so
                    // `idempotency_key_for_node` differentiates future retries.
                    // 3. **retry decision** — T4. If the per-node /
                    //      workflow-default `RetryConfig` has budget AND the global
                    //      `ExecutionBudget.max_total_retries` cap allows another attempt, promote
                    //      `Failed → WaitingRetry`, stamp `next_attempt_at`, increment
                    //      `total_retries`, push the node onto `retry_heap`, checkpoint, emit
                    //      `NodeRetryScheduled`, and skip the finalize path. The retry loop (Phase
                    //      0 next iteration) will re-dispatch when the timer fires —
                    //      cancel/terminate/budget guards run BEFORE the re-dispatch so a cancelled
                    //      execution never silently re-runs a node.
                    //   4. `apply_failure_recovery` — IgnoreErrors-only override of state + null
                    //      output (in-memory). Only on the no-retry path.
                    //   5. `route_failure_edges`    — evaluate outgoing edges; may write `{error,
                    //      node_id}` payload into `outputs[node_key]` for OnError input; may
                    //      enqueue successors into `ready_queue`. Only on the no-retry path.
                    //   6. `checkpoint_node`        — durable commit of state + outputs (abort on
                    //      Err; the discarded `ready_queue` mutations never surface).
                    //   7. `emit_event`             — observers (`NodeFailed` only on the no-retry
                    //      path; `NodeRetryScheduled` on the retry path), strictly after persist.
                    //
                    // Successors in `ready_queue` do NOT dispatch until
                    // Phase 1 of the next loop iteration; that runs
                    // after checkpoint. Nothing external observes a
                    // state the store has not committed.
                    mark_node_failed(exec_state, node_key.clone(), err);
                    let err_str = err.to_string();

                    // Push the failure attempt record so idempotency
                    // key, retry-decision, and post-mortem audit all
                    // see the same attempt history.
                    // T4: if recording fails (programming
                    // error: unknown node), force `Finalize` rather than
                    // letting `compute_retry_decision` see a stale
                    // `attempts.len()` — that path could bypass
                    // `max_attempts` (loop forever when no global cap
                    // is set) or collide idempotency keys on resume.
                    let failure_attempt_recorded = match exec_state.record_node_attempt(
                        node_key.clone(),
                        AttemptOutcome::Failure {
                            error: err_str.clone(),
                        },
                    ) {
                        Ok(_) => true,
                        Err(e) => {
                            tracing::warn!(
                                target = "engine::frontier",
                                %execution_id,
                                %node_key,
                                error = %e,
                                "record_node_attempt(failure) failed; forcing finalize \
                                 so stale attempt history cannot bypass max_attempts or \
                                 collide idempotency keys"
                            );
                            false
                        },
                    };

                    // T4 — retry decision. Skipped when
                    // attempt history could not be recorded.
                    let decision = if failure_attempt_recorded {
                        let retry_policy_resolved = node_map
                            .get(&node_key)
                            .and_then(|nd| {
                                effective_retry_policy(nd, workflow_retry_policy.as_ref())
                            })
                            .cloned();
                        compute_retry_decision(
                            &node_key,
                            exec_state,
                            retry_policy_resolved.as_ref(),
                            error_is_terminal(err),
                        )
                    } else {
                        RetryDecision::Finalize
                    };

                    if let RetryDecision::Retry { delay } = decision {
                        let attempt_number = exec_state
                            .node_states
                            .get(&node_key)
                            .map_or(1, |ns| ns.attempt_count() as u32);
                        let next_at =
                            next_retry_at(execution_id, &node_key, delay, self.clock.now());
                        match exec_state.schedule_node_retry(node_key.clone(), next_at) {
                            Ok(()) => {
                                // Persist the WaitingRetry transition,
                                // `next_attempt_at`, and the global
                                // counter bump in one CAS write.
                                if let Err(e) = self
                                    .checkpoint_node(
                                        scope,
                                        execution_id,
                                        node_key.clone(),
                                        outputs,
                                        exec_state,
                                        repo_version,
                                        fencing,
                                        vec![],
                                    )
                                    .await
                                {
                                    cancel_token.cancel();
                                    return Some((node_key.clone(), e.to_string()));
                                }
                                retry_heap.push(Reverse((next_at, node_key.clone())));
                                tracing::info!(
                                    target = "engine::retry",
                                    %execution_id,
                                    %node_key,
                                    attempt = attempt_number,
                                    delay_ms = delay.as_millis() as u64,
                                    next_attempt_at = %next_at,
                                    total_retries = exec_state.total_retries,
                                    "retry scheduled (Layer 2 / NodeDefinition.retry_policy)"
                                );
                                self.emit_event(ExecutionEvent::NodeRetryScheduled {
                                    execution_id,
                                    node_key: node_key.clone(),
                                    attempt: attempt_number,
                                    next_attempt_at: next_at,
                                    last_error: err_str.clone(),
                                });
                                // Done with this iteration — skip the
                                // finalize path entirely.
                                continue;
                            },
                            Err(schedule_err) => {
                                // `schedule_node_retry` rejected the
                                // promotion (e.g. node moved out of
                                // Failed mid-decision). Fall through
                                // to the finalize path so the failure
                                // surfaces honestly.
                                tracing::warn!(
                                    target = "engine::retry",
                                    %execution_id,
                                    %node_key,
                                    error = %schedule_err,
                                    "schedule_node_retry rejected; finalising failure"
                                );
                            },
                        }
                    }

                    // ── Finalize path (no retry / retry exhausted) ──
                    let outcome = classify_failure(error_strategy);
                    if let Err(e) =
                        apply_failure_recovery(outcome, node_key.clone(), exec_state, outputs)
                    {
                        cancel_token.cancel();
                        return Some((node_key.clone(), e.to_string()));
                    }

                    let abort = route_failure_edges(
                        outcome,
                        node_key.clone(),
                        &err_str,
                        error_strategy,
                        graph,
                        outputs,
                        &mut activated_edges,
                        &mut resolved_edges,
                        &required_count,
                        &mut ready_queue,
                        exec_state,
                    );

                    if let Err(e) = self
                        .checkpoint_node(
                            scope,
                            execution_id,
                            node_key.clone(),
                            outputs,
                            exec_state,
                            repo_version,
                            fencing,
                            vec![],
                        )
                        .await
                    {
                        cancel_token.cancel();
                        return Some((node_key.clone(), e.to_string()));
                    }

                    if outcome == FailureOutcome::Fail {
                        self.emit_event(ExecutionEvent::NodeFailed {
                            execution_id,
                            node_key: node_key.clone(),
                            details: NodeFailedDetails {
                                error_code: "ENGINE:NODE_FAILED".to_owned(),
                                display_message: err_str.clone(),
                            },
                        });
                    }

                    if let Some(err_msg) = abort {
                        cancel_token.cancel();
                        return Some((node_key.clone(), err_msg));
                    }
                },
                Err(join_err) => {
                    // Recover the real NodeKey via the task-id side
                    // map; falling back to a synthetic key would
                    // report a phantom node and lose the identity of
                    // the actually-panicked task (issue #301).
                    let task_id = join_err.id();
                    let panicked_node = task_nodes.remove(&task_id);
                    let err_msg = join_err.to_string();
                    tracing::error!(
                        ?task_id,
                        ?panicked_node,
                        error = %err_msg,
                        "node task panicked"
                    );

                    if let Some(node_key) = panicked_node {
                        self.handle_panicked_node(
                            scope,
                            execution_id,
                            node_key.clone(),
                            &err_msg,
                            outputs,
                            exec_state,
                            repo_version,
                            fencing,
                        )
                        .await;
                        cancel_token.cancel();
                        return Some((node_key, err_msg));
                    }

                    // No matching task id — this should be unreachable
                    // as we insert every spawn into `task_nodes`, but
                    // fall through defensively rather than inventing
                    // a node identity.
                    cancel_token.cancel();
                    return Some((
                        node_key!("_panicked"),
                        format!("panicked task with unknown id: {err_msg}"),
                    ));
                },
            }
        }

        None
    }

    /// Spawn a single node into the JoinSet.
    ///
    /// Returns `true` if the node was spawned, `false` if it failed during setup
    /// (e.g., param resolution error).
    #[expect(clippy::too_many_arguments)]
    fn spawn_node(
        &self,
        node_key: NodeKey,
        node_map: &HashMap<NodeKey, &nebula_workflow::NodeDefinition>,
        graph: &DependencyGraph,
        outputs: &Arc<DashMap<NodeKey, serde_json::Value>>,
        semaphore: &Arc<Semaphore>,
        cancel_token: &CancellationToken,
        exec_state: &mut ExecutionState,
        execution_id: ExecutionId,
        workflow_id: WorkflowId,
        input: &serde_json::Value,
        activated_edges: &HashMap<NodeKey, HashSet<NodeKey>>,
        join_set: &mut JoinSet<(
            NodeKey,
            Result<ActionResult<serde_json::Value>, EngineError>,
        )>,
        task_nodes: &mut HashMap<tokio::task::Id, NodeKey>,
    ) -> bool {
        let Some(node_def) = node_map.get(&node_key) else {
            // Unknown node — route through the setup-failure path so
            // the frontier loop records the error and checkpoints the
            // state (issues #300, #321).
            let _ = exec_state.mark_setup_failed(
                node_key.clone(),
                format!("node {node_key} is not in the workflow's node map"),
            );
            return false;
        };
        let action_key = node_def.action_key.as_str().to_owned();
        let interface_version = node_def.interface_version.clone();

        // Partition incoming connections into flow (to_port=None) and support (to_port=Some)
        let (node_input, support_inputs) = resolve_node_input_with_support(
            node_key.clone(),
            graph,
            outputs,
            input,
            activated_edges,
        );

        // Resolve node parameters (expressions, templates, references)
        let action_input =
            match self
                .resolver
                .resolve(&node_key, &node_def.parameters, &node_input, outputs)
            {
                Ok(Some(resolved_params)) => resolved_params,
                Ok(None) => node_input, // No parameters → use predecessor output
                Err(e) => {
                    // Parameter resolution failed. `mark_setup_failed`
                    // overrides the node state to Failed via
                    // `override_node_state` (Pending → Failed is not a
                    // valid forward transition) and bumps the parent
                    // version for CAS readers (issues #255, #300).
                    let _ = exec_state.mark_setup_failed(node_key.clone(), e.to_string());
                    return false;
                },
            };

        // Drive the node to Running via the typed state-machine
        // helper. `start_node_attempt` models the only legal
        // transition path (Pending → Ready → Running) and returns an
        // error for anything else — the engine does not retry, so
        // Failed is terminal at the node level. On error we do NOT
        // silently spawn the task on stale state — route through the
        // setup-failure path instead (issue #300).
        if let Err(err) = exec_state.start_node_attempt(node_key.clone()) {
            let _ = exec_state.mark_setup_failed(
                node_key.clone(),
                format!("cannot start node attempt: {err}"),
            );
            return false;
        }

        let runtime = self.runtime.clone();
        let cancel = cancel_token.clone();
        let sem = semaphore.clone();
        let outputs_ref = outputs.clone();

        // Build credential accessor with a **deny-by-default** per-action allowlist.
        //
        // Per `PRODUCT_CANON` / + audit : an action can only
        // acquire credential IDs explicitly declared for its `ActionKey` via
        // `WorkflowEngine::with_action_credentials`. If the node's action was
        // never declared — or was declared with an empty set — the accessor
        // refuses every `get`/`has` request with
        // `CredentialAccessError::AccessDenied`. No silent "allow all" fallback.
        let allowed_keys: HashSet<String> = self
            .action_credentials
            .get(&node_def.action_key)
            .cloned()
            .unwrap_or_default();
        let credentials: Arc<dyn CredentialAccessor> = if let Some(resolver_fn) =
            &self.credential_resolver
        {
            let resolver_fn = Arc::clone(resolver_fn);
            Arc::new(EngineCredentialAccessor::new(
                allowed_keys,
                move |id: &str| {
                    let resolver_fn = Arc::clone(&resolver_fn);
                    let credential_key_str = id.to_owned();
                    async move {
                        let snapshot = (resolver_fn)(&credential_key_str).await.map_err(|e| {
                            tracing::debug!(
                                credential_key = %credential_key_str,
                                error = %e,
                                "credential resolution failed"
                            );
                            match CredentialKey::new(&credential_key_str) {
                                Ok(key) => nebula_core::CoreError::credential_not_found(key),
                                Err(_) => nebula_core::CoreError::invalid_key(
                                    credential_key_str.clone(),
                                    "credential",
                                ),
                            }
                        })?;
                        Ok(Box::new(snapshot) as Box<dyn std::any::Any + Send + Sync>)
                    }
                },
                node_def.action_key.as_str().to_owned(),
            ))
        } else {
            default_credential_accessor()
        };

        // Build resource accessor: wrap the manager-backed global accessor in
        // a LayeredResourceAccessor (M6.1 — Phase 6). Phase 6 plugs in the
        // empty scoped map; Phase 7 (M6.2) swaps the inner scoped layer for
        // the per-branch DashMap implementation. Action call sites
        // (`ctx.acquire_resource_by_id`, `ctx.resource::<R>()`) consult the
        // layered accessor transparently — `scoped → global`, closest
        // ancestor wins.
        let resources: Arc<dyn ResourceAccessor> = if let Some(manager) = &self.resource_manager {
            let extra = self
                .execution_acquire_scopes
                .get(&execution_id)
                .map(|entry| entry.value().clone())
                .unwrap_or_default();
            let scope = nebula_core::scope::Scope {
                execution_id: Some(execution_id),
                workflow_id: Some(workflow_id),
                org_id: extra.org_id,
                workspace_id: extra.workspace_id,
                ..Default::default()
            };
            let slot_identities = self
                .resource_slot_identities_by_execution
                .get(&execution_id)
                .map(|entry| Arc::clone(entry.value()))
                .unwrap_or_else(|| Arc::new(HashMap::new()));
            let global: Arc<dyn ResourceAccessor> = Arc::new(
                EngineResourceAccessor::new(Arc::clone(manager), scope, cancel_token.clone())
                    .with_slot_identities_arc(slot_identities),
            );
            Arc::new(LayeredResourceAccessor::global_only(global))
        } else {
            default_resource_accessor()
        };

        // Only forward the refresh hook when a credential resolver is configured.
        // Without a resolver there are no credentials to refresh, so the hook
        // would fire unconditionally on every node — even actions that do not use
        // credentials at all.
        let credential_refresh = if self.credential_resolver.is_some() {
            self.credential_refresh.clone()
        } else {
            None
        };

        // Build rate limiter from node definition if configured.
        let rate_limiter = node_def.rate_limit.as_ref().and_then(|rl| {
            let refill_rate = rl.max_requests as f64 / rl.window_secs.max(1) as f64;
            nebula_resilience::rate_limiter::TokenBucket::new(rl.max_requests as usize, refill_rate)
                .ok()
                .map(Arc::new)
        });

        let handle = join_set.spawn(
            NodeTask {
                runtime,
                cancel,
                sem,
                outputs: outputs_ref,
                execution_id,
                node_key: node_key.clone(),
                workflow_id,
                action_key,
                node: Arc::new((*node_def).clone()),
                interface_version,
                input: action_input,
                support_inputs,
                credentials,
                resources,
                credential_refresh,
                rate_limiter,
            }
            .run(),
        );
        task_nodes.insert(handle.id(), node_key);

        true
    }
}
