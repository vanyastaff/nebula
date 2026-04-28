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
    ActionError, action::Action, metadata::ActionMetadata, result::ActionResult,
    stateless::StatelessAction,
};
use nebula_core::{ActionKey, DeclaresDependencies, NodeKey, action_key, id::WorkflowId, node_key};
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessSandbox,
    WorkflowEngine,
};
use nebula_execution::{ExecutionStatus, context::ExecutionBudget};
use nebula_metrics::naming::{
    NEBULA_ACTION_EXECUTIONS_TOTAL, NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL,
    NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL, NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL,
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

impl DeclaresDependencies for EchoHandler {}
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
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

/// Wraps the input in `{"doubled": <input * 2>}` (numeric only).
struct DoubleHandler {
    meta: ActionMetadata,
}

impl DeclaresDependencies for DoubleHandler {}
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
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
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

impl DeclaresDependencies for Add10Handler {}
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
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
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

impl DeclaresDependencies for SlowHandler {}
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
        ctx: &(impl nebula_action::ActionContext + ?Sized),
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

impl DeclaresDependencies for FailHandler {}
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
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        Err(ActionError::fatal("intentional failure"))
    }
}

/// Counts how many times it has been called (globally).
struct CounterHandler {
    meta: ActionMetadata,
    count: Arc<AtomicUsize>,
}

impl DeclaresDependencies for CounterHandler {}
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
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        // Small yield to allow concurrency observation
        tokio::task::yield_now().await;
        Ok(ActionResult::success(input))
    }
}

/// Returns `ActionResult::Skip` — exercises the engine's `propagate_skip`
/// recursive ladder. A skipped node's outgoing edges do not activate (per
/// `evaluate_edge`), so successors with no other active source are
/// transitively skipped.
struct SkipHandler {
    meta: ActionMetadata,
}

impl DeclaresDependencies for SkipHandler {}
impl Action for SkipHandler {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for SkipHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        _input: Self::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        Ok(ActionResult::skip("test skip"))
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

    let n = node_key!("n");
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

    let a = node_key!("a");
    let b = node_key!("b");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "echo").unwrap(),
            NodeDefinition::new(b.clone(), "B", "double").unwrap(),
        ],
        vec![Connection::new(a.clone(), b.clone())],
    );

    let result = engine
        .execute_workflow(&wf, serde_json::json!(5), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.node_output(&a), Some(&serde_json::json!(5)));
    assert_eq!(result.node_output(&b), Some(&serde_json::json!(10)));
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

    let a = node_key!("a");
    let b = node_key!("b");
    let c = node_key!("c");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "echo").unwrap(),
            NodeDefinition::new(b.clone(), "B", "double").unwrap(),
            NodeDefinition::new(c.clone(), "C", "add10").unwrap(),
        ],
        vec![
            Connection::new(a.clone(), b.clone()),
            Connection::new(a.clone(), c.clone()),
        ],
    );

    let result = engine
        .execute_workflow(&wf, serde_json::json!(7), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.node_output(&a), Some(&serde_json::json!(7)));
    assert_eq!(result.node_output(&b), Some(&serde_json::json!(14)));
    assert_eq!(result.node_output(&c), Some(&serde_json::json!(17)));
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

    let a = node_key!("a");
    let b = node_key!("b");
    let c = node_key!("c");
    let d = node_key!("d");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "echo").unwrap(),
            NodeDefinition::new(b.clone(), "B", "double").unwrap(),
            NodeDefinition::new(c.clone(), "C", "add10").unwrap(),
            NodeDefinition::new(d.clone(), "D", "echo").unwrap(), // echoes merged input
        ],
        vec![
            Connection::new(a.clone(), b.clone()),
            Connection::new(a.clone(), c.clone()),
            Connection::new(b.clone(), d.clone()),
            Connection::new(c.clone(), d.clone()),
        ],
    );

    let result = engine
        .execute_workflow(&wf, serde_json::json!(3), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.node_output(&a), Some(&serde_json::json!(3)));
    assert_eq!(result.node_output(&b), Some(&serde_json::json!(6)));
    assert_eq!(result.node_output(&c), Some(&serde_json::json!(13)));

    // D is a join node — its input is a merged object keyed by predecessor node IDs
    let d_output = result.node_output(&d).expect("D should have output");
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

    let a = node_key!("a");
    let b = node_key!("b");
    let c = node_key!("c");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "echo").unwrap(),
            NodeDefinition::new(b.clone(), "B", "fail").unwrap(),
            NodeDefinition::new(c.clone(), "C", "echo").unwrap(),
        ],
        vec![
            Connection::new(a.clone(), b.clone()),
            Connection::new(b.clone(), c.clone()),
        ],
    );

    let result = engine
        .execute_workflow(&wf, serde_json::json!("data"), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_failure());
    assert_eq!(result.status, ExecutionStatus::Failed);
    // A completed before B failed
    assert!(result.node_output(&a).is_some());
    // B failed, no output
    assert!(result.node_output(&b).is_none());
    // C was never reached
    assert!(result.node_output(&c).is_none());
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
    let entry = node_key!("entry");
    let slow = node_key!("slow");
    let fail = node_key!("fail");
    let downstream = node_key!("downstream");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(entry.clone(), "Entry", "echo").unwrap(),
            NodeDefinition::new(slow.clone(), "Slow", "slow").unwrap(),
            NodeDefinition::new(fail.clone(), "Fail", "fail").unwrap(),
            NodeDefinition::new(downstream.clone(), "Down", "echo").unwrap(),
        ],
        vec![
            Connection::new(entry.clone(), slow.clone()),
            Connection::new(entry.clone(), fail.clone()),
            Connection::new(slow.clone(), downstream.clone()),
            Connection::new(fail, downstream.clone()),
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
    assert!(result.node_output(&entry).is_some());
    // Slow was cancelled (no output stored)
    assert!(result.node_output(&slow).is_none());
    // Downstream never ran
    assert!(result.node_output(&downstream).is_none());
}

