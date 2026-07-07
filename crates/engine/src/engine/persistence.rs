//! Node-result recording, idempotency, checkpointing & final-state persistence.
//!
//! The methods that turn a finished node into durable state: idempotency
//! short-circuiting, per-node result + checkpoint writes (legacy store and the
//! port seam), panic handling, final-state persistence, and the terminal
//! event. Split out of `engine.rs` as part of the god-module decomposition
//! (audit 🔴-1). These remain `impl WorkflowEngine` methods in a child module,
//! retaining access to the engine's private fields, sibling methods, helper
//! free functions and types via `use super::*`.

use super::*;

impl WorkflowEngine {
    /// Check whether a node was already executed (idempotency key is set) and,
    /// if so, load its persisted output, mark it completed, and activate outgoing
    /// edges — all without re-dispatching the action.
    ///
    /// Returns `true` when the node was short-circuited (caller should `continue`
    /// to the next ready queue entry). Returns `false` when the node should be
    /// dispatched normally.
    #[expect(clippy::too_many_arguments)]
    pub(super) async fn check_and_apply_idempotency(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        node_key: NodeKey,
        outputs: &Arc<DashMap<NodeKey, serde_json::Value>>,
        exec_state: &mut ExecutionState,
        graph: &DependencyGraph,
        activated_edges: &mut HashMap<NodeKey, HashSet<NodeKey>>,
        resolved_edges: &mut HashMap<NodeKey, usize>,
        required_count: &HashMap<NodeKey, usize>,
        ready_queue: &mut VecDeque<NodeKey>,
    ) -> bool {
        // Replay is detected by a persisted node *output*: the port's
        // guard is check-and-mark with no read-only probe, so the
        // durable output is the authoritative "already ran" signal (a
        // present mark with a missing output is a partial write ⇒
        // re-execute). Without a store bundle there is no persistence,
        // so nothing can be replayed.
        let (output_value, stored_result) = if let Some(stores) = &self.stores {
            let id = execution_id.to_string();
            let output_value = match stores
                .node_results
                .load_node_output(scope, &id, node_key.as_str())
                .await
            {
                Ok(Some(record)) => record.json,
                Ok(None) => return false,
                Err(e) => {
                    tracing::warn!(
                        %execution_id,
                        %node_key,
                        error = %e,
                        "failed to load idempotent node output; re-executing"
                    );
                    return false;
                },
            };
            let stored_result = match stores
                .node_results
                .load_node_result(scope, &id, node_key.as_str())
                .await
            {
                Ok(Some(record)) => deserialize_stored_result(record.json, execution_id, &node_key),
                Ok(None) => {
                    tracing::warn!(
                        %execution_id,
                        %node_key,
                        "idempotency replay has no persisted ActionResult; \
                         synthesizing Success — Branch/Route/MultiOutput \
                         routing will not be preserved"
                    );
                    None
                },
                Err(e) => {
                    tracing::warn!(
                        %execution_id,
                        %node_key,
                        error = %e,
                        "failed to load persisted action result; \
                         falling back to synthesized Success"
                    );
                    None
                },
            };
            (output_value, stored_result)
        } else {
            return false;
        };

        outputs.insert(node_key.clone(), output_value.clone());
        mark_node_completed(exec_state, node_key.clone());

        let effective_result = stored_result.unwrap_or_else(|| ActionResult::success(output_value));
        process_outgoing_edges(
            node_key.clone(),
            Some(&effective_result),
            None,
            graph,
            activated_edges,
            resolved_edges,
            required_count,
            ready_queue,
            exec_state,
        );

        true
    }

