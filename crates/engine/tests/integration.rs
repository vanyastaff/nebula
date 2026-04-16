//! End-to-end integration tests for the workflow engine.
//!
//! These tests exercise the full stack: workflow → engine → runtime → sandbox → handler.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use nebula_action::{
    ActionError, action::Action, context::Context, dependency::ActionDependencies,
    metadata::ActionMetadata, result::ActionResult, stateless::StatelessAction,
};
use nebula_core::{
    ActionKey, action_key,
    id::{NodeId, WorkflowId},
};
use nebula_engine::WorkflowEngine;
use nebula_execution::{ExecutionStatus, context::ExecutionBudget};
use nebula_metrics::naming::{
    NEBULA_ACTION_EXECUTIONS_TOTAL, NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL,
    NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL, NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL,
};
use nebula_runtime::{
    ActionExecutor, ActionRuntime, DataPassingPolicy, InProcessSandbox, registry::ActionRegistry,
};
use nebula_telemetry::metrics::MetricsRegistry;
use nebula_workflow::{Connection, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition};

// ---------------------------------------------------------------------------
// Test action handlers
// ---------------------------------------------------------------------------

/// Echoes input unchanged.
struct EchoHandler {
    meta: ActionMetadata,
}

impl ActionDependencies for EchoHandler {}
impl Action for EchoHandler {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for EchoHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &impl Context,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

/// Wraps the input in `{"doubled": <input * 2>}` (numeric only).
struct DoubleHandler {
    meta: ActionMetadata,
}

impl ActionDependencies for DoubleHandler {}
impl Action for DoubleHandler {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for DoubleHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &impl Context,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        let n = input
            .as_i64()
            .ok_or_else(|| ActionError::fatal("expected number"))?;
        Ok(ActionResult::success(serde_json::json!(n * 2)))
    }
}

/// Adds 10 to a numeric input.
struct Add10Handler {
    meta: ActionMetadata,
}

impl ActionDependencies for Add10Handler {}
impl Action for Add10Handler {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for Add10Handler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &impl Context,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        let n = input
            .as_i64()
            .ok_or_else(|| ActionError::fatal("expected number"))?;
        Ok(ActionResult::success(serde_json::json!(n + 10)))
    }
}

/// Sleeps then echoes — used for cancellation testing.
struct SlowHandler {
    meta: ActionMetadata,
    delay: Duration,
}

impl ActionDependencies for SlowHandler {}
impl Action for SlowHandler {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for SlowHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &impl Context,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        tokio::select! {
            () = tokio::time::sleep(self.delay) => Ok(ActionResult::success(input)),
            () = ctx.cancellation().cancelled() => Err(ActionError::Cancelled),
        }
    }
}

/// Always fails.
struct FailHandler {
    meta: ActionMetadata,
}

impl ActionDependencies for FailHandler {}
impl Action for FailHandler {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for FailHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        _input: Self::Input,
        _ctx: &impl Context,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        Err(ActionError::fatal("intentional failure"))
    }
}

/// Counts how many times it has been called (globally).
struct CounterHandler {
    meta: ActionMetadata,
    count: Arc<AtomicUsize>,
}

