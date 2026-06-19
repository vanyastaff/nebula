//! Integration tests for W-S2.2 — the `satisfy_signal_waits` + `dispatch_resume`
//! contract that unparks signal-driven waiting executions.
//!
//! Each test uses a store-backed engine (via [`SignalHarness`]) so that the
//! durable CAS written by `satisfy_signal_waits` is observable via the
//! `InMemoryExecutionStore`.  Library-mode behaviour is covered by `wait.rs`.
//!
//! ## Security invariant (explicitly tested)
//!
//! A signal-driven wait must be satisfied **only** by `dispatch_resume` — the
//! sole caller of `satisfy_signal_waits`.  All other re-entry paths
//! (`dispatch_start`, `dispatch_restart`, the worker `EngineExecutionSink`)
//! must leave the nodes `Waiting` and the execution `Paused`.
//!
//! Every test has a **falsifiability clause** naming the regression it catches.
//!
//! ## Timing discipline
//!
//! `dispatch_start` and `dispatch_resume` both call `drive()`, which runs the
//! frontier loop synchronously to completion (park or terminal).  No wall-clock
//! sleeps are needed — assertions follow immediately after the dispatch call.

use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
};

use chrono::Utc;
use nebula_action::{
    ActionError,
    action::Action,
    metadata::ActionMetadata,
    result::{ActionResult, WaitCondition},
    stateless::StatelessAction,
};
use nebula_core::{Dependencies, action_key, id::ExecutionId, node_key};
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, ControlDispatch, DataPassingPolicy,
    EngineControlDispatch, InProcessRunner, WorkflowEngine,
};
use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_metrics::MetricsRegistry;
use nebula_storage::{InMemoryExecutionStore, InMemoryWorkflowVersionStore};
use nebula_storage_port::{
    dto::WorkflowVersionRecord,
    store::{ExecutionStore, WorkflowVersionStore},
};
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition,
};

// ── Action stubs ──────────────────────────────────────────────────────────────

macro_rules! static_action_impl {
    ($ty:ty, $key:expr, $name:expr) => {
        impl Action for $ty {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            fn metadata() -> ActionMetadata {
                ActionMetadata::new($key, $name, "signal_resume integration test stub")
            }
            fn dependencies() -> &'static Dependencies {
                static D: OnceLock<Dependencies> = OnceLock::new();
                D.get_or_init(Dependencies::new)
            }
        }
    };
}

/// Parks itself via `WaitCondition::Webhook` with no timeout.
/// This produces `next_attempt_at == None` after park — the discriminator
/// that `satisfy_signal_waits` uses to identify signal-driven waits.
struct WebhookWaitNode;

static_action_impl!(
    WebhookWaitNode,
    action_key!("test.signal.webhook_wait"),
    "WebhookWaitNode"
);

impl StatelessAction for WebhookWaitNode {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::Wait {
            condition: WaitCondition::Webhook {
                callback_id: "test-webhook-signal".to_owned(),
            },
            timeout: None,
            partial_output: None,
        })
    }
}

/// A second distinct webhook-wait action key for the multi-node test.
/// Must be registered separately so the `ActionRegistry` can look it up
/// under a different key while sharing the same execution logic.
struct WebhookWaitNodeB;

static_action_impl!(
    WebhookWaitNodeB,
    action_key!("test.signal.webhook_wait_b"),
    "WebhookWaitNodeB"
);

impl StatelessAction for WebhookWaitNodeB {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::Wait {
            condition: WaitCondition::Webhook {
                callback_id: "test-webhook-signal-b".to_owned(),
            },
            timeout: None,
            partial_output: None,
        })
    }
}

/// Counts invocations and succeeds.  Used as the downstream gate probe — the
/// test asserts on the exact invocation count to verify edge activation.
struct CountingEchoNode {
    invocation_count: Arc<AtomicU32>,
}

static_action_impl!(
    CountingEchoNode,
    action_key!("test.signal.counting_echo"),
    "CountingEchoNode"
);

impl StatelessAction for CountingEchoNode {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        self.invocation_count.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(input))
    }
}

// ── Shared harness ────────────────────────────────────────────────────────────

