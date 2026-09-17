//! Frontier execution — the level-by-level DAG executor.
//!
//! `run_frontier` drives ready nodes to completion with bounded concurrency,
//! and `spawn_node` builds the per-node task. The loop's per-stage extractions
//! and the Phase 3 success-arm handler live in the sibling child modules
//! (`complete`, `dispatch`, `heaps`, `spawn`, `wake`).
//! Split out of `engine.rs` as part of the god-module decomposition (audit
//! 🔴-1). These remain `impl WorkflowEngine` methods in child modules, so they
//! keep full access to the engine's private fields, sibling methods, helper
//! free functions, and types through `use super::*`.

use super::*;
use nebula_error::ErrorCode;

mod complete;
mod dispatch;
mod heaps;
mod spawn;
mod wake;

use self::wake::WakeReason;

/// Shared, loop-local state of the [`WorkflowEngine::run_frontier`] loop.
///
/// Bundles the edge maps, the ready queue, both timer heaps, the in-flight
/// task set, and the state that is borrowed across loop iterations
/// (`exec_state`, `outputs`, `repo_version`, `resume_rx`), so the planned
/// per-stage extractions (`drain_due_retries`, `drain_due_wait_wakes`,
/// `drain_ready_nodes`, `await_frontier_wake`, the success/failure arm
/// handlers) can each take `&mut FrontierCtx` plus step-local arguments
/// instead of 15-20 loose parameters. Private to this module: the stage
/// functions land in this same `frontier/` directory module.
struct FrontierCtx<'a> {
    /// Execution state across the whole loop; borrowed because the caller
    /// reads the final state after `run_frontier` returns.
    exec_state: &'a mut ExecutionState,
    /// Per-node outputs, shared with in-flight node tasks behind `Arc` in
    /// the caller.
    outputs: &'a Arc<DashMap<NodeKey, serde_json::Value>>,
    /// Optimistic-concurrency version threaded through every checkpoint CAS.
    repo_version: &'a mut u64,
    /// Live Resume channel; a fresh `recv()` is armed per select! iteration.
    resume_rx: &'a mut mpsc::Receiver<ResumeRequest>,
    /// Set once the Resume channel closes so the `recv()` select! arm is
    /// permanently disabled (avoids a busy-spin on `Ready(None)`).
    resume_rx_closed: bool,
    /// Edges activated per source node (resume-pre-populated).
    activated_edges: HashMap<NodeKey, HashSet<NodeKey>>,
    /// Resolved-incoming-edge count per target node (resume-pre-populated).
    resolved_edges: HashMap<NodeKey, usize>,
    /// Incoming-edge count that gates each node's readiness.
    required_count: HashMap<NodeKey, usize>,
    /// Nodes whose predecessors have all resolved and at least one edge has
    /// been activated.
    ready_queue: VecDeque<NodeKey>,
    /// Nodes parked in `WaitingRetry`, keyed by next attempt time.
    retry_heap: BinaryHeap<Reverse<(DateTime<Utc>, NodeKey)>>,
    /// Nodes parked in `Waiting` with a timer, keyed by wake deadline.
    wait_heap: BinaryHeap<Reverse<(DateTime<Utc>, NodeKey)>>,
    /// In-flight node tasks and their results.
    join_set: JoinSet<(
        NodeKey,
        Result<ActionResult<serde_json::Value>, EngineError>,
    )>,
    /// Side map from tokio task id to node key for panic attribution (#301).
    task_nodes: HashMap<tokio::task::Id, NodeKey>,
    /// Shared `$node` expression outputs handed to spawned tasks.
    shared_expression_outputs: Arc<DashMap<NodeKey, Arc<serde_json::Value>>>,
    /// Running output-byte total shared with in-flight tasks for the
    /// budget guard.
    total_output_bytes: Arc<AtomicU64>,
}

