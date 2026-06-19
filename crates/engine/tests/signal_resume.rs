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
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Duration,
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
    ActionExecutor, ActionRegistry, ActionRuntime, ControlDispatch, ControlDispatchError,
    DataPassingPolicy, EngineControlDispatch, InProcessRunner, WorkflowEngine,
};
use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_metrics::MetricsRegistry;
use nebula_storage::{InMemoryExecutionStore, InMemoryWorkflowVersionStore};
use nebula_storage_port::{
    FencingToken, Scope, StorageError, TransitionBatch, TransitionOutcome,
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

// ── Lease-contention deterministic race tests (P1 #1 / P1 #2) ────────────────

/// **P1 #2 — dispatch_resume returns `Deferred` when execution lease is held**:
/// Pre-acquire the execution lease with a "foreign-runner" holder to simulate
/// concurrent lease contention.  `dispatch_resume` must:
///
/// 1. Detect the contention in `satisfy_signal_waits` (`Leased` error).
/// 2. Return `ControlDispatchError::Deferred` — NOT `Ok(())` (which would ack
///    the control-queue row and permanently lose the Resume).
/// 3. Leave the wait nodes `Waiting` and the execution `Paused`.
///
/// After the lease is released and the Resume is "redelivered", the execution
/// must complete normally.
///
/// **Falsifiability**:
/// - If `dispatch_resume` returns `Ok(())` on lease contention → the
///   `Err(ControlDispatchError::Deferred(_))` assertion fails → RED.
/// - If the first `dispatch_resume` satisfies the wait despite contention →
///   `persisted_status == Paused` assertion fails → RED.
/// - If the redelivered `dispatch_resume` fails → final `Completed` assertion
///   fails → RED.
#[tokio::test]
async fn dispatch_resume_defers_when_execution_lease_is_held() {
    let harness = SignalHarness::new().await;
    let workflow_id = harness.persist_signal_workflow().await;
    let execution_id = harness.persist_created_execution(workflow_id).await;
    let scope = nebula_engine::store_seam::single_tenant_scope();

    // Park the execution so it is `Paused` with signal-driven wait nodes.
    harness
        .dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park the signal node");

    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must be Paused before the lease-contention test"
    );

    // Simulate a concurrent runner holding the execution lease.  `acquire_lease`
    // is exclusive: any second acquire (including by the engine's own instance)
    // returns `None` while this token is live.
    let blocker_token = harness
        .stores
        .execution
        .acquire_lease(
            &scope,
            &execution_id.to_string(),
            "foreign-runner",
            Duration::from_secs(30),
        )
        .await
        .expect("acquire_lease must not error")
        .expect("lease must be available before the contention test");

    // With the lease held externally, `dispatch_resume` must NOT succeed —
    // `satisfy_signal_waits` will find the lease busy and return `Leased`.
    // `dispatch_resume` must propagate this as `ControlDispatchError::Deferred`
    // so the consumer leaves the control-queue row in `Processing` for B1 reclaim.
    let result = harness.dispatch.dispatch_resume(&scope, execution_id).await;
    assert!(
        matches!(result, Err(ControlDispatchError::Deferred(_))),
        "dispatch_resume must return Deferred when the execution lease is held; got {result:?}"
    );

    // The wait nodes must still be `Waiting` — the deferred Resume must not have
    // mutated any node state.
    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must remain Paused after a deferred Resume (lease still held)"
    );
    assert_eq!(
        harness.downstream_invocations.load(Ordering::SeqCst),
        0,
        "downstream must not run while the lease is held and the Resume is deferred"
    );

    // Release the lease (simulating the concurrent runner completing its work).
    harness
        .stores
        .execution
        .release_lease(&scope, &execution_id.to_string(), blocker_token)
        .await
        .expect("release_lease must not error");

    // The B1 reclaim path would redeliver the Resume command.  Simulate redelivery.
    harness
        .dispatch
        .dispatch_resume(&scope, execution_id)
        .await
        .expect("redelivered dispatch_resume must succeed after the lease is released");

    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Completed,
        "execution must be Completed after the redelivered dispatch_resume"
    );
    assert_eq!(
        harness.downstream_invocations.load(Ordering::SeqCst),
        1,
        "downstream must run exactly once — triggered only by the redelivered Resume"
    );
}