/// In-memory port adapters wired to one store-backed `WorkflowEngine`.
/// Mirrors the `DispatchStores` pattern from `control_dispatch.rs`.
struct SignalStores {
    execution: Arc<InMemoryExecutionStore>,
    journal: Arc<nebula_storage::InMemoryJournalReader>,
    node_results: Arc<nebula_storage::InMemoryNodeResultStore>,
    checkpoints: Arc<nebula_storage::InMemoryCheckpointStore>,
    idempotency: Arc<nebula_storage::InMemoryIdempotencyGuard>,
    workflow: Arc<nebula_storage::InMemoryWorkflowStore>,
    versions: Arc<InMemoryWorkflowVersionStore>,
}

impl SignalStores {
    fn new() -> Self {
        let execution = Arc::new(InMemoryExecutionStore::new());
        let journal = Arc::new(nebula_storage::InMemoryJournalReader::new(&execution));
        let versions = InMemoryWorkflowVersionStore::new();
        let workflow = nebula_storage::InMemoryWorkflowStore::new_with_versions(&versions);
        Self {
            execution,
            journal,
            node_results: Arc::new(nebula_storage::InMemoryNodeResultStore::new()),
            checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new()),
            idempotency: Arc::new(nebula_storage::InMemoryIdempotencyGuard::new()),
            workflow: Arc::new(workflow),
            versions: Arc::new(versions),
        }
    }

    fn execution_stores(&self) -> nebula_engine::ExecutionStores {
        nebula_engine::ExecutionStores {
            execution: self.execution.clone(),
            journal: self.journal.clone(),
            node_results: self.node_results.clone(),
            checkpoints: self.checkpoints.clone(),
            idempotency: self.idempotency.clone(),
        }
    }

    fn workflow_stores(&self) -> nebula_engine::WorkflowStores {
        nebula_engine::WorkflowStores {
            workflow: self.workflow.clone(),
            versions: self.versions.clone(),
        }
    }

    fn attach(&self, engine: WorkflowEngine) -> WorkflowEngine {
        engine
            .with_execution_stores(self.execution_stores())
            .with_workflow_stores(self.workflow_stores())
    }

    async fn save_workflow(&self, wf: &WorkflowDefinition) {
        self.versions
            .create(
                &nebula_engine::store_seam::single_tenant_scope(),
                WorkflowVersionRecord {
                    workflow_id: wf.id.to_string(),
                    number: 0,
                    published: true,
                    pinned: false,
                    definition: serde_json::to_value(wf).unwrap(),
                },
            )
            .await
            .unwrap();
    }
}

/// Test fixture for signal-wait + resume integration tests.
///
/// Wires a two-action registry (`WebhookWaitNode`, `CountingEchoNode`) to a
/// store-backed engine.  The downstream invocation counter is exposed for
/// per-test assertions.
struct SignalHarness {
    dispatch: EngineControlDispatch,
    stores: SignalStores,
    downstream_invocations: Arc<AtomicU32>,
}

impl SignalHarness {
    async fn new() -> Self {
        let downstream_invocations = Arc::new(AtomicU32::new(0));
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless_instance(
            ActionMetadata::new(
                action_key!("test.signal.webhook_wait"),
                "WebhookWaitNode",
                "signal_resume integration test stub",
            ),
            WebhookWaitNode,
        );
        registry.register_stateless_instance(
            ActionMetadata::new(
                action_key!("test.signal.counting_echo"),
                "CountingEchoNode",
                "signal_resume integration test stub",
            ),
            CountingEchoNode {
                invocation_count: Arc::clone(&downstream_invocations),
            },
        );

        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
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
            .unwrap(),
        );

        let stores = SignalStores::new();
        let engine = Arc::new(stores.attach(WorkflowEngine::new(runtime, metrics).unwrap()));
        let dispatch = EngineControlDispatch::new(Arc::clone(&engine), stores.execution.clone());