impl ActionDependencies for CounterHandler {}
impl Action for CounterHandler {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for CounterHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &impl Context,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        // Small yield to allow concurrency observation
        tokio::task::yield_now().await;
        Ok(ActionResult::success(input))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_workflow(nodes: Vec<NodeDefinition>, connections: Vec<Connection>) -> WorkflowDefinition {
    let now = chrono::Utc::now();
    WorkflowDefinition {
        id: WorkflowId::new(),
        name: "integration-test".into(),
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

fn make_engine(registry: Arc<ActionRegistry>) -> (WorkflowEngine, MetricsRegistry) {
    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
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

/// Engine and runtime share the same metrics registry.
#[tokio::test]
async fn engine_and_runtime_share_metrics_registry() {
    let metrics = MetricsRegistry::new();

    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(EchoHandler {
        meta: meta(action_key!("echo")),
    });

    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let sandbox = Arc::new(InProcessSandbox::new(executor));

    let runtime = Arc::new(ActionRuntime::new(
        registry,
        sandbox,
        DataPassingPolicy::default(),
        metrics.clone(),
    ));
    let engine = WorkflowEngine::new(runtime, metrics.clone());

    let n = NodeId::new();
    let wf = make_workflow(
        vec![NodeDefinition::new(n, "echo", "echo").unwrap()],
        vec![],
    );

    let _result = engine
        .execute_workflow(&wf, serde_json::json!("hi"), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(metrics.counter(NEBULA_ACTION_EXECUTIONS_TOTAL).get() >= 1);
}

fn meta(key: ActionKey) -> ActionMetadata {
    let name = key.to_string();
    ActionMetadata::new(key, name, "integration test handler")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Two-node linear pipeline: A → B.
/// A echoes the input, B doubles it.
#[tokio::test]
async fn linear_pipeline_data_flows_through() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(EchoHandler {
        meta: meta(action_key!("echo")),
    });
    registry.register_stateless(DoubleHandler {
        meta: meta(action_key!("double")),
    });

    let (engine, _) = make_engine(registry);

    let a = NodeId::new();
    let b = NodeId::new();
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a, "A", "echo").unwrap(),
            NodeDefinition::new(b, "B", "double").unwrap(),
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
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(EchoHandler {
        meta: meta(action_key!("echo")),
    });
    registry.register_stateless(DoubleHandler {
        meta: meta(action_key!("double")),
    });
    registry.register_stateless(Add10Handler {
        meta: meta(action_key!("add10")),
    });

    let (engine, _) = make_engine(registry);

    let a = NodeId::new();
    let b = NodeId::new();
    let c = NodeId::new();
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a, "A", "echo").unwrap(),
            NodeDefinition::new(b, "B", "double").unwrap(),
            NodeDefinition::new(c, "C", "add10").unwrap(),
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
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(EchoHandler {
        meta: meta(action_key!("echo")),
    });
    registry.register_stateless(DoubleHandler {
        meta: meta(action_key!("double")),
    });
    registry.register_stateless(Add10Handler {
        meta: meta(action_key!("add10")),
    });

    let (engine, _) = make_engine(registry);

    let a = NodeId::new();
    let b = NodeId::new();
    let c = NodeId::new();
    let d = NodeId::new();
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a, "A", "echo").unwrap(),
            NodeDefinition::new(b, "B", "double").unwrap(),
            NodeDefinition::new(c, "C", "add10").unwrap(),
            NodeDefinition::new(d, "D", "echo").unwrap(), // echoes merged input
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
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(EchoHandler {
        meta: meta(action_key!("echo")),
    });
    registry.register_stateless(FailHandler {
        meta: meta(action_key!("fail")),
    });

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
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(SlowHandler {
        meta: meta(action_key!("slow")),
        delay: Duration::from_secs(10),
    });
    registry.register_stateless(FailHandler {
        meta: meta(action_key!("fail")),
    });
    registry.register_stateless(EchoHandler {
        meta: meta(action_key!("echo")),
    });

    let (engine, _) = make_engine(registry);

    // Entry → [Slow, Fail] → Downstream
    // Entry runs first, then Slow and Fail run in parallel,
    // Fail dies immediately, cancelling Slow. Downstream never runs.
    let entry = NodeId::new();
    let slow = NodeId::new();
    let fail = NodeId::new();
    let downstream = NodeId::new();
    let wf = make_workflow(
        vec![
            NodeDefinition::new(entry, "Entry", "echo").unwrap(),
            NodeDefinition::new(slow, "Slow", "slow").unwrap(),
            NodeDefinition::new(fail, "Fail", "fail").unwrap(),
            NodeDefinition::new(downstream, "Down", "echo").unwrap(),
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

/// Verify metrics cover the full lifecycle.
#[tokio::test]
async fn metrics_cover_full_lifecycle() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(EchoHandler {
        meta: meta(action_key!("echo")),
    });

    let (engine, metrics) = make_engine(registry);

    let a = NodeId::new();
    let b = NodeId::new();
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a, "A", "echo").unwrap(),
            NodeDefinition::new(b, "B", "echo").unwrap(),
        ],
        vec![Connection::new(a, b)],
    );

    let result = engine
        .execute_workflow(&wf, serde_json::json!(1), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_success());

    // Metrics should reflect the execution
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
    assert!(metrics.counter(NEBULA_ACTION_EXECUTIONS_TOTAL).get() >= 2);
}

/// Verify that execution with many parallel nodes works with concurrency control.
#[tokio::test]
async fn bounded_concurrency_with_multiple_parallel_nodes() {
    let counter = Arc::new(AtomicUsize::new(0));

    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(CounterHandler {
        meta: meta(action_key!("counter")),
        count: counter.clone(),
    });

    let (engine, _) = make_engine(registry);

    // Create 8 independent nodes (all entry nodes, no connections)
    let nodes: Vec<NodeDefinition> = (0..8)
        .map(|i| NodeDefinition::new(NodeId::new(), format!("N{i}"), "counter").unwrap())
        .collect();

    let wf = make_workflow(nodes, vec![]);

    // Limit concurrency to 2
    let budget = ExecutionBudget::default().with_max_concurrent_nodes(2);

    let result = engine
        .execute_workflow(&wf, serde_json::json!("parallel"), budget)
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.node_outputs.len(), 8);
    assert_eq!(counter.load(Ordering::SeqCst), 8, "all 8 nodes should run");
}