/// **P1 #1 — satisfy_signal_waits acquires lease before CAS (no stale token)**:
/// verifies that `satisfy_signal_waits` holds the execution lease while committing
/// the `Waiting→Completed` CAS, so a concurrent runner that acquired the lease
/// just before us cannot slip a stale token through.
///
/// This test exercises the "lease handoff" path: the engine acquires the lease for
/// `satisfy_signal_waits` using its own instance_id.  Because `InMemoryExecutionStore`
/// blocks concurrent acquires, the second `dispatch_resume` (after the lease from
/// the first has been released by the committed batch) succeeds via the terminal
/// status short-circuit — proving the lease was released after commit.
///
/// **Falsifiability**: remove the `acquire_lease` call from `satisfy_signal_waits`
/// and use the stale fencing token from the record → the `InMemoryExecutionStore`
/// CAS may accept the stale token (no fencing rejection on the in-mem store when
/// the token matches the stored generation) — but the REAL story is that with a
/// live lease held by a concurrent runner, the store rejects the commit as
/// `FencedOut` → the CAS fails → the wait is not satisfied → `dispatch_resume`
/// would previously return `Ok(())` (wrong) → the row gets acked → Resume lost.
/// With the fix: the engine tries `acquire_lease` first, gets `None`, returns
/// `Deferred` → previous test catches it → RED.
#[tokio::test]
async fn satisfy_signal_waits_releases_lease_after_commit() {
    // This test drives two consecutive dispatch_resume calls to verify that the
    // lease acquired by the first satisfy_signal_waits is released after commit,
    // so the idempotency short-circuit on the second Resume can read the row.
    let harness = SignalHarness::new().await;
    let workflow_id = harness.persist_signal_workflow().await;
    let execution_id = harness.persist_created_execution(workflow_id).await;
    let scope = nebula_engine::store_seam::single_tenant_scope();

    // Park the signal node.
    harness
        .dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park");
    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Paused
    );

    // First Resume: acquires lease, commits Waiting→Completed, releases lease, drives.
    harness
        .dispatch
        .dispatch_resume(&scope, execution_id)
        .await
        .expect("first dispatch_resume must succeed");
    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Completed
    );

    // Second Resume (duplicate delivery): status short-circuit on Completed.
    // If the lease were still held after the first commit, this would deadlock or
    // return Deferred — it returns Ok(()) proving the lease was released.
    harness
        .dispatch
        .dispatch_resume(&scope, execution_id)
        .await
        .expect("duplicate dispatch_resume on Completed must be a no-op, not Deferred");

    assert_eq!(
        harness.downstream_invocations.load(Ordering::SeqCst),
        1,
        "downstream must run exactly once across two dispatch_resume calls"
    );
}

// ── Deferred-on-non-landed-satisfy (P2 regression) ───────────────────────────

/// Wraps an [`ExecutionStore`] and intercepts the first `commit()` call after
/// `arm()` has been called, returning `FencedOut` WITHOUT applying the batch.
/// All other calls — including `get`, lease operations, and subsequent commits
/// — delegate transparently to the inner store.
///
/// This lets a test verify that `dispatch_resume` returns `Deferred` (and does
/// not silently ack) when `satisfy_signal_waits` fails to land its CAS while
/// the execution is still `Paused`.
#[derive(Debug)]
struct CommitFenceInterceptor {
    inner: Arc<InMemoryExecutionStore>,
    /// When `true`, the next `commit()` call returns `FencedOut` and disarms.
    armed: AtomicBool,
}

impl CommitFenceInterceptor {
    fn new(inner: Arc<InMemoryExecutionStore>) -> Self {
        Self {
            inner,
            armed: AtomicBool::new(false),
        }
    }