/// Verify metrics cover the full lifecycle.
#[tokio::test]
async fn metrics_cover_full_lifecycle() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(EchoHandler {
        meta: meta(action_key!("echo")),
    });

    let (engine, metrics) = make_engine(registry);

    let a = node_key!("a");
    let b = node_key!("b");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "echo").unwrap(),
            NodeDefinition::new(b.clone(), "B", "echo").unwrap(),
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
        .map(|i| {
            NodeDefinition::new(
                NodeKey::new(format!("node_{i}")).unwrap(),
                format!("N{i}"),
                "counter",
            )
            .unwrap()
        })
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

    let a = node_key!("a");
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

    let a = node_key!("a");
    let b = node_key!("b");
    let c = node_key!("c");
    let d = node_key!("d");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "echo").unwrap(), // echo(2) = 2
            NodeDefinition::new(b.clone(), "B", "double").unwrap(), // double(2) = 4
            NodeDefinition::new(c.clone(), "C", "double").unwrap(), // double(4) = 8
            NodeDefinition::new(d.clone(), "D", "double").unwrap(), // double(8) = 16
        ],
        vec![
            Connection::new(a.clone(), b.clone()),
            Connection::new(b.clone(), c.clone()),
            Connection::new(c.clone(), d.clone()),
        ],
    );

    let result = engine
        .execute_workflow(&wf, serde_json::json!(2), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.node_output(&a), Some(&serde_json::json!(2)));
    assert_eq!(result.node_output(&b), Some(&serde_json::json!(4)));
    assert_eq!(result.node_output(&c), Some(&serde_json::json!(8)));
    assert_eq!(result.node_output(&d), Some(&serde_json::json!(16)));
}

