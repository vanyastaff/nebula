//! Frontier failure — the `run_frontier` loop's Phase 3 failure arm.
//!
//! [`WorkflowEngine::process_joined_failure`] processes one failed node task
//! (the `Ok((task_id, (node_key, Err(..))))` join outcome): deferred-effect
//! abort, cooperative-cancel teardown, attempt recording, retry scheduling,
//! and the finalize path (failure recovery, error-edge routing, `NodeFailed`
//! emission, FailFast abort). The `Err(join_err)` panic-attribution arm stays
//! inline in `super` (issue-#301 decision). It acts only on the joined task's
//! own node; every other loop phase stays in `super`, which awaits it inline
//! — never spawned or raced (it holds only borrows).

use super::*;

impl WorkflowEngine {
    /// Process one failed node task (Phase 3 of the
    /// [`WorkflowEngine::run_frontier`] loop, the `Ok((task_id,
    /// (node_key, Err(ref err))))` arm).
    ///
    /// Removes the task-id side-map entry, aborts on a deferred effect, and
    /// either tears down under a cooperative cancel or drives the failure
    /// path through to the FailFast abort / edge routing. Awaits inline (the
    /// retry and finalize checkpoints) — never spawned or raced.
    ///
    /// Return mapping (1:1 with the inline arm):
    ///   - `Ok(None)` — the arm fell through (the retry-scheduled `continue`
    ///     or the natural end of the arm), or the cooperative-cancel
    ///     teardown's `break` (the loop's post-`break` tail is `Ok(None)`);
    ///     the loop iteration continues (or the loop ends, identical to the
    ///     inline `break`).
    ///   - `Ok(Some((node_key, error)))` — the arm's FailFast abort (an
    ///     OnError-routed abort message with no handler absorbing it);
    ///     `cancel_token` is cancelled first.
    ///   - `Err(e)` — the deferred-effect abort (`EngineError::Effect`) and
    ///     the checkpoint / recovery-failure sites (retry checkpoint,
    ///     `apply_failure_recovery`, finalize checkpoint; each cancels
    ///     first). There are no bare-`?` sites in this arm:
    ///     `record_node_attempt` is matched inline (its failure forces
    ///     `Finalize`), so every `Err` here propagates WITH a cancel
    ///     (`determine_final_status` keys `Cancelled` on
    ///     `cancel_token.is_cancelled`).
    #[expect(
        clippy::too_many_arguments,
        reason = "the handler mirrors the inline arm's free variables; bundling the \
                  step-local join error into the loop-carried scope/graph/strategy \
                  struct would re-shape the Phase 3 call site for no behavioral gain"
    )]
    pub(super) async fn process_joined_failure(
        &self,
        ctx: &mut FrontierCtx<'_>,
        scope: &Scope,
        graph: &DependencyGraph,
        cancel_token: &CancellationToken,
        execution_id: ExecutionId,
        fencing: Option<nebula_storage_port::FencingToken>,
        error_strategy: nebula_workflow::ErrorStrategy,
        node_map: &HashMap<NodeKey, &nebula_workflow::NodeDefinition>,
        workflow_retry_policy: Option<&nebula_workflow::RetryConfig>,
        task_id: tokio::task::Id,
        node_key: NodeKey,
        err: &EngineError,
    ) -> Result<Option<(NodeKey, String)>, EngineError> {
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
                // The inline arm `break`s out of the loop here; the loop's
                // post-`break` tail is `Ok(None)`, so this maps 1:1.
                return Ok(None);
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
                .and_then(|nd| effective_retry_policy(nd, workflow_retry_policy))
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
            let next_at = next_retry_at(execution_id, &node_key, delay, self.clock.now());
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
                    return Ok(None);
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
            apply_failure_recovery(outcome, node_key.clone(), ctx.exec_state, ctx.outputs)
        {
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
        // The arm's natural end: fall through so the loop iteration
        // continues.
        Ok(None)
    }
}
