//! Workflow execution engine.
//!
//! Executes workflows using a frontier-based approach: each node is spawned
//! as soon as all its incoming edges are resolved and at least one is activated,
//! rather than waiting for an entire topological level. This enables branching,
//! skip propagation, error routing, and conditional edges.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use nebula_action::ExecutionBudget;
use nebula_action::context::ActionContext;
use nebula_action::result::ActionResult;
use nebula_core::id::{ActionId, ExecutionId, NodeId, WorkflowId};
use nebula_core::scope::ScopeLevel;
use nebula_execution::ExecutionStatus;
use nebula_execution::plan::ExecutionPlan;
use nebula_execution::state::ExecutionState;
use nebula_runtime::ActionRuntime;
use nebula_telemetry::event::{EventBus, ExecutionEvent};
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
/// 6. Tracking execution state and emitting telemetry
pub struct WorkflowEngine {
    runtime: Arc<ActionRuntime>,
    event_bus: Arc<EventBus>,
    metrics: Arc<MetricsRegistry>,
    /// Maps action IDs (from node definitions) to registry keys.
    action_keys: HashMap<ActionId, String>,
    /// Node registry for node-level metadata and versioning.
    plugin_registry: PluginRegistry,
    /// Resolves node parameters (expressions, templates, references) to JSON.
    resolver: ParamResolver,
    /// Expression engine for evaluating edge conditions.
    #[allow(dead_code)]
    expression_engine: Arc<ExpressionEngine>,
}

impl WorkflowEngine {
    /// Create a new engine with the given components.
    pub fn new(
        runtime: Arc<ActionRuntime>,
        event_bus: Arc<EventBus>,
        metrics: Arc<MetricsRegistry>,
    ) -> Self {
        let expression_engine = Arc::new(ExpressionEngine::with_cache_size(1024));
        Self {
            runtime,
            event_bus,
            metrics,
            action_keys: HashMap::new(),
            plugin_registry: PluginRegistry::new(),
            resolver: ParamResolver::new(expression_engine.clone()),
            expression_engine,
        }
    }

    /// Register a mapping from an action ID to a registry key.
    ///
    /// The engine uses this to look up the correct handler in the
    /// runtime's action registry when executing a node.
    pub fn map_action(&mut self, action_id: ActionId, key: impl Into<String>) {
        self.action_keys.insert(action_id, key.into());
    }

    /// Access the node registry.
    pub fn plugin_registry(&self) -> &PluginRegistry {
        &self.plugin_registry
    }

    /// Mutable access to the node registry.
    pub fn plugin_registry_mut(&mut self) -> &mut PluginRegistry {
        &mut self.plugin_registry
    }

    /// Resolve the action registry key for a given action ID.
    fn resolve_action_key(&self, action_id: ActionId) -> Result<&str, EngineError> {
        self.action_keys
            .get(&action_id)
            .map(String::as_str)
            .ok_or(EngineError::ActionKeyNotFound { action_id })
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
        let execution_id = ExecutionId::v4();
        let started = Instant::now();

        // 1. Validate workflow (reuse ExecutionPlan for validation)
        let _plan = ExecutionPlan::from_workflow(execution_id, workflow, budget.clone())
            .map_err(|e| EngineError::PlanningFailed(e.to_string()))?;

        // 2. Build dependency graph
        let graph = DependencyGraph::from_definition(workflow)
            .map_err(|e| EngineError::PlanningFailed(e.to_string()))?;

        // 3. Validate action key mappings exist for all nodes
        for node in &workflow.nodes {
            self.resolve_action_key(node.action_id)?;
        }

        // 4. Initialize execution state
        let node_ids: Vec<NodeId> = workflow.nodes.iter().map(|n| n.id).collect();
        let mut exec_state = ExecutionState::new(execution_id, workflow.id, &node_ids);
        exec_state.transition_status(ExecutionStatus::Running)?;

        // 5. Create cancellation token
        let cancel_token = CancellationToken::new();

        // 6. Emit start event
        self.event_bus.emit(ExecutionEvent::Started {
            execution_id: execution_id.to_string(),
            workflow_id: workflow.id.to_string(),
        });
        self.metrics.counter("executions_started_total").inc();

        // 7. Build node lookup map
        let node_map: HashMap<NodeId, &nebula_workflow::NodeDefinition> =
            workflow.nodes.iter().map(|n| (n.id, n)).collect();

        // 8. Shared output storage (concurrent access from worker tasks)
        let outputs: Arc<DashMap<NodeId, serde_json::Value>> = Arc::new(DashMap::new());
        let semaphore = Arc::new(Semaphore::new(budget.max_concurrent_nodes));

        // 9. Execute using frontier-based loop
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
            )
            .await;

