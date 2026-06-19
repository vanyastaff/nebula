//! Engine-level wait-state integration tests.
//!
//! Covers the timer-wake park path: a node returning `ActionResult::Wait`
//! with `WaitCondition::Duration` or `WaitCondition::Until` must:
//!
//! 1. Park itself in `Waiting` (releasing the worker) without activating
//!    downstream edges.
//! 2. Transition directly to `Completed` once the timer fires.
//! 3. Activate downstream edges only after that `Completed` transition.
//!
//! Every test has a **falsifiability clause**: a comment naming the
//! regression it catches and how to make it go red.

use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use chrono::Utc;
use nebula_action::{
    ActionError,
    action::Action,
    metadata::ActionMetadata,
    output::ActionOutput,
    result::{ActionResult, WaitCondition},
    stateless::StatelessAction,
};
use nebula_core::{Dependencies, action_key, id::WorkflowId, node_key};
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, ExecutionEvent,
    InProcessRunner, WorkflowEngine,
};
use nebula_execution::{ExecutionStatus, context::ExecutionBudget};
use nebula_metrics::MetricsRegistry;
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition,
};

// ---------------------------------------------------------------------------
// Macro — Variant A trait shape with placeholder static metadata
// (same convention as retry.rs; real metadata flows through
// `register_stateless_instance`).
// ---------------------------------------------------------------------------

macro_rules! placeholder_action_impl {
    ($ty:ty, $key:expr, $name:expr, $desc:expr) => {
        impl Action for $ty {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            fn metadata() -> ActionMetadata {
                ActionMetadata::new($key, $name, $desc)
            }
            fn dependencies() -> &'static Dependencies {
                static D: OnceLock<Dependencies> = OnceLock::new();
                D.get_or_init(Dependencies::new)
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Test handlers
// ---------------------------------------------------------------------------

/// Immediately returns `ActionResult::Wait { condition: Duration(wait_for),
/// timeout: None, partial_output: Some(value) }`.
/// Simulates any action that parks itself for a timer-based condition.
struct WaitingHandler {
    wait_for: Duration,
    partial_output: serde_json::Value,
}

placeholder_action_impl!(
    WaitingHandler,
    action_key!("placeholder.waiting"),
    "WaitingPlaceholder",
    "placeholder"
);

impl StatelessAction for WaitingHandler {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::Wait {
            condition: WaitCondition::Duration {
                duration: self.wait_for,
            },
            timeout: None,
            partial_output: Some(ActionOutput::Value(self.partial_output.clone())),
        })
    }
}

/// Immediately returns `ActionResult::Wait` using `WaitCondition::Until`.
struct WaitUntilHandler {
    wake_at: chrono::DateTime<Utc>,
    partial_output: serde_json::Value,
}

placeholder_action_impl!(
    WaitUntilHandler,
    action_key!("placeholder.wait_until"),
    "WaitUntilPlaceholder",
    "placeholder"
);

impl StatelessAction for WaitUntilHandler {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::Wait {
            condition: WaitCondition::Until {
                datetime: self.wake_at,
            },
            timeout: None,
            partial_output: Some(ActionOutput::Value(self.partial_output.clone())),
        })
    }
}

/// Returns `ActionResult::Wait` with `WaitCondition::Webhook` — a
/// signal-driven condition that W-S1 does not support. Used to verify
/// `WaitConditionNotSupported` is returned rather than silently parking.
struct WebhookWaitHandler;

/// Returns `ActionResult::Wait` with a timer condition AND an explicit
/// `timeout`. W-S1 does not wire timeout-as-cancellation — parking while
/// silently ignoring the timeout would let the full timer run (potentially
/// hours) instead of the declared maximum. Must be rejected with a typed error.
struct WaitWithTimeoutHandler {
    wait_for: Duration,
    timeout: Duration,
}

placeholder_action_impl!(
    WaitWithTimeoutHandler,
    action_key!("placeholder.wait_with_timeout"),
    "WaitWithTimeoutPlaceholder",
    "placeholder"
);

