//! Workflow execution engine.
//!
//! Executes workflows by processing parallel groups level-by-level,
//! resolving inputs from predecessor outputs, and delegating action
//! execution to the runtime.

use std::collections::HashMap;
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
use nebula_workflow::{DependencyGraph, NodeState, WorkflowDefinition};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use nebula_node::NodeRegistry;

use crate::error::EngineError;
use crate::result::ExecutionResult;

/// The workflow execution engine.
///
/// Orchestrates end-to-end execution of workflow definitions by:
///
/// 1. Building an execution plan (parallel groups from the DAG)
/// 2. Executing nodes level-by-level with bounded concurrency
/// 3. Resolving each node's input from predecessor outputs
/// 4. Delegating action execution to the [`ActionRuntime`]
/// 5. Tracking execution state and emitting telemetry
pub struct WorkflowEngine {
    runtime: Arc<ActionRuntime>,
    event_bus: Arc<EventBus>,
    metrics: Arc<MetricsRegistry>,
    /// Maps action IDs (from node definitions) to registry keys.
    action_keys: HashMap<ActionId, String>,
    /// Node registry for node-level metadata and versioning.
    node_registry: NodeRegistry,
}

impl WorkflowEngine {
    /// Create a new engine with the given components.
    pub fn new(
        runtime: Arc<ActionRuntime>,
        event_bus: Arc<EventBus>,
        metrics: Arc<MetricsRegistry>,
    ) -> Self {
        Self {
            runtime,
            event_bus,
            metrics,
            action_keys: HashMap::new(),
            node_registry: NodeRegistry::new(),
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
    pub fn node_registry(&self) -> &NodeRegistry {
        &self.node_registry
    }

    /// Mutable access to the node registry.
    pub fn node_registry_mut(&mut self) -> &mut NodeRegistry {
        &mut self.node_registry
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
    /// Builds an execution plan, then processes parallel groups
    /// level-by-level. Within each level, nodes execute concurrently
    /// up to `budget.max_concurrent_nodes`.
    ///
    /// Entry nodes receive the workflow-level `input`. Subsequent nodes
    /// receive the output of their predecessors.
    pub async fn execute_workflow(
        &self,
        workflow: &WorkflowDefinition,
        input: serde_json::Value,
        budget: ExecutionBudget,
    ) -> Result<ExecutionResult, EngineError> {
        let execution_id = ExecutionId::v4();
        let started = Instant::now();

        // 1. Build execution plan
        let plan = ExecutionPlan::from_workflow(execution_id, workflow, budget.clone())
            .map_err(|e| EngineError::PlanningFailed(e.to_string()))?;

        // 2. Build dependency graph for predecessor lookup
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

        // 9. Execute level by level
        let failed_node = self
            .run_levels(
                &plan,
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

    /// Execute all parallel groups level-by-level.
    ///
    /// Returns `Some((node_id, error))` if a node failed, `None` if all succeeded.
    #[allow(clippy::too_many_arguments)]
    async fn run_levels(
        &self,
        plan: &ExecutionPlan,
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
        for group in &plan.parallel_groups {
            if cancel_token.is_cancelled() {
                break;
            }

            let mut join_set = self.spawn_level(
                group,
                node_map,
                graph,
                outputs,
                semaphore,
                cancel_token,
                exec_state,
                execution_id,
                workflow_id,
                input,
            );

            if let Some(failure) =
                collect_level_results(&mut join_set, exec_state, cancel_token).await
            {
                return Some(failure);
            }
        }
        None
    }

    /// Spawn all nodes in a single level into a JoinSet.
    #[allow(clippy::too_many_arguments)]
    fn spawn_level(
        &self,
        group: &[NodeId],
        node_map: &HashMap<NodeId, &nebula_workflow::NodeDefinition>,
        graph: &DependencyGraph,
        outputs: &Arc<DashMap<NodeId, serde_json::Value>>,
        semaphore: &Arc<Semaphore>,
        cancel_token: &CancellationToken,
        exec_state: &mut ExecutionState,
        execution_id: ExecutionId,
        workflow_id: WorkflowId,
        input: &serde_json::Value,
    ) -> JoinSet<(NodeId, Result<ActionResult<serde_json::Value>, EngineError>)> {
        let mut join_set = JoinSet::new();

        for &node_id in group {
            let Some(node_def) = node_map.get(&node_id) else {
                continue;
            };
            let Ok(action_key) = self.resolve_action_key(node_def.action_id) else {
                continue;
            };
            let action_key = action_key.to_owned();
            let node_input = resolve_node_input(node_id, graph, outputs, input);

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
                    input: node_input,
                }
                .run(),
            );
        }

        join_set
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
}

impl NodeTask {
    /// Execute this node: acquire semaphore, check cancellation, run action.
    async fn run(self) -> (NodeId, Result<ActionResult<serde_json::Value>, EngineError>) {
        let _permit = self.sem.acquire().await.expect("semaphore closed");

        if self.cancel.is_cancelled() {
            return (self.node_id, Err(EngineError::Cancelled));
        }

        let action_ctx = ActionContext::new(
            self.execution_id,
            self.node_id,
            self.workflow_id,
            ScopeLevel::Global,
        )
        .with_cancellation(self.cancel.child_token());

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

/// Collect results from a level's JoinSet and update execution state.
///
/// Returns `Some((node_id, error))` if a node failed, `None` if all succeeded.
async fn collect_level_results(
    join_set: &mut JoinSet<(NodeId, Result<ActionResult<serde_json::Value>, EngineError>)>,
    exec_state: &mut ExecutionState,
    cancel_token: &CancellationToken,
) -> Option<(NodeId, String)> {
    while let Some(join_result) = join_set.join_next().await {
        match join_result {
            Ok((node_id, Ok(_action_result))) => {
                mark_node_completed(exec_state, node_id);
            }
            Ok((node_id, Err(ref err))) => {
                mark_node_failed(exec_state, node_id, err);
                cancel_token.cancel();
                return Some((node_id, err.to_string()));
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

/// Resolve the input for a node from its predecessors' outputs.
///
/// - Entry nodes (no predecessors): receive the workflow-level input.
/// - Single predecessor: receive that node's output directly.
/// - Multiple predecessors: receive a JSON object with each predecessor's
///   output keyed by its node ID.
fn resolve_node_input(
    node_id: NodeId,
    graph: &DependencyGraph,
    outputs: &DashMap<NodeId, serde_json::Value>,
    workflow_input: &serde_json::Value,
) -> serde_json::Value {
    let predecessors = graph.predecessors(node_id);
    if predecessors.is_empty() {
        return workflow_input.clone();
    }
    if predecessors.len() == 1 {
        return outputs
            .get(&predecessors[0])
            .map(|v| v.value().clone())
            .unwrap_or(serde_json::Value::Null);
    }
    let mut merged = serde_json::Map::new();
    for pred_id in &predecessors {
        if let Some(output) = outputs.get(pred_id) {
            merged.insert(pred_id.to_string(), output.value().clone());
        }
    }
    serde_json::Value::Object(merged)
}

/// Extract the primary output value from an ActionResult for downstream input resolution.
fn extract_primary_output(result: &ActionResult<serde_json::Value>) -> Option<serde_json::Value> {
    match result {
        ActionResult::Success { output } => Some(output.clone()),
        ActionResult::Skip { output, .. } => output.clone(),
        ActionResult::Continue { output, .. } => Some(output.clone()),
        ActionResult::Break { output, .. } => Some(output.clone()),
        ActionResult::Branch { output, .. } => Some(output.clone()),
        ActionResult::Route { data, .. } => Some(data.clone()),
        ActionResult::MultiOutput { main_output, .. } => main_output.clone(),
        ActionResult::Wait { partial_output, .. } => partial_output.clone(),
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
}