        let elapsed = started.elapsed();

        // 10. Determine final status and emit events
        let final_status = determine_final_status(&failed_node, &cancel_token);
        let _ = exec_state.transition_status(final_status);
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
    ) -> Option<(NodeId, String)> {
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

                // Node failed during setup (e.g., param resolution).
                // Treat as a node failure: check for error handlers.
                let has_error_handler = !spawned
                    && process_outgoing_edges(
                        node_id,
                        None,
                        Some("parameter resolution failed"),
                        graph,
                        &mut activated_edges,
                        &mut resolved_edges,
                        &required_count,
                        &mut ready_queue,
                        exec_state,
                    );
                if !spawned && !has_error_handler {
                    cancel_token.cancel();
                    return Some((node_id, "parameter resolution failed".into()));
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
                    // Node failed at runtime
                    mark_node_failed(exec_state, node_id, err);

                    // Check for error handlers
                    let error_handled = process_outgoing_edges(
                        node_id,
                        None, // no successful result
                        Some(&err.to_string()),
                        graph,
                        &mut activated_edges,
                        &mut resolved_edges,
                        &required_count,
                        &mut ready_queue,
                        exec_state,
                    );

                    if error_handled {
                        // Store error info for OnError handler input
                        outputs.insert(
                            node_id,
                            serde_json::json!({
                                "error": err.to_string(),
                                "node_id": node_id.to_string(),
                            }),
                        );
                    } else {
                        // No error handler → fail-fast
                        cancel_token.cancel();
                        return Some((node_id, err.to_string()));
                    }
                }
                Err(join_err) => {
                    tracing::error!(?join_err, "node task panicked");
                    cancel_token.cancel();
                    return Some((NodeId::v4(), join_err.to_string()));
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
        let Ok(action_key) = self.resolve_action_key(node_def.action_id) else {
            return false;
        };
        let action_key = action_key.to_owned();

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
            }
            .run(),
        );

        true
    }

    /// Emit the final execution event and record metrics.
    fn emit_final_event(
        &self,
        execution_id: ExecutionId,
        status: ExecutionStatus,
        elapsed: std::time::Duration,
        failed_node: &Option<(NodeId, String)>,
    ) {
        match status {
            ExecutionStatus::Completed => {
                self.event_bus.emit(ExecutionEvent::Completed {
                    execution_id: execution_id.to_string(),
                    duration: elapsed,
                });
                self.metrics.counter("executions_completed_total").inc();
            }
            ExecutionStatus::Failed => {
                let error_msg = failed_node
                    .as_ref()
                    .map(|(_, e)| e.clone())
                    .unwrap_or_default();
                self.event_bus.emit(ExecutionEvent::Failed {
                    execution_id: execution_id.to_string(),
                    error: error_msg,
                });
                self.metrics.counter("executions_failed_total").inc();
            }
            ExecutionStatus::Cancelled => {
                self.event_bus.emit(ExecutionEvent::Cancelled {
                    execution_id: execution_id.to_string(),
                });
            }
            _ => {}
        }

        self.metrics
            .histogram("execution_duration_seconds")
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
    support_inputs: HashMap<String, Vec<serde_json::Value>>,
}

impl NodeTask {
    /// Execute this node: acquire semaphore, check cancellation, run action.
    async fn run(self) -> (NodeId, Result<ActionResult<serde_json::Value>, EngineError>) {
        let _permit = self.sem.acquire().await.expect("semaphore closed");

        if self.cancel.is_cancelled() {
            return (self.node_id, Err(EngineError::Cancelled));
        }

        let mut action_ctx = ActionContext::new(
            self.execution_id,
            self.node_id,
            self.workflow_id,
            ScopeLevel::Global,
        )
        .with_cancellation(self.cancel.child_token());
        if !self.support_inputs.is_empty() {
            action_ctx = action_ctx.with_support_inputs(self.support_inputs);
        }

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
    }
}

