//! Frontier dispatch — the `run_frontier` loop's Phase 1 ready-queue drain.
//!
//! [`WorkflowEngine::drain_ready_nodes`] drains `ctx.ready_queue` and
//! dispatches each node: the pre-pop budget check, the explicit disabled-node
//! bypass, the durable idempotency replay, and the `spawn_node` dispatch with
//! its setup-failure handling (attempt record, retry decision, OnError
//! staging). Awaited inline by the loop body in `super` — never spawned or
//! raced.

use super::*;

impl WorkflowEngine {
    /// Drain the ready queue → dispatch into the join set (Phase 1 of the
    /// [`WorkflowEngine::run_frontier`] loop).
    ///
    /// The cancel-check runs BEFORE `pop_front` so a node that observes the
    /// cancel signal mid-iteration stays in the queue and is collected by
    /// `drain_pending_to_cancelled` on the cancel/wall-clock teardown
    /// branches — popping first would drop the node and strand it as
    /// `Ready`, tripping frontier integrity (CAS on version).
    ///
    /// Awaits inline (the disabled-node bypass and setup-failure checkpoints)
    /// and is awaited by the loop body — never spawned or raced.
    ///
    /// Returns:
    ///   - `Ok(None)` — the queue drained without a frontier exit; the loop
    ///     body proceeds to Phase 2.
    ///   - `Ok(Some((node_key, error)))` — a budget violation (node held the
    ///     violation text) or a FailFast abort surfaced by the setup-failure
    ///     finalize path; `cancel_token` is cancelled first.
    ///   - `Err(e)` — a checkpoint / recovery failure propagated out of the
    ///     drain. Every explicit `Err` return site cancels `cancel_token`
    ///     first, except the disabled-node bypass's checkpoint failure, which
    ///     propagates WITHOUT a cancel (bare `?`), exactly as in the inline
    ///     path.
    #[expect(
        clippy::too_many_arguments,
        reason = "the drain mirrors the inline phase's parameter list; bundling the \
                  loop-carried scope/graph/strategy/fencing into a struct would \
                  re-shape the Phase 1..3 call sites for no behavioral gain"
    )]
    pub(super) async fn drain_ready_nodes(
        &self,
        ctx: &mut FrontierCtx<'_>,
        scope: &Scope,
        graph: &DependencyGraph,
        node_map: &HashMap<NodeKey, &nebula_workflow::NodeDefinition>,
        factory_dispatch: &FactoryDispatch<'_>,
        semaphore: &Arc<Semaphore>,
        cancel_token: &CancellationToken,
        execution_id: ExecutionId,
        workflow_id: WorkflowId,
        input: &serde_json::Value,
        fencing: Option<nebula_storage_port::FencingToken>,
        budget: &ExecutionBudget,
        started: &Instant,
        elapsed_before_turn: Duration,
        error_strategy: nebula_workflow::ErrorStrategy,
        workflow_retry_policy: Option<&nebula_workflow::RetryConfig>,
    ) -> Result<Option<(NodeKey, String)>, EngineError> {
        while !cancel_token.is_cancelled()
            && let Some(node_key) = ctx.ready_queue.pop_front()
        {
            // Check budget limits before dispatching
            if let Some(violation) = check_budget(
                budget,
                started,
                elapsed_before_turn,
                &ctx.total_output_bytes,
            ) {
                cancel_token.cancel();
                return Ok(Some((node_key, violation)));
            }

            // Preserve the explicit disabled-node bypass through the main edge.
            if node_map.get(&node_key).is_some_and(|nd| !nd.enabled) {
                mark_node_skipped(ctx.exec_state, node_key.clone());
                self.checkpoint_node(
                    scope,
                    execution_id,
                    node_key.clone(),
                    Some(nebula_execution::NodeCheckpoint::Bypassed {}),
                    ctx.outputs,
                    ctx.exec_state,
                    ctx.repo_version,
                    fencing,
                    vec![],
                )
                .await?;
                process_outgoing_edges(
                    node_key.clone(),
                    None,
                    None, // not failed
                    graph,
                    &mut ctx.activated_edges,
                    &mut ctx.resolved_edges,
                    &ctx.required_count,
                    &mut ctx.ready_queue,
                    ctx.exec_state,
                );
                continue;
            }

            // Durable idempotency check: if this node was already executed
            // (e.g., on a previous attempt), load the persisted output and
            // mark it completed without re-dispatching.
            if matches!(*factory_dispatch, FactoryDispatch::DirectRegistry)
                && self
                    .check_and_apply_idempotency(
                        scope,
                        execution_id,
                        node_key.clone(),
                        ctx.outputs,
                        ctx.exec_state,
                        graph,
                        &mut ctx.activated_edges,
                        &mut ctx.resolved_edges,
                        &ctx.required_count,
                        &mut ctx.ready_queue,
                    )
                    .await
            {
                continue;
            }

            let spawned = self.spawn_node(
                scope,
                fencing,
                node_key.clone(),
                node_map,
                factory_dispatch,
                graph,
                ctx.outputs,
                &ctx.shared_expression_outputs,
                semaphore,
                cancel_token,
                ctx.exec_state,
                execution_id,
                workflow_id,
                input,
                &ctx.activated_edges,
                &mut ctx.join_set,
                &mut ctx.task_nodes,
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
            let err_envelope = ctx
                .exec_state
                .node_state(node_key.clone())
                .and_then(|ns| ns.error_message.clone())
                .unwrap_or_else(|| {
                    setup_refusal(
                        ErrorCode::new(crate::error::codes::PARAM_RESOLUTION),
                        "parameter resolution failed",
                    )
                });

            // In-process projection for the event, retry, and OnError-payload
            // surfaces, which are plain strings by contract. Safe to derive from the
            // record: `Display` renders only the typed code and the bounded,
            // engine-authored message, never the failed action's own text.
            let err_msg = err_envelope.to_string();

            // Push the failure attempt record so retry-decision
            // and idempotency_key see the same attempt count.
            // T4: if recording fails (programming
            // error: unknown node), force `Finalize` rather than
            // running `compute_retry_decision` against a stale
            // `attempts.len()` — that would let `max_attempts`
            // be bypassed and risk an idempotency-key collision.
            let setup_attempt_recorded = match ctx.exec_state.record_node_attempt(
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
                    .and_then(|nd| effective_retry_policy(nd, workflow_retry_policy))
                    .cloned();
                // Setup failures (param resolution etc.) have no typed
                // `ActionError` and stay retry-eligible — "the action
                // never started", so no fatal short-circuit applies.
                compute_retry_decision(
                    &node_key,
                    ctx.exec_state,
                    setup_retry_policy.as_ref(),
                    false,
                )
            } else {
                RetryDecision::Finalize
            };

            // Asymmetry preserved: runtime warns on rejection, setup path
            // silent (deliberate).
            if let RetryDecision::Retry { delay } = setup_decision {
                let attempt_number = ctx
                    .exec_state
                    .node_states
                    .get(&node_key)
                    .map_or(1, |ns| ns.attempt_count() as u32);
                let next_at = next_retry_at(execution_id, &node_key, delay, self.clock.now());
                if ctx
                    .exec_state
                    .schedule_node_retry(node_key.clone(), next_at)
                    .is_ok()
                {
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
                apply_failure_recovery(outcome, node_key.clone(), ctx.exec_state, ctx.outputs)
            {
                cancel_token.cancel();
                return Err(e);
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

            if ctx
                .exec_state
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
                return Ok(Some((node_key, err_msg)));
            }
        }

        Ok(None)
    }
}
