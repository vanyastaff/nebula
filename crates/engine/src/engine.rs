//! Workflow execution engine.
//!
//! Executes workflows using a frontier-based approach: each node is spawned
//! as soon as all its incoming edges are resolved and at least one is activated,
//! rather than waiting for an entire topological level. This enables branching,
//! skip propagation, error routing, and conditional edges.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use dashmap::DashMap;
// TODO: ExecutionBudget moved to nebula-execution
use nebula_action::{ActionContext, ActionResult};
use nebula_core::id::{ExecutionId, NodeId, WorkflowId};
// ScopeLevel removed from ActionContext
// use nebula_core::scope::ScopeLevel;
use nebula_execution::ExecutionStatus;
use nebula_execution::context::ExecutionBudget;
use nebula_execution::plan::ExecutionPlan;
use nebula_execution::state::ExecutionState;
use nebula_metrics::naming::{
    NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS, NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL,
    NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL, NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL,
};
use nebula_runtime::ActionRuntime;
use nebula_telemetry::metrics::MetricsRegistry;
use nebula_workflow::{
    Connection, DependencyGraph, EdgeCondition, ErrorMatcher, NodeState, ResultMatcher,
    WorkflowDefinition,
};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use nebula_expression::{EvaluationContext, ExpressionEngine};
use nebula_plugin::PluginRegistry;

use crate::error::EngineError;
use crate::resolver::ParamResolver;
use crate::result::ExecutionResult;

/// The workflow execution engine.
///
/// Orchestrates end-to-end execution of workflow definitions by:
///
/// 1. Building a dependency graph from the workflow
/// 2. Executing nodes frontier-by-frontier with bounded concurrency
/// 3. Evaluating edge conditions to determine which successors to activate
/// 4. Resolving each node's input from activated predecessor outputs
/// 5. Delegating action execution to the [`ActionRuntime`]
/// 6. Tracking execution state and recording metrics
pub struct WorkflowEngine {
    runtime: Arc<ActionRuntime>,
    metrics: MetricsRegistry,
    /// Node registry for node-level metadata and versioning.
    plugin_registry: PluginRegistry,
    /// Resolves node parameters (expressions, templates, references) to JSON.
    resolver: ParamResolver,
    /// Expression engine for evaluating edge conditions.
    #[allow(dead_code)]
    expression_engine: Arc<ExpressionEngine>,
    /// Optional resource manager for providing resources to actions.
    resource_manager: Option<Arc<nebula_resource::Manager>>,
    /// Optional execution repository for persistent state storage.
    execution_repo: Option<Arc<dyn nebula_storage::ExecutionRepo>>,
}

impl WorkflowEngine {
    /// Create a new engine with the given components.
    pub fn new(runtime: Arc<ActionRuntime>, metrics: MetricsRegistry) -> Self {
        let expression_engine = Arc::new(ExpressionEngine::with_cache_size(1024));
        Self {
            runtime,
            metrics,
            plugin_registry: PluginRegistry::new(),
            resolver: ParamResolver::new(expression_engine.clone()),
            expression_engine,
            resource_manager: None,
            execution_repo: None,
        }
    }

    /// Access the node registry.
    pub fn plugin_registry(&self) -> &PluginRegistry {
        &self.plugin_registry
    }

    /// Mutable access to the node registry.
    pub fn plugin_registry_mut(&mut self) -> &mut PluginRegistry {
        &mut self.plugin_registry
    }