impl StatelessAction for WaitWithTimeoutHandler {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::Wait {
            condition: WaitCondition::Duration {
                duration: self.wait_for,
            },
            timeout: Some(self.timeout),
            partial_output: None,
        })
    }
}

/// Returns `ActionResult::Wait` with a timer condition and a large
/// `partial_output` that exceeds a tight `ExecutionBudget::max_output_bytes`.
/// Used to verify that the park path enforces the output-size budget before
/// committing the park, rather than letting an over-budget output sneak through.
struct WaitWithOversizedOutputHandler {
    wait_for: Duration,
    /// The output value; its JSON serialization intentionally exceeds the test budget.
    output: serde_json::Value,
}

placeholder_action_impl!(
    WaitWithOversizedOutputHandler,
    action_key!("placeholder.wait_oversized_output"),
    "WaitOversizedOutputPlaceholder",
    "placeholder"
);

impl StatelessAction for WaitWithOversizedOutputHandler {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::Wait {
            condition: WaitCondition::Duration {
                duration: self.wait_for,
            },
            timeout: None,
            partial_output: Some(ActionOutput::Value(self.output.clone())),
        })
    }
}

placeholder_action_impl!(
    WebhookWaitHandler,
    action_key!("placeholder.webhook_wait"),
    "WebhookWaitPlaceholder",
    "placeholder"
);

impl StatelessAction for WebhookWaitHandler {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::Wait {
            condition: WaitCondition::Webhook {
                callback_id: "test-callback-id".to_owned(),
            },
            timeout: None,
            partial_output: None,
        })
    }
}

/// Simple echo: records how many times it was called and succeeds.
struct EchoHandler {
    invocations: Arc<AtomicU32>,
}

placeholder_action_impl!(
    EchoHandler,
    action_key!("placeholder.echo"),
    "EchoPlaceholder",
    "placeholder"
);

impl StatelessAction for EchoHandler {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(input))
    }
}

// ---------------------------------------------------------------------------
// Engine / workflow assembly helpers (mirrors retry.rs)
// ---------------------------------------------------------------------------

fn make_engine(registry: Arc<ActionRegistry>) -> WorkflowEngine {
    let metrics = MetricsRegistry::new();
    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let runner = Arc::new(InProcessRunner::new(executor));
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .unwrap(),
    );
    WorkflowEngine::new(runtime, metrics).unwrap()
}

