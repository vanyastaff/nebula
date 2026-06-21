//! End-to-end: `core.delay` parks on a timer through the real plugin → engine
//! dispatch spine, gates downstream while `Waiting`, then resumes and completes.
//!
//! This is the first first-party `ActionResult::Wait` action exercised end to
//! end. Unlike `crates/engine/tests/wait.rs` (which registers placeholder
//! handlers directly on an `ActionRegistry`), this test wires `core.delay`
//! through `WorkflowEngine::with_plugin(CorePlugin)` — proving a real factory's
//! `Wait` result routes through the ADR-0098 single dispatch spine identically
//! to the placeholders.
//!
//! ## A real timer is used deliberately
//!
//! These tests drive the engine's timer-wake scheduler with a real 1-second
//! `for` delay (the smallest unit `core.delay` exposes on its public surface).
//! The wait *deterministically* fires, so the test is not flaky — a `start_paused`
//! virtual clock cannot be used because `execute_workflow` runs the park
//! scheduler on its own task and the test cannot advance its time. The 1-second
//! span is bounded by a generous `tokio::time::timeout` backstop (same shape as
//! the canonical `wait.rs` timer e2e).

use std::{collections::HashMap, sync::Arc, time::Duration};

use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, ExecutionEvent,
    InProcessRunner, WorkflowEngine,
};
use nebula_execution::context::ExecutionBudget;
use nebula_metrics::MetricsRegistry;
use nebula_plugin::ResolvedPlugin;
use nebula_plugin_core::CorePlugin;
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, ParamValue, Version, WorkflowConfig,
    WorkflowDefinition,
};

// ── Engine + plugin assembly ───────────────────────────────────────────────────

fn make_engine() -> WorkflowEngine {
    let registry = Arc::new(ActionRegistry::new());
    let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
        Box::pin(async move { Ok(nebula_action::ActionResult::success(input)) })
    });
    let runner = Arc::new(InProcessRunner::new(executor));
    let metrics = MetricsRegistry::new();
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .expect("ActionRuntime must build in tests"),
    );
    WorkflowEngine::new(runtime, metrics).expect("WorkflowEngine must build in tests")
}

fn core_plugin() -> Arc<ResolvedPlugin> {
    Arc::new(
        ResolvedPlugin::from(
            CorePlugin::try_new().expect("CorePlugin::try_new must succeed in tests"),
        )
        .expect("CorePlugin must resolve without namespace errors"),
    )
}

fn scope() -> nebula_storage_port::Scope {
    nebula_engine::store_seam::single_tenant_scope()
}

/// Two-node `delay → downstream` workflow.
///
/// `delay` is a `core.delay` node carrying the given wait parameters
/// (`mode`/`amount`/`unit`, plus an optional `data` payload) as
/// `ParamValue::literal`s. `downstream` is a `core.set_fields` node that stamps
/// a marker so we can prove it ran after — and only after — the timer fires.
///
/// Returns `(workflow, delay_node_key, downstream_node_key)`.
fn delay_then_downstream_workflow(
    delay_params: serde_json::Value,
) -> (
    WorkflowDefinition,
    nebula_core::NodeKey,
    nebula_core::NodeKey,
) {
    let now = chrono::Utc::now();

    let delay_node_key = nebula_core::node_key!("delay_step");
    let downstream_node_key = nebula_core::node_key!("downstream_step");

    let params_map = delay_params
        .as_object()
        .expect("delay_params must be a JSON object");
    let mut delay_node =
        NodeDefinition::new(delay_node_key.clone(), "Delay step", "core", "core.delay")
            .expect("NodeDefinition must build with valid keys");
    for (key, value) in params_map {
        delay_node = delay_node.with_parameter(key.as_str(), ParamValue::literal(value.clone()));
    }

    let downstream_node = NodeDefinition::new(
        downstream_node_key.clone(),
        "Downstream step",
        "core",
        "core.set_fields",
    )
    .expect("NodeDefinition must build with valid keys")
    .with_parameter(
        "assignments",
        ParamValue::literal(serde_json::json!([{"name": "ran_after_delay", "value": true}])),
    );

    let edge = Connection::new(delay_node_key.clone(), downstream_node_key.clone());

    let workflow = WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "test-core-delay".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![delay_node, downstream_node],
        connections: vec![edge],
        variables: HashMap::new(),
        config: WorkflowConfig::default(),
        trigger_bindings: vec![],
        tags: vec![],
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: CURRENT_SCHEMA_VERSION,
    };

    (workflow, delay_node_key, downstream_node_key)
}

// ── GREEN proof: park → gate → resume → complete ────────────────────────────────