    /// Attach a resource manager for providing resources to actions.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_resource_manager(mut self, manager: Arc<nebula_resource::Manager>) -> Self {
        self.resource_manager = Some(manager);
        self
    }

    /// Set the execution repository for persistent state storage.
    ///
    /// When set, the engine persists execution state after creation and
    /// after each node completes (checkpoint). Without a repo, state
    /// is in-memory only (suitable for testing).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_execution_repo(mut self, repo: Arc<dyn nebula_storage::ExecutionRepo>) -> Self {
        self.execution_repo = Some(repo);
        self
    }

    /// Execute a workflow from start to finish.
    ///
    /// Builds an execution plan for validation, then processes nodes
    /// frontier-by-frontier. Each node is spawned as soon as all its
    /// incoming edges are resolved and at least one is activated,
    /// up to `budget.max_concurrent_nodes`.
    ///
    /// Entry nodes receive the workflow-level `input`. Subsequent nodes
    /// receive the output of their activated predecessors.
    pub async fn execute_workflow(
        &self,
        workflow: &WorkflowDefinition,
        input: serde_json::Value,
        budget: ExecutionBudget,
    ) -> Result<ExecutionResult, EngineError> {
        let execution_id = ExecutionId::new();
        let started = Instant::now();

        // 1. Validate workflow (reuse ExecutionPlan for validation)
        let _plan = ExecutionPlan::from_workflow(execution_id, workflow, budget.clone())
            .map_err(|e| EngineError::PlanningFailed(e.to_string()))?;

        // 2. Build dependency graph
        let graph = DependencyGraph::from_definition(workflow)
            .map_err(|e| EngineError::PlanningFailed(e.to_string()))?;

        // 3. Validate action key mappings exist for all nodes
        // 4. Initialize execution state
        let node_ids: Vec<NodeId> = workflow.nodes.iter().map(|n| n.id).collect();
        let mut exec_state = ExecutionState::new(execution_id, workflow.id, &node_ids);
        exec_state.transition_status(ExecutionStatus::Running)?;

        // 4b. Persist initial execution state
        let mut repo_version: u64 = 0;
        if let Some(repo) = &self.execution_repo {
            let state_json = serde_json::to_value(&exec_state)
                .map_err(|e| EngineError::PlanningFailed(format!("serialize state: {e}")))?;
            repo.create(execution_id, workflow.id, state_json)
                .await
                .map_err(|e| EngineError::PlanningFailed(format!("persist initial state: {e}")))?;
            repo_version = 1;
        }

        // 5. Create cancellation token
        let cancel_token = CancellationToken::new();

        // 6. Record start metric
        self.metrics
            .counter(NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL)
            .inc();

        // 7. Build node lookup map
        let node_map: HashMap<NodeId, &nebula_workflow::NodeDefinition> =
            workflow.nodes.iter().map(|n| (n.id, n)).collect();

        // 8. Shared output storage (concurrent access from worker tasks)
        let outputs: Arc<DashMap<NodeId, serde_json::Value>> = Arc::new(DashMap::new());
        let semaphore = Arc::new(Semaphore::new(budget.max_concurrent_nodes));

        // 9. Execute using frontier-based loop
        let error_strategy = workflow.config.error_strategy;
        let failed_node = self
            .run_frontier(
                &graph,
                &node_map,
                &outputs,
                &semaphore,
                &cancel_token,
                &mut exec_state,
                execution_id,
                workflow.id,
                &input,
                &mut repo_version,
                &budget,
                &started,
                error_strategy,
            )
            .await;

        let elapsed = started.elapsed();

        // 10. Determine final status and emit events
        let final_status = determine_final_status(&failed_node, &cancel_token);
        let _ = exec_state.transition_status(final_status);

        // Persist final execution state (best-effort)
        if let Some(repo) = &self.execution_repo
            && let Ok(state_json) = serde_json::to_value(&exec_state)
        {
            match repo
                .transition(execution_id, repo_version, state_json)
                .await
            {
                Ok(true) => { /* success */ }
                Ok(false) => {
                    tracing::warn!(
                        %execution_id,
                        "final state checkpoint CAS mismatch"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        %execution_id,
                        error = %e,
                        "final state checkpoint failed"
                    );
                }
            }
        }

        self.emit_final_event(execution_id, final_status, elapsed, &failed_node);

        // 11. Collect outputs
        let node_outputs: HashMap<NodeId, serde_json::Value> = outputs
            .iter()
            .map(|r| (*r.key(), r.value().clone()))
            .collect();

        Ok(ExecutionResult {
            execution_id,
            status: final_status,
            node_outputs,
            duration: elapsed,
        })
    }

    /// Execute all reachable nodes using a frontier-based approach.
    ///
    /// Nodes are spawned as soon as all their incoming edges have been resolved
    /// and at least one edge has been activated. This supports branching, skip
    /// propagation, and error routing.
    ///
    /// Returns `Some((node_id, error))` if a node failed without an error handler,
    /// `None` if all reachable nodes completed (or were skipped).
    #[allow(clippy::too_many_arguments)]
    async fn run_frontier(
        &self,
        graph: &DependencyGraph,
        node_map: &HashMap<NodeId, &nebula_workflow::NodeDefinition>,
        outputs: &Arc<DashMap<NodeId, serde_json::Value>>,
        semaphore: &Arc<Semaphore>,
        cancel_token: &CancellationToken,
        exec_state: &mut ExecutionState,
        execution_id: ExecutionId,
        workflow_id: WorkflowId,
        input: &serde_json::Value,
        repo_version: &mut u64,
        budget: &ExecutionBudget,
        started: &Instant,
        error_strategy: nebula_workflow::ErrorStrategy,
    ) -> Option<(NodeId, String)> {
        let total_output_bytes = Arc::new(AtomicU64::new(0));
        // Precompute how many incoming edges each node has
        let required_count: HashMap<NodeId, usize> = node_map
            .keys()
            .map(|&nid| (nid, graph.incoming_connections(nid).len()))
            .collect();

        // Track edge resolution state
        let mut activated_edges: HashMap<NodeId, HashSet<NodeId>> = HashMap::new();
        let mut resolved_edges: HashMap<NodeId, HashSet<NodeId>> = HashMap::new();

        // Queue of nodes ready to execute
        let mut ready_queue: VecDeque<NodeId> = VecDeque::new();

        // Seed with entry nodes (no incoming edges)
        for entry_id in graph.entry_nodes() {
            ready_queue.push_back(entry_id);
        }

        // In-flight tasks
        let mut join_set: JoinSet<(NodeId, Result<ActionResult<serde_json::Value>, EngineError>)> =
            JoinSet::new();

        // Main frontier loop
        loop {
            // Phase 1: Drain ready queue → spawn into join_set
            while let Some(node_id) = ready_queue.pop_front() {
                if cancel_token.is_cancelled() {
                    break;
                }

                // Check budget limits before dispatching
                if let Some(violation) = check_budget(budget, started, &total_output_bytes) {
                    cancel_token.cancel();
                    return Some((node_id, violation));
                }

                let spawned = self.spawn_node(
                    node_id,
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
                );
                if spawned {
                    continue;
                }

                // Node failed during setup (e.g., param resolution).
                let abort = handle_node_failure(
                    node_id,
                    "parameter resolution failed",
                    error_strategy,
                    graph,
                    outputs,
                    &mut activated_edges,
                    &mut resolved_edges,
                    &required_count,
                    &mut ready_queue,
                    exec_state,
                );
                if let Some(err_msg) = abort {
                    cancel_token.cancel();
                    return Some((node_id, err_msg));
                }
            }

            // Phase 2: Wait for one completion (or exit if nothing in flight)
            if join_set.is_empty() {
                break;
            }

            if cancel_token.is_cancelled() {
                while join_set.join_next().await.is_some() {}
                break;
            }

            let Some(join_result) = join_set.join_next().await else {
                break;
            };

            // Phase 3: Process the completed task
            match join_result {
                Ok((node_id, Ok(action_result))) => {
                    // Node ran and produced a result
                    mark_node_completed(exec_state, node_id);

                    // Track output size for budget enforcement
                    if let Some(output) = outputs.get(&node_id) {
                        let bytes = serde_json::to_string(output.value())
                            .map(|s| s.len() as u64)
                            .unwrap_or(0);
                        total_output_bytes.fetch_add(bytes, Ordering::Relaxed);
                    }

                    // Checkpoint: persist node output + execution state
                    self.checkpoint_node(execution_id, node_id, outputs, exec_state, repo_version)
                        .await;

                    // Evaluate outgoing edges and update frontier
                    process_outgoing_edges(
                        node_id,
                        Some(&action_result),
                        None, // not failed
                        graph,
                        &mut activated_edges,
                        &mut resolved_edges,
                        &required_count,
                        &mut ready_queue,
                        exec_state,
                    );
                }
                Ok((node_id, Err(ref err))) => {
                    // Node failed at runtime — delegate to
                    // strategy-aware handler.
                    mark_node_failed(exec_state, node_id, err);

                    let abort = handle_node_failure(
                        node_id,
                        &err.to_string(),
                        error_strategy,
                        graph,
                        outputs,
                        &mut activated_edges,
                        &mut resolved_edges,
                        &required_count,
                        &mut ready_queue,
                        exec_state,
                    );

                    // Checkpoint *after* handle_node_failure so the persisted
                    // node state reflects the final resolved state (e.g.
                    // Completed for IgnoreErrors, Failed for FailFast/Continue).
                    self.checkpoint_node(execution_id, node_id, outputs, exec_state, repo_version)
                        .await;

                    if let Some(err_msg) = abort {
                        cancel_token.cancel();
                        return Some((node_id, err_msg));
                    }
                }
                Err(join_err) => {
                    tracing::error!(?join_err, "node task panicked");
                    cancel_token.cancel();
                    return Some((NodeId::new(), join_err.to_string()));
                }
            }
        }

        None
    }

    /// Spawn a single node into the JoinSet.
    ///
    /// Returns `true` if the node was spawned, `false` if it failed during setup
    /// (e.g., param resolution error).
    #[allow(clippy::too_many_arguments)]
    fn spawn_node(
        &self,
        node_id: NodeId,
        node_map: &HashMap<NodeId, &nebula_workflow::NodeDefinition>,
        graph: &DependencyGraph,
        outputs: &Arc<DashMap<NodeId, serde_json::Value>>,
        semaphore: &Arc<Semaphore>,
        cancel_token: &CancellationToken,
        exec_state: &mut ExecutionState,
        execution_id: ExecutionId,
        workflow_id: WorkflowId,
        input: &serde_json::Value,
        activated_edges: &HashMap<NodeId, HashSet<NodeId>>,
        join_set: &mut JoinSet<(NodeId, Result<ActionResult<serde_json::Value>, EngineError>)>,
    ) -> bool {
        let Some(node_def) = node_map.get(&node_id) else {
            return false;
        };
        let action_key = node_def.action_key.as_str().to_owned();

        // Partition incoming connections into flow (to_port=None) and support (to_port=Some)
        let (node_input, support_inputs) =
            resolve_node_input_with_support(node_id, graph, outputs, input, activated_edges);

        // Resolve node parameters (expressions, templates, references)
        let action_input =
            match self
                .resolver
                .resolve(node_id, &node_def.parameters, &node_input, outputs)
            {
                Ok(Some(resolved_params)) => resolved_params,
                Ok(None) => node_input, // No parameters → use predecessor output
                Err(e) => {
                    // Mark node as failed and signal failure
                    if let Some(ns) = exec_state.node_states.get_mut(&node_id) {
                        let _ = ns.transition_to(NodeState::Ready);
                        let _ = ns.transition_to(NodeState::Running);
                        let _ = ns.transition_to(NodeState::Failed);
                        ns.error_message = Some(e.to_string());
                    }
                    return false;
                }
            };

        // Mark node as running in execution state
        if let Some(ns) = exec_state.node_states.get_mut(&node_id) {
            let _ = ns.transition_to(NodeState::Ready);
            let _ = ns.transition_to(NodeState::Running);
        }

        let runtime = self.runtime.clone();
        let cancel = cancel_token.clone();
        let sem = semaphore.clone();
        let outputs_ref = outputs.clone();

        // TODO: Restore resource provider once ResourceProvider trait is available
        // let resource_provider = self.resource_manager.as_ref().map(|mgr| {
        //     Arc::new(crate::resource::Resources::new(
        //         mgr.clone(),
        //         workflow_id.to_string(),
        //         execution_id.to_string(),
        //         cancel_token.child_token(),
        //     )) as Arc<dyn nebula_action::provider::ResourceProvider>
        // });

        join_set.spawn(
            NodeTask {
                runtime,
                cancel,
                sem,
                outputs: outputs_ref,
                execution_id,
                node_id,
                workflow_id,
                action_key,
                input: action_input,
                support_inputs,
                // resource_provider,
            }
            .run(),
        );

        true
    }

    /// Persist node output and execution state to the repository (best-effort).
    ///
    /// Silently ignores errors — checkpoint failures must not abort
    /// an otherwise healthy execution.
    async fn checkpoint_node(
        &self,
        execution_id: ExecutionId,
        node_id: NodeId,
        outputs: &Arc<DashMap<NodeId, serde_json::Value>>,
        exec_state: &ExecutionState,
        repo_version: &mut u64,
    ) {
        let Some(repo) = &self.execution_repo else {
            return;
        };

        // Save node output individually
        if let Some(output) = outputs.get(&node_id) {
            let attempt = exec_state
                .node_states
                .get(&node_id)
                .map(|ns| ns.attempt_count().max(1) as u32)
                .unwrap_or(1);
            if let Err(e) = repo
                .save_node_output(execution_id, node_id, attempt, output.value().clone())
                .await
            {
                tracing::warn!(%execution_id, %node_id, error = %e, "failed to persist node output");
            }
        }

        // Save execution state snapshot
        if let Ok(state_json) = serde_json::to_value(exec_state) {
            match repo
                .transition(execution_id, *repo_version, state_json)
                .await
            {
                Ok(true) => *repo_version += 1,
                Ok(false) => {
                    // CAS mismatch — re-read current version to recover
                    tracing::warn!(
                        %execution_id,
                        "checkpoint CAS mismatch, re-reading version"
                    );
                    if let Ok(Some((current_version, _))) = repo.get_state(execution_id).await {
                        *repo_version = current_version;
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        %execution_id,
                        error = %e,
                        "checkpoint persist failed"
                    );
                }
            }
        }
    }

    /// Record final execution metrics.
    fn emit_final_event(
        &self,
        _execution_id: ExecutionId,
        status: ExecutionStatus,
        elapsed: std::time::Duration,
        _failed_node: &Option<(NodeId, String)>,
    ) {
        match status {
            ExecutionStatus::Completed => {
                self.metrics
                    .counter(NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL)
                    .inc();
            }
            ExecutionStatus::Failed => {
                self.metrics
                    .counter(NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL)
                    .inc();
            }
            _ => {}
        }

        self.metrics
            .histogram(NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS)
            .observe(elapsed.as_secs_f64());
    }
}