fn make_workflow(
    nodes: Vec<NodeDefinition>,
    connections: Vec<Connection>,
    config: WorkflowConfig,
) -> WorkflowDefinition {
    let now = Utc::now();
    WorkflowDefinition {
        id: WorkflowId::new(),
        name: "wait-test".to_owned(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes,
        connections,
        variables: Default::default(),
        config,
        trigger_bindings: Vec::new(),
        tags: vec![],
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: CURRENT_SCHEMA_VERSION,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// 1) **Timer-based gate with mid-park `downstream_calls == 0` assertion**:
/// a two-node workflow `node1 → node2` where `node1` parks itself via
/// `WaitCondition::Duration`. The test asserts:
///
/// - While `node1` is `Waiting` (at the moment `NodeParked` is received),
///   `downstream_calls == 0` — downstream is gated by the park.
/// - A `NodeWaitCompleted` event is subsequently emitted.
/// - After the workflow finishes, `downstream_calls == 1`.
///
/// **Falsifiability (gate assertion)**:
/// - Remove the `continue` in the park path (letting it fall through to
///   `mark_node_completed` + `process_outgoing_edges`) → `node2` runs
///   before the timer fires → `downstream_calls == 1` at the `NodeParked`
///   instant → the `== 0` assertion fails. Restore the `continue` → green.
/// - Remove the `continue` in the park path entirely → no `NodeParked`
///   event is emitted → the event-wait loop never receives it → the
///   `tokio::time::timeout` on the event-wait fires → test fails.
#[tokio::test]
async fn wait_node_gates_downstream_until_timer_then_resumes() {
    let downstream_calls = Arc::new(AtomicU32::new(0));

    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("waiter"), "Waiter", "parks for 60ms"),
        WaitingHandler {
            wait_for: Duration::from_millis(60),
            partial_output: serde_json::json!({ "stage": "parked" }),
        },
    );
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("downstream"), "Downstream", "echo"),
        EchoHandler {
            invocations: Arc::clone(&downstream_calls),
        },
    );

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(128);
    let mut events_rx = event_bus.subscribe();
    let engine = Arc::new(make_engine(registry).with_event_bus(event_bus));

    let n1 = node_key!("node1");
    let n2 = node_key!("node2");
    let node1 = NodeDefinition::new(n1.clone(), "waiter_node", "core", "waiter").unwrap();
    let node2 = NodeDefinition::new(n2.clone(), "downstream_node", "core", "downstream").unwrap();
    let conn = Connection::new(n1.clone(), n2.clone());
    let wf = make_workflow(vec![node1, node2], vec![conn], WorkflowConfig::default());

    // Spawn the workflow so we can observe events while it runs.
    let engine_h = Arc::clone(&engine);
    let wf_arc = Arc::new(wf);
    let wf_for_task = Arc::clone(&wf_arc);
    let task = tokio::spawn(async move {
        engine_h
            .execute_workflow(
                &nebula_engine::store_seam::single_tenant_scope(),
                &wf_for_task,
                serde_json::json!(null),
                ExecutionBudget::default(),
            )
            .await
    });

    // Wait for `NodeParked` — node1 is now in `Waiting`, downstream gated.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events_rx.recv().await {
                Some(ExecutionEvent::NodeParked { node_key, .. }) if node_key == n1 => break,
                Some(_) => continue,
                None => panic!("event bus closed before NodeParked"),
            }
        }
    })
    .await
    .expect("timed out waiting for NodeParked event");

    // Mid-park gate assertion: downstream must NOT have run yet.
    // This is the falsifiable core: if `process_outgoing_edges` fires in
    // the park path, `downstream_calls` will already be 1 here.
    assert_eq!(
        downstream_calls.load(Ordering::SeqCst),
        0,
        "downstream must NOT run while node1 is Waiting — gate is broken if this fails"
    );

    // Now wait for the workflow to complete (the 60ms timer fires).
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("workflow must complete within 5s after NodeParked")
        .unwrap()
        .unwrap();

    // Final assertions.
    assert_eq!(
        result.status,
        ExecutionStatus::Completed,
        "workflow must complete successfully after wait timer fires"
    );
    assert_eq!(
        downstream_calls.load(Ordering::SeqCst),
        1,
        "downstream node must run exactly once after the wait is satisfied"
    );

    // Verify both lifecycle events were emitted.
    let mut saw_node_parked = false;
    let mut saw_node_wait_completed = false;
    while let Some(event) = events_rx.try_recv() {
        match event {
            ExecutionEvent::NodeParked { node_key, .. } if node_key == n1 => {
                saw_node_parked = true;
            },
            ExecutionEvent::NodeWaitCompleted { node_key, .. } if node_key == n1 => {
                saw_node_wait_completed = true;
            },
            _ => {},
        }
    }
    // `NodeParked` was consumed from the live stream above; confirm either
    // it was also queued for the drain OR re-assert we observed it above.
    // The live-stream receive already proved the event fired; this drain
    // catches `NodeWaitCompleted` which arrives after the timer.
    let _ = saw_node_parked; // already asserted live above
    assert!(
        saw_node_wait_completed,
        "a NodeWaitCompleted event must be emitted when the timer fires"
    );
}