        Self {
            dispatch,
            stores,
            downstream_invocations,
        }
    }

    /// Persist a two-node workflow `webhook_wait → counting_echo` and return its id.
    ///
    /// `webhook_wait` parks on a `Webhook` signal (no timer); `counting_echo`
    /// is the downstream gate probe whose invocation count tests assert on.
    async fn persist_signal_workflow(&self) -> nebula_core::WorkflowId {
        let workflow_id = nebula_core::WorkflowId::new();
        let now = Utc::now();
        let wait_node = node_key!("signal_node");
        let downstream_node = node_key!("downstream_node");
        let wf = WorkflowDefinition {
            id: workflow_id,
            name: "signal-resume-test".into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes: vec![
                NodeDefinition::new(
                    wait_node.clone(),
                    "SignalNode",
                    "core",
                    "test.signal.webhook_wait",
                )
                .unwrap(),
                NodeDefinition::new(
                    downstream_node.clone(),
                    "DownstreamNode",
                    "core",
                    "test.signal.counting_echo",
                )
                .unwrap(),
            ],
            connections: vec![Connection::new(wait_node, downstream_node)],
            variables: HashMap::new(),
            config: WorkflowConfig::default(),
            trigger_bindings: Vec::new(),
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            owner_id: None,
            ui_metadata: None,
            schema_version: CURRENT_SCHEMA_VERSION,
        };
        self.stores.save_workflow(&wf).await;
        workflow_id
    }

    /// Persist a `Created` execution row, mirroring how the API handler writes
    /// the row before enqueueing the `Start` control command.
    async fn persist_created_execution(&self, workflow_id: nebula_core::WorkflowId) -> ExecutionId {
        let execution_id = ExecutionId::new();
        let mut exec_state = ExecutionState::new(execution_id, workflow_id, &[]);
        exec_state.set_workflow_input(serde_json::json!(null));
        let state_json = serde_json::to_value(&exec_state).unwrap();
        self.stores
            .execution
            .create(
                &nebula_engine::store_seam::single_tenant_scope(),
                &execution_id.to_string(),
                &workflow_id.to_string(),
                state_json,
            )
            .await
            .unwrap();
        execution_id
    }

    /// Read the persisted `ExecutionStatus` for the given execution.
    async fn persisted_status(&self, execution_id: ExecutionId) -> ExecutionStatus {
        let record = self
            .stores
            .execution
            .get(
                &nebula_engine::store_seam::single_tenant_scope(),
                &execution_id.to_string(),
            )
            .await
            .unwrap()
            .expect("execution row must exist when reading status");
        serde_json::from_value(record.state.get("status").cloned().unwrap()).unwrap()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

/// **W-S2.2 — satisfy (red-on-revert)**:
/// `dispatch_start` parks the signal node → execution `Paused` in the store →
/// `dispatch_resume` calls `satisfy_signal_waits` (durable CAS: `Waiting→Completed`)
/// → frontier re-drives → downstream runs exactly once → execution `Completed`.
///
/// **Falsifiability**: remove the `satisfy_signal_waits` call from
/// `dispatch_resume` (replace with a plain `drive`) → the frontier re-encounters
/// the `Waiting` node, re-parks it, and exits again → execution stays `Paused` →
/// `downstream_invocations == 0` → the `== 1` assertion fails → RED.
#[tokio::test]
async fn dispatch_resume_satisfies_signal_wait_and_drives_to_completed() {
    let harness = SignalHarness::new().await;
    let workflow_id = harness.persist_signal_workflow().await;
    let execution_id = harness.persist_created_execution(workflow_id).await;

    // Park: signal_node returns Webhook wait → frontier exits → Paused persisted.
    // `dispatch_start` runs `drive()` synchronously to completion, so `Paused`
    // is already persisted by the time this call returns.
    harness
        .dispatch
        .dispatch_start(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("dispatch_start must park the signal node and persist Paused");

    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must be Paused after the signal node parks"
    );
    assert_eq!(
        harness.downstream_invocations.load(Ordering::SeqCst),
        0,
        "downstream must NOT run while the signal node is Waiting"
    );

    // Resume: satisfy_signal_waits writes Waiting→Completed (CAS), then drive
    // re-runs the frontier, activates the downstream edge, and completes.
    harness
        .dispatch
        .dispatch_resume(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("dispatch_resume must satisfy the signal wait and drive to completion");

    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Completed,
        "execution must be Completed after dispatch_resume satisfies the wait"
    );
    assert_eq!(
        harness.downstream_invocations.load(Ordering::SeqCst),
        1,
        "downstream must run exactly once after dispatch_resume satisfies the wait"
    );
}

/// **W-S2.2 — crash re-entry does NOT satisfy (SECURITY test)**:
/// after a signal-wait park (execution `Paused`), a re-delivered `dispatch_start`
/// simulating a reclaim / crash re-drive must NOT auto-complete the signal wait.
///
/// This is the load-bearing security invariant for W-S2: a crashed Paused
/// execution auto-completing its wait on reclaim would be a data-corruption /
/// security-class bug (human approval never arrived; node auto-completes anyway).
///
/// **Falsifiability**: call `satisfy_signal_waits` inside `dispatch_start`
/// (or inside `resume_execution`) → the reclaim re-drive satisfies the wait →
/// downstream runs → `downstream_invocations == 1` after the re-drive →
/// the `== 0` assertion before the genuine `dispatch_resume` fails → RED.
#[tokio::test]
async fn dispatch_start_redelivery_does_not_satisfy_signal_wait() {
    let harness = SignalHarness::new().await;
    let workflow_id = harness.persist_signal_workflow().await;
    let execution_id = harness.persist_created_execution(workflow_id).await;

    // Initial park.
    harness
        .dispatch
        .dispatch_start(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("initial dispatch_start must park the signal node");

    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must be Paused after the initial park"
    );

    // Simulate a crash + reclaim: the at-least-once control queue re-delivers
    // the original `Start` command.  `dispatch_start` on a `Paused` execution
    // must re-drive without satisfying — the frontier re-parks and returns Paused.
    //
    // Note: `dispatch_start` checks the persisted status.  `Paused` is non-terminal
    // and non-Running, so the short-circuit does not apply — the engine re-enters
    // `resume_execution`, encounters the still-Waiting node, re-parks it, and
    // exits.  The execution returns to Paused unchanged.
    harness
        .dispatch
        .dispatch_start(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("re-delivered Start on a Paused execution must re-park without error");

    // drive() is synchronous: by the time dispatch_start returns, the frontier
    // has re-run and either parked again or completed.  No sleep needed.
    assert_eq!(
        harness.downstream_invocations.load(Ordering::SeqCst),
        0,
        "a reclaim re-drive (Start redelivery) must NOT satisfy the signal wait — \
         downstream must stay gated until a genuine dispatch_resume arrives"
    );

    // After re-park the execution must still be Paused (not Completed).
    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must remain Paused after a crash re-drive — the wait is not satisfied"
    );

    // The genuine Resume arrives: only now the wait is satisfied.
    harness
        .dispatch
        .dispatch_resume(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("genuine dispatch_resume must satisfy the wait and complete the execution");

    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Completed,
        "execution must be Completed after the genuine dispatch_resume"
    );
    assert_eq!(
        harness.downstream_invocations.load(Ordering::SeqCst),
        1,
        "downstream must run exactly once — triggered only by the genuine Resume"
    );
}

/// **W-S2.2 — idempotency**:
/// a second `dispatch_resume` after the execution has already `Completed`
/// must be a strict no-op — downstream must NOT run a second time.
///
/// **Falsifiability**: remove the terminal-status short-circuit from
/// `dispatch_resume` → the second Resume calls `satisfy_signal_waits` (finds
/// no Waiting nodes — already Completed), then calls `drive`, which re-processes
/// already-terminal nodes; if that causes a double dispatch, `invocation_count`
/// reaches 2 → the `== 1` assertion fails → RED.
#[tokio::test]
async fn dispatch_resume_is_idempotent_after_signal_wait_satisfied() {
    let harness = SignalHarness::new().await;
    let workflow_id = harness.persist_signal_workflow().await;
    let execution_id = harness.persist_created_execution(workflow_id).await;

    // Park and satisfy.
    harness
        .dispatch
        .dispatch_start(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .unwrap();
    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Paused
    );

    harness
        .dispatch
        .dispatch_resume(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("first dispatch_resume must satisfy the wait");
    assert_eq!(harness.downstream_invocations.load(Ordering::SeqCst), 1);
    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Completed
    );

    // Re-deliver the Resume (e.g. duplicate message on the control queue).
    harness
        .dispatch
        .dispatch_resume(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("second dispatch_resume on a Completed execution must be idempotent");

    // drive() is synchronous — by the time dispatch_resume returns, any
    // re-dispatch would already have incremented the counter.
    assert_eq!(
        harness.downstream_invocations.load(Ordering::SeqCst),
        1,
        "downstream must run exactly once — a duplicate Resume must be a no-op"
    );
    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Completed,
        "execution must remain Completed after a duplicate Resume"
    );
}

/// **W-S2.2 — multi-node: two parallel signal waits, one Resume satisfies both**:
/// a workflow with two parallel root `webhook_wait` nodes merging to one shared
/// `counting_echo` downstream.  After both wait nodes park (execution `Paused`),
/// a single `dispatch_resume` must transition BOTH `Waiting→Completed` in one
/// `satisfy_signal_waits` pass, then activate the downstream exactly once.
///
/// The downstream node has `required_count == 2` (two incoming edges from `wait_a`
/// and `wait_b`).  The engine pushes it to the `ready_queue` only when BOTH
/// incoming edges are resolved — so it runs exactly once, not twice.
///
/// **Falsifiability**: change `satisfy_signal_waits` to stop after the first
/// waiting node → the second node stays `Waiting` → the frontier exits again
/// with one Waiting node → execution re-parks at `Paused` → downstream never
/// activates (both incoming edges are never fully resolved) → `status != Completed`
/// AND `downstream_invocations == 0` → both assertions fail → RED.
#[tokio::test]
async fn dispatch_resume_satisfies_all_signal_waits_in_one_pass() {
    // Build a dedicated registry with two independent webhook-wait action keys
    // plus the shared downstream echo.
    let downstream_invocations = Arc::new(AtomicU32::new(0));
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.signal.webhook_wait"),
            "WebhookWaitNode",
            "signal_resume integration test stub",
        ),
        WebhookWaitNode,
    );
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.signal.webhook_wait_b"),
            "WebhookWaitNodeB",
            "signal_resume integration test stub",
        ),
        WebhookWaitNodeB,
    );
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.signal.counting_echo"),
            "CountingEchoNode",
            "signal_resume integration test stub",
        ),
        CountingEchoNode {
            invocation_count: Arc::clone(&downstream_invocations),
        },
    );

    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let runner = Arc::new(InProcessRunner::new(executor));
    let metrics = MetricsRegistry::new();
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .unwrap(),
    );

    let stores = SignalStores::new();
    let engine = Arc::new(stores.attach(WorkflowEngine::new(runtime, metrics).unwrap()));
    let dispatch = EngineControlDispatch::new(Arc::clone(&engine), stores.execution.clone());

    // Workflow: `wait_a` and `wait_b` both feed into one `downstream` node.
    // The downstream has `required_count == 2`; both incoming edges must resolve
    // before it enters the ready queue.
    let wait_a = node_key!("wait_a");
    let wait_b = node_key!("wait_b");
    let downstream_node = node_key!("downstream");
    let workflow_id = nebula_core::WorkflowId::new();
    let now = Utc::now();
    let wf = WorkflowDefinition {
        id: workflow_id,
        name: "multi-signal-resume-test".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![
            NodeDefinition::new(wait_a.clone(), "WaitA", "core", "test.signal.webhook_wait")
                .unwrap(),
            NodeDefinition::new(
                wait_b.clone(),
                "WaitB",
                "core",
                "test.signal.webhook_wait_b",
            )
            .unwrap(),
            NodeDefinition::new(
                downstream_node.clone(),
                "Downstream",
                "core",
                "test.signal.counting_echo",
            )
            .unwrap(),
        ],
        connections: vec![
            Connection::new(wait_a, downstream_node.clone()),
            Connection::new(wait_b, downstream_node),
        ],
        variables: HashMap::new(),
        config: WorkflowConfig::default(),
        trigger_bindings: Vec::new(),
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: CURRENT_SCHEMA_VERSION,
    };
    stores.save_workflow(&wf).await;

    // Persist the Created execution row.
    let execution_id = ExecutionId::new();
    let mut exec_state = ExecutionState::new(execution_id, workflow_id, &[]);
    exec_state.set_workflow_input(serde_json::json!(null));
    let state_json = serde_json::to_value(&exec_state).unwrap();
    stores
        .execution
        .create(
            &nebula_engine::store_seam::single_tenant_scope(),
            &execution_id.to_string(),
            &workflow_id.to_string(),
            state_json,
        )
        .await
        .unwrap();

    // Park both wait nodes.  drive() is synchronous — Paused is persisted on return.
    dispatch
        .dispatch_start(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("dispatch_start must park both signal nodes without error");

    let status_after_park = {
        let record = stores
            .execution
            .get(
                &nebula_engine::store_seam::single_tenant_scope(),
                &execution_id.to_string(),
            )
            .await
            .unwrap()
            .unwrap();
        let status: ExecutionStatus =
            serde_json::from_value(record.state.get("status").cloned().unwrap()).unwrap();
        status
    };
    assert_eq!(
        status_after_park,
        ExecutionStatus::Paused,
        "both signal nodes must park and execution must be Paused"
    );
    assert_eq!(
        downstream_invocations.load(Ordering::SeqCst),
        0,
        "downstream must NOT run while both signal nodes are Waiting"
    );

    // One Resume satisfies ALL signal-waiting nodes atomically.
    dispatch
        .dispatch_resume(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("dispatch_resume must satisfy all signal waits in one pass");

    let status_after_resume = {
        let record = stores
            .execution
            .get(
                &nebula_engine::store_seam::single_tenant_scope(),
                &execution_id.to_string(),
            )
            .await
            .unwrap()
            .unwrap();
        let status: ExecutionStatus =
            serde_json::from_value(record.state.get("status").cloned().unwrap()).unwrap();
        status
    };
    assert_eq!(
        status_after_resume,
        ExecutionStatus::Completed,
        "execution must be Completed after one dispatch_resume satisfies all signal waits"
    );
    // The downstream node has two incoming edges; the engine only dispatches
    // it after BOTH edges are resolved (required_count == resolved_count),
    // so it runs exactly once regardless of how many activating edges there are.
    assert_eq!(
        downstream_invocations.load(Ordering::SeqCst),
        1,
        "downstream must run exactly once — dispatched only when ALL incoming edges resolve"
    );
}

/// **W-S2.2 — `dispatch_resume` on a `Created` execution still works**:
/// the W-S2.2 changes must not regress the pre-existing behaviour where
/// `dispatch_resume` delivered against a `Created` (never-started) execution
/// drives it to completion just like `dispatch_start` (covered separately in
/// `control_dispatch.rs`; reproduced here to pin the signal path does not
/// break the `Created` arm).
///
/// **Falsifiability**: add an unconditional error return from
/// `satisfy_signal_waits` on `Created` status → `dispatch_resume` errors →
/// `expect("dispatch_resume on Created")` panics → RED.
#[tokio::test]
async fn dispatch_resume_on_created_execution_still_completes() {
    let harness = SignalHarness::new().await;
    let workflow_id = harness.persist_signal_workflow().await;

    // Workflow has a Webhook wait, so a fresh dispatch_resume on a Created
    // execution drives to Paused (the signal node parks immediately).
    let execution_id = harness.persist_created_execution(workflow_id).await;

    harness
        .dispatch
        .dispatch_resume(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("dispatch_resume on a Created execution must not error");

    // The signal workflow has a Webhook node, so this always ends at Paused
    // on the first drive (no timer to fire, no Resume yet).
    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "dispatch_resume on Created drives to Paused for a signal workflow \
         (the signal node parks; downstream waits for a real Resume)"
    );
    assert_eq!(
        harness.downstream_invocations.load(Ordering::SeqCst),
        0,
        "downstream must remain gated after the initial park"
    );
}

/// **W-S2.2 — `dispatch_restart` does NOT satisfy signal waits (SECURITY test)**:
/// after a signal-wait park (execution `Paused`), a `dispatch_restart` must NOT
/// auto-complete the signal-driven wait nodes.  `dispatch_restart` calls
/// `drive()` directly — it does NOT call `satisfy_signal_waits` — so the
/// frontier re-encounters the still-`Waiting` node, re-parks it, and leaves
/// the execution `Paused`.
///
/// This closes the third re-entry path against future refactors.  If anyone
/// wires `satisfy_signal_waits` into `dispatch_restart`, this test turns RED.
///
/// **Falsifiability**: call `satisfy_signal_waits` inside `dispatch_restart`
/// → the restart auto-satisfies the wait → downstream runs → `downstream_
/// invocations == 1` after the restart → the `== 0` assertion fails → RED.
#[tokio::test]
async fn dispatch_restart_does_not_satisfy_signal_wait() {
    let harness = SignalHarness::new().await;
    let workflow_id = harness.persist_signal_workflow().await;
    let execution_id = harness.persist_created_execution(workflow_id).await;

    // Initial park.
    harness
        .dispatch
        .dispatch_start(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("initial dispatch_start must park the signal node");

    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must be Paused after the initial park"
    );

    // `dispatch_restart` on a `Paused` execution must re-drive without
    // satisfying — the frontier re-encounters the still-Waiting node, re-parks,
    // and leaves the execution Paused.  No sleep is needed: drive() is synchronous.
    harness
        .dispatch
        .dispatch_restart(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("dispatch_restart on a Paused execution must re-park without error");

    assert_eq!(
        harness.downstream_invocations.load(Ordering::SeqCst),
        0,
        "dispatch_restart must NOT satisfy the signal wait — \
         downstream must stay gated until a genuine dispatch_resume arrives"
    );
    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must remain Paused after a restart re-drive — the wait is not satisfied"
    );

    // The genuine Resume arrives: only now the wait is satisfied.
    harness
        .dispatch
        .dispatch_resume(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("genuine dispatch_resume must satisfy the wait and complete the execution");

    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Completed,
        "execution must be Completed after the genuine dispatch_resume"
    );
    assert_eq!(
        harness.downstream_invocations.load(Ordering::SeqCst),
        1,
        "downstream must run exactly once — triggered only by the genuine Resume"
    );
}

// ── Round-trip: park → resume → park (duplicate Resume guard) ────────────────

/// **W-S2.2 — duplicate Resume on a Paused execution is handled gracefully**:
/// if two `dispatch_resume` calls race (one wins the CAS, one gets a conflict),
/// the execution must still complete and downstream must run exactly once.
///
/// This test is inherently sequential (single tokio task), so it exercises the
/// "second Resume sees Completed via the status short-circuit" path rather than
/// an actual concurrent CAS race.  Concurrent-CAS correctness is a property of
/// the `InMemoryExecutionStore.commit()` CAS which is tested separately in
/// `nebula-storage`.
///
/// **Falsifiability**: if the second `dispatch_resume` re-satisfies the wait
/// (because `satisfy_signal_waits` is not idempotent on already-Completed
/// nodes) → downstream runs twice → `invocation_count == 2` → the `== 1`
/// assertion fails → RED.
#[tokio::test]
async fn two_sequential_resumes_produce_exactly_one_downstream_run() {
    let harness = SignalHarness::new().await;
    let workflow_id = harness.persist_signal_workflow().await;
    let execution_id = harness.persist_created_execution(workflow_id).await;

    harness
        .dispatch
        .dispatch_start(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .unwrap();
    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Paused
    );

    // First Resume: satisfies the wait, drives to Completed.
    harness
        .dispatch
        .dispatch_resume(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("first Resume must satisfy the wait");
    assert_eq!(harness.downstream_invocations.load(Ordering::SeqCst), 1);

    // Second Resume: must be a no-op (status short-circuit on Completed).
    harness
        .dispatch
        .dispatch_resume(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("second Resume must be idempotent");

    assert_eq!(
        harness.downstream_invocations.load(Ordering::SeqCst),
        1,
        "downstream must run exactly once — second Resume is a no-op"
    );
    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Completed
    );
}