impl<'a> FrontierCtx<'a> {
    /// Bundle the borrowed state and start with empty containers. The
    /// caller seeds the heaps from `exec_state` and the ready queue from
    /// `seed_nodes` before entering the loop.
    fn new(
        exec_state: &'a mut ExecutionState,
        outputs: &'a Arc<DashMap<NodeKey, serde_json::Value>>,
        repo_version: &'a mut u64,
        resume_rx: &'a mut mpsc::Receiver<ResumeRequest>,
    ) -> Self {
        let total_output_bytes = Arc::new(AtomicU64::new(exec_state.total_output_bytes));
        Self {
            exec_state,
            outputs,
            repo_version,
            resume_rx,
            resume_rx_closed: false,
            activated_edges: HashMap::new(),
            resolved_edges: HashMap::new(),
            required_count: HashMap::new(),
            ready_queue: VecDeque::new(),
            retry_heap: BinaryHeap::new(),
            wait_heap: BinaryHeap::new(),
            join_set: JoinSet::new(),
            task_nodes: HashMap::new(),
            shared_expression_outputs: Arc::new(DashMap::new()),
            total_output_bytes,
        }
    }
}

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
        factory_dispatch: FactoryDispatch<'_>,
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
        elapsed_before_turn: Duration,
        error_strategy: nebula_workflow::ErrorStrategy,
        workflow_retry_policy: Option<nebula_workflow::RetryConfig>,
        seed_nodes: Vec<NodeKey>,
        initial_activated: HashMap<NodeKey, HashSet<NodeKey>>,
        initial_resolved: HashMap<NodeKey, usize>,
    ) -> Result<Option<(NodeKey, String)>, EngineError> {
        let mut ctx = FrontierCtx::new(exec_state, outputs, repo_version, resume_rx);

        // Precompute how many incoming edges each node has
        ctx.required_count = node_map
            .keys()
            .map(|nid| (nid.clone(), graph.incoming_connections(nid.clone()).len()))
            .collect();

        // Track edge resolution state (pre-populated for resume)
        ctx.activated_edges = initial_activated;
        ctx.resolved_edges = initial_resolved;

        // Seed with the provided nodes (entry nodes for fresh; frontier for resume)
        for node_key in seed_nodes {
            ctx.ready_queue.push_back(node_key);
        }

        // Min-heap (via `Reverse`) of `(next_attempt_at, NodeKey)` for
        // nodes parked in `WaitingRetry` per T5. The
        // heap is the engine's source of truth for "what to dispatch
        // next when the current frontier is otherwise idle"; cancel /
        // terminate / budget guards run AFTER the timer fires so a
        // cancelled execution does not silently re-dispatch a node.
        //
        // Resume seeding: any node already in `WaitingRetry` from a
        // prior run (its `next_attempt_at` survived via JSONB) needs
        // to land on the heap before the loop starts. Otherwise a
        // resumed retry would silently never re-dispatch.
        for (key, ns) in &ctx.exec_state.node_states {
            if ns.state == NodeState::WaitingRetry
                && let Some(when) = ns.next_attempt_at
            {
                ctx.retry_heap.push(Reverse((when, key.clone())));
            }
        }

        // Min-heap of `(wake_at, NodeKey)` for nodes parked in `Waiting`
        // with a timer-based condition (`Until` / `Duration`). Mirrors
        // `retry_heap` but drains to `Completed` instead of `Ready` —
        // a satisfied wait condition means the node is done, not
        // restarted. Signal-only parked nodes (webhook/approval/execution
        // with no timeout) are NOT on this heap; they stay parked until
        // a `Resume` signal arrives (built separately).
        //
        // Resume seeding for `Waiting` nodes: a crashed engine may have
        // persisted a node in `Waiting` with a `next_attempt_at` timer;
        // re-seed the heap so the wake fires without requiring a fresh
        // `ActionResult::Wait` dispatch.
        for (key, ns) in &ctx.exec_state.node_states {
            if ns.state == NodeState::Waiting
                && let Some(when) = ns.next_attempt_at
            {
                ctx.wait_heap.push(Reverse((when, key.clone())));
            }
        }

        // `ctx.join_set` holds the in-flight node tasks; `ctx.task_nodes`
        // is the side map from tokio task id → NodeKey so that panics
        // (where the inner future's `(NodeKey, _)` payload is lost) can
        // still be attributed to the real node instead of a synthesized
        // placeholder (issue #301).
        //
        // `ctx.resume_rx_closed` disarms the `resume_rx.recv()` select!
        // arm after the first `None` (channel closed). Without this guard
        // the arm would poll `Ready(None)` on every iteration — a
        // busy-spin for the full run duration. This fires immediately on
        // the replay path (the Sender is dropped at the `RunningEntry`
        // construction site) and also defends against any premature drop
        // of the Running registration in the execute path.

        // Main frontier loop
        loop {
            // Phase 0: drain due retries from the retry_heap into the
            // ready_queue (frontier/heaps.rs).
            self.drain_due_retries(&mut ctx, execution_id);

            // Phase 0b: drain due timer-wakes from `wait_heap`
            // (frontier/heaps.rs). On `Ok(Some(..))` the timeout fail-path's
            // FailFast abort surfaces; on `Err` a checkpoint failure
            // propagates (cancels already applied inside, exactly as in the
            // inline path).
            if let Some(failed_node) = self
                .drain_due_wait_wakes(
                    &mut ctx,
                    scope,
                    graph,
                    execution_id,
                    error_strategy,
                    fencing,
                    cancel_token,
                )
                .await?
            {
                return Ok(Some(failed_node));
            }

            // Phase 1: drain the ready queue → dispatch (frontier/dispatch.rs).
            // On `Ok(Some(..))` a budget violation or a FailFast abort
            // surfaces; on `Err` a checkpoint / recovery failure propagates
            // (cancels already applied inside, exactly as in the inline path).
            // The disabled-node bypass's checkpoint failure propagates WITHOUT
            // a cancel, also as in the inline path.
            if let Some(failed_node) = self
                .drain_ready_nodes(
                    &mut ctx,
                    scope,
                    graph,
                    node_map,
                    &factory_dispatch,
                    semaphore,
                    cancel_token,
                    execution_id,
                    workflow_id,
                    input,
                    fencing,
                    budget,
                    started,
                    elapsed_before_turn,
                    error_strategy,
                    workflow_retry_policy.as_ref(),
                )
                .await?
            {
                return Ok(Some(failed_node));
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
                ctx.join_set.abort_all();
                while ctx.join_set.join_next_with_id().await.is_some() {}
                ctx.task_nodes.clear();
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
                    &mut ctx.retry_heap,
                    &mut ctx.wait_heap,
                    &mut ctx.ready_queue,
                    ctx.exec_state,
                    execution_id,
                );
                break;
            }

            // Exit only when join_set, retry_heap, AND wait_heap are
            // all drained — a non-empty heap with an empty join_set
            // is a legal "everything paused for a timer" state.
            if ctx.join_set.is_empty() && ctx.retry_heap.is_empty() && ctx.wait_heap.is_empty() {
                break;
            }

            // Build this iteration's sleep/notify barrier and wait until one
            // wake source fires; every post-wake side effect lives in the
            // match below, in the loop body.
            let wake = self
                .await_frontier_wake(&mut ctx, cancel_token, budget, started, elapsed_before_turn)
                .await;

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
                    if let Some(control) = req.control {
                        let Some(fence) = fencing else {
                            let _ = req.ack.send(ResumeOutcome::Claimed(
                                ClaimedControlTurnOutcome::NotAccepted(
                                    EngineError::MissingExactRuntime,
                                ),
                            ));
                            continue;
                        };
                        let armed = match self
                            .commit_claimed_control(
                                scope,
                                execution_id,
                                &control,
                                ctx.exec_state,
                                ctx.repo_version,
                                fence,
                            )
                            .await
                        {
                            Ok(armed) => armed,
                            Err(control_turn::ControlCommitFailure::ClaimSuperseded) => {
                                let _ = req.ack.send(ResumeOutcome::Claimed(
                                    ClaimedControlTurnOutcome::ClaimSuperseded,
                                ));
                                continue;
                            },
                            Err(failure) => {
                                let _ =
                                    req.ack.send(ResumeOutcome::Claimed(failure.into_outcome()));
                                return Err(EngineError::ControlTurnInterrupted);
                            },
                        };
                        let _ = req.ack.send(ResumeOutcome::Claimed(
                            ClaimedControlTurnOutcome::Accepted(Ok(())),
                        ));
                        let now = self.clock.now();
                        let armed_set: HashSet<&NodeKey> = armed.iter().collect();
                        let retained: Vec<_> = std::mem::take(&mut ctx.wait_heap)
                            .into_iter()
                            .filter(|Reverse((_, key))| !armed_set.contains(key))
                            .collect();
                        ctx.wait_heap.extend(retained);
                        for node in armed {
                            ctx.wait_heap.push(Reverse((now, node)));
                        }
                        continue;
                    }
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
                    let to_arm = arm_signal_waits_under_lease(
                        ctx.exec_state,
                        req.resume_target.as_ref(),
                        now,
                    );
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
                    ctx.exec_state.version += 1;
                    ctx.exec_state.updated_at = now;
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
                            None,
                            ctx.outputs,
                            ctx.exec_state,
                            ctx.repo_version,
                            fencing,
                            vec![],
                        )
                        .await
                    {
                        let _ = req.ack.send(ResumeOutcome::ArmFailed);
                        cancel_token.cancel();
                        return Err(e);
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
                    let retained: Vec<_> = std::mem::take(&mut ctx.wait_heap)
                        .into_iter()
                        .filter(|Reverse((_, key))| !armed.contains(key))
                        .collect();
                    ctx.wait_heap.extend(retained);
                    for node_key in to_arm {
                        ctx.wait_heap.push(Reverse((now, node_key.clone())));
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
                    ctx.resume_rx_closed = true;
                    tracing::trace!(
                        target = "engine::wait",
                        %execution_id,
                        "resume channel closed; no live Resume producer remains — arm disarmed"
                    );
                    continue;
                },
                WakeReason::WallClock => {
                    cancel_token.cancel();
                    ctx.join_set.abort_all();
                    while ctx.join_set.join_next_with_id().await.is_some() {}
                    ctx.task_nodes.clear();
                    drain_pending_to_cancelled(
                        &mut ctx.retry_heap,
                        &mut ctx.wait_heap,
                        &mut ctx.ready_queue,
                        ctx.exec_state,
                        execution_id,
                    );
                    return Ok(Some((
                        node_key!("_timeout"),
                        "execution budget exceeded: max_duration".to_string(),
                    )));
                },
                WakeReason::Cancel => {
                    ctx.join_set.abort_all();
                    while ctx.join_set.join_next_with_id().await.is_some() {}
                    ctx.task_nodes.clear();
                    drain_pending_to_cancelled(
                        &mut ctx.retry_heap,
                        &mut ctx.wait_heap,
                        &mut ctx.ready_queue,
                        ctx.exec_state,
                        execution_id,
                    );
                    break;
                },
            };

            // Phase 3: Process the completed task
            match join_result {
                Ok((task_id, (node_key, Ok(action_result)))) => {
                    // Phase 3 success processing (frontier/complete.rs).
                    // The arm's single `continue` (the post-park skip) maps
                    // to `Ok(None)`; every abort / checkpoint-failure site
                    // maps 1:1 to `Ok(Some(..))` / `Err(..)` and returns
                    // from `run_frontier`.
                    match self
                        .process_joined_success(
                            &mut ctx,
                            scope,
                            graph,
                            cancel_token,
                            execution_id,
                            fencing,
                            budget,
                            started,
                            task_id,
                            node_key,
                            action_result,
                        )
                        .await
                    {
                        Ok(None) => {},
                        passthrough => return passthrough,
                    }
                },
                Ok((task_id, (node_key, Err(ref err)))) => {
                    ctx.task_nodes.remove(&task_id);
                    if let EngineError::Effect(effect) = err
                        && effect.is_deferred()
                    {
                        cancel_token.cancel();
                        return Err(EngineError::Effect(*effect));
                    }
                    ctx.outputs.remove(&node_key);

                    if let Some(state) = ctx.exec_state.node_states.get_mut(&node_key) {
                        state.current_output = None;
                    }

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
                        if ctx
                            .exec_state
                            .transition_node(node_key.clone(), NodeState::Cancelled)
                            .is_ok()
                        {
                            ctx.join_set.abort_all();
                            while ctx.join_set.join_next_with_id().await.is_some() {}
                            ctx.task_nodes.clear();
                            drain_pending_to_cancelled(
                                &mut ctx.retry_heap,
                                &mut ctx.wait_heap,
                                &mut ctx.ready_queue,
                                ctx.exec_state,
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
                    // `mark_node_failed` builds the envelope once; reuse its
                    // return below instead of a second `durable_error_envelope`
                    // build over the same message.
                    let err_envelope = mark_node_failed(ctx.exec_state, node_key.clone(), err);
                    // In-process projection for the event, retry, and OnError-payload
                    // surfaces, which are plain strings by contract.
                    let err_str = err_envelope.to_string();

                    // Push the failure attempt record so idempotency
                    // key, retry-decision, and post-mortem audit all
                    // see the same attempt history.
                    // T4: if recording fails (programming
                    // error: unknown node), force `Finalize` rather than
                    // letting `compute_retry_decision` see a stale
                    // `attempts.len()` — that path could bypass
                    // `max_attempts` (loop forever when no global cap
                    // is set) or collide idempotency keys on resume.
                    let failure_attempt_recorded = match ctx.exec_state.record_node_attempt(
                        node_key.clone(),
                        AttemptOutcome::Failure {
                            error: err_envelope,
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
                            ctx.exec_state,
                            retry_policy_resolved.as_ref(),
                            error_is_terminal(err),
                        )
                    } else {
                        RetryDecision::Finalize
                    };

                    if let RetryDecision::Retry { delay } = decision {
                        let attempt_number = ctx
                            .exec_state
                            .node_states
                            .get(&node_key)
                            .map_or(1, |ns| ns.attempt_count() as u32);
                        let next_at =
                            next_retry_at(execution_id, &node_key, delay, self.clock.now());
                        match ctx
                            .exec_state
                            .schedule_node_retry(node_key.clone(), next_at)
                        {
                            Ok(()) => {
                                // Persist the WaitingRetry transition,
                                // `next_attempt_at`, and the global
                                // counter bump in one CAS write.
                                if let Err(e) = self
                                    .checkpoint_node(
                                        scope,
                                        execution_id,
                                        node_key.clone(),
                                        Some(nebula_execution::NodeCheckpoint::Failed {
                                            error_port_output: None,
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
                                ctx.retry_heap.push(Reverse((next_at, node_key.clone())));
                                tracing::info!(
                                    target = "engine::retry",
                                    %execution_id,
                                    %node_key,
                                    attempt = attempt_number,
                                    delay_ms = delay.as_millis() as u64,
                                    next_attempt_at = %next_at,
                                    total_retries = ctx.exec_state.total_retries,
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
                    if let Err(e) = apply_failure_recovery(
                        outcome,
                        node_key.clone(),
                        ctx.exec_state,
                        ctx.outputs,
                    ) {
                        cancel_token.cancel();
                        return Err(e);
                    }

                    let abort = route_failure_edges(
                        outcome,
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

                    if let Err(e) = self
                        .checkpoint_node(
                            scope,
                            execution_id,
                            node_key.clone(),
                            Some(checkpoint::failure_checkpoint(
                                outcome,
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
                        return Ok(Some((node_key.clone(), err_msg)));
                    }
                },
                Err(join_err) => {
                    // Recover the real NodeKey via the task-id side
                    // map; falling back to a synthetic key would
                    // report a phantom node and lose the identity of
                    // the actually-panicked task (issue #301).
                    let task_id = join_err.id();
                    let panicked_node = ctx.task_nodes.remove(&task_id);
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
                            ctx.outputs,
                            ctx.exec_state,
                            ctx.repo_version,
                            fencing,
                        )
                        .await?;
                        cancel_token.cancel();
                        return Ok(Some((node_key, err_msg)));
                    }

                    // No matching task id — this should be unreachable
                    // as we insert every spawn into `task_nodes`, but
                    // fall through defensively rather than inventing
                    // a node identity.
                    cancel_token.cancel();
                    return Ok(Some((
                        node_key!("_panicked"),
                        format!("panicked task with unknown id: {err_msg}"),
                    )));
                },
            }
        }

        Ok(None)
    }
}