/// 2) **`WaitCondition::Until` path**: same structural proof but via the
/// absolute-datetime variant. Verifies the `wake_at = Some(datetime)`
/// branch in the park-path compute.
#[tokio::test]
async fn wait_until_condition_gates_and_resumes() {
    let downstream_calls = Arc::new(AtomicU32::new(0));

    let registry = Arc::new(ActionRegistry::new());
    // Wake 80ms from now.
    let wake_at = Utc::now() + chrono::Duration::milliseconds(80);
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("waiter_until"), "WaiterUntil", "parks Until"),
        WaitUntilHandler {
            wake_at,
            partial_output: serde_json::json!("from_wait"),
        },
    );
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("ds_until"), "DsUntil", "downstream echo"),
        EchoHandler {
            invocations: Arc::clone(&downstream_calls),
        },
    );

    let engine = make_engine(registry);
    let n1 = node_key!("w1");
    let n2 = node_key!("d1");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(n1.clone(), "waiter_until_node", "core", "waiter_until").unwrap(),
            NodeDefinition::new(n2, "ds_until_node", "core", "ds_until").unwrap(),
        ],
        vec![Connection::new(n1, node_key!("d1"))],
        WorkflowConfig::default(),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        engine.execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        ),
    )
    .await
    .expect("workflow must complete within 5s")
    .unwrap();

    assert_eq!(result.status, ExecutionStatus::Completed);
    assert_eq!(
        downstream_calls.load(Ordering::SeqCst),
        1,
        "downstream must run exactly once after Until fires"
    );
}

/// 3) **No-worker-held proof**: a sibling node completes while `node1` is
/// parked in `Waiting`. If the park path held the worker semaphore (i.e.
/// a worker slot was not released), a single-slot semaphore would deadlock
/// and the sibling would never complete. The test passes only if the
/// engine released the worker on park.
///
/// **Falsifiability**: add a `semaphore.acquire().await` in the park path
/// without releasing it → the sibling blocks forever → `tokio::time::timeout`
/// fires → test fails with "timed out". Restore the release to go green.
#[tokio::test]
async fn parked_wait_node_holds_no_worker_sibling_completes() {
    let sibling_calls = Arc::new(AtomicU32::new(0));

    let registry = Arc::new(ActionRegistry::new());
    // 150ms park — long enough that the sibling can complete first.
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("slow_waiter"), "SlowWaiter", "parks for 150ms"),
        WaitingHandler {
            wait_for: Duration::from_millis(150),
            partial_output: serde_json::json!(null),
        },
    );
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("sibling"), "Sibling", "completes immediately"),
        EchoHandler {
            invocations: Arc::clone(&sibling_calls),
        },
    );

    let engine = make_engine(registry);
    let waiter = node_key!("waiter");
    let sibling = node_key!("sibling");

    // Two independent root nodes — no connection between them.
    let wf = make_workflow(
        vec![
            NodeDefinition::new(waiter, "waiter_node", "core", "slow_waiter").unwrap(),
            NodeDefinition::new(sibling, "sibling_node", "core", "sibling").unwrap(),
        ],
        vec![],
        WorkflowConfig::default(),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        engine.execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        ),
    )
    .await
    .expect("sibling must complete while waiter is parked — timeout means worker was held")
    .unwrap();

    assert_eq!(result.status, ExecutionStatus::Completed);
    assert_eq!(
        sibling_calls.load(Ordering::SeqCst),
        1,
        "sibling must complete exactly once while the other node is parked"
    );
}

