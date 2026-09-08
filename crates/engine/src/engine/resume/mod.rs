//! Resume & signal-wait lifecycle.
//!
//! `resume_execution` rebuilds an incomplete execution after a process
//! restart; the `satisfy_*_signal_waits` methods deliver external signals to
//! parked nodes; the `cancel_dangling_*` methods tear down nodes left ready
//! when an execution is cancelled.

use super::*;
use nebula_core::WorkerFlavorRevisionId;

mod lease;
use lease::{ExactTurnFailure, LeasePreparation, ResumeLeaseRequest, ResumeLeaseSource};
mod recorded_execution;
use recorded_execution::ValidatedRecordedExecution;
mod recovery;
pub use recovery::{
    ClaimedStartOutcome, ClaimedStartRequest, RecoveryTurnOutcome, RecoveryTurnRequest,
};

struct LoadedExactExecution {
    repository_version: u64,
    workflow_id: WorkflowId,
    original_status: ExecutionStatus,
    state: ExecutionState,
    recorded: ValidatedRecordedExecution,
    worker_flavor_revision_id: WorkerFlavorRevisionId,
    elapsed_before_turn: Duration,
}

struct PreparedExactExecution {
    loaded_repository_version: u64,
    workflow_id: WorkflowId,
    original_status: ExecutionStatus,
    worker_flavor_revision_id: WorkerFlavorRevisionId,
    elapsed_before_turn: Duration,
    loaded: crate::revision_catalog::LoadedPlanFlavorRevision,
    workflow: nebula_plugin::ExecutableGraph,
    factories: HashMap<NodeKey, Arc<dyn nebula_action::ActionFactory>>,
    graph: DependencyGraph,
    state: ExecutionState,
    outputs: Arc<DashMap<NodeKey, serde_json::Value>>,
    semaphore: Arc<Semaphore>,
    budget: ExecutionBudget,
    seed_nodes: Vec<NodeKey>,
    activated_edges: HashMap<NodeKey, HashSet<NodeKey>>,
    resolved_edges: HashMap<NodeKey, usize>,
}

struct ExactExecutionBody<'a> {
    scope: &'a Scope,
    execution_id: ExecutionId,
    started: Instant,
    loaded: crate::revision_catalog::LoadedPlanFlavorRevision,
    workflow: nebula_plugin::ExecutableGraph,
    factories: HashMap<NodeKey, Arc<dyn nebula_action::ActionFactory>>,
    graph: DependencyGraph,
    state: ExecutionState,
    outputs: Arc<DashMap<NodeKey, serde_json::Value>>,
    semaphore: Arc<Semaphore>,
    cancel_token: CancellationToken,
    repository_version: u64,
    workflow_id: WorkflowId,
    elapsed_before_turn: Duration,
    budget: ExecutionBudget,
    seed_nodes: Vec<NodeKey>,
    activated_edges: HashMap<NodeKey, HashSet<NodeKey>>,
    resolved_edges: HashMap<NodeKey, usize>,
    lease: Option<LeaseGuard>,
}