/// Check if an OnError condition matches (for failed nodes).
fn match_error_condition(matcher: &ErrorMatcher) -> bool {
    match matcher {
        ErrorMatcher::Any => true,
        // For Code and Expression matchers, default to matching for now.
        // Full implementation would check error codes or evaluate expressions.
        ErrorMatcher::Code { .. } | ErrorMatcher::Expression { .. } => true,
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
    use nebula_action::ParameterCollection;
    use nebula_action::capability::IsolationLevel;
    use nebula_action::handler::InternalHandler;
    use nebula_action::metadata::{ActionMetadata, ActionType};
    use nebula_action::result::ActionResult;
    use nebula_core::Version;
    use nebula_core::id::ActionId;
    use nebula_runtime::DataPassingPolicy;
    use nebula_runtime::registry::ActionRegistry;
    use nebula_sandbox_inprocess::{ActionExecutor, InProcessSandbox};
    use nebula_workflow::{Connection, NodeDefinition, WorkflowConfig, WorkflowDefinition};

    // -- Test handlers --

    struct EchoHandler {
        meta: ActionMetadata,
    }

    #[async_trait::async_trait]
    impl InternalHandler for EchoHandler {
        async fn execute(
            &self,
            input: serde_json::Value,
            _ctx: ActionContext,
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            Ok(ActionResult::success(input))
        }
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
        fn action_type(&self) -> ActionType {
            ActionType::Process
        }
        fn parameters(&self) -> Option<&ParameterCollection> {
            None
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
            _ctx: ActionContext,
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            Err(ActionError::fatal("intentional failure"))
        }
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
        fn action_type(&self) -> ActionType {
            ActionType::Process
        }
        fn parameters(&self) -> Option<&ParameterCollection> {
            None
        }
    }

    // -- Helpers --

    fn make_workflow(
        nodes: Vec<NodeDefinition>,
        connections: Vec<Connection>,
    ) -> WorkflowDefinition {
        let now = chrono::Utc::now();
        WorkflowDefinition {
            id: WorkflowId::v4(),
            name: "test".into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes,
            connections,
            variables: HashMap::new(),
            config: WorkflowConfig::default(),
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    fn make_engine(
        registry: Arc<ActionRegistry>,
    ) -> (WorkflowEngine, Arc<EventBus>, Arc<MetricsRegistry>) {
        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let sandbox = Arc::new(InProcessSandbox::new(executor));
        let event_bus = Arc::new(EventBus::new(64));
        let metrics = Arc::new(MetricsRegistry::new());

        let runtime = Arc::new(ActionRuntime::new(
            registry,
            sandbox,
            DataPassingPolicy::default(),
            event_bus.clone(),
            metrics.clone(),
        ));

        let engine = WorkflowEngine::new(runtime, event_bus.clone(), metrics.clone());
        (engine, event_bus, metrics)
    }

    // -- Tests --

    #[tokio::test]
    async fn single_node_workflow() {
        let action_id = ActionId::v4();
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new("echo", "Echo", "echoes input")
                .with_isolation(IsolationLevel::None),
        }));

        let (mut engine, _, _) = make_engine(registry);
        engine.map_action(action_id, "echo");

        let n = NodeId::v4();
        let wf = make_workflow(vec![NodeDefinition::new(n, "echo", action_id)], vec![]);

        let result = engine
            .execute_workflow(&wf, serde_json::json!("hello"), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(result.node_output(n), Some(&serde_json::json!("hello")));
    }

    #[tokio::test]
    async fn linear_two_node_workflow() {
        let echo_id = ActionId::v4();
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new("echo", "Echo", "echoes input")
                .with_isolation(IsolationLevel::None),
        }));

        let (mut engine, _, _) = make_engine(registry);
        engine.map_action(echo_id, "echo");

        let n1 = NodeId::v4();
        let n2 = NodeId::v4();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(n1, "A", echo_id),
                NodeDefinition::new(n2, "B", echo_id),
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
        let echo_id = ActionId::v4();
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new("echo", "Echo", "echoes input")
                .with_isolation(IsolationLevel::None),
        }));

        let (mut engine, _, _) = make_engine(registry);
        engine.map_action(echo_id, "echo");

        let a = NodeId::v4();
        let b = NodeId::v4();
        let c = NodeId::v4();
        let d = NodeId::v4();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a, "A", echo_id),
                NodeDefinition::new(b, "B", echo_id),
                NodeDefinition::new(c, "C", echo_id),
                NodeDefinition::new(d, "D", echo_id),
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
        let echo_id = ActionId::v4();
        let fail_id = ActionId::v4();
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new("echo", "Echo", "echoes input")
                .with_isolation(IsolationLevel::None),
        }));
        registry.register(Arc::new(FailHandler {
            meta: ActionMetadata::new("fail", "Fail", "always fails")
                .with_isolation(IsolationLevel::None),
        }));

        let (mut engine, _, _) = make_engine(registry);
        engine.map_action(echo_id, "echo");
        engine.map_action(fail_id, "fail");

        let n1 = NodeId::v4();
        let n2 = NodeId::v4();
        let n3 = NodeId::v4();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(n1, "A", echo_id),
                NodeDefinition::new(n2, "B", fail_id),
                NodeDefinition::new(n3, "C", echo_id),
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
        let unknown_action = ActionId::v4();
        let registry = Arc::new(ActionRegistry::new());
        let (engine, _, _) = make_engine(registry);

        let n = NodeId::v4();
        let wf = make_workflow(vec![NodeDefinition::new(n, "A", unknown_action)], vec![]);

        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await;

        assert!(matches!(result, Err(EngineError::ActionKeyNotFound { .. })));
    }

    #[tokio::test]
    async fn empty_workflow_returns_planning_error() {
        let registry = Arc::new(ActionRegistry::new());
        let (engine, _, _) = make_engine(registry);

        let wf = make_workflow(vec![], vec![]);
        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await;

        assert!(matches!(result, Err(EngineError::PlanningFailed(_))));
    }

    #[tokio::test]
    async fn telemetry_events_emitted() {
        let echo_id = ActionId::v4();
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new("echo", "Echo", "echoes input")
                .with_isolation(IsolationLevel::None),
        }));

        let (mut engine, event_bus, metrics) = make_engine(registry);
        engine.map_action(echo_id, "echo");

        let mut sub = event_bus.subscribe();

        let n = NodeId::v4();
        let wf = make_workflow(vec![NodeDefinition::new(n, "echo", echo_id)], vec![]);

        engine
            .execute_workflow(&wf, serde_json::json!("test"), ExecutionBudget::default())
            .await
            .unwrap();

        // Should have events from both engine (Started, Completed) and runtime
        let mut event_count = 0;
        while sub.try_recv().is_some() {
            event_count += 1;
        }
        assert!(event_count >= 3);

        assert!(metrics.counter("executions_started_total").get() > 0);
        assert!(metrics.counter("executions_completed_total").get() > 0);
    }

    #[tokio::test]
    async fn metrics_recorded_on_failure() {
        let fail_id = ActionId::v4();
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(FailHandler {
            meta: ActionMetadata::new("fail", "Fail", "always fails")
                .with_isolation(IsolationLevel::None),
        }));

        let (mut engine, _, metrics) = make_engine(registry);
        engine.map_action(fail_id, "fail");

        let n = NodeId::v4();
        let wf = make_workflow(vec![NodeDefinition::new(n, "fail", fail_id)], vec![]);

        let result = engine
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await
            .unwrap();

        assert!(result.is_failure());
        assert!(metrics.counter("executions_started_total").get() > 0);
        assert!(metrics.counter("executions_failed_total").get() > 0);
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
            _ctx: ActionContext,
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            Ok(ActionResult::skip("skipped by test"))
        }
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
        fn action_type(&self) -> ActionType {
            ActionType::Process
        }
        fn parameters(&self) -> Option<&ParameterCollection> {
            None
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
            _ctx: ActionContext,
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
        fn action_type(&self) -> ActionType {
            ActionType::Process
        }
        fn parameters(&self) -> Option<&ParameterCollection> {
            None
        }
    }

    // -- Frontier-specific tests --

    /// A → Branch(selects "true") → B (branch_key="true") / C (branch_key="false") → D
    /// Only B should execute; C should be skipped; D should still run (via B).
    #[tokio::test]
    async fn branch_workflow_only_selected_path_executes() {
        let echo_id = ActionId::v4();
        let branch_id = ActionId::v4();
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new("echo", "Echo", "echoes input")
                .with_isolation(IsolationLevel::None),
        }));
        registry.register(Arc::new(BranchHandler {
            meta: ActionMetadata::new("branch", "Branch", "branches")
                .with_isolation(IsolationLevel::None),
            selected: "true".into(),
        }));

        let (mut engine, _, _) = make_engine(registry);
        engine.map_action(echo_id, "echo");
        engine.map_action(branch_id, "branch");

        let a = NodeId::v4();
        let b = NodeId::v4();
        let c = NodeId::v4();
        let d = NodeId::v4();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a, "A", branch_id),
                NodeDefinition::new(b, "B", echo_id),
                NodeDefinition::new(c, "C", echo_id),
                NodeDefinition::new(d, "D", echo_id),
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
        let echo_id = ActionId::v4();
        let skip_id = ActionId::v4();
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new("echo", "Echo", "echoes input")
                .with_isolation(IsolationLevel::None),
        }));
        registry.register(Arc::new(SkipHandler {
            meta: ActionMetadata::new("skip", "Skip", "always skips")
                .with_isolation(IsolationLevel::None),
        }));

        let (mut engine, _, _) = make_engine(registry);
        engine.map_action(echo_id, "echo");
        engine.map_action(skip_id, "skip");

        let a = NodeId::v4();
        let b = NodeId::v4();
        let c = NodeId::v4();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a, "A", echo_id),
                NodeDefinition::new(b, "B", skip_id),
                NodeDefinition::new(c, "C", echo_id),
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
        let echo_id = ActionId::v4();
        let fail_id = ActionId::v4();
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new("echo", "Echo", "echoes input")
                .with_isolation(IsolationLevel::None),
        }));
        registry.register(Arc::new(FailHandler {
            meta: ActionMetadata::new("fail", "Fail", "always fails")
                .with_isolation(IsolationLevel::None),
        }));

        let (mut engine, _, _) = make_engine(registry);
        engine.map_action(echo_id, "echo");
        engine.map_action(fail_id, "fail");

        let a = NodeId::v4();
        let b = NodeId::v4();
        let c = NodeId::v4();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a, "A", echo_id),
                NodeDefinition::new(b, "B", fail_id),
                NodeDefinition::new(c, "C", echo_id),
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
        let echo_id = ActionId::v4();
        let fail_id = ActionId::v4();
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new("echo", "Echo", "echoes input")
                .with_isolation(IsolationLevel::None),
        }));
        registry.register(Arc::new(FailHandler {
            meta: ActionMetadata::new("fail", "Fail", "always fails")
                .with_isolation(IsolationLevel::None),
        }));

        let (mut engine, _, _) = make_engine(registry);
        engine.map_action(echo_id, "echo");
        engine.map_action(fail_id, "fail");

        let a = NodeId::v4();
        let b = NodeId::v4();
        let c = NodeId::v4();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a, "A", echo_id),
                NodeDefinition::new(b, "B", fail_id),
                NodeDefinition::new(c, "C", echo_id),
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
        let echo_id = ActionId::v4();
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new("echo", "Echo", "echoes input")
                .with_isolation(IsolationLevel::None),
        }));

        let (mut engine, _, _) = make_engine(registry);
        engine.map_action(echo_id, "echo");

        let a = NodeId::v4();
        let b = NodeId::v4();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a, "A", echo_id),
                NodeDefinition::new(b, "B", echo_id),
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
        let echo_id = ActionId::v4();
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new("echo", "Echo", "echoes input")
                .with_isolation(IsolationLevel::None),
        }));

        let (mut engine, _, _) = make_engine(registry);
        engine.map_action(echo_id, "echo");

        let a = NodeId::v4();
        let b = NodeId::v4();
        let c = NodeId::v4();
        let d = NodeId::v4();
        let wf = make_workflow(
            vec![
                NodeDefinition::new(a, "A", echo_id),
                NodeDefinition::new(b, "B", echo_id),
                NodeDefinition::new(c, "C", echo_id),
                NodeDefinition::new(d, "D", echo_id),
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
}