/// Bundled parameters for a single node execution task.
struct NodeTask {
    runtime: Arc<ActionRuntime>,
    cancel: CancellationToken,
    sem: Arc<Semaphore>,
    outputs: Arc<DashMap<NodeId, serde_json::Value>>,
    execution_id: ExecutionId,
    node_id: NodeId,
    workflow_id: WorkflowId,
    action_key: String,
    input: serde_json::Value,
    /// Data for support input ports, keyed by port name.
    #[allow(dead_code)] // reserved for multi-input actions
    support_inputs: HashMap<String, Vec<serde_json::Value>>,
    // /// Optional resource provider for this execution.
    // resource_provider: Option<Arc<dyn nebula_action::provider::ResourceProvider>>,
}

impl NodeTask {
    /// Execute this node: acquire semaphore, check cancellation, run action.
    async fn run(self) -> (NodeId, Result<ActionResult<serde_json::Value>, EngineError>) {
        let _permit = match self.sem.acquire().await {
            Ok(permit) => permit,
            Err(_) => return (self.node_id, Err(EngineError::Cancelled)),
        };

        if self.cancel.is_cancelled() {
            return (self.node_id, Err(EngineError::Cancelled));
        }

        let action_ctx = ActionContext::new(
            self.execution_id,
            self.node_id,
            self.workflow_id,
            self.cancel.child_token(),
        );

        // TODO: support_inputs and resource_provider removed from ActionContext
        // if !self.support_inputs.is_empty() {
        //     action_ctx = action_ctx.with_support_inputs(self.support_inputs);
        // }
        // if let Some(resources) = self.resource_provider {
        //     action_ctx = action_ctx.with_resources(resources);
        // }

        let result = self
            .runtime
            .execute_action(&self.action_key, self.input, action_ctx)
            .await;

        match result {
            Ok(action_result) => {
                // Extract the primary output for downstream node input resolution.
                if let Some(output) = extract_primary_output(&action_result) {
                    self.outputs.insert(self.node_id, output);
                }
                (self.node_id, Ok(action_result))
            }
            Err(e) => (self.node_id, Err(EngineError::Runtime(e))),
        }
    }
}

// ── Edge evaluation ─────────────────────────────────────────────────────────