impl WorkflowEngine {
    /// Resume an incomplete execution after process restart.
    ///
    /// Loads execution state and its exact recorded graph from storage, identifies
    /// which nodes are already complete, and re-executes from the frontier of
    /// ready-but-not-yet-executed nodes (nodes whose predecessors are all
    /// terminal but which are not yet terminal themselves).
    ///
    /// The execution lease is acquired by this call — use
    /// [`resume_execution_leased`](Self::resume_execution_leased) when the
    /// durable handoff already minted it.
    ///
    /// Persisted outputs are pre-loaded into the shared output map so that
    /// resumed nodes receive the correct predecessor data.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::PlanningFailed`] if:
    /// - `execution_repo` is not configured on this engine
    /// - The execution is not found in storage
    /// - The execution is already in a terminal state
    /// - The persisted state cannot be deserialized
    ///
    /// Exact revision failures, missing pins/configuration, unsupported recorded
    /// semantics and unresolved binding admission return typed errors before dispatch.
    pub async fn resume_execution(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<ExecutionResult, EngineError> {
        self.resume_execution_inner(scope, execution_id, ResumeLeaseSource::Acquire)
            .await
    }

    /// Resume an execution whose lease the durable handoff already minted.
    ///
    /// The orchestrator's handoff (`ExecutionTurnHandoff::accept_turn`)
    /// acknowledged the dispatch row and acquired this execution's lease in
    /// one transaction; `fence` is the token it returned. This call adopts
    /// that fence — renewing and releasing it on the same heartbeat loop the
    /// acquire path uses — instead of acquiring a second lease.
    ///
    /// # Errors
    ///
    /// Same error contract as [`resume_execution`](Self::resume_execution);
    /// additionally returns [`EngineError::PlanningFailed`] when no execution
    /// stores are configured, since a handoff fence without a storage seam is
    /// a wiring bug.
    pub async fn resume_execution_leased(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        fence: nebula_storage_port::FencingToken,
    ) -> Result<ExecutionResult, EngineError> {
        self.resume_execution_inner(scope, execution_id, ResumeLeaseSource::Adopt { fence })
            .await
    }

    async fn resume_execution_inner(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        lease_source: ResumeLeaseSource<'_>,
    ) -> Result<ExecutionResult, EngineError> {
        let result = self
            .drive_exact_execution(scope, execution_id, lease_source)
            .await
            .map_err(|failure| match failure {
                ExactTurnFailure::BeforeLease(error)
                | ExactTurnFailure::AfterLease(error)
                | ExactTurnFailure::AcceptanceUnknown(error) => error,
                ExactTurnFailure::ClaimSuperseded
                | ExactTurnFailure::CandidateSuperseded
                | ExactTurnFailure::NotReady => EngineError::Leased {
                    execution_id,
                    holder: self.instance_id.to_string(),
                },
            });
        if let Err(error) = &result {
            tracing::warn!(%execution_id, %error, "durable execution turn rejected");
            // A rejected preparation has not constructed its heartbeat guard.
            // Release only the handed-off generation; a successor's lease is untouched.
            if let ResumeLeaseSource::Adopt { fence } = lease_source
                && let Some(stores) = &self.stores
                && let Err(release_error) = stores
                    .execution
                    .release_lease(scope, &execution_id.to_string(), fence)
                    .await
            {
                tracing::warn!(%execution_id, error = %release_error,
                    "rejected durable turn lease release failed; lease expires at TTL");
            }
        }
        result
    }

    async fn load_exact_execution(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        lease_source: ResumeLeaseSource<'_>,
        turn_started_at: DateTime<Utc>,
    ) -> Result<LoadedExactExecution, ExactTurnFailure> {
        // The scoped execution-store bundle is required for resume; its absence
        // preserves the historical `execution_repo` wording asserted by callers.
        let stores = self
            .stores
            .as_ref()
            .ok_or_else(|| EngineError::PlanningFailed("no execution_repo configured".into()))?;
        let execution_key = execution_id.to_string();
        let record = stores
            .execution
            .get(scope, &execution_key)
            .await
            .map_err(|source| EngineError::ExecutionRead { source })?
            .ok_or_else(|| {
                EngineError::PlanningFailed(format!("execution not found: {execution_id}"))
            })?;
        if record.scope != *scope || record.id != execution_key {
            return Err(EngineError::InvalidRecordedExecution.into());
        }
        let workflow_id = record
            .workflow_id
            .parse()
            .map_err(|_| EngineError::InvalidRecordedExecution)?;

        checkpoint::validate_encoded_checkpoint_size(&record.state)?;
        // Deserialize via JSON bytes because domain keys expect borrowed strings.
        let encoded_state = serde_json::to_vec(&record.state)
            .map_err(|error| EngineError::PlanningFailed(format!("serialize state: {error}")))?;
        let state: ExecutionState = serde_json::from_slice(&encoded_state)
            .map_err(|_| EngineError::InvalidRecordedExecution)?;
        if state.execution_id != execution_id || state.workflow_id != workflow_id {
            return Err(EngineError::InvalidRecordedExecution.into());
        }
        let original_status = state.status;

        if matches!(lease_source, ResumeLeaseSource::Recovery(_)) {
            let is_ready_for_recovery = match state.status {
                ExecutionStatus::Created | ExecutionStatus::Running => true,
                ExecutionStatus::Paused => state.node_states.values().any(|node| {
                    matches!(node.state, NodeState::Waiting | NodeState::WaitingRetry)
                        && node
                            .next_attempt_at
                            .is_some_and(|wake| wake <= turn_started_at)
                }),
                _ => false,
            };
            if !is_ready_for_recovery {
                return Err(ExactTurnFailure::NotReady);
            }
        }

        if state.status.is_terminal() {
            return Err(EngineError::PlanningFailed(format!(
                "execution {execution_id} is already terminal ({})",
                state.status
            ))
            .into());
        }

        let recorded = self
            .validate_recorded_execution(scope, execution_id, workflow_id, &state)
            .await?;
        let worker_flavor_revision_id = state
            .worker_flavor_revision_id
            .ok_or(EngineError::MissingRevisionPins)?;
        let elapsed_before_turn = match state.started_at {
            Some(started_at) => turn_started_at
                .signed_duration_since(started_at)
                .to_std()
                .map_err(|_| EngineError::InvalidRecordedExecution)?,
            None if state.status == ExecutionStatus::Created => Duration::ZERO,
            None => return Err(EngineError::InvalidRecordedExecution.into()),
        };

        Ok(LoadedExactExecution {
            repository_version: record.version,
            workflow_id,
            original_status,
            state,
            recorded,
            worker_flavor_revision_id,
            elapsed_before_turn,
        })
    }

    async fn prepare_exact_execution(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        lease_source: ResumeLeaseSource<'_>,
        turn_started_at: DateTime<Utc>,
        exact_execution: LoadedExactExecution,
    ) -> Result<PreparedExactExecution, ExactTurnFailure> {
        let LoadedExactExecution {
            repository_version: loaded_repository_version,
            workflow_id,
            original_status,
            state,
            recorded,
            worker_flavor_revision_id,
            elapsed_before_turn,
        } = exact_execution;
        let ValidatedRecordedExecution {
            loaded,
            workflow,
            persisted_outputs,
            factories,
        } = recorded;
        let graph = DependencyGraph::from_parts(workflow.nodes(), workflow.connections())
            .map_err(|error| EngineError::PlanningFailed(error.to_string()))?;

        let mut state = state;
        if state.node_states.is_empty() {
            for node in workflow.nodes() {
                state.set_node_state(
                    node.id.clone(),
                    nebula_execution::state::NodeExecutionState::new(),
                );
            }
        }
        let interrupted_nodes: Vec<NodeKey> = state
            .node_states
            .iter()
            .filter(|(_, node_state)| {
                !node_state.state.is_terminal()
                    && node_state.state != NodeState::WaitingRetry
                    && node_state.state != NodeState::Waiting
            })
            .map(|(node_id, _)| node_id.clone())
            .collect();
        for node_id in interrupted_nodes {
            let _ = state.override_node_state(node_id, NodeState::Pending);
        }

        let outputs = Arc::new(DashMap::new());
        for (node_id, output) in persisted_outputs {
            outputs.insert(node_id, output);
        }
        let node_map: HashMap<NodeKey, &nebula_workflow::NodeDefinition> = workflow
            .nodes()
            .iter()
            .map(|node| (node.id.clone(), node))
            .collect();
        if let Err(rejection) =
            self.validate_declared_output_ports_exact(&graph, &node_map, &factories)
        {
            if matches!(
                lease_source,
                ResumeLeaseSource::ControlStart(_)
                    | ResumeLeaseSource::Recovery(_)
                    | ResumeLeaseSource::Control(_)
            ) {
                return Err(rejection.into());
            }
            let handoff_fence = match lease_source {
                ResumeLeaseSource::Adopt { fence } => Some(fence),
                ResumeLeaseSource::Acquire
                | ResumeLeaseSource::ControlStart(_)
                | ResumeLeaseSource::Recovery(_)
                | ResumeLeaseSource::Control(_) => None,
            };
            self.fail_cold_start_preflight(
                scope,
                execution_id,
                state,
                loaded_repository_version,
                &rejection,
                handoff_fence,
            )
            .await;
            return Err(rejection.into());
        }

        let mut activated_edges: HashMap<NodeKey, HashSet<NodeKey>> = HashMap::new();
        let mut resolved_edges: HashMap<NodeKey, usize> = HashMap::new();
        let mut seed_nodes = Vec::new();
        for (node_id, node_state) in &state.node_states {
            if !node_state.state.is_terminal() {
                continue;
            }
            let routing = match node_state.state {
                NodeState::Skipped
                    if state
                        .checkpoint
                        .as_ref()
                        .and_then(|checkpoint| checkpoint.nodes().get(node_id))
                        .is_some_and(|evidence| {
                            matches!(evidence, nebula_execution::NodeCheckpoint::Bypassed {})
                        }) =>
                {
                    checkpoint::CheckpointRouting::Bypass
                },
                NodeState::Skipped | NodeState::Cancelled => checkpoint::CheckpointRouting::None,
                _ => checkpoint::checkpoint_routing(
                    state
                        .checkpoint
                        .as_ref()
                        .and_then(|checkpoint| checkpoint.nodes().get(node_id))
                        .ok_or(EngineError::InvalidRecordedCheckpoint)?,
                )?,
            };
            for connection in graph.outgoing_connections(node_id.clone()) {
                let target = connection.to_node.clone();
                *resolved_edges.entry(target.clone()).or_insert(0) += 1;
                let activates_target = match &routing {
                    checkpoint::CheckpointRouting::Action(result) => {
                        evaluate_edge(connection.effective_from_port(), Some(result), false)
                    },
                    checkpoint::CheckpointRouting::Main | checkpoint::CheckpointRouting::Bypass => {
                        evaluate_edge(connection.effective_from_port(), None, false)
                    },
                    checkpoint::CheckpointRouting::Error => {
                        evaluate_edge(connection.effective_from_port(), None, true)
                    },
                    checkpoint::CheckpointRouting::None => false,
                };
                if activates_target {
                    activated_edges
                        .entry(target)
                        .or_default()
                        .insert(node_id.clone());
                }
            }
        }

        let required_edges: HashMap<_, _> = node_map
            .keys()
            .map(|node_id| {
                (
                    node_id.clone(),
                    graph.incoming_connections(node_id.clone()).len(),
                )
            })
            .collect();
        let mut newly_ready = VecDeque::new();
        let unreachable_nodes: Vec<_> = state
            .node_states
            .iter()
            .filter(|(node_id, node_state)| {
                node_state.state == NodeState::Pending
                    && required_edges.get(*node_id).is_some_and(|required| {
                        *required > 0
                            && resolved_edges.get(*node_id).copied().unwrap_or(0) == *required
                    })
                    && activated_edges.get(*node_id).is_none_or(HashSet::is_empty)
            })
            .map(|(node_id, _)| node_id.clone())
            .collect();
        for node_id in unreachable_nodes {
            propagate_skip(
                node_id,
                &graph,
                &mut state,
                &mut resolved_edges,
                &activated_edges,
                &required_edges,
                &mut newly_ready,
            );
        }
        for (node_id, node_state) in &state.node_states {
            if node_state.state.is_terminal()
                || matches!(
                    node_state.state,
                    NodeState::WaitingRetry | NodeState::Waiting
                )
            {
                continue;
            }
            let required = graph.incoming_connections(node_id.clone()).len();
            let resolved = resolved_edges.get(node_id).copied().unwrap_or(0);
            if required == 0
                || (resolved == required
                    && activated_edges
                        .get(node_id)
                        .is_some_and(|sources| !sources.is_empty()))
            {
                seed_nodes.push(node_id.clone());
            }
        }

        let mut budget = state
            .budget
            .clone()
            .ok_or(EngineError::InvalidRecordedBudget)?;
        budget
            .validate_for_execution()
            .map_err(|_| EngineError::InvalidRecordedBudget)?;
        if workflow.config().max_parallel_nodes == 0 {
            return Err(EngineError::InvalidRecordedBudget.into());
        }
        budget.max_concurrent_nodes = budget
            .max_concurrent_nodes
            .min(workflow.config().max_parallel_nodes);
        if budget.max_concurrent_nodes > Semaphore::MAX_PERMITS {
            return Err(EngineError::InvalidRecordedBudget.into());
        }
        budget.max_duration = match (budget.max_duration, workflow.config().timeout) {
            (Some(admitted), Some(recorded)) => Some(admitted.min(recorded)),
            (admitted, recorded) => admitted.or(recorded),
        };
        let semaphore = Arc::new(Semaphore::new(budget.max_concurrent_nodes));
        if matches!(lease_source, ResumeLeaseSource::Recovery(_))
            && seed_nodes.is_empty()
            && state
                .node_states
                .values()
                .any(|node| !node.state.is_terminal())
            && !state.node_states.values().any(|node| {
                matches!(node.state, NodeState::Waiting | NodeState::WaitingRetry)
                    && node
                        .next_attempt_at
                        .is_some_and(|wake| wake <= turn_started_at)
            })
        {
            return Err(ExactTurnFailure::NotReady);
        }

        Ok(PreparedExactExecution {
            loaded_repository_version,
            workflow_id,
            original_status,
            worker_flavor_revision_id,
            elapsed_before_turn,
            loaded,
            workflow,
            factories,
            graph,
            state,
            outputs,
            semaphore,
            budget,
            seed_nodes,
            activated_edges,
            resolved_edges,
        })
    }

    async fn execute_exact_execution_body(
        &self,
        body: ExactExecutionBody<'_>,
    ) -> Result<ExecutionResult, EngineError> {
        let ExactExecutionBody {
            scope,
            execution_id,
            started,
            loaded,
            workflow,
            factories,
            graph,
            mut state,
            outputs,
            semaphore,
            cancel_token,
            mut repository_version,
            workflow_id,
            elapsed_before_turn,
            budget,
            seed_nodes,
            activated_edges,
            resolved_edges,
            lease,
        } = body;
        let node_map: HashMap<NodeKey, &nebula_workflow::NodeDefinition> = workflow
            .nodes()
            .iter()
            .map(|node| (node.id.clone(), node))
            .collect();

        // Command arming preserves the original stored status. Enter Running
        // only after the command and its intent have durably committed.
        if !state.status.is_terminal() && state.status != ExecutionStatus::Running {
            let _ = state.transition_status(ExecutionStatus::Running);
        }
        let fencing = lease.as_ref().and_then(LeaseGuard::fencing_token);

        // Publish the cancel token only after this runtime owns the lease.
        let registration_id = NEXT_REGISTRATION_ID.fetch_add(1, Ordering::Relaxed);
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
        let error_strategy = workflow.config().error_strategy;
        let workflow_retry_policy = workflow.config().retry_policy.clone();
        let workflow_input = if let Some(input) = state.workflow_input.clone() {
            input
        } else {
            tracing::warn!(
                %execution_id,
                "resume: persisted execution state is missing workflow_input; \
                 falling back to Null — entry nodes that did not complete \
                 on the original run will receive Null input"
            );
            serde_json::Value::Null
        };

        // Keep the frontier borrow inside this scope so finalization can read state.
        let landed = {
            let frontier = self.run_frontier(
                scope,
                &graph,
                &node_map,
                FactoryDispatch::Frozen {
                    factories: &factories,
                    plan: loaded.plan(),
                },
                &outputs,
                &semaphore,
                &cancel_token,
                &mut resume_rx,
                &mut state,
                execution_id,
                workflow_id,
                &workflow_input,
                &mut repository_version,
                fencing,
                &budget,
                &started,
                elapsed_before_turn,
                error_strategy,
                workflow_retry_policy,
                seed_nodes,
                activated_edges,
                resolved_edges,
            );
            tokio::pin!(frontier);
            tokio::select! {
                biased;
                failed_node = &mut frontier => Some(failed_node),
                () = self.shutdown.cancelled() => {
                    tokio::time::timeout(SHUTDOWN_FRONTIER_GRACE, &mut frontier)
                        .await
                        .ok()
                }
            }
        };
        let Some(failed_node) = landed else {
            tracing::warn!(
                %execution_id,
                grace_ms = SHUTDOWN_FRONTIER_GRACE.as_millis() as u64,
                "runtime shutting down: abandoning the frontier and releasing the \
                 execution lease so a successor can take over without waiting out the TTL"
            );
            self.runtime.clear_execution_output_totals(execution_id);
            if let Some(guard) = lease {
                guard.shutdown().await;
            }
            return Err(EngineError::ShutdownInterrupted { execution_id });
        };

        self.runtime.clear_execution_output_totals(execution_id);
        let failed_node = match failed_node {
            Ok(failed_node) => failed_node,
            Err(error) => {
                if let Some(guard) = lease {
                    guard.shutdown().await;
                }
                return Err(error);
            },
        };
        let elapsed = started.elapsed();
        let heartbeat_lost = lease.as_ref().is_some_and(LeaseGuard::heartbeat_lost);
        let FinalStatusDecision {
            status: final_status,
            termination_reason,
            integrity_violation,
        } = determine_final_status(&failed_node, &cancel_token, &state);
        if final_status == ExecutionStatus::Cancelled && state.status == ExecutionStatus::Running {
            let _ = state.transition_status(ExecutionStatus::Cancelling);
        }
        let _ = state.transition_status(final_status);

        let reported_status = if heartbeat_lost {
            tracing::error!(
                %execution_id,
                "resume: final state persistence skipped: heartbeat lost this runner's lease; \
                 another runner now owns the execution"
            );
            if let Some(guard) = lease {
                guard.shutdown().await;
            }
            return Err(EngineError::Leased {
                execution_id,
                holder: self.instance_id.to_string(),
            });
        } else {
            match self
                .persist_final_state(
                    scope,
                    execution_id,
                    &state,
                    &mut repository_version,
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
                         reporting Failed instead of silently completing"
                    );
                    ExecutionStatus::Failed
                },
                Err(error) => {
                    tracing::error!(
                        %execution_id,
                        %error,
                        "resume: final state persist failed; \
                         reporting Failed instead of silently completing"
                    );
                    ExecutionStatus::Failed
                },
            }
        };

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

