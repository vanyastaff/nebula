//! End-to-end integration tests for the workflow engine.
//!
//! These tests exercise the full stack: workflow → engine → runtime → sandbox → handler.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use nebula_action::ParameterCollection;
use nebula_action::capability::IsolationLevel;
use nebula_action::context::ActionContext;
use nebula_action::handler::InternalHandler;
use nebula_action::metadata::{ActionMetadata, ActionType};
use nebula_action::result::ActionResult;
use nebula_action::{ActionError, ExecutionBudget};
use nebula_core::Version;
use nebula_core::id::{ActionId, NodeId, WorkflowId};
use nebula_engine::WorkflowEngine;
use nebula_execution::ExecutionStatus;
use nebula_runtime::registry::ActionRegistry;
use nebula_runtime::{ActionRuntime, DataPassingPolicy};
use nebula_sandbox_inprocess::{ActionExecutor, InProcessSandbox};
use nebula_telemetry::event::EventBus;
use nebula_telemetry::metrics::MetricsRegistry;
use nebula_workflow::{Connection, NodeDefinition, WorkflowConfig, WorkflowDefinition};

// ---------------------------------------------------------------------------
// Test action handlers
// ---------------------------------------------------------------------------

/// Echoes input unchanged.
struct EchoHandler {
    meta: ActionMetadata,
}

#[async_trait]
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

/// Wraps the input in `{"doubled": <input * 2>}` (numeric only).
struct DoubleHandler {
    meta: ActionMetadata,
}