/// Process outgoing edges from a completed/failed/skipped node.
///
/// For each outgoing edge, evaluates whether it should activate, updates
/// the tracking maps, and checks if any target node becomes ready or should
/// be skipped.
///
/// Returns `true` if the error was handled (at least one OnError edge activated).
#[allow(clippy::too_many_arguments)]
fn process_outgoing_edges(
    source_id: NodeId,
    result: Option<&ActionResult<serde_json::Value>>,
    error_msg: Option<&str>,
    graph: &DependencyGraph,
    activated_edges: &mut HashMap<NodeId, HashSet<NodeId>>,
    resolved_edges: &mut HashMap<NodeId, HashSet<NodeId>>,
    required_count: &HashMap<NodeId, usize>,
    ready_queue: &mut VecDeque<NodeId>,
    exec_state: &mut ExecutionState,
) -> bool {
    let outgoing = graph.outgoing_connections(source_id);
    let node_failed = error_msg.is_some();
    let mut error_handled = false;

    for conn in &outgoing {
        let target = conn.to_node;
        let activate = evaluate_edge(conn, result, node_failed);

        resolved_edges.entry(target).or_default().insert(source_id);
        if activate {
            activated_edges.entry(target).or_default().insert(source_id);
            if node_failed {
                error_handled = true;
            }
        }

        // Check if target is now fully resolved
        let resolved = resolved_edges.get(&target).map_or(0, |s| s.len());
        let required = required_count.get(&target).copied().unwrap_or(0);
        let activated = activated_edges.get(&target).map_or(0, |s| s.len());

        if resolved == required {
            if activated > 0 {
                ready_queue.push_back(target);
            } else {
                propagate_skip(
                    target,
                    graph,
                    exec_state,
                    resolved_edges,
                    activated_edges,
                    required_count,
                    ready_queue,
                );
            }
        }
    }

    error_handled
}

/// Evaluate whether an edge should activate given the source node's outcome.
///
/// Rules:
/// - `Skip` results don't activate any edges
/// - Failed nodes only activate `OnError` edges
/// - `Branch` results only activate edges whose `branch_key` matches `selected`
/// - `Route` results only activate edges whose `from_port` matches `port`
/// - `MultiOutput` results only activate edges whose `from_port` is in `outputs`
/// - `Always` activates on success (not on error, not on skip)
/// - `OnResult` activates when the matcher matches
/// - `Expression` activates when the expression evaluates to truthy
/// - `OnError` activates only when the node failed
fn evaluate_edge(
    conn: &Connection,
    result: Option<&ActionResult<serde_json::Value>>,
    node_failed: bool,
) -> bool {
    // Skip results don't activate any edges
    if let Some(ActionResult::Skip { .. }) = result {
        return false;
    }

    // For failed nodes, only OnError edges can activate
    if node_failed {
        return match &conn.condition {
            EdgeCondition::OnError { matcher } => match_error_condition(matcher),
            _ => false,
        };
    }

    // Check branch_key for Branch results
    if let Some(ActionResult::Branch { selected, .. }) = result
        && let Some(ref key) = conn.branch_key
        && key != selected
    {
        return false;
    }

    // Check from_port for Route results
    if let Some(ActionResult::Route { port, .. }) = result
        && let Some(ref key) = conn.from_port
        && key != port
    {
        return false;
    }

    // Check from_port for MultiOutput results
    if let Some(ActionResult::MultiOutput {
        outputs: port_outputs,
        ..
    }) = result
        && let Some(ref key) = conn.from_port
        && !port_outputs.contains_key(key)
    {
        return false;
    }

    // Evaluate the edge condition
    match &conn.condition {
        EdgeCondition::Always => true,
        EdgeCondition::OnError { .. } => false, // Not failed, so OnError doesn't activate
        EdgeCondition::OnResult { matcher } => match_result_condition(matcher, result),
        EdgeCondition::Expression { expr } => evaluate_expression_condition(expr, result),
        _ => false,
    }
}

/// Check if an OnError condition matches (for failed nodes).
fn match_error_condition(matcher: &ErrorMatcher) -> bool {
    match matcher {
        ErrorMatcher::Any => true,
        // For Code and Expression matchers, default to matching for now.
        // Full implementation would check error codes or evaluate expressions.
        ErrorMatcher::Code { .. } | ErrorMatcher::Expression { .. } => true,
        _ => false,
    }
}

/// Check if an OnResult condition matches the node's output.
fn match_result_condition(
    matcher: &ResultMatcher,
    result: Option<&ActionResult<serde_json::Value>>,
) -> bool {
    match matcher {
        ResultMatcher::Success => true,
        ResultMatcher::FieldEquals { field, value } => {
            let output = result.and_then(extract_primary_output);
            output
                .as_ref()
                .and_then(|v| v.get(field))
                .is_some_and(|v| v == value)
        }
        // Expression-based result matching; default to true for now.
        ResultMatcher::Expression { .. } => true,
        _ => false,
    }
}

/// Evaluate an Expression edge condition against the node's output.
fn evaluate_expression_condition(
    expr: &str,
    result: Option<&ActionResult<serde_json::Value>>,
) -> bool {
    // Build a minimal expression engine for evaluation.
    // In production, this should be shared; here we use a lightweight approach.
    let engine = ExpressionEngine::new();
    let mut ctx = EvaluationContext::new();
    if let Some(output) = result.and_then(extract_primary_output) {
        ctx.set_input(output);
    }
    match engine.evaluate(expr, &ctx) {
        Ok(value) => value.as_bool().unwrap_or(false),
        Err(_) => false,
    }
}

/// Recursively mark a node and its unreachable successors as skipped.
fn propagate_skip(
    node_id: NodeId,
    graph: &DependencyGraph,
    exec_state: &mut ExecutionState,
    resolved_edges: &mut HashMap<NodeId, HashSet<NodeId>>,
    activated_edges: &HashMap<NodeId, HashSet<NodeId>>,
    required_count: &HashMap<NodeId, usize>,
    ready_queue: &mut VecDeque<NodeId>,
) {
    // Guard against double-processing
    if let Some(ns) = exec_state.node_states.get(&node_id)
        && ns.state.is_terminal()
    {
        return;
    }

    if let Some(ns) = exec_state.node_states.get_mut(&node_id) {
        let _ = ns.transition_to(NodeState::Skipped);
    }

    // Mark all outgoing edges as resolved (dead) for their targets
    for conn in graph.outgoing_connections(node_id) {
        let target = conn.to_node;
        resolved_edges.entry(target).or_default().insert(node_id);

        let resolved = resolved_edges.get(&target).map_or(0, |s| s.len());
        let required = required_count.get(&target).copied().unwrap_or(0);
        let activated = activated_edges.get(&target).map_or(0, |s| s.len());

        if resolved == required {
            if activated > 0 {
                ready_queue.push_back(target);
            } else {
                propagate_skip(
                    target,
                    graph,
                    exec_state,
                    resolved_edges,
                    activated_edges,
                    required_count,
                    ready_queue,
                );
            }
        }
    }
}

// ── Node state helpers ──────────────────────────────────────────────────────

/// Check whether any budget limit has been exceeded.
///
/// Returns `Some(reason)` if a limit is exceeded, `None` otherwise.
fn check_budget(
    budget: &ExecutionBudget,
    started: &Instant,
    total_output_bytes: &AtomicU64,
) -> Option<String> {
    if let Some(max_dur) = budget.max_duration
        && started.elapsed() > max_dur
    {
        return Some("execution budget exceeded: max_duration".into());
    }
    if let Some(max_bytes) = budget.max_output_bytes
        && total_output_bytes.load(Ordering::Relaxed) > max_bytes
    {
        return Some("execution budget exceeded: max_output_bytes".into());
    }
    None
}