        let node_outputs = outputs
            .iter()
            .map(|output| (output.key().clone(), output.value().clone()))
            .collect();
        let node_errors = state
            .node_states
            .iter()
            .filter_map(|(node_id, node_state)| {
                node_state
                    .error_message
                    .as_ref()
                    .map(|message| (node_id.clone(), message.clone()))
            })
            .collect();

        Ok(ExecutionResult {
            execution_id,
            status: reported_status,
            node_outputs,
            node_errors,
            duration: elapsed,
            termination_reason,
        })
    }

    #[tracing::instrument(skip(self, scope, lease_source), fields(%execution_id))]
    async fn drive_exact_execution(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        lease_source: ResumeLeaseSource<'_>,
    ) -> Result<ExecutionResult, ExactTurnFailure> {
        let started = Instant::now();
        let turn_started_at = self.clock.now();
        let exact_execution = self
            .load_exact_execution(scope, execution_id, lease_source, turn_started_at)
            .await?;
        let prepared = self
            .prepare_exact_execution(
                scope,
                execution_id,
                lease_source,
                turn_started_at,
                exact_execution,
            )
            .await?;
        let PreparedExactExecution {
            loaded_repository_version,
            workflow_id,
            original_status,
            worker_flavor_revision_id,
            elapsed_before_turn,
            loaded,
            workflow,
            factories,
            graph,
            mut state,
            outputs,
            semaphore,
            budget,
            seed_nodes,
            activated_edges,
            resolved_edges,
        } = prepared;

        let cancel_token = CancellationToken::new();
        let mut repository_version = loaded_repository_version;
        let lease = match self
            .prepare_resume_lease(ResumeLeaseRequest {
                scope,
                execution_id,
                source: lease_source,
                cancel_token: cancel_token.clone(),
                execution_state: &mut state,
                repository_version: &mut repository_version,
                loaded_repository_version,
                worker_flavor_revision_id,
                original_status,
                outputs: &outputs,
                started,
            })
            .await?
        {
            LeasePreparation::Drive(lease) => lease,
            LeasePreparation::Completed(result) => return Ok(result),
        };

        self.execute_exact_execution_body(ExactExecutionBody {
            scope,
            execution_id,
            started,
            loaded,
            workflow,
            factories,
            graph,
            state,
            outputs,
            semaphore,
            cancel_token,
            repository_version,
            workflow_id,
            elapsed_before_turn,
            budget,
            seed_nodes,
            activated_edges,
            resolved_edges,
            lease,
        })
        .await
        .map_err(ExactTurnFailure::AfterLease)
    }

    /// Durably fail a graph-preflight rejection under the acquired/adopted fence.
    /// This is called only after the plan and frozen registry have been checked;
    /// worker/configuration incompatibility never terminalizes an execution.
    ///
    /// `resume_execution` never creates its own execution row — production's
    /// API start handler already durably persisted this row as `Created`
    /// before `resume_execution` was ever invoked. That makes this call site
    /// unique: unlike `execute_workflow_scoped` (which runs the same
    /// pre-flight *before* it creates its own row, so a rejection there
    /// requires zero teardown), a rejection here must actively transition an
    /// already-existing row, or it is orphaned in `Created` forever — nothing
    /// downstream (`EngineControlDispatch::drive`, `ControlConsumer::mark_failed`)
    /// ever marks the execution row itself, only the control-queue row.
    ///
    /// Mirrors [`nebula_execution::state::ExecutionState::mark_setup_failed`]'s
    /// record-then-fail pattern (used by the frontier loop for a single
    /// node's setup failure), applied at the execution level: the offending
    /// node is recorded as `Failed` with the rejection reason so it surfaces
    /// through the normal per-node error-message read path, then the
    /// execution status is driven `Created → Running → Failed` (direct
    /// `Created → Failed` is not a valid transition) and committed with the
    /// version this call loaded — nothing else has written to the row since.
    ///
    /// Best-effort by design: on a lease conflict or CAS race this logs and
    /// returns without transitioning. The caller still propagates the
    /// original `EngineError` regardless of whether this durable mark
    /// succeeds, so no rejection is ever silently swallowed — a row left
    /// stuck in `Created` after a failed best-effort attempt remains
    /// reachable by the existing crash-recovery reclaim path.
    ///
    /// `handoff_fence` carries the durable handoff's fence on the adopt path
    /// The handoff already holds the lease, so this function commits
    /// (and afterwards releases) under that fence instead of acquiring — a
    /// live lease blocks acquisition outright, even for the same holder, and
    /// the queue row is already acknowledged, so an acquire failure here
    /// would orphan the row in `Created` with nothing left to redeliver it.
    /// `None` is the acquire path: this function takes and releases the lease
    /// itself.
    pub(super) async fn fail_cold_start_preflight(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        mut exec_state: ExecutionState,
        repo_version: u64,
        reject: &EngineError,
        handoff_fence: Option<nebula_storage_port::FencingToken>,
    ) {
        let Some(stores) = &self.stores else {
            // Library mode (no storage) — no durable row to repair.
            return;
        };

        let id = execution_id.to_string();
        let holder = self.instance_id.to_string();

        let lease_token = if let Some(fence) = handoff_fence {
            fence
        } else {
            match stores
                .execution
                .acquire_lease(scope, &id, &holder, self.lease_ttl)
                .await
            {
                Ok(Some(token)) => token,
                Ok(None) => {
                    tracing::warn!(
                        %execution_id,
                        error = %reject,
                        "cold-start preflight rejection: execution lease held by another runner; \
                         leaving the execution row for that runner to resolve"
                    );
                    return;
                },
                Err(e) => {
                    tracing::warn!(
                        %execution_id,
                        error = %e,
                        reject = %reject,
                        "cold-start preflight rejection: failed to acquire lease; the execution row \
                         remains Created (recoverable via reclaim)"
                    );
                    return;
                },
            }
        };

        // `mark_setup_failed` and each `transition_status` call bump
        // `exec_state.version` internally (one per logical mutation — up to
        // two here: the setup-failure record and the `Failed` transition),
        // so the serialized blob's `version` can end up higher than
        // `repo_version + 1`, the row version this commit's CAS will
        // actually produce. That is expected, not a bug to normalize away:
        // `ExecutionState::version` is write-time-only bookkeeping of how
        // many logical mutations were applied to this snapshot (see the
        // per-call-site asserts in `crates/execution/src/state.rs`, e.g.
        // "must be bumped" / "must NOT bump version" on no-op paths) — no
        // reader anywhere deserializes the blob and trusts its `version`
        // against the row; every CAS (`expected_version`/`new_version`) is
        // sourced exclusively from the storage row's own counter
        // (`ExecutionRecord::version`, tracked here as `repo_version`).
        // `persist_final_state_port` and `checkpoint_node_port` commit the
        // same way — never normalizing `exec_state.version` to the row
        // version — and `satisfy_signal_waits` documents the identical
        // rationale for its own direct-mutation bump.
        if let EngineError::UndeclaredOutputPort { from_node, .. } = reject {
            let _ = exec_state.mark_setup_failed(from_node.clone(), reject.to_string());
        }
        if exec_state.status == ExecutionStatus::Created {
            let _ = exec_state.transition_status(ExecutionStatus::Running);
        }
        if !exec_state.status.is_terminal() {
            let _ = exec_state.transition_status(ExecutionStatus::Failed);
        }

        let commit_outcome = async {
            let state_json =
                serde_json::to_value(&exec_state).map_err(|e| EngineError::CheckpointFailed {
                    node_key: final_state_node_key(),
                    reason: format!("cold-start preflight rejection: serialise failed state: {e}"),
                })?;
            let batch = nebula_storage_port::TransitionBatch::builder()
                .scope(scope.clone())
                .execution_id(&id)
                .expected_version(repo_version)
                .fencing(lease_token)
                .new_state(state_json)
                .build()
                .map_err(|e| EngineError::CheckpointFailed {
                    node_key: final_state_node_key(),
                    reason: format!("cold-start preflight rejection: build batch: {e}"),
                })?;
            stores
                .execution
                .commit(batch)
                .await
                .map_err(|e| EngineError::CheckpointFailed {
                    node_key: final_state_node_key(),
                    reason: format!("cold-start preflight rejection: store commit: {e}"),
                })
        }
        .await;

        match commit_outcome {
            Ok(nebula_storage_port::TransitionOutcome::Applied { new_version }) => {
                tracing::error!(
                    %execution_id,
                    new_version,
                    error = %reject,
                    "cold-start preflight rejected the execution; marked the execution row \
                     Failed instead of leaving it orphaned in Created (W0 U2 gap fix)"
                );
                revoke_resume_tokens_best_effort(stores, scope, &id).await;
            },
            Ok(other) => {
                tracing::warn!(
                    %execution_id,
                    outcome = ?other,
                    error = %reject,
                    "cold-start preflight rejection: CAS did not apply while marking the \
                     execution row Failed; row may remain Created (recoverable via reclaim)"
                );
            },
            Err(e) => {
                tracing::warn!(
                    %execution_id,
                    error = %e,
                    reject = %reject,
                    "cold-start preflight rejection: failed to persist Failed status; the \
                     execution row remains Created (recoverable via reclaim)"
                );
            },
        }

        if let Err(e) = stores
            .execution
            .release_lease(scope, &id, lease_token)
            .await
        {
            tracing::warn!(
                %execution_id,
                error = %e,
                "cold-start preflight rejection: best-effort lease release failed (will expire \
                 at TTL)"
            );
        }
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
    /// `ExecutionStore` with a version-CAS + fencing batch (the same
    /// durability contract as `checkpoint_node`). The subsequent `drive`
    /// re-seeds the armed node into the frontier `wait_heap`; the wait drain
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
    /// # Targeting
    ///
    /// `resume_target` selects which parked signal wait this Resume arms:
    /// `Some(target)` arms only the node whose persisted [`WaitSignal`] matches
    /// the target by kind + identity (a webhook target never satisfies an
    /// approval gate — the kind-confusion safety rule); `None` arms every
    /// signal-driven wait (the untargeted behavior).
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
    /// [`Self::satisfy_signal_waits`] for the crash-recovery path.
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
    ///   the caller defers (recovery reclaim redelivers once the lease frees, or the
    ///   live owner's own resume channel handles it). This is the cross-runner /
    ///   not-yet-crashed case.
    /// - lease acquired (free, or TTL-expired ⇒ the parking runner crashed and
    ///   no owner remains) — proceed. We arm the matching signal wait(s) under
    ///   the owned lease via the SAME [`Self::satisfy_signal_waits_under_lease`]
    ///   inner the `Paused` path uses, so the version-CAS + fencing commit and
    ///   the kind-aware [`arm_signal_waits_under_lease`] targeting are identical.
    ///   A targeted recovery (`Some(resume_target)`) arms ONLY the matching
    ///   node; an untargeted one arms every signal wait. The caller then
    ///   re-drives via `drive_armed_resume`, whose wait drain completes the armed
    ///   wait on the main port.
    ///
    /// The own-the-lease-before-read-modify-write invariant is preserved:
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

        // Arm the signal-driven waits selected by `resume_target` for the wait drain:
        // completion: set each match's wake instant to `now` and LEAVE it
        // `Waiting`. The subsequent `drive` re-seeds it into the frontier
        // `wait_heap` (its `next_attempt_at` is now `Some`), and the wait drain
        // transitions it `Waiting → Completed` and activates downstream through
        // the PORT-AWARE `process_outgoing_edges` — the same path a timer wait
        // takes. Transitioning to `Completed` HERE would instead route the
        // node's edges through `resume_execution`'s port-blind rebuild, which
        // activates every outgoing edge (a multi-port wait would fire its
        // `error`/custom branch on a normal Resume).
        //
        // `arm_signal_waits_under_lease` is the shared armer: a `Some(target)`
        // Resume arms only the kind+identity match; a `None` Resume arms every
        // signal wait. It runs under the lease we hold, so the
        // own-the-lease-before-read-modify-write invariant is preserved.
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
        // the same bump the live-frontier self-arm performs.)
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
                    "satisfy_signal_waits: armed signal waits for frontier completion — \
                     drive will complete them and activate downstream on the main port"
                );
                // No `NodeWaitCompleted` is emitted here: the node is still
                // `Waiting`. The wait drain emits that event when it transitions the
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
    /// own teardown, or recovery reclaim, completes the cancel). Only acts when the
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
                 (live frontier teardown or recovery reclaim will complete the cancel)"
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

        // The run is already over — cancelled earlier (idempotent redelivery),
        // or it reached `Completed` / `Failed` / `TimedOut` before the cancel
        // landed. Either way there is nothing left to cancel; ack.
        if exec_state.status.is_terminal() {
            return Ok(CancelDanglingOutcome::NothingToCancel);
        }

        // Past this point the cancel is genuine and unfinished, and **this**
        // runner performs it — under the lease it holds, in one commit.
        //
        // It did not always work that way: the API used to write `Cancelled`
        // before enqueuing the command, so this path only tidied up nodes and
        // deferred whenever it arrived first. That made an HTTP handler the
        // writer of a terminal state the engine had not reached, over a fencing
        // token rebuilt from a read. The command row is the durable intent now,
        // and arriving here *is* the authorization to act — there is no
        // producer-ordering window left to defer for.
        //
        // `all_nodes_terminal` no longer short-circuits: a `Created` execution
        // cancelled before any node was parked has vacuously terminal nodes and
        // still needs its status moved to `Cancelled`.
        let count = if exec_state.all_nodes_terminal() {
            0
        } else {
            // `Waiting → Cancelled` (and every other non-terminal `→ Cancelled`)
            // is in the node transition table; a transition error here is a
            // table regression, surfaced not swallowed.
            exec_state.cancel_nonterminal_nodes().map_err(|e| {
                EngineError::PlanningFailed(format!(
                    "cancel_dangling_nodes: terminalize nodes for {execution_id}: {e}"
                ))
            })?
        };

        // Drive the execution itself to the terminal `Cancelled`, bridging
        // through `Cancelling` where the table requires it. `Created` goes
        // straight to `Cancelled` — a pre-start cancel must not fabricate a
        // `Running` phase it never had.
        if matches!(
            exec_state.status,
            ExecutionStatus::Running | ExecutionStatus::Paused
        ) {
            exec_state
                .transition_status(ExecutionStatus::Cancelling)
                .map_err(|e| {
                    EngineError::PlanningFailed(format!(
                        "cancel_dangling_nodes: bridge to Cancelling for {execution_id}: {e}"
                    ))
                })?;
        }
        exec_state
            .transition_status(ExecutionStatus::Cancelled)
            .map_err(|e| {
                EngineError::PlanningFailed(format!(
                    "cancel_dangling_nodes: finalize Cancelled for {execution_id}: {e}"
                ))
            })?;

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
                // In the no-live-runner cancel-of-parked path, this is the
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
                // An execution that reached a terminal state must say so,
                // whichever path terminalized it. This one used to stay silent:
                // it only tidied up nodes while the API's own write carried the
                // terminal status, so nothing here looked like an execution
                // finishing. Now that this *is* the cancel, a subscriber that
                // missed the event would see a parked execution simply stop
                // existing.
                self.emit_event(ExecutionEvent::ExecutionFinished {
                    execution_id,
                    success: false,
                    // The engine never ran a frontier here — there is no
                    // measured span to report, and inventing one would put a
                    // fabricated duration into the same stream that carries
                    // real ones.
                    elapsed: Duration::ZERO,
                    termination_reason: None,
                });
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