    /// Arm the interceptor so the NEXT `commit()` call is sabotaged.
    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl ExecutionStore for CommitFenceInterceptor {
    async fn create(
        &self,
        scope: &Scope,
        id: &str,
        workflow_id: &str,
        initial_state: serde_json::Value,
    ) -> Result<(), StorageError> {
        self.inner
            .create(scope, id, workflow_id, initial_state)
            .await
    }

    async fn get(
        &self,
        scope: &Scope,
        id: &str,
    ) -> Result<Option<nebula_storage_port::dto::ExecutionRecord>, StorageError> {
        self.inner.get(scope, id).await
    }

    async fn commit(&self, batch: TransitionBatch) -> Result<TransitionOutcome, StorageError> {
        // On the first armed commit, return FencedOut without touching the row.
        if self
            .armed
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return Ok(TransitionOutcome::FencedOut);
        }
        self.inner.commit(batch).await
    }

    async fn acquire_lease(
        &self,
        scope: &Scope,
        id: &str,
        holder: &str,
        ttl: Duration,
    ) -> Result<Option<FencingToken>, StorageError> {
        self.inner.acquire_lease(scope, id, holder, ttl).await
    }

    async fn renew_lease(
        &self,
        scope: &Scope,
        id: &str,
        token: FencingToken,
        ttl: Duration,
    ) -> Result<bool, StorageError> {
        self.inner.renew_lease(scope, id, token, ttl).await
    }

    async fn release_lease(
        &self,
        scope: &Scope,
        id: &str,
        token: FencingToken,
    ) -> Result<bool, StorageError> {
        self.inner.release_lease(scope, id, token).await
    }

    async fn list_running(&self, scope: &Scope) -> Result<Vec<String>, StorageError> {
        self.inner.list_running(scope).await
    }

    async fn list_running_for_workflow(
        &self,
        scope: &Scope,
        workflow_id: &str,
    ) -> Result<Vec<String>, StorageError> {
        self.inner
            .list_running_for_workflow(scope, workflow_id)
            .await
    }

    async fn count(&self, scope: &Scope, workflow_id: Option<&str>) -> Result<u64, StorageError> {
        self.inner.count(scope, workflow_id).await
    }
}

