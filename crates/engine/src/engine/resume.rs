//! Resume & signal-wait lifecycle.
//!
//! `resume_execution` rebuilds an incomplete execution after a process
//! restart; the `satisfy_*_signal_waits` methods deliver external signals to
//! parked nodes; the `cancel_dangling_*` methods tear down nodes left ready
//! when an execution is cancelled. Split out of `engine.rs` as part of the
//! god-module decomposition (audit 🔴-1). These remain `impl WorkflowEngine`
//! methods in a child module, retaining access to the engine's private fields,
//! sibling methods, helper free functions and types via `use super::*`.

use super::*;

impl WorkflowEngine {
    /// Resume an incomplete execution after process restart.
    ///
    /// Loads execution state and workflow definition from storage, identifies
    /// which nodes are already complete, and re-executes from the frontier of
    /// ready-but-not-yet-executed nodes (nodes whose predecessors are all
    /// terminal but which are not yet terminal themselves).
    ///
    /// Persisted outputs are pre-loaded into the shared output map so that
    /// resumed nodes receive the correct predecessor data.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::PlanningFailed`] if:
    /// - `execution_repo` or `workflow_repo` is not configured on this engine
    /// - The execution or workflow is not found in storage
    /// - The execution is already in a terminal state
    /// - The persisted state cannot be deserialized
    pub async fn resume_execution(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<ExecutionResult, EngineError> {
        let started = Instant::now();

        // 1-5. Load persisted state + workflow definition + node
        // outputs. Dual-dispatch: spec-16 port bundles when configured,
        // else the legacy repos. Both yield `(repo_version, state_json,
        // workflow_json, Vec<(NodeKey, output)>)` so the reconstruction
        // below is shared. In the spec-16 split the workflow definition
        // lives on the published *version* record, not the workflow row.
        let (repo_version_loaded, state_json, workflow_json, persisted_outputs): (
            u64,
            serde_json::Value,
            serde_json::Value,
            Vec<(NodeKey, serde_json::Value)>,
        ) = {
            // The scoped execution-store bundle is required for resume;
            // its absence preserves the historical `execution_repo`
            // wording the resume contract (and tests) assert on.
            let stores = self.stores.as_ref().ok_or_else(|| {
                EngineError::PlanningFailed("no execution_repo configured".into())
            })?;
            // The workflow bundle is likewise required; its absence
            // preserves the historical `workflow_repo` wording.
            let workflow_stores = self
                .workflow_stores
                .as_ref()
                .ok_or_else(|| EngineError::PlanningFailed("no workflow_repo configured".into()))?;
            let id = execution_id.to_string();
            let record = stores
                .execution
                .get(scope, &id)
                .await
                .map_err(|e| EngineError::PlanningFailed(format!("load state: {e}")))?
                .ok_or_else(|| {
                    EngineError::PlanningFailed(format!("execution not found: {execution_id}"))
                })?;
            let workflow_id = record.workflow_id.clone();
            let workflow_json = workflow_stores
                .versions
                .get_published(scope, &workflow_id)
                .await
                .map_err(|e| EngineError::PlanningFailed(format!("load workflow: {e}")))?
                .ok_or_else(|| {
                    EngineError::PlanningFailed(format!("workflow not found: {workflow_id}"))
                })?
                .definition;
            // Reload the raw per-node *outputs* (not the typed-result
            // slot): the result slot stores the serialized `ActionResult`
            // envelope, whereas successors consume the bare output payload
            // — reading results here would feed a crash-resumed run a
            // different value than a non-crashed run.
            let outputs = stores
                .node_results
                .load_all_node_outputs(scope, &id)
                .await
                .map_err(|e| EngineError::PlanningFailed(format!("load outputs: {e}")))?
                .into_iter()
                .filter_map(|(node_id, rec)| NodeKey::new(&node_id).ok().map(|k| (k, rec.json)))
                .collect();
            (record.version, record.state, workflow_json, outputs)
        };

        // Deserialize via JSON string to avoid `serde_json::from_value` issues
        // with Key<D> types that expect borrowed strings (domain-key serde impl).
        let state_str = serde_json::to_string(&state_json)
            .map_err(|e| EngineError::PlanningFailed(format!("serialize state: {e}")))?;
        let exec_state: ExecutionState = serde_json::from_str(&state_str)
            .map_err(|e| EngineError::PlanningFailed(format!("deserialize state: {e}")))?;

        // 3. Guard against resuming a terminal execution.
        if exec_state.status.is_terminal() {
            return Err(EngineError::PlanningFailed(format!(
                "execution {execution_id} is already terminal ({})",
                exec_state.status
            )));
        }

        // Deserialize via JSON string to avoid `serde_json::from_value` issues
        // with borrowed key types (e.g. `ActionKey` uses `#[serde(borrow)]`).
        let workflow_str = serde_json::to_string(&workflow_json)
            .map_err(|e| EngineError::PlanningFailed(format!("serialize workflow: {e}")))?;
        let workflow: WorkflowDefinition = serde_json::from_str(&workflow_str)
            .map_err(|e| EngineError::PlanningFailed(format!("deserialize workflow: {e}")))?;

        let workflow_id = exec_state.workflow_id;

        // 6. Build dependency graph.
        let graph = DependencyGraph::from_definition(&workflow)
            .map_err(|e| EngineError::PlanningFailed(e.to_string()))?;

        // 7. Reconstruct the execution state, resetting non-terminal nodes. Nodes that were Running
        //    at crash time need to be re-executed. This is a recovery path, so the reset bypasses
        //    the forward state machine via `override_node_state` but still bumps the version per
        //    transition so CAS readers see the change (issue #255).
        let mut exec_state = exec_state;
        // Cold-start seam : the API's start handler persists an
        // `ExecutionState::new(id, workflow_id, &[])` row — no per-node entries,
        // because the handler does not load the workflow on the hot path. The
        // first `ControlCommand::Start` that drains via `EngineControlDispatch`
        // lands here; seed `node_states` from the workflow definition so the
        // frontier seeder below treats graph entry nodes as the natural starting
        // set. A warm resume (post-crash, with persisted per-node state) skips
        // this branch untouched.
        //
        // Captured before the seeding mutation below so the W0 U2 pre-flight
        // further down (`validate_declared_output_ports`) can tell a genuine
        // first attempt (production `Start` traffic — this is production's
        // ONLY entry point for a brand-new execution; `execute_workflow_scoped`
        // is reachable only from tests/direct-embed callers) from an actual
        // resume of an already-in-progress execution, after this same check
        // has already emptied the condition it reads.
        let is_first_attempt = exec_state.node_states.is_empty();
        if is_first_attempt {
            for node in &workflow.nodes {
                exec_state.set_node_state(
                    node.id.clone(),
                    nebula_execution::state::NodeExecutionState::new(),
                );
            }
        }
        // Reset non-terminal nodes that crashed mid-attempt back to
        // `Pending` so the frontier loop can re-dispatch them. Retry
        // waits are different — a `WaitingRetry` node carries a
        // durable `next_attempt_at` that the frontier loop's
        // `retry_heap` consumes via the Phase-0 drain. Resetting it
        // would lose the persisted backoff and re-dispatch the node
        // immediately on resume, defeating T2's
        // resume guarantee. (Crashed `Running` attempts have no
        // such timestamp and must be re-driven from `Pending`.)
        //
        // `Waiting` nodes are excluded for the same reason: they are
        // durably parked for an external wait condition (timer or
        // signal). Their `next_attempt_at` is the timer wake instant;
        // the Phase-0b drain re-seeds `wait_heap` from them below.
        // Resetting a `Waiting` node to `Pending` would immediately
        // re-dispatch it, defeating the durable park guarantee.
        let non_terminal: Vec<NodeKey> = exec_state
            .node_states
            .iter()
            .filter(|(_, ns)| {
                !ns.state.is_terminal()
                    && ns.state != NodeState::WaitingRetry
                    && ns.state != NodeState::Waiting
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in non_terminal {
            let _ = exec_state.override_node_state(id, NodeState::Pending);
        }
        // Transition back to Running so the frontier loop can proceed.
        // The persisted state may be Created, Paused, or already Running after a crash.
        // Use transition_status when the transition is valid; skip if already Running.
        if !exec_state.status.is_terminal() && exec_state.status != ExecutionStatus::Running {
            // Ignoring the result is intentional: if this fails the status is left
            // as-is (e.g. Paused), which is still non-terminal and the loop will proceed.
            let _ = exec_state.transition_status(ExecutionStatus::Running);
        }

        // 8. Populate shared output map from persisted outputs.
        let outputs: Arc<DashMap<NodeKey, serde_json::Value>> = Arc::new(DashMap::new());
        for (node_key, value) in persisted_outputs {
            outputs.insert(node_key.clone(), value);
        }

        // 9. Compute the resume frontier and pre-populate edge-tracking maps.
        //
        //    A node is on the frontier if:
        //    - it is not yet terminal (Pending after the reset above), AND
        //    - all its predecessor nodes are terminal in the loaded state.
        //
        //    We also rebuild `activated_edges` and `resolved_edges` for terminal
        //    nodes so that `run_frontier`'s bookkeeping stays consistent when
        //    it evaluates edges from the frontier.
        let node_map: HashMap<NodeKey, &nebula_workflow::NodeDefinition> =
            workflow.nodes.iter().map(|n| (n.id.clone(), n)).collect();

        // W0 U2 fresh-execution-only pre-flight, gated on `is_first_attempt`
        // (captured above, before the cold-start seed emptied the condition
        // it was read from): reject connections wired to an output port the
        // source action never declared. This is production's ONLY path for
        // validating a brand-new execution — `execute_workflow_scoped` (which
        // runs the same check) is never reached from production dispatch;
        // production `Start`/`Resume`/`Restart` all converge on this function
        // via `EngineControlDispatch::drive` → `resume_execution`.
        //
        // Runs ONLY on a genuine first attempt, never on an actual resume of
        // an already-in-progress execution: re-validating a resumed run
        // against `resume_execution`'s reloaded *latest published* workflow
        // version (rather than the version the execution actually started
        // under — a separate, pre-existing bug, see the module-level
        // rationale on `validate_declared_output_ports`) could hard-fail an
        // in-flight execution over a version mismatch its operator never had
        // a chance to see. A true first attempt carries no such risk: there
        // is no prior in-flight state to conflict with, and the workflow
        // version it loads here IS the version it is starting under.
        //
        // Placement is load-bearing, same principle as `execute_workflow_scoped`:
        // this must run before the execution lease is acquired below
        // (`acquire_and_heartbeat_lease`, ~100 lines down) — verified by
        // reading every intervening line: nothing between here and the lease
        // acquire persists anything (`resume_execution` never creates the
        // execution row itself; the API's start handler already did, before
        // this function was ever called) or takes a lease, so a rejection
        // here requires zero teardown.
        if is_first_attempt {
            self.validate_declared_output_ports(&graph, &node_map)?;
        }

        let mut activated_edges: HashMap<NodeKey, HashSet<NodeKey>> = HashMap::new();
        let mut resolved_edges: HashMap<NodeKey, usize> = HashMap::new();
        let mut seed_nodes: Vec<NodeKey> = Vec::new();

        // Mark edges from terminal nodes as resolved (and activated, since they
        // completed successfully or were skipped).
        for (node_key, ns) in &exec_state.node_states {
            if !ns.state.is_terminal() {
                continue;
            }
            for conn in graph.outgoing_connections(node_key.clone()) {
                let target = conn.to_node.clone();
                // Increment per-edge count so multiple edges from the same terminal
                // source to the same target are each counted during resume.
                *resolved_edges.entry(target.clone()).or_insert(0) += 1;
                // Completed and Skipped nodes activate their outgoing edges so
                // that downstream nodes see a resolved predecessor.
                if matches!(ns.state, NodeState::Completed | NodeState::Skipped) {
                    activated_edges
                        .entry(target.clone())
                        .or_default()
                        .insert(node_key.clone());
                }
            }
        }

        // Identify frontier nodes: non-terminal nodes whose incoming edges are
        // all resolved (i.e., all predecessors are terminal).
        //
        // Note: we do NOT require that at least one edge is activated here.
        // During crash recovery we cannot know which edges were activated — that
        // state is not persisted separately. The conservative check (all
        // predecessors terminal → node is eligible) is correct for crash recovery:
        // the node may have been waiting for an edge that was never activated, but
        // the activated_edges map reconstructed above from Completed/Skipped
        // predecessors gives run_frontier the correct activation context to
        // evaluate edge conditions normally once the node is dispatched.
        for (node_key, ns) in &exec_state.node_states {
            if ns.state.is_terminal() {
                continue;
            }
            // T5 — `WaitingRetry` nodes belong to the
            // retry-pending heap, not to the seed/ready_queue. The
            // frontier loop seeds the heap from `WaitingRetry` nodes
            // separately; including them here would re-dispatch
            // immediately and bypass the persisted backoff timer.
            if ns.state == NodeState::WaitingRetry {
                continue;
            }
            // `Waiting` nodes are durably parked for an external
            // wait condition. Like `WaitingRetry`, they must NOT
            // enter the seed/ready_queue. The Phase-0b wait drain
            // re-seeds `wait_heap` from these nodes and drives
            // timer-based `Waiting→Completed` transitions.
            if ns.state == NodeState::Waiting {
                continue;
            }
            let incoming = graph.incoming_connections(node_key.clone());
            let required = incoming.len();
            let resolved = resolved_edges.get(node_key).copied().unwrap_or(0);

            if required == 0 || resolved == required {
                seed_nodes.push(node_key.clone());
            }
        }

        // 10. Build remaining infrastructure for the frontier loop.
        //
        // Restore the `ExecutionBudget` the original run was configured
        // with (issue #289). Legacy states that predate budget
        // persistence deserialize the field as `None` — fall back to
        // `ExecutionBudget::default()` with a warning so the degraded
        // limits are visible in logs instead of silently swapping
        // operator-configured limits for default ones.
        let budget = if let Some(b) = exec_state.budget.clone() {
            b
        } else {
            tracing::warn!(
                %execution_id,
                "resume: persisted execution state is missing budget; \
                 falling back to ExecutionBudget::default() — \
                 concurrency, timeout, and output-size limits from \
                 the original run are not being honoured (issue #289)"
            );
            ExecutionBudget::default()
        };
        let semaphore = Arc::new(Semaphore::new(budget.max_concurrent_nodes));
        let cancel_token = CancellationToken::new();
        let mut repo_version = repo_version_loaded;

        // Acquire the execution lease before running the frontier (ADR
        // 0008, #325). Resume is explicitly a second entry point for an
        // existing execution — if another runner is already driving it
        // (whether because the crash recovery loop picked it up or an
        // operator issued two resumes back-to-back), we fence this call
        // with `EngineError::Leased` instead of running nodes in parallel
        // with the existing runner.
        let lease = self
            .acquire_and_heartbeat_lease(scope, execution_id, cancel_token.clone())
            .await?;

        // Fencing token threaded into every checkpoint / final-state
        // commit. `Some` only on the spec-16 port lease path.
        let fencing = lease.as_ref().and_then(LeaseGuard::fencing_token);

        // Publish the cancel token into the running registry ONLY after
        // the lease is ours . Symmetric to
        // `execute_workflow` — see its comment for the full rationale
        // and the #482 Copilot review context.
        let registration_id = NEXT_REGISTRATION_ID.fetch_add(1, Ordering::Relaxed);
        // Live-frontier resume channel (W-S2b) — symmetric to
        // `execute_workflow`; see its comment for the rationale.
        let (resume_tx, mut resume_rx) = mpsc::channel::<ResumeRequest>(RESUME_CHANNEL_CAPACITY);
        self.running.insert(
            execution_id,
            RunningEntry {
                registration_id,
                token: cancel_token.clone(),
                resume_tx,
            },
        );
        let _cancel_registration = RunningRegistration {
            running: Arc::clone(&self.running),
            execution_id,
            registration_id,
        };

        self.workflow_executions_started.inc();

        let error_strategy = workflow.config.error_strategy;
        let workflow_retry_policy = workflow.config.retry_policy.clone();
        // Restore the original trigger payload from the persisted
        // execution state. Legacy states that predate #311 deserialize
        // the field as `None` — fall back to `Null` with a warning so
        // the regression is visible in logs.
        let workflow_input = if let Some(v) = exec_state.workflow_input.clone() {
            v
        } else {
            tracing::warn!(
                %execution_id,
                "resume: persisted execution state is missing workflow_input; \
                 falling back to Null — entry nodes that did not complete \
                 on the original run will receive Null input"
            );
            serde_json::Value::Null
        };
        let failed_node = self
            .run_frontier(
                scope,
                &graph,
                &node_map,
                &outputs,
                &semaphore,
                &cancel_token,
                &mut resume_rx,
                &mut exec_state,
                execution_id,
                workflow_id,
                &workflow_input,
                &mut repo_version,
                fencing,
                &budget,
                &started,
                error_strategy,
                workflow_retry_policy,
                seed_nodes,
                activated_edges,
                resolved_edges,
            )
            .await;

        self.runtime.clear_execution_output_totals(execution_id);

        let elapsed = started.elapsed();

        let heartbeat_lost = lease.as_ref().is_some_and(LeaseGuard::heartbeat_lost);
        let FinalStatusDecision {
            status: final_status,
            termination_reason,
            integrity_violation,
        } = determine_final_status(&failed_node, &cancel_token, &exec_state);
        // Use the validated transition path. Ignoring the result is intentional:
        // if the current status is already terminal (e.g. the execution was
        // cancelled during the frontier loop), we do not overwrite it.
        //
        // Bridge `Running → Cancelling → Cancelled` when the cancel token
        // fired mid-flight — one-step `Running → Cancelled` is not in the
        // valid-transition table (issue #273), so without the bridge the
        // invalid-transition error is silently swallowed and the row stays
        // at `Running`, producing a two-truth violation.
        if final_status == ExecutionStatus::Cancelled
            && exec_state.status == ExecutionStatus::Running
        {
            let _ = exec_state.transition_status(ExecutionStatus::Cancelling);
        }
        let _ = exec_state.transition_status(final_status);

        // Heartbeat loss: another runner now owns the canonical state.
        // Skip final persist and surface as Leased — mirrors the
        // execute_workflow contract. ADR 0008 / / #325.
        let reported_status = if heartbeat_lost {
            tracing::error!(
                %execution_id,
                "resume: final state persistence skipped: heartbeat lost this runner's lease; \
                 another runner now owns the execution (ADR 0008, §12.2, #325)"
            );
            if let Some(guard) = lease {
                guard.shutdown().await;
            }
            return Err(EngineError::Leased {
                execution_id,
                holder: self.instance_id.to_string(),
            });
        } else {
            // Persist final state with CAS-conflict reconciliation
            // (issue #333). `self.stores` is guaranteed `Some` here: the
            // load path at the top of `resume_execution` returns early with
            // `PlanningFailed` when stores are absent, so control can only
            // reach this point when the spec-16 bundle is configured.
            // Mirrors `execute_workflow` — see its comment for the full contract.
            match self
                .persist_final_state(
                    scope,
                    execution_id,
                    &mut exec_state,
                    &mut repo_version,
                    fencing,
                )
                .await
            {
                Ok(None) => final_status,
                Ok(Some(external_status)) => external_status,
                Err(EngineError::CasConflict {
                    expected_version,
                    observed_version,
                    observed_status,
                    ..
                }) => {
                    tracing::error!(
                        %execution_id,
                        expected_version,
                        observed_version,
                        %observed_status,
                        "resume: final state CAS conflict could not be reconciled; \
                         reporting Failed instead of silently completing (§11.5, #333)"
                    );
                    ExecutionStatus::Failed
                },
                Err(e) => {
                    tracing::error!(
                        %execution_id,
                        error = %e,
                        "resume: final state persist failed; \
                         reporting Failed instead of silently completing (§11.5, #333)"
                    );
                    ExecutionStatus::Failed
                },
            }
        };

        // Release the lease after the final persist completes.
        if let Some(guard) = lease {
            guard.shutdown().await;
        }

        self.emit_final_event(execution_id, reported_status, elapsed, &failed_node);
        self.emit_frontier_integrity_if_violated(execution_id, integrity_violation);
        tracing::info!(
            target = "engine",
            %execution_id,
            ?reported_status,
            ?termination_reason,
            ?elapsed,
            "execution_finished"
        );
        self.emit_event(ExecutionEvent::ExecutionFinished {
            execution_id,
            success: reported_status == ExecutionStatus::Completed,
            elapsed,
            termination_reason: termination_reason.clone(),
        });

        let node_outputs: HashMap<NodeKey, serde_json::Value> = outputs
            .iter()
            .map(|r| (r.key().clone(), r.value().clone()))
            .collect();

        let node_errors: HashMap<NodeKey, String> = exec_state
            .node_states
            .iter()
            .filter_map(|(id, ns)| {
                ns.error_message
                    .as_ref()
                    .map(|msg| (id.clone(), msg.clone()))
            })
            .collect();

        Ok(ExecutionResult {
            execution_id,
            status: reported_status,
            node_outputs,
            node_errors,
            duration: elapsed,
            termination_reason: termination_reason.clone(),
        })
    }

    /// Durably satisfy all signal-driven waits on a `Paused` execution.
    ///
    /// A signal-driven wait is a node in `Waiting` state with `next_attempt_at == None`
    /// (no timer). The node was parked by a `Webhook` / `Approval` / `Execution`
    /// `WaitCondition` and released its worker; the execution sits at `Paused` waiting
    /// for an external Resume signal.
    ///
    /// This method *arms* every such node for completion by setting its
    /// `next_attempt_at = now` while LEAVING it `Waiting`, persisted via the
    /// spec-16 `ExecutionStore` with a version-CAS + fencing batch (the same
    /// durability contract as `checkpoint_node`). The subsequent `drive`
    /// re-seeds the armed node into the frontier `wait_heap` and Phase-0b
    /// transitions it `Waiting → Completed` and activates its downstream edges
    /// through the **port-aware** `process_outgoing_edges` — exactly the path a
    /// timer wait takes. Completing the node here instead would route its edges
    /// through `resume_execution`'s port-blind rebuild, which activates *every*
    /// outgoing edge (so a multi-port wait would fire its `error`/custom branch
    /// on a normal Resume). The CAS serialises concurrent Resume calls: a
    /// second caller sees the node already armed (`next_attempt_at == Some`) or
    /// `Completed` and returns `NothingToSatisfy`, and the status short-circuit
    /// in `dispatch_resume` makes a post-completion duplicate a no-op.
    ///
    /// A signal-driven wait is satisfied ONLY by this method — it is the sole
    /// writer of `next_attempt_at` on a signal-`Waiting{None}` node. A reclaim
    /// re-drive (`dispatch_start`, `dispatch_restart`, worker `EngineExecutionSink`)
    /// enters `resume_execution` without calling this method first, so the node
    /// stays `Waiting{next_attempt_at == None}`, is never wait-heap-seeded, and
    /// the execution returns to `Paused` unchanged. That structural
    /// discriminator prevents a crashed Paused execution from auto-completing
    /// its wait on reclaim (data-corruption / security class bug).
    ///
    /// # Lease contract
    ///
    /// This method acquires the execution lease for the duration of the
    /// read-modify-write cycle so that the CAS token is always fresh and
    /// authoritative. Acquiring a lease prevents concurrent runners from
    /// modifying the execution row between our read and our commit:
    ///
    /// - If the lease is held elsewhere, returns [`EngineError::Leased`] —
    ///   the caller must defer and let the current lease holder finish.
    /// - On success, the lease is released (best-effort) after the commit.
    ///
    /// # Targeting (W-S3a)
    ///
    /// `resume_target` selects which parked signal wait this Resume arms:
    /// `Some(target)` arms only the node whose persisted [`WaitSignal`] matches
    /// the target by kind + identity (a webhook target never satisfies an
    /// approval gate — the kind-confusion safety rule); `None` arms every
    /// signal-driven wait (the W-S2b untargeted behavior).
    ///
    /// # Returns
    ///
    /// [`SatisfyOutcome::Satisfied`] carrying `n` when `n` signal-driven waiting
    /// nodes were armed for completion, or [`SatisfyOutcome::NothingToSatisfy`]
    /// when none match.
    ///
    /// # Errors
    ///
    /// - [`EngineError::Leased`] if the lease is held by another runner —
    ///   callers should defer (redeliver) rather than treating this as a
    ///   permanent failure.
    /// - [`EngineError::PlanningFailed`] if the execution row cannot be loaded or
    ///   its state cannot be deserialised.
    /// - [`EngineError::CasConflict`] if the durable write is rejected by a
    ///   concurrent transition (version or fencing mismatch after our lease was
    ///   released by another path — should not happen under normal flow).
    /// - [`EngineError::CheckpointFailed`] on serialisation or store errors.
    ///
    /// [`WaitSignal`]: nebula_execution::state::WaitSignal
    pub(crate) async fn satisfy_signal_waits(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        resume_target: Option<&ResumeTarget>,
    ) -> Result<SatisfyOutcome, EngineError> {
        let Some(stores) = &self.stores else {
            // Library mode (no storage) — signal-park resume is a no-op.
            return Ok(SatisfyOutcome::NothingToSatisfy);
        };

        let id = execution_id.to_string();
        let holder = self.instance_id.to_string();

        // Acquire the execution lease before the read-modify-write. This
        // ensures the fencing token we commit with is the authoritative
        // current generation — no stale token from a previous runner can
        // slip through under concurrent lease acquisition.
        //
        // A live lease held elsewhere means another runner is already driving
        // this execution. Return `Leased` so the caller defers the Resume
        // rather than racing the CAS and potentially dropping the signal.
        let lease_token = stores
            .execution
            .acquire_lease(scope, &id, &holder, self.lease_ttl)
            .await
            .map_err(|e| {
                EngineError::PlanningFailed(format!(
                    "satisfy_signal_waits: acquire lease for {execution_id}: {e}"
                ))
            })?;
        let Some(lease_token) = lease_token else {
            tracing::warn!(
                %execution_id,
                %holder,
                "satisfy_signal_waits: execution lease held by another runner; \
                 deferring Resume (will redeliver)"
            );
            return Err(EngineError::Leased {
                execution_id,
                holder,
            });
        };

        // Lease acquired — proceed under mutual exclusion.
        let outcome = self
            .satisfy_signal_waits_under_lease(
                stores,
                scope,
                execution_id,
                &id,
                lease_token,
                resume_target,
            )
            .await;

        // Release the lease best-effort. The commit already wrote the new
        // fencing generation, so a release failure leaves the lease to expire
        // at TTL — the correct fail-safe behaviour (another runner can then
        // re-acquire after TTL rather than being blocked indefinitely).
        if let Err(e) = stores
            .execution
            .release_lease(scope, &id, lease_token)
            .await
        {
            tracing::warn!(
                %execution_id,
                error = %e,
                "satisfy_signal_waits: best-effort lease release failed (will expire at TTL)"
            );
        }

        outcome
    }

    /// Recover a no-live-owner **`Running`** execution by arming its signal
    /// waits under a freshly-acquired lease — the structural sibling of
    /// [`Self::satisfy_signal_waits`] for the crash-recovery path (ADR-0099
    /// W-S3b).
    ///
    /// A signal wait parked WITH a timeout keeps its execution `Running` (the
    /// timeout timer lives on the parking runner's `wait_heap`). When that
    /// runner crashes, its in-process frontier loop is gone but the durable row
    /// stays `Running` with the wait node still parked. A `Resume` for such an
    /// execution reaches [`WorkflowEngine::resume_live`] with no live
    /// `RunningEntry` on this runner ([`ResumeDelivery::NoLiveEntry`]) — either
    /// the parking runner crashed with a now-TTL-expired lease, or the Resume
    /// landed on a different runner than the (possibly still live) owner. This
    /// method distinguishes the two and recovers only the genuinely no-live case.
    ///
    /// The lease IS the dead-vs-live oracle, exactly as in `satisfy_signal_waits`:
    ///
    /// - [`EngineError::Leased`] — the lease is still LIVE elsewhere, so a real
    ///   owner is actively driving this execution. We must NOT touch the row;
    ///   the caller defers (B1 reclaim redelivers once the lease frees, or the
    ///   live owner's own resume channel handles it). This is the cross-runner /
    ///   not-yet-crashed case.
    /// - lease acquired (free, or TTL-expired ⇒ the parking runner crashed and
    ///   no owner remains) — proceed. We arm the matching signal wait(s) under
    ///   the owned lease via the SAME [`Self::satisfy_signal_waits_under_lease`]
    ///   inner the `Paused` path uses, so the version-CAS + fencing commit and
    ///   the kind-aware [`arm_signal_waits_under_lease`] targeting are identical.
    ///   A targeted recovery (`Some(resume_target)`) arms ONLY the matching
    ///   node; an untargeted one arms every signal wait. The caller then
    ///   re-drives via `drive_armed_resume`, whose Phase-0b completes the armed
    ///   wait on the main port.
    ///
    /// The own-the-lease-before-read-modify-write invariant (#856) is preserved:
    /// the lease is held across the whole inner commit and released best-effort
    /// only afterwards (mirroring `satisfy_signal_waits`), so a stale token can
    /// never be manufactured from persisted metadata.
    ///
    /// # Security
    ///
    /// Only a genuine `Resume` calls this method — `dispatch_resume`'s
    /// `NoLiveEntry` arm. A plain crash-recovery re-drive (the worker sink /
    /// `dispatch_start` / `dispatch_restart`) re-enters `resume_execution`
    /// WITHOUT arming, so it re-parks the wait rather than auto-completing it.
    /// That is the same structural discriminator `satisfy_signal_waits`
    /// enforces for the `Paused` case, extended to the `Running` case here.
    ///
    /// # Returns / Errors
    ///
    /// Same [`SatisfyOutcome`] / [`EngineError`] contract as
    /// [`Self::satisfy_signal_waits`] (it shares the inner): `Satisfied(n)` /
    /// `NothingToSatisfy` / `ExecutionNotResumable` on success; `Leased` (live
    /// owner — defer) / `PlanningFailed` / `CasConflict` / `CheckpointFailed`
    /// on error.
    pub(crate) async fn satisfy_running_signal_waits(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        resume_target: Option<&ResumeTarget>,
    ) -> Result<SatisfyOutcome, EngineError> {
        let Some(stores) = &self.stores else {
            // Library mode (no storage) — signal-park recovery is a no-op.
            return Ok(SatisfyOutcome::NothingToSatisfy);
        };

        let id = execution_id.to_string();
        let holder = self.instance_id.to_string();

        // Acquire the execution lease before the read-modify-write — the
        // dead-vs-live oracle. A free or TTL-expired lease (the parking runner
        // crashed) is acquirable here; a lease still LIVE elsewhere returns
        // `None` ⇒ `Leased`, so the caller defers and lets the real owner drive.
        let lease_token = stores
            .execution
            .acquire_lease(scope, &id, &holder, self.lease_ttl)
            .await
            .map_err(|e| {
                EngineError::PlanningFailed(format!(
                    "satisfy_running_signal_waits: acquire lease for {execution_id}: {e}"
                ))
            })?;
        let Some(lease_token) = lease_token else {
            tracing::warn!(
                %execution_id,
                %holder,
                "satisfy_running_signal_waits: execution lease held by another runner \
                 (live owner elsewhere); deferring Resume recovery (will redeliver)"
            );
            return Err(EngineError::Leased {
                execution_id,
                holder,
            });
        };

        // Lease acquired (crashed owner / no live frontier) — recover under
        // mutual exclusion through the SAME inner the Paused path uses.
        let outcome = self
            .satisfy_signal_waits_under_lease(
                stores,
                scope,
                execution_id,
                &id,
                lease_token,
                resume_target,
            )
            .await;

        // Release the lease best-effort (mirror `satisfy_signal_waits`): the
        // commit already wrote the new fencing generation, so a release failure
        // leaves the lease to expire at TTL (the correct fail-safe).
        if let Err(e) = stores
            .execution
            .release_lease(scope, &id, lease_token)
            .await
        {
            tracing::warn!(
                %execution_id,
                error = %e,
                "satisfy_running_signal_waits: best-effort lease release failed \
                 (will expire at TTL)"
            );
        }

        outcome
    }

    /// Inner read-modify-write under an already-held lease.
    ///
    /// Extracted so the lease release in `satisfy_signal_waits` is guaranteed
    /// to run even when this inner path errors.
    async fn satisfy_signal_waits_under_lease(
        &self,
        stores: &crate::store_seam::ExecutionStores,
        scope: &Scope,
        execution_id: ExecutionId,
        id: &str,
        lease_token: nebula_storage_port::FencingToken,
        resume_target: Option<&ResumeTarget>,
    ) -> Result<SatisfyOutcome, EngineError> {
        let record = stores
            .execution
            .get(scope, id)
            .await
            .map_err(|e| {
                EngineError::PlanningFailed(format!(
                    "satisfy_signal_waits: load execution {execution_id}: {e}"
                ))
            })?
            .ok_or_else(|| {
                EngineError::PlanningFailed(format!(
                    "satisfy_signal_waits: execution not found: {execution_id}"
                ))
            })?;

        let repo_version = record.version;

        let state_str = serde_json::to_string(&record.state).map_err(|e| {
            EngineError::PlanningFailed(format!(
                "satisfy_signal_waits: serialise state for {execution_id}: {e}"
            ))
        })?;
        let mut exec_state: ExecutionState = serde_json::from_str(&state_str).map_err(|e| {
            EngineError::PlanningFailed(format!(
                "satisfy_signal_waits: deserialise state for {execution_id}: {e}"
            ))
        })?;

        // Re-check the execution status under the lease before satisfying any
        // node. `dispatch_resume` read `Paused` BEFORE acquiring the lease; a
        // concurrent Cancel/Terminate may have committed a terminal (or
        // `Cancelling`) status in that window. The per-node `Waiting → Completed`
        // CAS below guards only the node version, not the execution status, so
        // without this gate we would flip — and durably commit — a wait node on
        // an already-cancelled execution, corrupting its terminal audit state.
        // Treat it as an idempotent no-op; the caller acks the Resume.
        if exec_state.status.is_terminal() || exec_state.status == ExecutionStatus::Cancelling {
            tracing::info!(
                %execution_id,
                status = %exec_state.status,
                "satisfy_signal_waits: execution left Paused before the under-lease reload \
                 (concurrent cancel/terminate); skipping satisfy as idempotent no-op"
            );
            return Ok(SatisfyOutcome::ExecutionNotResumable);
        }

        // Arm the signal-driven waits selected by `resume_target` for Phase-0b
        // completion: set each match's wake instant to `now` and LEAVE it
        // `Waiting`. The subsequent `drive` re-seeds it into the frontier
        // `wait_heap` (its `next_attempt_at` is now `Some`) and Phase-0b
        // transitions it `Waiting → Completed` and activates downstream through
        // the PORT-AWARE `process_outgoing_edges` — the same path a timer wait
        // takes. Transitioning to `Completed` HERE would instead route the
        // node's edges through `resume_execution`'s port-blind rebuild, which
        // activates every outgoing edge (a multi-port wait would fire its
        // `error`/custom branch on a normal Resume).
        //
        // `arm_signal_waits_under_lease` is the shared armer: a `Some(target)`
        // Resume arms only the kind+identity match; a `None` Resume arms every
        // signal wait (W-S2b behavior). It runs under the lease we hold, so the
        // own-the-lease-before-RMW invariant (#856) is preserved.
        let now = self.clock.now();
        let armed = arm_signal_waits_under_lease(&mut exec_state, resume_target, now);

        if armed.is_empty() {
            return Ok(SatisfyOutcome::NothingToSatisfy);
        }

        let satisfied_count = armed.len();

        // Mirror `ExecutionState::transition_node`: direct field mutation must
        // still advance `version`/`updated_at` so the serialized blob's
        // denormalized version matches the row the store CAS produces. A reader
        // that reconstructs `ExecutionState` from the blob and keys its own CAS
        // on `exec_state.version` must not accept a stale snapshot whose version
        // never moved. (The store CAS below keys on `repo_version`, so the
        // commit is correct regardless; this keeps the in-blob copy honest —
        // the same bump the W-S2b live-frontier self-arm performs.)
        exec_state.version += 1;
        exec_state.updated_at = now;

        // Persist the satisfy-CAS. This is the single discriminator between
        // a genuine Resume and a reclaim re-drive: only this code path arms
        // `next_attempt_at` on a signal-`Waiting{None}` node before `drive`
        // runs. A reclaim that re-enters `resume_execution` without calling
        // this method first sees the node still `Waiting{next_attempt_at ==
        // None}`, never wait-heap-seeds it, and re-parks — returning the
        // execution to `Paused` without completing the wait.
        //
        // We use the freshly-acquired lease token — not the stale generation
        // read from the row — so the CAS is always guarded by the
        // authoritative current generation. A concurrent runner that acquired
        // the lease between our read and this write would be blocked because
        // we hold the lease here.
        let state_json =
            serde_json::to_value(&exec_state).map_err(|e| EngineError::CheckpointFailed {
                node_key: final_state_node_key(),
                reason: format!("satisfy_signal_waits: serialise updated state: {e}"),
            })?;

        let batch = nebula_storage_port::TransitionBatch::builder()
            .scope(scope.clone())
            .execution_id(id)
            .expected_version(repo_version)
            .fencing(lease_token)
            .new_state(state_json)
            .build()
            .map_err(|e| EngineError::CheckpointFailed {
                node_key: final_state_node_key(),
                reason: format!("satisfy_signal_waits: build batch: {e}"),
            })?;

        match stores.execution.commit(batch).await {
            Ok(nebula_storage_port::TransitionOutcome::Applied { new_version }) => {
                tracing::info!(
                    target = "engine::wait",
                    %execution_id,
                    satisfied_count,
                    new_version,
                    "satisfy_signal_waits: armed signal waits for Phase-0b completion — \
                     drive will complete them and activate downstream on the main port"
                );
                // No `NodeWaitCompleted` is emitted here: the node is still
                // `Waiting`. Phase-0b emits that event when it transitions the
                // node `Waiting → Completed` (the single completion site shared
                // with timer waits), so observers never see a completion for a
                // node that is not yet durably `Completed`.
                Ok(SatisfyOutcome::Satisfied(satisfied_count))
            },
            Ok(nebula_storage_port::TransitionOutcome::FencedOut) => {
                // We hold the lease, so FencedOut should not occur in normal
                // flow — it would mean the store rejected our own lease token.
                // Surface as a CAS conflict so the caller can redeliver.
                tracing::warn!(
                    %execution_id,
                    "satisfy_signal_waits: CAS fenced out under our own lease — \
                     store inconsistency or lease TTL expired during commit"
                );
                Err(EngineError::CasConflict {
                    execution_id,
                    expected_version: repo_version,
                    observed_version: repo_version,
                    observed_status: "fenced_out".to_owned(),
                })
            },
            Ok(nebula_storage_port::TransitionOutcome::VersionConflict { actual }) => {
                // The row version advanced between our read and the commit —
                // a concurrent transition (outside our lease window) beat us.
                // Surface as a CAS conflict; the caller decides whether to
                // redeliver or treat as already-satisfied.
                tracing::warn!(
                    %execution_id,
                    expected_version = repo_version,
                    actual_version = actual,
                    "satisfy_signal_waits: version conflict — concurrent transition \
                     occurred inside our lease window"
                );
                Err(EngineError::CasConflict {
                    execution_id,
                    expected_version: repo_version,
                    observed_version: actual,
                    observed_status: "version_conflict".to_owned(),
                })
            },
            Err(e) => Err(EngineError::CheckpointFailed {
                node_key: final_state_node_key(),
                reason: format!("satisfy_signal_waits: store commit: {e}"),
            }),
        }
    }

    /// Durably terminalize the dangling non-terminal nodes of a cancelled,
    /// no-live-runner execution (e.g. a signal-`Paused` execution).
    ///
    /// The API cancel path writes the execution status `Cancelled` and enqueues
    /// `Cancel`; `dispatch_cancel` signals the live frontier's `CancellationToken`
    /// so a running execution tears its nodes down via the loop teardown. But a
    /// `Paused` (signal-wait) execution has NO live frontier — nothing tears
    /// down its parked `Waiting` nodes — so a `Cancelled` execution is left with
    /// non-terminal nodes (terminal-execution ⇒ all-nodes-terminal invariant
    /// violation). This method closes that gap.
    ///
    /// # Lease contract
    ///
    /// Acquires the execution lease for the read-modify-write. A live runner
    /// (in-process or cross-runner) holds the lease, so a held lease means a
    /// frontier is still driving — we must NOT terminalize nodes it owns;
    /// returns [`EngineError::Leased`] so the caller defers (the live runner's
    /// own teardown, or B1 reclaim, completes the cancel). Only acts when the
    /// execution is itself `Cancelled`/`Cancelling` (the cancel was durably
    /// recorded); idempotent — a re-delivered Cancel finds all nodes terminal
    /// and returns [`CancelDanglingOutcome::NothingToCancel`].
    ///
    /// # Errors
    ///
    /// - [`EngineError::Leased`] if the lease is held by another runner.
    /// - [`EngineError::PlanningFailed`] if the row cannot be loaded/deserialised.
    /// - [`EngineError::CasConflict`] / [`EngineError::CheckpointFailed`] on a
    ///   rejected or failed durable commit.
    pub(crate) async fn cancel_dangling_nodes(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<CancelDanglingOutcome, EngineError> {
        let Some(stores) = &self.stores else {
            // Library mode (no storage) — no durable row to repair.
            return Ok(CancelDanglingOutcome::NothingToCancel);
        };

        let id = execution_id.to_string();
        let holder = self.instance_id.to_string();

        let lease_token = stores
            .execution
            .acquire_lease(scope, &id, &holder, self.lease_ttl)
            .await
            .map_err(|e| {
                EngineError::PlanningFailed(format!(
                    "cancel_dangling_nodes: acquire lease for {execution_id}: {e}"
                ))
            })?;
        let Some(lease_token) = lease_token else {
            tracing::warn!(
                %execution_id,
                %holder,
                "cancel_dangling_nodes: execution lease held by another runner; deferring \
                 (live frontier teardown or B1 reclaim will complete the cancel)"
            );
            return Err(EngineError::Leased {
                execution_id,
                holder,
            });
        };

        let outcome = self
            .cancel_dangling_nodes_under_lease(stores, scope, execution_id, &id, lease_token)
            .await;

        if let Err(e) = stores
            .execution
            .release_lease(scope, &id, lease_token)
            .await
        {
            tracing::warn!(
                %execution_id,
                error = %e,
                "cancel_dangling_nodes: best-effort lease release failed (will expire at TTL)"
            );
        }

        outcome
    }

    /// Inner read-modify-write under an already-held lease (extracted so the
    /// lease release in [`Self::cancel_dangling_nodes`] always runs).
    async fn cancel_dangling_nodes_under_lease(
        &self,
        stores: &crate::store_seam::ExecutionStores,
        scope: &Scope,
        execution_id: ExecutionId,
        id: &str,
        lease_token: nebula_storage_port::FencingToken,
    ) -> Result<CancelDanglingOutcome, EngineError> {
        let record = stores
            .execution
            .get(scope, id)
            .await
            .map_err(|e| {
                EngineError::PlanningFailed(format!(
                    "cancel_dangling_nodes: load execution {execution_id}: {e}"
                ))
            })?
            .ok_or_else(|| {
                EngineError::PlanningFailed(format!(
                    "cancel_dangling_nodes: execution not found: {execution_id}"
                ))
            })?;

        let repo_version = record.version;
        let state_str = serde_json::to_string(&record.state).map_err(|e| {
            EngineError::PlanningFailed(format!(
                "cancel_dangling_nodes: serialise state for {execution_id}: {e}"
            ))
        })?;
        let mut exec_state: ExecutionState = serde_json::from_str(&state_str).map_err(|e| {
            EngineError::PlanningFailed(format!(
                "cancel_dangling_nodes: deserialise state for {execution_id}: {e}"
            ))
        })?;

        // Nothing to clean: every node is already terminal — idempotent
        // re-delivery, OR a `Created` cold-start execution (empty `node_states`,
        // vacuously all-terminal) that a cross-runner / early Cancel reached
        // before any node was parked. Ack; there are no dangling nodes to
        // terminalize regardless of status.
        if exec_state.all_nodes_terminal() {
            return Ok(CancelDanglingOutcome::NothingToCancel);
        }

        // Dangling non-terminal nodes exist. Act only in a genuine cancel
        // context — three cases on the persisted status (read under the lease):
        //   - `Cancelled` / `Cancelling`: the cancel is durably recorded —
        //     proceed to terminalize the parked nodes (below).
        //   - any OTHER terminal status (`Completed` / `Failed` / `TimedOut`):
        //     the execution finished on a non-cancel path — anomalous with
        //     dangling nodes, but the run is over; ack.
        //   - a non-terminal, non-`Cancelling` status (`Paused` / `Running`):
        //     the cancel is NOT yet durably recorded. The API writes `Cancelled`
        //     before enqueuing `Cancel`, so a `Cancel` reaching here with parked
        //     nodes but no recorded cancel is a transient producer-ordering
        //     window — DEFER (not ack-and-silently-drop) so B1 reclaim
        //     redelivers until the status reflects the cancel.
        if !matches!(
            exec_state.status,
            ExecutionStatus::Cancelled | ExecutionStatus::Cancelling
        ) {
            if exec_state.status.is_terminal() {
                return Ok(CancelDanglingOutcome::NothingToCancel);
            }
            return Ok(CancelDanglingOutcome::StatusNotCancelled);
        }

        // `Waiting → Cancelled` (and every other non-terminal `→ Cancelled`) is
        // in the node transition table; a transition error here is a table
        // regression, surfaced not swallowed. `count > 0` (the all-terminal
        // early-return above ruled out zero).
        let count = exec_state.cancel_nonterminal_nodes().map_err(|e| {
            EngineError::PlanningFailed(format!(
                "cancel_dangling_nodes: terminalize nodes for {execution_id}: {e}"
            ))
        })?;

        // If the execution was mid-cancel (`Cancelling`), finalize it to the
        // terminal `Cancelled` in the SAME commit as the node cleanup —
        // otherwise a no-live-runner repair would ack while the execution stayed
        // non-terminal `Cancelling` forever (`Cancelling → Cancelled` is in the
        // execution transition table).
        if exec_state.status == ExecutionStatus::Cancelling {
            exec_state
                .transition_status(ExecutionStatus::Cancelled)
                .map_err(|e| {
                    EngineError::PlanningFailed(format!(
                        "cancel_dangling_nodes: finalize Cancelling→Cancelled for \
                         {execution_id}: {e}"
                    ))
                })?;
        }

        let state_json =
            serde_json::to_value(&exec_state).map_err(|e| EngineError::CheckpointFailed {
                node_key: final_state_node_key(),
                reason: format!("cancel_dangling_nodes: serialise updated state: {e}"),
            })?;
        let batch = nebula_storage_port::TransitionBatch::builder()
            .scope(scope.clone())
            .execution_id(id)
            .expected_version(repo_version)
            .fencing(lease_token)
            .new_state(state_json)
            .build()
            .map_err(|e| EngineError::CheckpointFailed {
                node_key: final_state_node_key(),
                reason: format!("cancel_dangling_nodes: build batch: {e}"),
            })?;

        match stores.execution.commit(batch).await {
            Ok(nebula_storage_port::TransitionOutcome::Applied { new_version }) => {
                tracing::info!(
                    target = "engine::wait",
                    %execution_id,
                    cancelled_count = count,
                    new_version,
                    "cancel_dangling_nodes: terminalized parked nodes of a cancelled \
                     no-live-runner execution"
                );
                // W-S3e — the no-live-runner cancel-of-parked path: this is the
                // most likely sink to hold live un-consumed tokens (a signal-parked
                // node minted one at park, then the execution was cancelled). The
                // commit above made the execution durably `Cancelled` (terminal), so
                // revoke its leftover resume tokens. POST-commit and best-effort by
                // design (same rationale as `persist_final_state_port`): mint rides
                // the batch atomically, revoke is a separate call; the crash window
                // leaves only dead rows backstopped by the FK `ON DELETE CASCADE`
                // and no-op-resume (see `nebula_storage_port::store::resume_token`
                // module docs), so a revoke failure must not fail the cancel.
                revoke_resume_tokens_best_effort(stores, scope, id).await;
                Ok(CancelDanglingOutcome::Cancelled(count))
            },
            Ok(nebula_storage_port::TransitionOutcome::FencedOut) => {
                tracing::warn!(
                    %execution_id,
                    "cancel_dangling_nodes: CAS fenced out under our own lease"
                );
                Err(EngineError::CasConflict {
                    execution_id,
                    expected_version: repo_version,
                    observed_version: repo_version,
                    observed_status: "fenced_out".to_owned(),
                })
            },
            Ok(nebula_storage_port::TransitionOutcome::VersionConflict { actual }) => {
                tracing::warn!(
                    %execution_id,
                    expected_version = repo_version,
                    actual_version = actual,
                    "cancel_dangling_nodes: version conflict inside our lease window"
                );
                Err(EngineError::CasConflict {
                    execution_id,
                    expected_version: repo_version,
                    observed_version: actual,
                    observed_status: "version_conflict".to_owned(),
                })
            },
            Err(e) => Err(EngineError::CheckpointFailed {
                node_key: final_state_node_key(),
                reason: format!("cancel_dangling_nodes: store commit: {e}"),
            }),
        }
    }
}