/// Metrics are accurate after a failed workflow.
#[tokio::test]
async fn metrics_accurate_on_failure() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(FailHandler {
        meta: meta(action_key!("fail")),
    });

    let (engine, metrics) = make_engine(registry);

    let a = node_key!("a");
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

    let a = node_key!("a");
    let b = node_key!("b");
    let c = node_key!("c");

    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "echo").unwrap(),
            NodeDefinition::new(b.clone(), "B", "echo")
                .unwrap()
                .disabled(),
            NodeDefinition::new(c.clone(), "C", "echo").unwrap(),
        ],
        vec![
            Connection::new(a.clone(), b.clone()),
            Connection::new(b.clone(), c.clone()),
        ],
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
        result.node_output(&a),
        Some(&serde_json::json!("hello")),
        "A should execute normally"
    );
    // B was skipped — no output entry
    assert!(
        result.node_output(&b).is_none(),
        "B should be skipped (no output)"
    );
    // C executed — B's disabled-skip activated the B→C edge so C entered the frontier.
    // C receives null because B produced no output.
    assert_eq!(
        result.node_output(&c),
        Some(&serde_json::json!(null)),
        "C should execute after B is skipped, receiving null (B produced no output)"
    );
}

// ---------------------------------------------------------------------------
// Skip-propagation regression tests (ROADMAP §M1.1)
//
// Pin behaviour of `engine::propagate_skip` — the recursive ladder that
// walks the graph when a node returns `ActionResult::Skip` (or any
// non-activating result) and transitively marks unreachable successors as
// Skipped. The pre-existing `disabled_node_is_skipped_and_successor_executes`
// covers the disabled-node BYPASS (which activates outgoing edges with null
// and is a different code path); these tests cover the propagate_skip ladder
// triggered by non-activating `ActionResult` variants.
// ---------------------------------------------------------------------------

/// Three-hop chain A → B → C → D where B returns `ActionResult::Skip`.
///
/// Expected: A=Completed, B=Skipped, C=Skipped (one hop transitive), D=Skipped
/// (two hops transitive). Verifies the recursive `propagate_skip` walk via
/// `resolved == required && activated == 0` reaches the full chain, not just
/// the direct successor.
#[tokio::test]
async fn skip_propagates_transitively_through_three_hop_chain() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(EchoHandler {
        meta: meta(action_key!("echo")),
    });
    registry.register_stateless(SkipHandler {
        meta: meta(action_key!("skip")),
    });
    let (engine, _) = make_engine(registry);

    let a = node_key!("a");
    let b = node_key!("b");
    let c = node_key!("c");
    let d = node_key!("d");

    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "echo").unwrap(),
            NodeDefinition::new(b.clone(), "B", "skip").unwrap(),
            NodeDefinition::new(c.clone(), "C", "echo").unwrap(),
            NodeDefinition::new(d.clone(), "D", "echo").unwrap(),
        ],
        vec![
            Connection::new(a.clone(), b.clone()),
            Connection::new(b.clone(), c.clone()),
            Connection::new(c.clone(), d.clone()),
        ],
    );

    let result = engine
        .execute_workflow(&wf, serde_json::json!("hi"), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(
        result.is_success(),
        "workflow should succeed even when middle node skips"
    );
    assert_eq!(
        result.node_output(&a),
        Some(&serde_json::json!("hi")),
        "A should execute normally"
    );
    assert!(
        result.node_output(&b).is_none() && !result.node_errors.contains_key(&b),
        "B returned Skip — no output, no error"
    );
    assert!(
        result.node_output(&c).is_none() && !result.node_errors.contains_key(&c),
        "C transitively skipped (one hop from B)"
    );
    assert!(
        result.node_output(&d).is_none() && !result.node_errors.contains_key(&d),
        "D transitively skipped (two hops from B)"
    );
}