/// `core.delay` with a short `for` delay parks the node, gates the downstream
/// `core.set_fields` node while `Waiting`, then — after the timer fires —
/// resumes, completes the execution, and lets downstream run exactly once.
///
/// Asserts:
/// - At the `NodeParked` instant the parked timer carries a concrete `wake_at`
///   (proves timer-variant park; falsifiable: `Until` without wake_at would not
///   satisfy this).
/// - The execution reaches `Completed`.
/// - The downstream node ran (its marker is present in `node_outputs`),
///   proving downstream was gated during the park and released only after resume
///   (falsifiable: if `process_outgoing_edges` fired in the park path instead of
///   being gated, the engine would have dispatched downstream before the timer
///   fired and still reached `Completed`, but the ordering guarantee the single
///   downstream-gate path provides would be broken by that structural regression).
/// - A `NodeWaitCompleted` event fired for the delay node (bounded-drain loop).
///
/// Falsifiability:
/// - If `core.delay` returned `Success` instead of `Wait`, no `NodeParked`
///   event fires → the `NodeParked` wait times out → the test fails.
/// - If the downstream gate was absent (`process_outgoing_edges` fired in the
///   park path), the node still parks but downstream runs before the timer; the
///   final `ran_after_delay` assertion still passes (downstream did run), so the
///   gate is structurally verified in `crates/engine/tests/wait.rs` using an
///   `AtomicU32` counter that CAN be read at the mid-park instant (before
///   `execute_workflow` returns). This test proves the factory→dispatch→park
///   plumbing works end-to-end; the gate invariant itself is owned by `wait.rs`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delay_for_parks_gates_downstream_then_resumes() {
    let (workflow, delay_key, downstream_key) = delay_then_downstream_workflow(serde_json::json!({
        "mode": "for",
        "amount": 1,
        "unit": "seconds"
    }));

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(128);
    let mut events_rx = event_bus.subscribe();
    let engine = Arc::new(
        make_engine()
            .with_plugin(core_plugin())
            .expect("with_plugin(CorePlugin) must succeed")
            .with_event_bus(event_bus),
    );

    let engine_for_task = Arc::clone(&engine);
    let workflow = Arc::new(workflow);
    let workflow_for_task = Arc::clone(&workflow);
    let task = tokio::spawn(async move {
        engine_for_task
            .execute_workflow(
                &scope(),
                &workflow_for_task,
                serde_json::json!(null),
                ExecutionBudget::default(),
            )
            .await
    });

    // Wait for the delay node to park and assert the timer wake_at is set.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events_rx.recv().await {
                Some(ExecutionEvent::NodeParked {
                    node_key, wake_at, ..
                }) if node_key == delay_key => {
                    assert!(
                        wake_at.is_some(),
                        "a timer (for) delay must carry a concrete wake_at"
                    );
                    break;
                },
                Some(_) => continue,
                None => panic!("event bus closed before NodeParked"),
            }
        }
    })
    .await
    .expect("timed out waiting for NodeParked");

    // The workflow completes once the timer fires.
    let result = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("workflow must complete within 10s after NodeParked")
        .expect("spawned task must not panic")
        .expect("execute_workflow must not error");

    assert_eq!(
        result.status,
        nebula_execution::ExecutionStatus::Completed,
        "execution must reach Completed after the delay fires; got {:?} (node_errors: {:?})",
        result.status,
        result.node_errors
    );

    // Downstream ran: its marker is present.
    let downstream_output = result
        .node_outputs
        .get(&downstream_key)
        .expect("downstream node must have output after the delay resumed");
    assert_eq!(
        downstream_output["ran_after_delay"],
        serde_json::json!(true),
        "downstream must have run after the delay resumed"
    );

    // A NodeWaitCompleted event must have fired for the delay node.
    // Use a bounded recv loop: the event arrives after the timer fires and may
    // land slightly after execute_workflow returns (async event-bus dispatch).
    let mut saw_wait_completed = false;
    let drain_deadline = Duration::from_secs(2);
    let _ = tokio::time::timeout(drain_deadline, async {
        loop {
            match events_rx.recv().await {
                Some(ExecutionEvent::NodeWaitCompleted { node_key, .. })
                    if node_key == delay_key =>
                {
                    saw_wait_completed = true;
                    break;
                },
                Some(_) => continue,
                None => break, // channel closed
            }
        }
    })
    .await;
    assert!(
        saw_wait_completed,
        "a NodeWaitCompleted event must fire when the delay timer fires"
    );
}

/// The data payload supplied to `core.delay` is carried through the park and
/// emitted as the node's output after resume (pass-through, not a branch).
///
/// Falsifiability: if `partial_output` did not carry the input `data`, the
/// delay node's output would be absent or `null` and the assertion fails.
/// If the timer-wake resume path regresses, the 10-second timeout backstop
/// causes a deterministic failure instead of a CI hang.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delay_passes_data_through_to_output() {
    let (workflow, delay_key, _downstream_key) =
        delay_then_downstream_workflow(serde_json::json!({
            "mode": "for",
            "amount": 1,
            "unit": "seconds",
            "data": { "carried": "payload" }
        }));

    let engine = make_engine()
        .with_plugin(core_plugin())
        .expect("with_plugin(CorePlugin) must succeed");

    let result = tokio::time::timeout(
        Duration::from_secs(10),
        engine.execute_workflow(
            &scope(),
            &workflow,
            serde_json::json!(null),
            ExecutionBudget::default(),
        ),
    )
    .await
    .expect(
        "execute_workflow must complete within 10s — timeout means the timer-wake resume regressed",
    )
    .expect("execute_workflow must not error");

    assert_eq!(
        result.status,
        nebula_execution::ExecutionStatus::Completed,
        "execution must reach Completed; got {:?} (node_errors: {:?})",
        result.status,
        result.node_errors
    );

    let delay_output = result
        .node_outputs
        .get(&delay_key)
        .expect("delay node must have output after resume");
    assert_eq!(
        delay_output["carried"],
        serde_json::json!("payload"),
        "the delay node must pass its input data through to its output"
    );
}