    /// Persist the full [`ActionResult`] variant for a successfully
    /// executed node so that idempotent replay can reconstruct the
    /// exact routing semantics (Branch/Route/MultiOutput/Skip) instead
    /// of synthesising a flat `Success` (issue #299).
    ///
    /// Best-effort: failures are logged and ignored. Backends that do
    /// not override `save_node_result` no-op via the default trait
    /// implementation.
    pub(super) async fn record_node_result(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        node_key: NodeKey,
        action_result: &ActionResult<serde_json::Value>,
    ) {
        let value = match serde_json::to_value(action_result) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    %execution_id,
                    %node_key,
                    error = %e,
                    "failed to serialize action result for persistence"
                );
                return;
            },
        };
        // Best-effort persist through the spec-16 node-result store when
        // a store bundle is configured (in-memory-only mode skips it).
        if let Some(stores) = &self.stores {
            let record = match crate::store_seam::node_result_record(value) {
                Ok(record) => record,
                Err(e) => {
                    tracing::warn!(
                        %execution_id,
                        %node_key,
                        error = %e,
                        "refusing to persist malformed action result"
                    );
                    return;
                },
            };
            if let Err(e) = stores
                .node_results
                .save_node_result(scope, &execution_id.to_string(), node_key.as_str(), record)
                .await
            {
                tracing::warn!(
                    %execution_id,
                    %node_key,
                    error = %e,
                    "failed to persist action result"
                );
            }
        }
    }

    /// Record an idempotency key for a successfully executed node (best-effort).
    ///
    /// Silently logs and ignores errors — idempotency key recording failures
    /// must not abort an otherwise healthy execution.
    pub(super) async fn record_idempotency(
        &self,
        scope: &Scope,
        exec_state: &ExecutionState,
        execution_id: ExecutionId,
        node_key: NodeKey,
    ) {
        // Dual-dispatch. The port guard is check-and-mark on
        // `{scope}:{exec}:{node}:{attempt}`; the attempt is derived the
        // same way as `idempotency_key_for_node` (
        // `attempt_count + 1`) so the guard key stays in lockstep with
        // the persisted output/result rows.
        if let Some(stores) = &self.stores {
            let attempt = exec_state
                .node_states
                .get(&node_key)
                .map_or(1, |ns| (ns.attempt_count() as u32).saturating_add(1));
            if let Err(e) = stores
                .idempotency
                .check_and_mark(scope, &execution_id.to_string(), node_key.as_str(), attempt)
                .await
            {
                tracing::warn!(
                    %execution_id,
                    %node_key,
                    error = %e,
                    "failed to mark node as idempotent"
                );
            }
        }
    }

    /// Persist node output and execution state to the repository.
    ///
    /// Returns `Err(EngineError::CheckpointFailed)` when the store cannot
    /// durably commit — `save_node_output` failure, `transition()` error,
    /// or CAS mismatch (the row moved beneath the engine). Callers in
    /// `run_frontier` MUST abort the node's progression (no edge routing,
    /// no event emission) on `Err` so that observers and the frontier
    /// never act on an unpersisted transition (, #297).
    /// Persist final Failed state + emit NodeFailed for a panicked task.
    ///
    /// Best-effort: checkpoint failures are logged at `warn!` level (not
    /// propagated) so that the engine still returns a cohesive panic
    /// error to `run_frontier`'s caller. The real durability gap —
    /// `save_node_output` after panic — is already logged by
    /// `checkpoint_node` itself.
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors checkpoint_node's arity; the fencing token is required by the \
                  dual-dispatch storage seam"
    )]
    pub(super) async fn handle_panicked_node(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        node_key: NodeKey,
        err_msg: &str,
        outputs: &Arc<DashMap<NodeKey, serde_json::Value>>,
        exec_state: &mut ExecutionState,
        repo_version: &mut u64,
        fencing: Option<nebula_storage_port::FencingToken>,
    ) {
        let panic_err = EngineError::TaskPanicked(err_msg.to_owned());
        mark_node_failed(exec_state, node_key.clone(), &panic_err);
        let checkpoint_result = self
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
            .await;
        if let Err(e) = checkpoint_result {
            tracing::warn!(
                %execution_id,
                %node_key,
                error = %e,
                "failed to checkpoint panicked node state"
            );
        }
        self.emit_event(ExecutionEvent::NodeFailed {
            execution_id,
            node_key,
            details: NodeFailedDetails {
                error_code: "ENGINE:TASK_PANICKED".to_owned(),
                display_message: err_msg.to_owned(),
            },
        });
    }

    // Private helper — `scope` carries the originating message's tenant so
    // every port call enforces cross-tenant isolation (invariant #7); the
    // borrow checker then prevents any caller from substituting an alternate
    // scope. Combined with the fencing token this pushes past the 7-arg
    // threshold that is designed for public APIs.
    #[expect(clippy::too_many_arguments)]
    pub(super) async fn checkpoint_node(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        node_key: NodeKey,
        outputs: &Arc<DashMap<NodeKey, serde_json::Value>>,
        exec_state: &ExecutionState,
        repo_version: &mut u64,
        fencing: Option<nebula_storage_port::FencingToken>,
        resume_tokens: Vec<ResumeTokenRow>,
    ) -> Result<(), EngineError> {
        // No store bundle ⇒ single-process library mode: nothing to
        // checkpoint.
        let Some(stores) = &self.stores else {
            return Ok(());
        };
        // A configured store always implies an acquired lease (the
        // no-lease branch only fires when `stores` is absent), so the
        // fencing token is present. A missing token here is a wiring
        // bug, surfaced as a typed checkpoint failure rather than a panic.
        let Some(token) = fencing else {
            return Err(EngineError::CheckpointFailed {
                node_key,
                reason: "no fencing token for a configured store checkpoint".to_owned(),
            });
        };
        // Spec-16 port path: state, output, and (empty) outbox/journal
        // commit through one `TransitionBatch`; a superseded fencing
        // token is rejected even on a matching version (closes the
        // zombie-runner hole).
        self.checkpoint_node_port(
            scope,
            stores,
            execution_id,
            node_key,
            outputs,
            exec_state,
            repo_version,
            token,
            resume_tokens,
        )
        .await
    }

    /// Spec-16 port variant of [`Self::checkpoint_node`]: save the node
    /// output through [`nebula_storage_port::store::NodeResultStore`] and
    /// commit the state snapshot through a fencing-gated
    /// [`nebula_storage_port::TransitionBatch`]. A superseded fencing
    /// token yields [`EngineError::CasConflict`] (the new holder owns the
    /// canonical state — ADR 0008, ); a CAS version mismatch follows
    /// the same #333 refetch-and-abort contract as the legacy path.
    ///
    /// `resume_tokens` is non-empty only on signal-park commits (W-S3c);
    /// the batch builder defaults to empty so non-park checkpoints are
    /// unaffected.
    #[expect(clippy::too_many_arguments)]
    async fn checkpoint_node_port(
        &self,
        scope: &Scope,
        stores: &crate::store_seam::ExecutionStores,
        execution_id: ExecutionId,
        node_key: NodeKey,
        outputs: &Arc<DashMap<NodeKey, serde_json::Value>>,
        exec_state: &ExecutionState,
        repo_version: &mut u64,
        token: nebula_storage_port::FencingToken,
        resume_tokens: Vec<ResumeTokenRow>,
    ) -> Result<(), EngineError> {
        let id = execution_id.to_string();

        if let Some(output) = outputs.get(&node_key) {
            let record = crate::store_seam::node_output_record(output.value().clone());
            if let Err(e) = stores
                .node_results
                .save_node_output(scope, &id, node_key.as_str(), record)
                .await
            {
                return Err(EngineError::CheckpointFailed {
                    node_key,
                    reason: format!("save_node_output: {e}"),
                });
            }
        }

        let state_json =
            serde_json::to_value(exec_state).map_err(|e| EngineError::CheckpointFailed {
                node_key: node_key.clone(),
                reason: format!("serialize state: {e}"),
            })?;

        let batch = nebula_storage_port::TransitionBatch::builder()
            .scope(scope.clone())
            .execution_id(&id)
            .expected_version(*repo_version)
            .fencing(token)
            .new_state(state_json)
            .resume_tokens(resume_tokens)
            .build()
            .map_err(|e| EngineError::CheckpointFailed {
                node_key: node_key.clone(),
                reason: format!("build transition batch: {e}"),
            })?;

        match stores.execution.commit(batch).await {
            Ok(nebula_storage_port::TransitionOutcome::Applied { new_version }) => {
                *repo_version = new_version;
                Ok(())
            },
            Ok(nebula_storage_port::TransitionOutcome::FencedOut) => {
                tracing::error!(
                    %execution_id,
                    %node_key,
                    "checkpoint fenced out — lease superseded; aborting node progression \
                     (ADR 0008, §12.2)"
                );
                Err(EngineError::CasConflict {
                    execution_id,
                    expected_version: *repo_version,
                    observed_version: *repo_version,
                    observed_status: "fenced_out".to_owned(),
                })
            },
            Ok(nebula_storage_port::TransitionOutcome::VersionConflict { actual }) => {
                let expected_version = *repo_version;
                *repo_version = actual;
                let observed_status = match stores.execution.get(scope, &id).await {
                    Ok(Some(rec)) => {
                        *repo_version = rec.version;
                        parse_observed_status(&rec.state)
                    },
                    Ok(None) => "unknown".to_owned(),
                    Err(e) => {
                        tracing::warn!(
                            %execution_id,
                            %node_key,
                            error = %e,
                            "checkpoint CAS mismatch: failed to refetch persisted state"
                        );
                        "unknown".to_owned()
                    },
                };
                tracing::warn!(
                    %execution_id,
                    %node_key,
                    expected_version,
                    observed_version = *repo_version,
                    %observed_status,
                    "checkpoint CAS mismatch — aborting node progression (§11.1, #333)"
                );
                Err(EngineError::CasConflict {
                    execution_id,
                    expected_version,
                    observed_version: *repo_version,
                    observed_status,
                })
            },
            Err(e) => Err(EngineError::CheckpointFailed {
                node_key,
                reason: e.to_string(),
            }),
        }
    }

    /// Persist the final execution state, reconciling with any
    /// externally-driven concurrent update (issue #333).
    ///
    /// Contract:
    ///
    /// * On CAS success: commit and return the engine-local final status (unchanged from pre-fix
    ///   behaviour).
    /// * On CAS mismatch, **reload the full persisted state**, then:
    ///     - If the observed persisted status is already terminal (`Cancelled` / `Failed` /
    ///       `TimedOut` / `Completed`), honor it. The external actor (API cancel, admin mutation,
    ///       sibling runner) produced an authoritative terminal transition the engine may not
    ///       overwrite — `Ok(Some(external_status))` is returned so the caller reports the external
    ///       status in `ExecutionResult`.
    ///     - Otherwise, copy the engine's local `final_status` onto the freshly-loaded state, bump
    ///       the observed version, and retry the transition exactly once. On repeated CAS mismatch
    ///       or a storage error, return [`EngineError::CasConflict`] /
    ///       [`EngineError::CheckpointFailed`] rather than silently reporting success.
    ///
    /// Pre-fix this path was `log-and-continue` (see `tracing::warn!`
    /// "final state checkpoint CAS mismatch" before #333) — that
    /// silently dropped the final write and let the engine report
    /// `Completed` on an un-persisted state, violating `docs/PRODUCT_CANON.md`
    /// (durability precedes visibility) and (no silent
    /// log-and-continue on state-transition failures).
    ///
    /// # Returns
    ///
    /// * `Ok(None)` — the engine's local final status was durably persisted (either on first try or
    ///   on the retry).
    /// * `Ok(Some(status))` — the CAS was still conflicting but the persisted state is already
    ///   terminal; the caller should surface `status` as the execution outcome instead of the
    ///   engine's local decision.
    /// * `Err(EngineError::CasConflict { .. })` — after a retry the row was still moving and not
    ///   terminal; the engine cannot honor the conflict without more context, so the caller
    ///   surfaces a typed failure instead of a silent success.
    pub(super) async fn persist_final_state(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        exec_state: &mut ExecutionState,
        repo_version: &mut u64,
        fencing: Option<nebula_storage_port::FencingToken>,
    ) -> Result<Option<ExecutionStatus>, EngineError> {
        // The caller only routes here when `self.stores` is configured,
        // and a configured store always implies an acquired lease (the
        // no-lease branch only fires when `stores` is absent), so the
        // fencing token is present. A missing token here is a wiring
        // bug, surfaced as a typed checkpoint failure rather than a panic.
        let (Some(stores), Some(token)) = (self.stores.clone(), fencing) else {
            return Err(EngineError::CheckpointFailed {
                node_key: final_state_node_key(),
                reason: "no fencing-gated store configured for final-state persist".to_owned(),
            });
        };
        self.persist_final_state_port(
            scope,
            &stores,
            execution_id,
            exec_state,
            repo_version,
            token,
        )
        .await
    }

    /// Spec-16 port variant of [`Self::persist_final_state`]: the final
    /// state snapshot commits through a fencing-gated
    /// [`nebula_storage_port::TransitionBatch`]. Same #333
    /// reconcile-and-retry contract as the legacy path; a superseded
    /// fencing token yields [`EngineError::CasConflict`] (the new lease
    /// holder owns the canonical state — ADR 0008, ).
    pub(super) async fn persist_final_state_port(
        &self,
        scope: &Scope,
        stores: &crate::store_seam::ExecutionStores,
        execution_id: ExecutionId,
        exec_state: &mut ExecutionState,
        repo_version: &mut u64,
        token: nebula_storage_port::FencingToken,
    ) -> Result<Option<ExecutionStatus>, EngineError> {
        let id = execution_id.to_string();

        let build_batch = |version: u64,
                           json: serde_json::Value|
         -> Result<nebula_storage_port::TransitionBatch, EngineError> {
            nebula_storage_port::TransitionBatch::builder()
                .scope(scope.clone())
                .execution_id(&id)
                .expected_version(version)
                .fencing(token)
                .new_state(json)
                .build()
                .map_err(|e| EngineError::CheckpointFailed {
                    node_key: final_state_node_key(),
                    reason: format!("build final transition batch: {e}"),
                })
        };

        let state_json =
            serde_json::to_value(&*exec_state).map_err(|e| EngineError::CheckpointFailed {
                node_key: final_state_node_key(),
                reason: format!("serialize final state: {e}"),
            })?;

        let outcome = stores
            .execution
            .commit(build_batch(*repo_version, state_json)?)
            .await
            .map_err(|e| EngineError::CheckpointFailed {
                node_key: final_state_node_key(),
                reason: format!("final state persist: {e}"),
            })?;

        match outcome {
            nebula_storage_port::TransitionOutcome::Applied { new_version } => {
                *repo_version = new_version;
                // W-S3e — proactively revoke any un-consumed resume tokens once
                // the execution is durably terminal. Deliberately POST-commit and
                // non-atomic: mint-on-park rides the `TransitionBatch` atomically,
                // but revoke is a separate best-effort call. A crash in the window
                // (terminal committed, revoke not yet run) leaves only un-reachable
                // dead token rows — backstopped by the `port_resume_tokens`
                // `ON DELETE CASCADE` FK and the no-op-resume against a terminal
                // execution (see `nebula_storage_port::store::resume_token` module
                // docs). So a revoke failure must NOT fail the already-terminal
                // transition.
                if exec_state.status.is_terminal() {
                    revoke_resume_tokens_best_effort(stores, scope, &id).await;
                }
                Ok(None)
            },
            nebula_storage_port::TransitionOutcome::FencedOut => Err(EngineError::CasConflict {
                execution_id,
                expected_version: *repo_version,
                observed_version: *repo_version,
                observed_status: "fenced_out".to_owned(),
            }),
            nebula_storage_port::TransitionOutcome::VersionConflict { actual } => {
                let expected_version = *repo_version;
                let observed = match stores.execution.get(scope, &id).await {
                    Ok(Some(rec)) => rec,
                    Ok(None) => {
                        *repo_version = actual;
                        return Err(EngineError::CasConflict {
                            execution_id,
                            expected_version,
                            observed_version: actual,
                            observed_status: "missing".to_owned(),
                        });
                    },
                    Err(e) => {
                        return Err(EngineError::CheckpointFailed {
                            node_key: final_state_node_key(),
                            reason: format!("final CAS refetch failed: {e}"),
                        });
                    },
                };
                let observed_version = observed.version;
                let observed_json = observed.state;
                *repo_version = observed_version;
                let observed_status_enum = parse_observed_execution_status(&observed_json);
                let observed_status_str = parse_observed_status(&observed_json);

                if observed_status_enum
                    .as_ref()
                    .is_some_and(ExecutionStatus::is_terminal)
                {
                    tracing::warn!(
                        %execution_id,
                        expected_version,
                        observed_version,
                        %observed_status_str,
                        "final state CAS mismatch: external transition is terminal — \
                         honoring external status instead of overwriting (§11.5, #333)"
                    );
                    // W-S3e — revoke un-consumed tokens before honoring the
                    // external terminal state. The external writer may be a
                    // non-engine path (e.g. the API `cancel_execution` handler,
                    // which writes `Cancelled` directly without calling
                    // `revoke_on_terminal`). Engine sinks (persist_final_state_port
                    // / cancel_dangling_nodes_under_lease) do revoke, but we
                    // cannot know which writer produced the observed terminal here.
                    // `revoke_on_terminal` is idempotent and scope-bound: revoking
                    // here is safe even if an engine sink already revoked (it just
                    // deletes 0 rows). Best-effort: a revoke failure must NOT
                    // prevent honoring the external terminal.
                    revoke_resume_tokens_best_effort(stores, scope, &id).await;
                    return Ok(observed_status_enum);
                }

                let retry_json = match serde_json::to_value(&*exec_state) {
                    Ok(v) => v,
                    Err(e) => {
                        return Err(EngineError::CheckpointFailed {
                            node_key: final_state_node_key(),
                            reason: format!("serialize retry state: {e}"),
                        });
                    },
                };
                match stores
                    .execution
                    .commit(build_batch(*repo_version, retry_json)?)
                    .await
                {
                    Ok(nebula_storage_port::TransitionOutcome::Applied { new_version }) => {
                        tracing::info!(
                            %execution_id,
                            expected_version,
                            observed_version,
                            "final state CAS retry succeeded after external bump (§11.5, #333)"
                        );
                        *repo_version = new_version;
                        // W-S3e — same terminal cleanup as the first Applied arm:
                        // the retry is where the terminal state lands on the
                        // CAS-reconcile path (external non-terminal bump → we
                        // refetched, observed non-terminal, retried). Only call
                        // after `new_version` is stored so the FK-guarded revoke
                        // sees the committed terminal row.
                        if exec_state.status.is_terminal() {
                            revoke_resume_tokens_best_effort(stores, scope, &id).await;
                        }
                        Ok(None)
                    },
                    Ok(nebula_storage_port::TransitionOutcome::FencedOut) => {
                        Err(EngineError::CasConflict {
                            execution_id,
                            expected_version,
                            observed_version: *repo_version,
                            observed_status: "fenced_out".to_owned(),
                        })
                    },
                    Ok(nebula_storage_port::TransitionOutcome::VersionConflict {
                        actual: retry_actual,
                    }) => {
                        let (latest_version, latest_status) =
                            match stores.execution.get(scope, &id).await {
                                Ok(Some(rec)) => {
                                    let s = parse_observed_status(&rec.state);
                                    (rec.version, s)
                                },
                                _ => (retry_actual, observed_status_str.clone()),
                            };
                        *repo_version = latest_version;
                        Err(EngineError::CasConflict {
                            execution_id,
                            expected_version,
                            observed_version: latest_version,
                            observed_status: latest_status,
                        })
                    },
                    Err(e) => Err(EngineError::CheckpointFailed {
                        node_key: final_state_node_key(),
                        reason: format!("final CAS retry failed: {e}"),
                    }),
                }
            },
        }
    }

    /// Record final execution metrics.
    pub(super) fn emit_final_event(
        &self,
        _execution_id: ExecutionId,
        status: ExecutionStatus,
        elapsed: Duration,
        _failed_node: &Option<(NodeKey, String)>,
    ) {
        match status {
            ExecutionStatus::Completed => {
                self.workflow_executions_completed.inc();
            },
            ExecutionStatus::Failed => {
                self.workflow_executions_failed.inc();
            },
            _ => {},
        }

        self.workflow_execution_duration_seconds
            .observe(elapsed.as_secs_f64());
    }
}