/// Handle a node failure according to the configured error strategy.
///
/// Returns `Some(error_message)` when the caller should cancel + return
/// (i.e., fail-fast), or `None` when execution may continue.
#[allow(clippy::too_many_arguments)]
fn handle_node_failure(
    node_id: NodeId,
    error_msg: &str,
    error_strategy: nebula_workflow::ErrorStrategy,
    graph: &DependencyGraph,
    outputs: &Arc<DashMap<NodeId, serde_json::Value>>,
    activated_edges: &mut HashMap<NodeId, HashSet<NodeId>>,
    resolved_edges: &mut HashMap<NodeId, HashSet<NodeId>>,
    required_count: &HashMap<NodeId, usize>,
    ready_queue: &mut VecDeque<NodeId>,
    exec_state: &mut ExecutionState,
) -> Option<String> {
    // IgnoreErrors: treat the failure as a successful null result so
    // downstream nodes activate normally.
    if error_strategy == nebula_workflow::ErrorStrategy::IgnoreErrors {
        // The node was already marked Failed by the caller; recover it to
        // Completed since we are ignoring the error, keeping state consistent.
        if let Some(ns) = exec_state.node_states.get_mut(&node_id) {
            ns.state = NodeState::Completed;
            ns.error_message = None;
        }
        outputs.insert(node_id, serde_json::json!(null));
        process_outgoing_edges(
            node_id,
            Some(&ActionResult::success(serde_json::json!(null))),
            None,
            graph,
            activated_edges,
            resolved_edges,
            required_count,
            ready_queue,
            exec_state,
        );
        return None;
    }

    // For FailFast / ContinueOnError: evaluate edges as a failure to
    // check for OnError handlers.
    let error_handled = process_outgoing_edges(
        node_id,
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
        // Store error info for the OnError handler's input.
        outputs.insert(
            node_id,
            serde_json::json!({
                "error": error_msg,
                "node_id": node_id.to_string(),
            }),
        );
        return None;
    }

    match error_strategy {
        nebula_workflow::ErrorStrategy::ContinueOnError => {
            // Edges already resolved (not activated) above — dependents
            // will be skipped; unaffected branches continue.
            None
        }
        // FailFast and future variants
        _ => Some(error_msg.to_owned()),
    }
}

/// Mark a node as completed in the execution state.
fn mark_node_completed(exec_state: &mut ExecutionState, node_id: NodeId) {
    if let Some(ns) = exec_state.node_states.get_mut(&node_id) {
        let _ = ns.transition_to(NodeState::Completed);
    }
}

/// Mark a node as failed in the execution state.
fn mark_node_failed(exec_state: &mut ExecutionState, node_id: NodeId, err: &EngineError) {
    if let Some(ns) = exec_state.node_states.get_mut(&node_id) {
        let _ = ns.transition_to(NodeState::Failed);
        ns.error_message = Some(err.to_string());
    }
}

/// Determine the final execution status.
fn determine_final_status(
    failed_node: &Option<(NodeId, String)>,
    cancel_token: &CancellationToken,
) -> ExecutionStatus {
    if failed_node.is_some() {
        ExecutionStatus::Failed
    } else if cancel_token.is_cancelled() {
        ExecutionStatus::Cancelled
    } else {
        ExecutionStatus::Completed
    }
}

// ── Input resolution ────────────────────────────────────────────────────────

/// Resolve node input, partitioning by `to_port` into flow input and support inputs.
///
/// Connections with `to_port = None` feed the main flow input (same as before).
/// Connections with `to_port = Some(port_name)` are collected into a per-port
/// map of values, delivered to the action via `ActionContext::support_inputs`.
fn resolve_node_input_with_support(
    node_id: NodeId,
    graph: &DependencyGraph,
    outputs: &DashMap<NodeId, serde_json::Value>,
    workflow_input: &serde_json::Value,
    activated_edges: &HashMap<NodeId, HashSet<NodeId>>,
) -> (serde_json::Value, HashMap<String, Vec<serde_json::Value>>) {
    let activated: HashSet<NodeId> = activated_edges.get(&node_id).cloned().unwrap_or_default();

    // Partition incoming connections by to_port
    let incoming = graph.incoming_connections(node_id);
    let mut flow_predecessors: Vec<NodeId> = Vec::new();
    let mut support_inputs: HashMap<String, Vec<serde_json::Value>> = HashMap::new();

    for conn in &incoming {
        let source = conn.from_node;
        if !activated.contains(&source) {
            continue;
        }
        match &conn.to_port {
            None => {
                if !flow_predecessors.contains(&source) {
                    flow_predecessors.push(source);
                }
            }
            Some(port_name) => {
                if let Some(output) = outputs.get(&source) {
                    support_inputs
                        .entry(port_name.clone())
                        .or_default()
                        .push(output.value().clone());
                }
            }
        }
    }

    // Resolve main flow input from flow predecessors
    let flow_input = if flow_predecessors.is_empty() {
        // No flow predecessors — use workflow-level input (entry node) or Null
        if activated.is_empty() {
            workflow_input.clone()
        } else {
            serde_json::Value::Null
        }
    } else if flow_predecessors.len() == 1 {
        outputs
            .get(&flow_predecessors[0])
            .map(|v| v.value().clone())
            .unwrap_or(serde_json::Value::Null)
    } else {
        let mut merged = serde_json::Map::new();
        for pred_id in &flow_predecessors {
            if let Some(output) = outputs.get(pred_id) {
                merged.insert(pred_id.to_string(), output.value().clone());
            }
        }
        serde_json::Value::Object(merged)
    };

    (flow_input, support_inputs)
}