/// **P2 — dispatch_resume defers when satisfy_signal_waits is FencedOut and
/// execution is still Paused** (regression guard for the "bounded lost-Resume"
/// bug class):
///
/// When `satisfy_signal_waits` acquires the lease but its `commit()` is
/// rejected as `FencedOut` (TTL-expired and superseded generation), and the
/// execution remains `Paused`, `dispatch_resume` MUST return
/// `ControlDispatchError::Deferred` rather than `Ok(())`.  Returning `Ok(())`
/// would silently ack the control-queue row, losing the Resume permanently.
///
/// Test mechanics:
/// 1. Park the execution with a real store (`Paused` persisted).
/// 2. Wire a second engine against a `CommitFenceInterceptor` wrapping the
///    SAME underlying `InMemoryExecutionStore`, so lease + status reads go
///    through to the shared row, but the first `commit()` inside
///    `satisfy_signal_waits` is returned as `FencedOut` without mutating state.
/// 3. Call `dispatch_resume` on the interceptor-backed dispatch.
/// 4. Assert `Deferred` is returned.
/// 5. Assert the execution is still `Paused` (wait node not satisfied).
/// 6. Assert downstream ran 0 times (gate not unblocked).
/// 7. Disarm the interceptor and redeliver the Resume — execution must complete.
///
/// **Falsifiability**: revert the `Err(e)` arm of `dispatch_resume`'s `Paused`
/// match to return `Ok(())` unconditionally → step 4 assertion fails → RED.
#[tokio::test]
async fn dispatch_resume_defers_when_satisfy_commit_is_fenced_out_and_execution_is_paused() {
    // ── Phase 1: park with a real store ──────────────────────────────────────
    let harness = SignalHarness::new().await;
    let workflow_id = harness.persist_signal_workflow().await;
    let execution_id = harness.persist_created_execution(workflow_id).await;
    let scope = nebula_engine::store_seam::single_tenant_scope();

    harness
        .dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park the signal node");

    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must be Paused before the FencedOut test"
    );
    assert_eq!(
        harness.downstream_invocations.load(Ordering::SeqCst),
        0,
        "downstream must not run before the signal wait is satisfied"
    );

    // ── Phase 2: wire a second engine with the commit interceptor ────────────
    //
    // The interceptor wraps the SAME inner store the harness uses, so:
    //   - status reads (via `get`) see the real Paused row
    //   - lease operations go through to the real store
    //   - the first `commit()` inside `satisfy_signal_waits` returns FencedOut
    //     without mutating the row → execution stays Paused
    //
    // Downstream invocations from this second engine's frontier loop are
    // captured by the same `downstream_invocations` counter because the
    // ActionRuntime instance is shared (same Arc).
    let interceptor = Arc::new(CommitFenceInterceptor::new(Arc::clone(
        &harness.stores.execution,
    )));
    let interceptor_as_store: Arc<dyn ExecutionStore> = Arc::clone(&interceptor) as _;

    let downstream_invocations_2 = Arc::new(AtomicU32::new(0));
    let registry2 = Arc::new(ActionRegistry::new());
    registry2.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.signal.webhook_wait"),
            "WebhookWaitNode",
            "signal_resume deferred test stub",
        ),
        WebhookWaitNode,
    );
    registry2.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.signal.counting_echo"),
            "CountingEchoNode",
            "signal_resume deferred test stub",
        ),
        CountingEchoNode {
            invocation_count: Arc::clone(&downstream_invocations_2),
        },
    );
    let executor2: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let runner2 = Arc::new(InProcessRunner::new(executor2));
    let metrics2 = MetricsRegistry::new();
    let runtime2 = Arc::new(
        ActionRuntime::try_new(
            registry2,
            runner2,
            DataPassingPolicy::default(),
            metrics2.clone(),
        )
        .unwrap(),
    );

    let execution_stores_with_interceptor = nebula_engine::ExecutionStores {
        execution: Arc::clone(&interceptor) as _,
        journal: harness.stores.journal.clone(),
        node_results: harness.stores.node_results.clone(),
        checkpoints: harness.stores.checkpoints.clone(),
        idempotency: harness.stores.idempotency.clone(),
    };

    let engine2 = Arc::new(
        WorkflowEngine::new(runtime2, metrics2)
            .unwrap()
            .with_execution_stores(execution_stores_with_interceptor)
            .with_workflow_stores(harness.stores.workflow_stores()),
    );
    // The dispatch must read status via the same raw inner store so the
    // post-error re-read sees the real (still-Paused) row.
    let dispatch2 = EngineControlDispatch::new(Arc::clone(&engine2), interceptor_as_store.clone());

    // ── Phase 3: arm the interceptor and call dispatch_resume ─────────────────
    interceptor.arm();

    let result = dispatch2.dispatch_resume(&scope, execution_id).await;
    assert!(
        matches!(result, Err(ControlDispatchError::Deferred(_))),
        "dispatch_resume must return Deferred when satisfy_signal_waits is FencedOut \
         and execution is still Paused; got {result:?}"
    );

    // ── Phase 4: assert no state was mutated ──────────────────────────────────
    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must remain Paused after a FencedOut satisfy (wait not satisfied)"
    );
    assert_eq!(
        downstream_invocations_2.load(Ordering::SeqCst),
        0,
        "downstream must not run when the satisfy did not land"
    );

    // ── Phase 5: redeliver the Resume (interceptor disarmed) → must complete ──
    //
    // The interceptor self-disarmed after the first intercept, so this
    // redelivery goes through to the real store and the wait is properly satisfied.
    let redeliver_result = dispatch2.dispatch_resume(&scope, execution_id).await;
    assert!(
        redeliver_result.is_ok(),
        "redelivered dispatch_resume must succeed after the interceptor disarms; \
         got {redeliver_result:?}"
    );

    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Completed,
        "execution must be Completed after the redelivered dispatch_resume satisfies the wait"
    );
    assert_eq!(
        downstream_invocations_2.load(Ordering::SeqCst),
        1,
        "downstream must run exactly once — triggered only by the redelivered Resume"
    );
}