#[async_trait]
impl InternalHandler for DoubleHandler {
    async fn execute(
        &self,
        input: serde_json::Value,
        _ctx: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        let n = input
            .as_i64()
            .ok_or_else(|| ActionError::fatal("expected number"))?;
        Ok(ActionResult::success(serde_json::json!(n * 2)))
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

/// Adds 10 to a numeric input.
struct Add10Handler {
    meta: ActionMetadata,
}

#[async_trait]
impl InternalHandler for Add10Handler {
    async fn execute(
        &self,
        input: serde_json::Value,
        _ctx: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        let n = input
            .as_i64()
            .ok_or_else(|| ActionError::fatal("expected number"))?;
        Ok(ActionResult::success(serde_json::json!(n + 10)))
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

/// Sleeps then echoes — used for cancellation testing.
struct SlowHandler {
    meta: ActionMetadata,
    delay: Duration,
}

#[async_trait]
impl InternalHandler for SlowHandler {
    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        tokio::select! {
            () = tokio::time::sleep(self.delay) => Ok(ActionResult::success(input)),
            () = ctx.cancellation.cancelled() => Err(ActionError::Cancelled),
        }
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

/// Always fails.
struct FailHandler {
    meta: ActionMetadata,
}

#[async_trait]
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

/// Counts how many times it has been called (globally).
struct CounterHandler {
    meta: ActionMetadata,
    count: Arc<AtomicUsize>,
}

#[async_trait]
impl InternalHandler for CounterHandler {
    async fn execute(
        &self,
        input: serde_json::Value,
        _ctx: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        // Small yield to allow concurrency observation
        tokio::task::yield_now().await;
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_workflow(nodes: Vec<NodeDefinition>, connections: Vec<Connection>) -> WorkflowDefinition {
    let now = chrono::Utc::now();
    WorkflowDefinition {
        id: WorkflowId::v4(),
        name: "integration-test".into(),
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
    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let sandbox = Arc::new(InProcessSandbox::new(executor));
    let event_bus = Arc::new(EventBus::new(128));
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

fn meta(key: &str) -> ActionMetadata {
    ActionMetadata::new(key, key, "integration test handler").with_isolation(IsolationLevel::None)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Two-node linear pipeline: A → B.
/// A echoes the input, B doubles it.
#[tokio::test]
async fn linear_pipeline_data_flows_through() {
    let echo_id = ActionId::v4();
    let double_id = ActionId::v4();

    let registry = Arc::new(ActionRegistry::new());
    registry.register(Arc::new(EchoHandler { meta: meta("echo") }));
    registry.register(Arc::new(DoubleHandler {
        meta: meta("double"),
    }));

    let (mut engine, _, _) = make_engine(registry);
    engine.map_action(echo_id, "echo");
    engine.map_action(double_id, "double");

    let a = NodeId::v4();
    let b = NodeId::v4();
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a, "A", echo_id),
            NodeDefinition::new(b, "B", double_id),
        ],
        vec![Connection::new(a, b)],
    );

    let result = engine
        .execute_workflow(&wf, serde_json::json!(5), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.node_output(a), Some(&serde_json::json!(5)));
    assert_eq!(result.node_output(b), Some(&serde_json::json!(10)));
}

/// Three-node fan-out: A → B and A → C (parallel).
/// B doubles, C adds 10. Both get A's output (the workflow input).
#[tokio::test]
async fn fan_out_parallel_execution() {
    let echo_id = ActionId::v4();
    let double_id = ActionId::v4();
    let add10_id = ActionId::v4();

    let registry = Arc::new(ActionRegistry::new());
    registry.register(Arc::new(EchoHandler { meta: meta("echo") }));
    registry.register(Arc::new(DoubleHandler {
        meta: meta("double"),
    }));
    registry.register(Arc::new(Add10Handler {
        meta: meta("add10"),
    }));

    let (mut engine, _, _) = make_engine(registry);
    engine.map_action(echo_id, "echo");
    engine.map_action(double_id, "double");
    engine.map_action(add10_id, "add10");

    let a = NodeId::v4();
    let b = NodeId::v4();
    let c = NodeId::v4();
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a, "A", echo_id),
            NodeDefinition::new(b, "B", double_id),
            NodeDefinition::new(c, "C", add10_id),
        ],
        vec![Connection::new(a, b), Connection::new(a, c)],
    );

    let result = engine
        .execute_workflow(&wf, serde_json::json!(7), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.node_output(a), Some(&serde_json::json!(7)));
    assert_eq!(result.node_output(b), Some(&serde_json::json!(14)));
    assert_eq!(result.node_output(c), Some(&serde_json::json!(17)));
}

/// Diamond workflow: A → B, A → C, B → D, C → D.
/// D is the join node receiving merged output from B and C.
#[tokio::test]
async fn diamond_merge_receives_combined_outputs() {
    let echo_id = ActionId::v4();
    let double_id = ActionId::v4();
    let add10_id = ActionId::v4();

    let registry = Arc::new(ActionRegistry::new());
    registry.register(Arc::new(EchoHandler { meta: meta("echo") }));
    registry.register(Arc::new(DoubleHandler {
        meta: meta("double"),
    }));
    registry.register(Arc::new(Add10Handler {
        meta: meta("add10"),
    }));

    let (mut engine, _, _) = make_engine(registry);
    engine.map_action(echo_id, "echo");
    engine.map_action(double_id, "double");
    engine.map_action(add10_id, "add10");

    let a = NodeId::v4();
    let b = NodeId::v4();
    let c = NodeId::v4();
    let d = NodeId::v4();
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a, "A", echo_id),
            NodeDefinition::new(b, "B", double_id),
            NodeDefinition::new(c, "C", add10_id),
            NodeDefinition::new(d, "D", echo_id), // echoes merged input
        ],
        vec![
            Connection::new(a, b),
            Connection::new(a, c),
            Connection::new(b, d),
            Connection::new(c, d),
        ],
    );

    let result = engine
        .execute_workflow(&wf, serde_json::json!(3), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.node_output(a), Some(&serde_json::json!(3)));
    assert_eq!(result.node_output(b), Some(&serde_json::json!(6)));
    assert_eq!(result.node_output(c), Some(&serde_json::json!(13)));

    // D is a join node — its input is a merged object keyed by predecessor node IDs
    let d_output = result.node_output(d).expect("D should have output");
    assert!(d_output.is_object(), "join node output should be an object");
    let obj = d_output.as_object().unwrap();
    assert_eq!(obj.len(), 2, "should have outputs from B and C");
    // Verify the values are present (keyed by node ID strings)
    assert_eq!(obj.get(&b.to_string()), Some(&serde_json::json!(6)));
    assert_eq!(obj.get(&c.to_string()), Some(&serde_json::json!(13)));
}

/// Error propagation: A → B(fail) → C. B fails, C should not run.
#[tokio::test]
async fn error_propagation_stops_downstream() {
    let echo_id = ActionId::v4();
    let fail_id = ActionId::v4();

    let registry = Arc::new(ActionRegistry::new());
    registry.register(Arc::new(EchoHandler { meta: meta("echo") }));
    registry.register(Arc::new(FailHandler { meta: meta("fail") }));

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
        .execute_workflow(&wf, serde_json::json!("data"), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_failure());
    assert_eq!(result.status, ExecutionStatus::Failed);
    // A completed before B failed
    assert!(result.node_output(a).is_some());
    // B failed, no output
    assert!(result.node_output(b).is_none());
    // C was never reached
    assert!(result.node_output(c).is_none());
}

/// Cancellation via sibling failure: parallel nodes A(slow) and B(fail).
/// When B fails, the cancellation token fires and A should stop.
/// Downstream node C should not execute.
#[tokio::test]
async fn cancellation_via_sibling_failure() {
    let slow_id = ActionId::v4();
    let fail_id = ActionId::v4();
    let echo_id = ActionId::v4();

    let registry = Arc::new(ActionRegistry::new());
    registry.register(Arc::new(SlowHandler {
        meta: meta("slow"),
        delay: Duration::from_secs(10),
    }));
    registry.register(Arc::new(FailHandler { meta: meta("fail") }));
    registry.register(Arc::new(EchoHandler { meta: meta("echo") }));

    let (mut engine, _, _) = make_engine(registry);
    engine.map_action(slow_id, "slow");
    engine.map_action(fail_id, "fail");
    engine.map_action(echo_id, "echo");

    // Entry → [Slow, Fail] → Downstream
    // Entry runs first, then Slow and Fail run in parallel,
    // Fail dies immediately, cancelling Slow. Downstream never runs.
    let entry = NodeId::v4();
    let slow = NodeId::v4();
    let fail = NodeId::v4();
    let downstream = NodeId::v4();
    let wf = make_workflow(
        vec![
            NodeDefinition::new(entry, "Entry", echo_id),
            NodeDefinition::new(slow, "Slow", slow_id),
            NodeDefinition::new(fail, "Fail", fail_id),
            NodeDefinition::new(downstream, "Down", echo_id),
        ],
        vec![
            Connection::new(entry, slow),
            Connection::new(entry, fail),
            Connection::new(slow, downstream),
            Connection::new(fail, downstream),
        ],
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        engine.execute_workflow(&wf, serde_json::json!("go"), ExecutionBudget::default()),
    )
    .await
    .expect("should complete within 5s");

    let result = result.unwrap();
    assert!(result.is_failure());
    // Entry ran successfully
    assert!(result.node_output(entry).is_some());
    // Slow was cancelled (no output stored)
    assert!(result.node_output(slow).is_none());
    // Downstream never ran
    assert!(result.node_output(downstream).is_none());
}

/// Verify telemetry events cover the full lifecycle.
#[tokio::test]
async fn telemetry_covers_full_lifecycle() {
    let echo_id = ActionId::v4();

    let registry = Arc::new(ActionRegistry::new());
    registry.register(Arc::new(EchoHandler { meta: meta("echo") }));

    let (mut engine, event_bus, metrics) = make_engine(registry);
    engine.map_action(echo_id, "echo");

    let mut sub = event_bus.subscribe();

    let a = NodeId::v4();
    let b = NodeId::v4();
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a, "A", echo_id),
            NodeDefinition::new(b, "B", echo_id),
        ],
        vec![Connection::new(a, b)],
    );

    let result = engine
        .execute_workflow(&wf, serde_json::json!(1), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_success());

    // Collect all events
    let mut events = Vec::new();
    while let Some(event) = sub.try_recv() {
        events.push(event);
    }

    // Should have: execution started, node_a started, node_a completed,
    // node_b started, node_b completed, execution completed
    // (runtime emits node events, engine emits execution events)
    assert!(
        events.len() >= 6,
        "expected >= 6 events, got {}",
        events.len()
    );

    // Metrics should reflect the execution
    assert!(metrics.counter("executions_started_total").get() > 0);
    assert!(metrics.counter("executions_completed_total").get() > 0);
    assert!(metrics.counter("actions_executed_total").get() >= 2);
}