/// Extract the primary output value from an ActionResult for downstream input resolution.
fn extract_primary_output(result: &ActionResult<serde_json::Value>) -> Option<serde_json::Value> {
    match result {
        ActionResult::Success { output } => output.as_value().cloned(),
        ActionResult::Skip { output, .. } => output.as_ref().and_then(|o| o.as_value().cloned()),
        ActionResult::Continue { output, .. } => output.as_value().cloned(),
        ActionResult::Break { output, .. } => output.as_value().cloned(),
        ActionResult::Branch { output, .. } => output.as_value().cloned(),
        ActionResult::Route { data, .. } => data.as_value().cloned(),
        ActionResult::MultiOutput { main_output, .. } => {
            main_output.as_ref().and_then(|o| o.as_value().cloned())
        }
        ActionResult::Wait { partial_output, .. } => {
            partial_output.as_ref().and_then(|o| o.as_value().cloned())
        }
        ActionResult::Retry { .. } => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_action::ActionError;
    use nebula_action::InternalHandler;
    use nebula_action::metadata::ActionMetadata;
    use nebula_action::result::ActionResult;
    use nebula_action::{ActionContext, TriggerContext};
    use nebula_core::Version;
    use nebula_core::action_key;
    use nebula_runtime::DataPassingPolicy;
    use nebula_runtime::registry::ActionRegistry;
    use nebula_runtime::{ActionExecutor, InProcessSandbox};
    use nebula_storage::ExecutionRepo;
    use nebula_workflow::{
        Connection, ErrorStrategy, NodeDefinition, WorkflowConfig, WorkflowDefinition,
    };
    use std::time::Duration;

    // -- Test handlers --

    struct EchoHandler {
        meta: ActionMetadata,
    }

    #[async_trait::async_trait]
    impl InternalHandler for EchoHandler {
        async fn execute(
            &self,
            input: serde_json::Value,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            Ok(ActionResult::success(input))
        }
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    struct FailHandler {
        meta: ActionMetadata,
    }

    #[async_trait::async_trait]
    impl InternalHandler for FailHandler {
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            Err(ActionError::fatal("intentional failure"))
        }
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    struct SlowHandler {
        meta: ActionMetadata,
        delay: Duration,
    }

    #[async_trait::async_trait]
    impl InternalHandler for SlowHandler {
        async fn execute(
            &self,
            input: serde_json::Value,
            ctx: &ActionContext,
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            tokio::select! {
                () = tokio::time::sleep(self.delay) => Ok(ActionResult::success(input)),
                () = ctx.cancellation.cancelled() => Err(ActionError::Cancelled),
            }
        }
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    // -- Helpers --

    fn make_workflow(
        nodes: Vec<NodeDefinition>,
        connections: Vec<Connection>,
    ) -> WorkflowDefinition {
        let now = chrono::Utc::now();
        WorkflowDefinition {
            id: WorkflowId::new(),
            name: "test".into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes,
            connections,
            variables: HashMap::new(),
            config: WorkflowConfig::default(),
            trigger: None,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            owner_id: None,
            ui_metadata: None,
            schema_version: 1,
        }
    }

    fn make_workflow_with_config(
        nodes: Vec<NodeDefinition>,
        connections: Vec<Connection>,
        config: WorkflowConfig,
    ) -> WorkflowDefinition {
        let now = chrono::Utc::now();
        WorkflowDefinition {
            id: WorkflowId::new(),
            name: "test".into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes,
            connections,
            variables: HashMap::new(),
            config,
            trigger: None,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            owner_id: None,
            ui_metadata: None,
            schema_version: 1,
        }
    }

    fn make_engine(registry: Arc<ActionRegistry>) -> (WorkflowEngine, MetricsRegistry) {
        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let sandbox = Arc::new(InProcessSandbox::new(executor));
        let metrics = MetricsRegistry::new();

        let runtime = Arc::new(ActionRuntime::new(
            registry,
            sandbox,
            DataPassingPolicy::default(),
            metrics.clone(),
        ));

        let engine = WorkflowEngine::new(runtime, metrics.clone());
        (engine, metrics)
    }

    // -- Tests --

    #[tokio::test]
    async fn single_node_workflow() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        }));

        let (engine, _) = make_engine(registry);

        let n = NodeId::new();
        let wf = make_workflow(
            vec![NodeDefinition::new(n, "echo", "echo").unwrap()],
            vec![],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("hello"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(result.node_output(n), Some(&serde_json::json!("hello")));
    }

    #[tokio::test]
    async fn linear_two_node_workflow() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        }));

        let (engine, _) = make_engine(registry);

        let n1 = NodeId::new();
        let n2 = NodeId::new();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(n1, "A", "echo").unwrap(),
                NodeDefinition::new(n2, "B", "echo").unwrap(),
            ],
            vec![Connection::new(n1, n2)],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!(42), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(result.node_output(n1), Some(&serde_json::json!(42)));
        // B echoes its input, which is A's output (42)
        assert_eq!(result.node_output(n2), Some(&serde_json::json!(42)));
    }

    #[tokio::test]
    async fn diamond_workflow() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        }));

        let (engine, _) = make_engine(registry);

        let a = NodeId::new();
        let b = NodeId::new();
        let c = NodeId::new();
        let d = NodeId::new();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a, "A", "echo").unwrap(),
                NodeDefinition::new(b, "B", "echo").unwrap(),
                NodeDefinition::new(c, "C", "echo").unwrap(),
                NodeDefinition::new(d, "D", "echo").unwrap(),
            ],
            vec![
                Connection::new(a, b),
                Connection::new(a, c),
                Connection::new(b, d),
                Connection::new(c, d),
            ],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("start"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(result.node_outputs.len(), 4);
        assert_eq!(result.node_output(a), Some(&serde_json::json!("start")));
        assert_eq!(result.node_output(b), Some(&serde_json::json!("start")));
        assert_eq!(result.node_output(c), Some(&serde_json::json!("start")));
        // Join node gets merged outputs from b and c
        let d_output = result.node_output(d).unwrap();
        assert!(d_output.is_object());
    }

    #[tokio::test]
    async fn failing_node_stops_execution() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        }));
        registry.register(Arc::new(FailHandler {
            meta: ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        }));

        let (engine, _) = make_engine(registry);

        let n1 = NodeId::new();
        let n2 = NodeId::new();
        let n3 = NodeId::new();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(n1, "A", "echo").unwrap(),
                NodeDefinition::new(n2, "B", "fail").unwrap(),
                NodeDefinition::new(n3, "C", "echo").unwrap(),
            ],
            vec![Connection::new(n1, n2), Connection::new(n2, n3)],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("input"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_failure());
        assert!(result.node_output(n1).is_some());
        assert!(result.node_output(n2).is_none());
        assert!(result.node_output(n3).is_none());
    }

    #[tokio::test]
    async fn missing_action_key_returns_error() {
        let registry = Arc::new(ActionRegistry::new());
        let (engine, _) = make_engine(registry);

        let n = NodeId::new();
        let wf = make_workflow(
            vec![NodeDefinition::new(n, "A", "unknown").unwrap()],
            vec![],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await
            .expect("engine returns Ok even when a node fails");

        // When action key is not in registry, the node fails and execution result is failure
        assert!(!result.is_success());
    }

    #[tokio::test]
    async fn empty_workflow_returns_planning_error() {
        let registry = Arc::new(ActionRegistry::new());
        let (engine, _) = make_engine(registry);

        let wf = make_workflow(vec![], vec![]);
        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await;

        assert!(matches!(result, Err(EngineError::PlanningFailed(_))));
    }

    #[tokio::test]
    async fn telemetry_events_emitted() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        }));

        let (engine, metrics) = make_engine(registry);

        let n = NodeId::new();
        let wf = make_workflow(
            vec![NodeDefinition::new(n, "echo", "echo").unwrap()],
            vec![],
        );

        engine
            .execute_workflow(&wf, serde_json::json!("test"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(
            metrics
                .counter(NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL)
                .get()
                > 0
        );
        assert!(
            metrics
                .counter(NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL)
                .get()
                > 0
        );
    }

    #[tokio::test]
    async fn metrics_recorded_on_failure() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(FailHandler {
            meta: ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        }));

        let (engine, metrics) = make_engine(registry);

        let n = NodeId::new();
        let wf = make_workflow(
            vec![NodeDefinition::new(n, "fail", "fail").unwrap()],
            vec![],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_failure());
        assert!(
            metrics
                .counter(NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL)
                .get()
                > 0
        );
        assert!(
            metrics
                .counter(NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL)
                .get()
                > 0
        );
    }

    #[tokio::test]
    async fn trigger_context_construction_is_usable_in_engine() {
        let ctx = TriggerContext::new(
            WorkflowId::new(),
            NodeId::new(),
            tokio_util::sync::CancellationToken::new(),
        );
        assert!(!ctx.has_credential("missing").await);
        assert!(
            ctx.schedule_after(std::time::Duration::from_millis(1))
                .await
                .is_err()
        );
        assert!(
            ctx.emit_execution(serde_json::json!({"event":"tick"}))
                .await
                .is_err()
        );
    }

    // -- Frontier-specific test handlers --

    struct SkipHandler {
        meta: ActionMetadata,
    }

    #[async_trait::async_trait]
    impl InternalHandler for SkipHandler {
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            Ok(ActionResult::skip("skipped by test"))
        }
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    struct BranchHandler {
        meta: ActionMetadata,
        selected: String,
    }

    #[async_trait::async_trait]
    impl InternalHandler for BranchHandler {
        async fn execute(
            &self,
            input: serde_json::Value,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            Ok(ActionResult::Branch {
                selected: self.selected.clone(),
                output: nebula_action::output::ActionOutput::Value(input),
                alternatives: std::collections::HashMap::new(),
            })
        }
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    // -- Frontier-specific tests --

    /// A → Branch(selects "true") → B (branch_key="true") / C (branch_key="false") → D
    /// Only B should execute; C should be skipped; D should still run (via B).
    #[tokio::test]
    async fn branch_workflow_only_selected_path_executes() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        }));
        registry.register(Arc::new(BranchHandler {
            meta: ActionMetadata::new(action_key!("branch"), "Branch", "branches"),
            selected: "true".into(),
        }));

        let (engine, _) = make_engine(registry);

        let a = NodeId::new();
        let b = NodeId::new();
        let c = NodeId::new();
        let d = NodeId::new();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a, "A", "branch").unwrap(),
                NodeDefinition::new(b, "B", "echo").unwrap(),
                NodeDefinition::new(c, "C", "echo").unwrap(),
                NodeDefinition::new(d, "D", "echo").unwrap(),
            ],
            vec![
                Connection::new(a, b).with_branch_key("true"),
                Connection::new(a, c).with_branch_key("false"),
                Connection::new(b, d),
                Connection::new(c, d),
            ],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("input"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_success());
        // A executed (branch node)
        assert!(result.node_output(a).is_some());
        // B executed (true branch)
        assert!(result.node_output(b).is_some());
        // C was NOT executed (false branch, skipped)
        assert!(result.node_output(c).is_none());
        // D executed (received input from B only)
        assert!(result.node_output(d).is_some());
    }

    /// A → B(skip) → C. Verify C is skipped and doesn't execute.
    #[tokio::test]
    async fn skip_propagation() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        }));
        registry.register(Arc::new(SkipHandler {
            meta: ActionMetadata::new(action_key!("skip"), "Skip", "always skips"),
        }));

        let (engine, _) = make_engine(registry);

        let a = NodeId::new();
        let b = NodeId::new();
        let c = NodeId::new();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a, "A", "echo").unwrap(),
                NodeDefinition::new(b, "B", "skip").unwrap(),
                NodeDefinition::new(c, "C", "echo").unwrap(),
            ],
            vec![Connection::new(a, b), Connection::new(b, c)],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("input"), ExecutionBudget::default())
            .await
            .unwrap();

        // Execution succeeds overall (skip is not a failure)
        assert!(result.is_success());
        // A executed
        assert!(result.node_output(a).is_some());
        // B executed but produced Skip result (no output stored since skip has no output)
        assert!(result.node_output(b).is_none());
        // C was skipped (never executed)
        assert!(result.node_output(c).is_none());
    }

    /// A → B(fails) --OnError--> C. Verify C receives error data and execution succeeds.
    #[tokio::test]
    async fn error_routing_with_handler() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        }));
        registry.register(Arc::new(FailHandler {
            meta: ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        }));

        let (engine, _) = make_engine(registry);

        let a = NodeId::new();
        let b = NodeId::new();
        let c = NodeId::new();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a, "A", "echo").unwrap(),
                NodeDefinition::new(b, "B", "fail").unwrap(),
                NodeDefinition::new(c, "C", "echo").unwrap(),
            ],
            vec![
                Connection::new(a, b),
                Connection::new(b, c).with_condition(EdgeCondition::OnError {
                    matcher: ErrorMatcher::Any,
                }),
            ],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("input"), ExecutionBudget::default())
            .await
            .unwrap();

        // Execution succeeds because the error was handled
        assert!(result.is_success());
        // A executed
        assert!(result.node_output(a).is_some());
        // B failed but error data was stored
        assert!(result.node_output(b).is_some());
        // C executed with error data from B
        let c_output = result.node_output(c).unwrap();
        assert!(c_output.get("error").is_some());
    }

    /// A → B(fails) → C (Always). No OnError handler → fail-fast (same as today).
    #[tokio::test]
    async fn error_without_handler_fails_fast() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        }));
        registry.register(Arc::new(FailHandler {
            meta: ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        }));

        let (engine, _) = make_engine(registry);

        let a = NodeId::new();
        let b = NodeId::new();
        let c = NodeId::new();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a, "A", "echo").unwrap(),
                NodeDefinition::new(b, "B", "fail").unwrap(),
                NodeDefinition::new(c, "C", "echo").unwrap(),
            ],
            vec![Connection::new(a, b), Connection::new(b, c)],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("input"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_failure());
        assert!(result.node_output(a).is_some());
        // B failed, no error handler → fail-fast
        assert!(result.node_output(c).is_none());
    }

    /// A → B with OnResult(Success) condition. B should run when A succeeds.
    #[tokio::test]
    async fn conditional_edge_on_result() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        }));

        let (engine, _) = make_engine(registry);

        let a = NodeId::new();
        let b = NodeId::new();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a, "A", "echo").unwrap(),
                NodeDefinition::new(b, "B", "echo").unwrap(),
            ],
            vec![
                Connection::new(a, b).with_condition(EdgeCondition::OnResult {
                    matcher: ResultMatcher::Success,
                }),
            ],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("hello"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(result.node_output(a), Some(&serde_json::json!("hello")));
        assert_eq!(result.node_output(b), Some(&serde_json::json!("hello")));
    }

    /// Diamond with mixed conditions:
    /// A → B (Always), A → C (OnResult{Success}), B → D, C → D
    /// All should execute when A succeeds.
    #[tokio::test]
    async fn diamond_with_mixed_conditions() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        }));

        let (engine, _) = make_engine(registry);

        let a = NodeId::new();
        let b = NodeId::new();
        let c = NodeId::new();
        let d = NodeId::new();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a, "A", "echo").unwrap(),
                NodeDefinition::new(b, "B", "echo").unwrap(),
                NodeDefinition::new(c, "C", "echo").unwrap(),
                NodeDefinition::new(d, "D", "echo").unwrap(),
            ],
            vec![
                Connection::new(a, b), // Always
                Connection::new(a, c).with_condition(EdgeCondition::OnResult {
                    matcher: ResultMatcher::Success,
                }),
                Connection::new(b, d),
                Connection::new(c, d),
            ],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("start"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(result.node_outputs.len(), 4);
        assert!(result.node_output(a).is_some());
        assert!(result.node_output(b).is_some());
        assert!(result.node_output(c).is_some());
        // D should have merged input from B and C
        let d_output = result.node_output(d).unwrap();
        assert!(d_output.is_object());
    }

    // -- ExecutionRepo persistence tests --

    #[tokio::test]
    async fn persists_execution_state_on_success() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        }));

        let repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let (engine, _) = make_engine(registry);
        let engine = engine.with_execution_repo(repo.clone());

        let n = NodeId::new();
        let wf = make_workflow(
            vec![NodeDefinition::new(n, "echo", "echo").unwrap()],
            vec![],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("hello"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_success());

        // Verify state was persisted
        let entry = repo.get_state(result.execution_id).await.unwrap();
        assert!(entry.is_some(), "execution state should be persisted");
        let (version, state) = entry.unwrap();
        assert!(version >= 2, "repo version should have been bumped");
        assert_eq!(
            state.get("status").and_then(|s| s.as_str()),
            Some("completed")
        );

        // Verify node output was saved
        let node_output = repo.load_node_output(result.execution_id, n).await.unwrap();
        assert_eq!(node_output, Some(serde_json::json!("hello")));
    }

    #[tokio::test]
    async fn persists_execution_state_on_failure() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(FailHandler {
            meta: ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        }));

        let repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let (engine, _) = make_engine(registry);
        let engine = engine.with_execution_repo(repo.clone());

        let n = NodeId::new();
        let wf = make_workflow(
            vec![NodeDefinition::new(n, "fail", "fail").unwrap()],
            vec![],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_failure());

        // Verify final state was persisted as failed
        let entry = repo.get_state(result.execution_id).await.unwrap();
        assert!(entry.is_some(), "execution state should be persisted");
        let (_version, state) = entry.unwrap();
        assert_eq!(state.get("status").and_then(|s| s.as_str()), Some("failed"));
    }

    #[tokio::test]
    async fn persists_node_outputs_for_multi_node_workflow() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        }));

        let repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
        let (engine, _) = make_engine(registry);
        let engine = engine.with_execution_repo(repo.clone());

        let n1 = NodeId::new();
        let n2 = NodeId::new();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(n1, "A", "echo").unwrap(),
                NodeDefinition::new(n2, "B", "echo").unwrap(),
            ],
            vec![Connection::new(n1, n2)],
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!(42), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_success());

        // Both node outputs should be persisted
        let all_outputs = repo.load_all_outputs(result.execution_id).await.unwrap();
        assert_eq!(all_outputs.len(), 2);
        assert_eq!(all_outputs[&n1], serde_json::json!(42));
        assert_eq!(all_outputs[&n2], serde_json::json!(42));
    }

    // -- Budget enforcement tests --

    #[tokio::test]
    async fn budget_max_duration_exceeded() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(SlowHandler {
            meta: ActionMetadata::new(action_key!("slow"), "Slow", "sleeps"),
            delay: Duration::from_millis(100),
        }));
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        }));

        let (engine, _) = make_engine(registry);

        // Slow → Echo. Budget allows only 1ms.
        let a = NodeId::new();
        let b = NodeId::new();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a, "Slow", "slow").unwrap(),
                NodeDefinition::new(b, "B", "echo").unwrap(),
            ],
            vec![Connection::new(a, b)],
        );

        let budget = ExecutionBudget::default().with_max_duration(Duration::from_millis(1));

        let result = engine
            .execute_workflow(&wf, serde_json::json!("data"), budget)
            .await
            .unwrap();

        // The slow action takes >1ms, so budget should trigger before
        // the next node is dispatched.
        assert!(result.is_failure());
    }

    #[tokio::test]
    async fn budget_max_output_bytes_exceeded() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        }));

        let (engine, _) = make_engine(registry);

        // A → B. Each echoes a payload. Budget allows very few bytes.
        let a = NodeId::new();
        let b = NodeId::new();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a, "A", "echo").unwrap(),
                NodeDefinition::new(b, "B", "echo").unwrap(),
            ],
            vec![Connection::new(a, b)],
        );

        // Budget: max 5 bytes of total output (the JSON "hello" is 7 bytes)
        let budget = ExecutionBudget::default().with_max_output_bytes(5);

        let result = engine
            .execute_workflow(&wf, serde_json::json!("hello"), budget)
            .await
            .unwrap();

        // A's output exceeds 5 bytes → budget violation before B runs
        assert!(result.is_failure());
    }

    // -- Error strategy tests --

    #[tokio::test]
    async fn error_strategy_continue_on_error_skips_dependents() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        }));
        registry.register(Arc::new(FailHandler {
            meta: ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        }));

        let (engine, _) = make_engine(registry);

        // Entry → [Fail, Echo(C)]
        // Fail → B
        // With ContinueOnError: Fail fails, B is skipped, C still runs.
        let entry = NodeId::new();
        let fail_node = NodeId::new();
        let b = NodeId::new();
        let c = NodeId::new();

        let mut config = WorkflowConfig::default();
        config.error_strategy = ErrorStrategy::ContinueOnError;

        let wf = make_workflow_with_config(
            vec![
                NodeDefinition::new(entry, "Entry", "echo").unwrap(),
                NodeDefinition::new(fail_node, "Fail", "fail").unwrap(),
                NodeDefinition::new(b, "B", "echo").unwrap(),
                NodeDefinition::new(c, "C", "echo").unwrap(),
            ],
            vec![
                Connection::new(entry, fail_node),
                Connection::new(entry, c),
                Connection::new(fail_node, b),
            ],
            config,
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("data"), ExecutionBudget::default())
            .await
            .unwrap();

        // Workflow completes (not fail-fast)
        assert!(result.is_success() || result.status == ExecutionStatus::Completed);
        // Entry ran
        assert!(result.node_output(entry).is_some());
        // C is independent and should have run
        assert!(result.node_output(c).is_some());
        // B depends on the failed node — should be skipped (no output)
        assert!(result.node_output(b).is_none());
    }

    #[tokio::test]
    async fn error_strategy_ignore_errors_continues_downstream() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        }));
        registry.register(Arc::new(FailHandler {
            meta: ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        }));

        let (engine, _) = make_engine(registry);

        // A(fail) → B(echo)
        // With IgnoreErrors: A fails but B should still run with null input
        let a = NodeId::new();
        let b = NodeId::new();

        let mut config = WorkflowConfig::default();
        config.error_strategy = ErrorStrategy::IgnoreErrors;

        let wf = make_workflow_with_config(
            vec![
                NodeDefinition::new(a, "A", "fail").unwrap(),
                NodeDefinition::new(b, "B", "echo").unwrap(),
            ],
            vec![Connection::new(a, b)],
            config,
        );

        let result = engine
            .execute_workflow(&wf, serde_json::json!("data"), ExecutionBudget::default())
            .await
            .unwrap();

        // Workflow should complete successfully
        assert_eq!(result.status, ExecutionStatus::Completed);
        // A's output was replaced with null
        assert_eq!(result.node_output(a), Some(&serde_json::json!(null)));
        // B ran and received null as input
        assert!(result.node_output(b).is_some());
        assert_eq!(result.node_output(b), Some(&serde_json::json!(null)));
    }
}