/// 4) **Cancel during wait**: `cancel_execution` while a node is in
/// `Waiting` must drain the `wait_heap` (Waiting → Cancelled) and
/// return `Cancelled` status without hanging.
///
/// **Falsifiability**: remove the `wait_heap` drain from
/// `drain_pending_to_cancelled` → the `Waiting` node stays non-terminal
/// → the frontier integrity check fires, OR the engine loops forever
/// waiting for the heap to drain → timeout.
#[tokio::test]
async fn cancel_during_wait_drains_heap() {
    let registry = Arc::new(ActionRegistry::new());
    // 1-minute park — will not fire naturally during the test.
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("long_waiter"), "LongWaiter", "parks for 1 min"),
        WaitingHandler {
            wait_for: Duration::from_mins(1),
            partial_output: serde_json::json!(null),
        },
    );

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();
    let engine = Arc::new(make_engine(registry).with_event_bus(event_bus));

    let n = node_key!("long_wait");
    let wf = make_workflow(
        vec![NodeDefinition::new(n, "long_wait_node", "core", "long_waiter").unwrap()],
        vec![],
        WorkflowConfig::default(),
    );

    let engine_h = Arc::clone(&engine);
    let task = tokio::spawn(async move {
        engine_h
            .execute_workflow(
                &nebula_engine::store_seam::single_tenant_scope(),
                &wf,
                serde_json::json!(null),
                ExecutionBudget::default(),
            )
            .await
    });

    // Wait for the NodeParked event so we know the node is in Waiting
    // before we cancel (otherwise the cancel races the park).
    let execution_id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events_rx.recv().await {
                Some(ExecutionEvent::NodeParked {
                    execution_id: id, ..
                }) => break id,
                Some(_) => continue,
                None => panic!("event bus closed before NodeParked"),
            }
        }
    })
    .await
    .expect("timed out waiting for NodeParked event");

    let cancelled = engine.cancel_execution(execution_id);
    assert!(cancelled, "cancel_execution must find the live frontier");

    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("workflow must wind down within 5s after cancel")
        .unwrap()
        .unwrap();

    assert!(
        matches!(result.status, ExecutionStatus::Cancelled),
        "expected Cancelled, got {:?}",
        result.status
    );
}

/// 5) **Signal-driven condition rejected (W-S1 boundary)**:
/// a node returning `ActionResult::Wait` with `WaitCondition::Webhook`
/// must fail with a `WaitConditionNotSupported` error rather than parking
/// without a timer entry and stalling the execution indefinitely.
///
/// **Falsifiability**: change the park branch to handle `Webhook` the same
/// as `Duration` (with `wake_at = None`) → the node parks, the frontier
/// empties with a non-terminal `Waiting` node, and the engine either hangs
/// (no timer to fire) or triggers `FrontierIntegrityViolation`. Restore the
/// typed error return → the node fails immediately → `Failed` status → green.
#[tokio::test]
async fn webhook_wait_condition_returns_unsupported_error() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("webhook_waiter"),
            "WebhookWaiter",
            "waits for webhook callback",
        ),
        WebhookWaitHandler,
    );

    let engine = make_engine(registry);
    let n = node_key!("webhook_node");
    let wf = make_workflow(
        vec![NodeDefinition::new(n, "webhook_wait_node", "core", "webhook_waiter").unwrap()],
        vec![],
        WorkflowConfig::default(),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        engine.execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        ),
    )
    .await
    .expect("workflow must settle quickly — Webhook condition must be rejected immediately")
    .unwrap();

    // The workflow must end in Failed (not hung, not Completed, not Cancelled).
    assert_eq!(
        result.status,
        ExecutionStatus::Failed,
        "Webhook WaitCondition must cause a Failed execution, got {:?}",
        result.status
    );

    // At least one node error must reference the unsupported condition.
    let all_errors: String = result
        .node_errors
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("; ");
    assert!(
        all_errors.contains("Webhook") || all_errors.contains("not supported"),
        "node errors must reference 'Webhook' or 'not supported', got: {all_errors}"
    );
}