/// Verify that execution with many parallel nodes works with concurrency control.
#[tokio::test]
async fn bounded_concurrency_with_multiple_parallel_nodes() {
    let counter = Arc::new(AtomicUsize::new(0));
    let echo_id = ActionId::v4();

    let registry = Arc::new(ActionRegistry::new());
    registry.register(Arc::new(CounterHandler {
        meta: meta("counter"),
        count: counter.clone(),
    }));

    let (mut engine, _, _) = make_engine(registry);
    engine.map_action(echo_id, "counter");

    // Create 8 independent nodes (all entry nodes, no connections)
    let nodes: Vec<NodeDefinition> = (0..8)
        .map(|i| NodeDefinition::new(NodeId::v4(), format!("N{i}"), echo_id))
        .collect();

    let wf = make_workflow(nodes, vec![]);

    // Limit concurrency to 2
    let budget = ExecutionBudget {
        max_concurrent_nodes: 2,
        ..Default::default()
    };

    let result = engine
        .execute_workflow(&wf, serde_json::json!("parallel"), budget)
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.node_outputs.len(), 8);
    assert_eq!(counter.load(Ordering::SeqCst), 8, "all 8 nodes should run");
}

/// Three-level chain: A → B → C → D. Verify output propagation at each stage.
#[tokio::test]
async fn deep_chain_propagates_outputs() {
    let echo_id = ActionId::v4();
    let double_id = ActionId::v4();

    let registry = Arc::new(ActionRegistry::new());
    registry.register(Arc::new(EchoHandler { meta: meta("echo") }));
    registry.register(Arc::new(DoubleHandler {
        meta: meta("double"),
    }));

    let (mut engine, _, _) = make_engine(registry);
    engine.map_action(echo_id, "echo");
    engine.map_action(double_id, "double");

    let a = NodeId::v4();
    let b = NodeId::v4();
    let c = NodeId::v4();
    let d = NodeId::v4();
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a, "A", echo_id),   // echo(2) = 2
            NodeDefinition::new(b, "B", double_id), // double(2) = 4
            NodeDefinition::new(c, "C", double_id), // double(4) = 8
            NodeDefinition::new(d, "D", double_id), // double(8) = 16
        ],
        vec![
            Connection::new(a, b),
            Connection::new(b, c),
            Connection::new(c, d),
        ],
    );

    let result = engine
        .execute_workflow(&wf, serde_json::json!(2), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.node_output(a), Some(&serde_json::json!(2)));
    assert_eq!(result.node_output(b), Some(&serde_json::json!(4)));
    assert_eq!(result.node_output(c), Some(&serde_json::json!(8)));
    assert_eq!(result.node_output(d), Some(&serde_json::json!(16)));
}

/// Metrics are accurate after a failed workflow.
#[tokio::test]
async fn metrics_accurate_on_failure() {
    let fail_id = ActionId::v4();

    let registry = Arc::new(ActionRegistry::new());
    registry.register(Arc::new(FailHandler { meta: meta("fail") }));

    let (mut engine, _, metrics) = make_engine(registry);
    engine.map_action(fail_id, "fail");

    let a = NodeId::v4();
    let wf = make_workflow(vec![NodeDefinition::new(a, "fail-node", fail_id)], vec![]);

    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_failure());
    assert_eq!(metrics.counter("executions_started_total").get(), 1);
    assert_eq!(metrics.counter("executions_failed_total").get(), 1);
    assert_eq!(metrics.counter("executions_completed_total").get(), 0);
}