/// Diamond pattern: A → {B, C} → D, where B returns Skip and C echoes.
///
/// Expected: D fires because at least one input edge (from C) activated —
/// resolved=2, required=2, activated=1. Verifies that `propagate_skip` does
/// NOT block a node that has any active source.
#[tokio::test]
async fn diamond_with_one_skipped_branch_still_completes() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(EchoHandler {
        meta: meta(action_key!("echo")),
    });
    registry.register_stateless(SkipHandler {
        meta: meta(action_key!("skip")),
    });
    let (engine, _) = make_engine(registry);

    let a = node_key!("a");
    let b = node_key!("b");
    let c = node_key!("c");
    let d = node_key!("d");

    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "echo").unwrap(),
            NodeDefinition::new(b.clone(), "B", "skip").unwrap(),
            NodeDefinition::new(c.clone(), "C", "echo").unwrap(),
            NodeDefinition::new(d.clone(), "D", "echo").unwrap(),
        ],
        vec![
            Connection::new(a.clone(), b.clone()),
            Connection::new(a.clone(), c.clone()),
            Connection::new(b.clone(), d.clone()),
            Connection::new(c.clone(), d.clone()),
        ],
    );

    let result = engine
        .execute_workflow(&wf, serde_json::json!("hi"), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_success());
    assert!(result.node_output(&a).is_some(), "A executed");
    assert!(
        result.node_output(&b).is_none() && !result.node_errors.contains_key(&b),
        "B skipped"
    );
    assert!(
        result.node_output(&c).is_some(),
        "C executed via the active branch"
    );
    assert!(
        result.node_output(&d).is_some(),
        "D fires because at least one input edge (from C) activated"
    );
}

/// Mixed-source aggregate: X(skip) and Y(echo) both feed Z. Z has resolved=2,
/// required=2, activated=1 → fires.
#[tokio::test]
async fn aggregate_with_one_skipped_source_fires() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(EchoHandler {
        meta: meta(action_key!("echo")),
    });
    registry.register_stateless(SkipHandler {
        meta: meta(action_key!("skip")),
    });
    let (engine, _) = make_engine(registry);

    let x = node_key!("x");
    let y = node_key!("y");
    let z = node_key!("z");

    let wf = make_workflow(
        vec![
            NodeDefinition::new(x.clone(), "X", "skip").unwrap(),
            NodeDefinition::new(y.clone(), "Y", "echo").unwrap(),
            NodeDefinition::new(z.clone(), "Z", "echo").unwrap(),
        ],
        vec![
            Connection::new(x.clone(), z.clone()),
            Connection::new(y.clone(), z.clone()),
        ],
    );

    let result = engine
        .execute_workflow(&wf, serde_json::json!(42), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_success());
    assert!(
        result.node_output(&x).is_none() && !result.node_errors.contains_key(&x),
        "X skipped"
    );
    assert!(result.node_output(&y).is_some(), "Y executed");
    assert!(
        result.node_output(&z).is_some(),
        "Z fires because Y's edge activates (1 of 2 sources)"
    );
}

/// All-sources-skipped aggregate: X(skip) and Y(skip) both feed Z. Z has
/// resolved=2, required=2, activated=0 → propagate_skip(Z) recurs from
/// the second arrival (whichever Skip-node's edge resolution fills the
/// counter last).
#[tokio::test]
async fn aggregate_with_all_sources_skipped_propagates_skip() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(SkipHandler {
        meta: meta(action_key!("skip")),
    });
    registry.register_stateless(EchoHandler {
        meta: meta(action_key!("echo")),
    });
    let (engine, _) = make_engine(registry);

    let x = node_key!("x");
    let y = node_key!("y");
    let z = node_key!("z");

    let wf = make_workflow(
        vec![
            NodeDefinition::new(x.clone(), "X", "skip").unwrap(),
            NodeDefinition::new(y.clone(), "Y", "skip").unwrap(),
            NodeDefinition::new(z.clone(), "Z", "echo").unwrap(),
        ],
        vec![
            Connection::new(x.clone(), z.clone()),
            Connection::new(y.clone(), z.clone()),
        ],
    );

    let result = engine
        .execute_workflow(&wf, serde_json::json!(0), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_success());
    assert!(
        result.node_output(&x).is_none() && !result.node_errors.contains_key(&x),
        "X skipped"
    );
    assert!(
        result.node_output(&y).is_none() && !result.node_errors.contains_key(&y),
        "Y skipped"
    );
    assert!(
        result.node_output(&z).is_none() && !result.node_errors.contains_key(&z),
        "Z transitively skipped — no source activated"
    );
}