/// 6) **Explicit `timeout` on a timer `Wait` is rejected (P2-1)**:
/// an action returning `ActionResult::Wait` with `WaitCondition::Duration`
/// AND `timeout: Some(5s)` must fail with `WaitConditionNotSupported` rather
/// than parking for the full duration while silently discarding the timeout.
///
/// **Falsifiability**: change the park branch to strip the `timeout` field
/// from the destructure (via `..`) → the engine parks the node for the full
/// `wait_for` duration → the 5s test timeout fires first → the test panics
/// with "workflow must settle quickly". Restore the `timeout` capture and
/// rejection → the node fails immediately → `Failed` status → green.
#[tokio::test]
async fn explicit_timeout_on_timer_wait_returns_unsupported_error() {
    let registry = Arc::new(ActionRegistry::new());
    // The action requests a 1-minute park but also declares a 5-second
    // timeout. Without the rejection fix, the engine would ignore the 5s
    // timeout and park for the full minute, breaking the test budget.
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("timeout_waiter"),
            "TimeoutWaiter",
            "parks with an explicit timeout",
        ),
        WaitWithTimeoutHandler {
            wait_for: Duration::from_mins(1),
            timeout: Duration::from_secs(5),
        },
    );

    let engine = make_engine(registry);
    let n = node_key!("timeout_node");
    let wf = make_workflow(
        vec![NodeDefinition::new(n, "timeout_wait_node", "core", "timeout_waiter").unwrap()],
        vec![],
        WorkflowConfig::default(),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        engine.execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        ),
    )
    .await
    .expect(
        "workflow must fail immediately (timeout rejection) — \
         if the engine silently parks for 1 min, the 5s test deadline fires first",
    )
    .unwrap();

    assert_eq!(
        result.status,
        ExecutionStatus::Failed,
        "explicit timeout on WaitCondition must cause a Failed execution, got {:?}",
        result.status
    );

    let all_errors: String = result
        .node_errors
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("; ");
    assert!(
        all_errors.contains("timeout") || all_errors.contains("not supported"),
        "node errors must reference 'timeout' or 'not supported', got: {all_errors}"
    );
}

/// 7) **Over-budget `partial_output` fails the node, not parks it (P2-2)**:
/// a two-node workflow `node1 → node2` where `node1` parks itself with a
/// `partial_output` larger than `ExecutionBudget::max_output_bytes`. The
/// engine must fail `node1` (not park it) so `node2` is NEVER dispatched.
///
/// **Falsifiability**: remove the budget enforcement block from the park path
/// (the `partial_output_bytes > 0` block) → `node1` parks successfully →
/// the timer fires → `node2` is dispatched → `downstream_calls == 1` →
/// the `== 0` assertion fails. Restore the budget check → `node1` fails
/// immediately → `downstream_calls` stays 0 → green.
#[tokio::test]
async fn oversized_partial_output_fails_node_not_parks_downstream_blocked() {
    let downstream_calls = Arc::new(AtomicU32::new(0));

    let registry = Arc::new(ActionRegistry::new());
    // The partial output is 50 bytes of JSON; the budget allows only 10.
    let big_output = serde_json::json!("this-string-is-longer-than-ten-bytes");
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("oversized_waiter"),
            "OversizedWaiter",
            "parks with an oversized partial output",
        ),
        WaitWithOversizedOutputHandler {
            wait_for: Duration::from_millis(60),
            output: big_output,
        },
    );
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("ds_oversized"),
            "DsOversized",
            "downstream echo",
        ),
        EchoHandler {
            invocations: Arc::clone(&downstream_calls),
        },
    );

    let engine = make_engine(registry);
    let n1 = node_key!("big_waiter");
    let n2 = node_key!("ds_big");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(n1.clone(), "big_waiter_node", "core", "oversized_waiter").unwrap(),
            NodeDefinition::new(n2, "ds_big_node", "core", "ds_oversized").unwrap(),
        ],
        vec![Connection::new(n1, node_key!("ds_big"))],
        WorkflowConfig::default(),
    );

    // Budget of 10 bytes; the partial_output JSON is longer than that.
    let budget = ExecutionBudget::default().with_max_output_bytes(10);

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        engine.execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            budget,
        ),
    )
    .await
    .expect("workflow must settle quickly — budget violation fails the node immediately")
    .unwrap();

    assert_eq!(
        result.status,
        ExecutionStatus::Failed,
        "over-budget partial_output must cause a Failed execution, got {:?}",
        result.status
    );
    assert_eq!(
        downstream_calls.load(Ordering::SeqCst),
        0,
        "downstream must NEVER run when node1 fails due to budget violation \
         (falsifiability: without budget check, node1 parks and timer fires → ds runs → count=1)"
    );
}