#[tokio::test]
async fn zero_concurrency_budget_returns_planning_error() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(EchoHandler {
        meta: meta(action_key!("echo")),
    });

    let (engine, _) = make_engine(registry);

    let a = NodeId::new();
    let wf = make_workflow(vec![NodeDefinition::new(a, "A", "echo").unwrap()], vec![]);

    let budget = ExecutionBudget {
        max_concurrent_nodes: 0,
        ..ExecutionBudget::default()
    };

    let err = engine
        .execute_workflow(&wf, serde_json::json!({}), budget)
        .await
        .expect_err("zero permits must not deadlock behind Semaphore::new(0)");

    let msg = err.to_string();
    assert!(
        msg.contains("max_concurrent_nodes") || msg.contains("deadlock"),
        "unexpected error message: {msg}"
    );
}

/// Three-level chain: A → B → C → D. Verify output propagation at each stage.
#[tokio::test]
async fn deep_chain_propagates_outputs() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(EchoHandler {
        meta: meta(action_key!("echo")),
    });
    registry.register_stateless(DoubleHandler {
        meta: meta(action_key!("double")),
    });

    let (engine, _) = make_engine(registry);

    let a = NodeId::new();
    let b = NodeId::new();
    let c = NodeId::new();
    let d = NodeId::new();
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a, "A", "echo").unwrap(), // echo(2) = 2
            NodeDefinition::new(b, "B", "double").unwrap(), // double(2) = 4
            NodeDefinition::new(c, "C", "double").unwrap(), // double(4) = 8
            NodeDefinition::new(d, "D", "double").unwrap(), // double(8) = 16
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
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(FailHandler {
        meta: meta(action_key!("fail")),
    });

    let (engine, metrics) = make_engine(registry);

    let a = NodeId::new();
    let wf = make_workflow(
        vec![NodeDefinition::new(a, "fail-node", "fail").unwrap()],
        vec![],
    );

    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_failure());
    assert_eq!(
        metrics
            .counter(NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL)
            .get(),
        1
    );
    assert_eq!(
        metrics
            .counter(NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL)
            .get(),
        1
    );
    assert_eq!(
        metrics
            .counter(NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL)
            .get(),
        0
    );
}

/// Three-node linear workflow A → B → C where B is disabled.
///
/// Expected: A executes normally; B is skipped (no output);
/// C executes because disabled-node skip activates outgoing edges,
/// but C receives null (B produced no output).
#[tokio::test]
async fn disabled_node_is_skipped_and_successor_executes() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(EchoHandler {
        meta: meta(action_key!("echo")),
    });

    let (engine, _) = make_engine(registry);

    let a = NodeId::new();
    let b = NodeId::new();
    let c = NodeId::new();

    let wf = make_workflow(
        vec![
            NodeDefinition::new(a, "A", "echo").unwrap(),
            NodeDefinition::new(b, "B", "echo").unwrap().disabled(),
            NodeDefinition::new(c, "C", "echo").unwrap(),
        ],
        vec![Connection::new(a, b), Connection::new(b, c)],
    );

    let result = engine
        .execute_workflow(&wf, serde_json::json!("hello"), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(
        result.is_success(),
        "workflow should succeed when B is disabled"
    );
    // A executed and produced its output
    assert_eq!(
        result.node_output(a),
        Some(&serde_json::json!("hello")),
        "A should execute normally"
    );
    // B was skipped — no output entry
    assert!(
        result.node_output(b).is_none(),
        "B should be skipped (no output)"
    );
    // C executed — B's disabled-skip activated the B→C edge so C entered the frontier.
    // C receives null because B produced no output.
    assert_eq!(
        result.node_output(c),
        Some(&serde_json::json!(null)),
        "C should execute after B is skipped, receiving null (B produced no output)"
    );
}