/// Multi-hop skip with sibling activation:
///
/// ```text
///     A ──► B(skip) ──► C ──► D
///                             ▲
///     Sib ────────────────────┘
/// ```
///
/// The A→B→C path dies at B's Skip and propagates to C; the Sib→D edge gives
/// D a sibling source. Expected: A=Completed, B=Skipped, C=Skipped (transitive),
/// Sib=Completed, D=Completed (D fires because Sib's edge activated). Pins
/// the interaction between propagate_skip from one branch and an active
/// sibling source feeding the same downstream join.
#[tokio::test]
async fn multi_hop_skip_with_sibling_activation_still_runs() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(EchoHandler {
        meta: meta(action_key!("echo")),
    });
    registry.register_stateless(SkipHandler {
        meta: meta(action_key!("skip")),
    });
    let (engine, _) = make_engine(registry);

    let a = node_key!("a");
    let b = node_key!("b");
    let c = node_key!("c");
    let d = node_key!("d");
    let sib = node_key!("sib");

    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "echo").unwrap(),
            NodeDefinition::new(b.clone(), "B", "skip").unwrap(),
            NodeDefinition::new(c.clone(), "C", "echo").unwrap(),
            NodeDefinition::new(d.clone(), "D", "echo").unwrap(),
            NodeDefinition::new(sib.clone(), "Sib", "echo").unwrap(),
        ],
        vec![
            Connection::new(a.clone(), b.clone()),
            Connection::new(b.clone(), c.clone()),
            Connection::new(c.clone(), d.clone()),
            Connection::new(sib.clone(), d.clone()),
        ],
    );

    let result = engine
        .execute_workflow(&wf, serde_json::json!("hi"), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_success());
    assert!(result.node_output(&a).is_some(), "A executed");
    assert!(
        result.node_output(&b).is_none() && !result.node_errors.contains_key(&b),
        "B skipped"
    );
    assert!(
        result.node_output(&c).is_none() && !result.node_errors.contains_key(&c),
        "C transitively skipped from B"
    );
    assert!(result.node_output(&sib).is_some(), "sibling root executed");
    assert!(
        result.node_output(&d).is_some(),
        "D fires via sibling's edge despite the A→B→C branch skipping"
    );
}

/// Duplicate edges from the same skipped source: `X(skip)` has two parallel
/// `Connection` edges into `Z`. Locks the per-edge counter invariant
/// documented inline at `propagate_skip`'s edge loop ("Increment per-edge
/// count (not per-source) so that multiple edges from the same skipped
/// source to the same target are each counted").
///
/// Expected: `Z` has `required = 2` (two incoming edges, even though only
/// one source). `X`'s single Skip resolves both edges via the per-edge loop;
/// resolved=2, required=2, activated=0 → `propagate_skip(Z)`. A regression
/// that switched to per-source counting would leave `Z` with resolved=1
/// forever, hanging or never skipping.
#[tokio::test]
async fn duplicate_edges_from_skipped_source_count_per_edge() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(SkipHandler {
        meta: meta(action_key!("skip")),
    });
    registry.register_stateless(EchoHandler {
        meta: meta(action_key!("echo")),
    });
    let (engine, _) = make_engine(registry);

    let x = node_key!("x");
    let z = node_key!("z");

    let wf = make_workflow(
        vec![
            NodeDefinition::new(x.clone(), "X", "skip").unwrap(),
            NodeDefinition::new(z.clone(), "Z", "echo").unwrap(),
        ],
        vec![
            // Two parallel edges from the same skipped source to the same target.
            Connection::new(x.clone(), z.clone()),
            Connection::new(x.clone(), z.clone()),
        ],
    );

    let result = engine
        .execute_workflow(&wf, serde_json::json!("in"), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_success());
    assert!(
        result.node_output(&x).is_none() && !result.node_errors.contains_key(&x),
        "X skipped"
    );
    assert!(
        result.node_output(&z).is_none() && !result.node_errors.contains_key(&z),
        "Z transitively skipped — both duplicate edges from X resolved without activating"
    );
}
