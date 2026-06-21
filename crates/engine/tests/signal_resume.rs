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
        Arc, Mutex, OnceLock,
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

/// Error-branch probe: a second counting node under a distinct action key so a
/// multi-port wait test can assert the `error`-port branch's invocation count
/// independently of the `main`-port branch.
struct CountingErrorNode {
    invocation_count: Arc<AtomicU32>,
}

static_action_impl!(
    CountingErrorNode,
    action_key!("test.signal.counting_error"),
    "CountingErrorNode"
);

impl StatelessAction for CountingErrorNode {
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
            resume_tokens: Arc::new(self.execution.resume_token_store()),
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

    /// Force the persisted execution `status`, mirroring how the API
    /// `cancel_execution` handler writes a raw-JSON `status` under a fencing
    /// token before the `Cancel` control command is drained. The node states are
    /// left untouched — exactly the state the engine's `cancel_dangling_nodes`
    /// must repair.
    async fn force_status(&self, execution_id: ExecutionId, status: ExecutionStatus) {
        let scope = nebula_engine::store_seam::single_tenant_scope();
        let id = execution_id.to_string();
        let token = self
            .stores
            .execution
            .acquire_lease(&scope, &id, "test-api-cancel", Duration::from_secs(30))
            .await
            .unwrap()
            .expect("lease must be free for the simulated API cancel write");
        let record = self
            .stores
            .execution
            .get(&scope, &id)
            .await
            .unwrap()
            .expect("execution row must exist");
        let mut state = record.state;
        state
            .as_object_mut()
            .unwrap()
            .insert("status".to_owned(), serde_json::json!(status.to_string()));
        let batch = TransitionBatch::builder()
            .scope(scope.clone())
            .execution_id(&id)
            .expected_version(record.version)
            .fencing(token)
            .new_state(state)
            .build()
            .unwrap();
        assert!(matches!(
            self.stores.execution.commit(batch).await.unwrap(),
            TransitionOutcome::Applied { .. }
        ));
        self.stores
            .execution
            .release_lease(&scope, &id, token)
            .await
            .unwrap();
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

/// **W-S2.2 — satisfy (red-on-revert)**:
/// `dispatch_start` parks the signal node → execution `Paused` in the store →
/// `dispatch_resume` calls `satisfy_signal_waits` (durable CAS arming the wait's
/// `next_attempt_at`) → frontier re-drives → Phase-0b completes the node and runs
/// downstream exactly once → execution `Completed`.
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

    // Resume: satisfy_signal_waits arms the wait (CAS on next_attempt_at), then
    // drive re-runs the frontier; Phase-0b completes the node, activates the
    // downstream edge, and the execution completes.
    harness
        .dispatch
        .dispatch_resume(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
            None,
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
            None,
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
            None,
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
            None,
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
/// a single `dispatch_resume` must arm BOTH waits in one `satisfy_signal_waits`
/// pass; Phase-0b then completes both and activates the downstream exactly once.
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
            None,
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
            None,
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
            None,
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
            None,
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
            None,
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
    let result = harness
        .dispatch
        .dispatch_resume(&scope, execution_id, None)
        .await;
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
        .dispatch_resume(&scope, execution_id, None)
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
/// the arming CAS (next_attempt_at), so a concurrent runner that acquired the lease
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

    // First Resume: acquires lease, commits the arm (next_attempt_at), releases
    // lease, drives (Phase-0b completes the node).
    harness
        .dispatch
        .dispatch_resume(&scope, execution_id, None)
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
        .dispatch_resume(&scope, execution_id, None)
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
        resume_tokens: Arc::new(harness.stores.execution.resume_token_store()),
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

    let result = dispatch2.dispatch_resume(&scope, execution_id, None).await;
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
    let redeliver_result = dispatch2.dispatch_resume(&scope, execution_id, None).await;
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

/// Wraps an [`InMemoryExecutionStore`] and simulates a **concurrent Cancel
/// landing during lease acquisition**: once `acquire_lease` has run, the next
/// `get` (the under-lease reload inside `satisfy_signal_waits`) returns a record
/// whose execution status is rewritten to `Cancelled`, while the earlier status
/// read (`dispatch_resume`'s pre-lease `read_status`) still observes the real
/// `Paused` row. Records whether any `commit` was attempted.
#[derive(Debug)]
struct CancelDuringSatisfyInterceptor {
    inner: Arc<InMemoryExecutionStore>,
    /// When `true`, the first `get` AFTER `acquire_lease` returns a `Cancelled`
    /// view of the row (then self-disarms).
    armed: AtomicBool,
    /// Set once `acquire_lease` has been called, so only the under-lease reload
    /// (not the pre-lease `read_status`) sees the injected cancel.
    lease_acquired: AtomicBool,
    /// Set if any `commit` is attempted — the fix must prevent a durable write
    /// once the under-lease reload observes a terminal status.
    commit_attempted: AtomicBool,
}

impl CancelDuringSatisfyInterceptor {
    fn new(inner: Arc<InMemoryExecutionStore>) -> Self {
        Self {
            inner,
            armed: AtomicBool::new(false),
            lease_acquired: AtomicBool::new(false),
            commit_attempted: AtomicBool::new(false),
        }
    }

    /// Arm the injection: the first `get` after the lease is acquired returns a
    /// `Cancelled` view.
    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }

    fn commit_attempted(&self) -> bool {
        self.commit_attempted.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl ExecutionStore for CancelDuringSatisfyInterceptor {
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
        let rec = self.inner.get(scope, id).await?;
        // Inject the concurrent cancel only on the under-lease reload (after
        // acquire_lease), and only once.
        if self.lease_acquired.load(Ordering::SeqCst)
            && self
                .armed
                .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            return Ok(rec.map(|mut record| {
                if let Some(obj) = record.state.as_object_mut() {
                    obj.insert(
                        "status".to_owned(),
                        serde_json::to_value(ExecutionStatus::Cancelled)
                            .expect("serialize ExecutionStatus::Cancelled"),
                    );
                }
                record
            }));
        }
        Ok(rec)
    }

    async fn commit(&self, batch: TransitionBatch) -> Result<TransitionOutcome, StorageError> {
        self.commit_attempted.store(true, Ordering::SeqCst);
        self.inner.commit(batch).await
    }

    async fn acquire_lease(
        &self,
        scope: &Scope,
        id: &str,
        holder: &str,
        ttl: Duration,
    ) -> Result<Option<FencingToken>, StorageError> {
        let token = self.inner.acquire_lease(scope, id, holder, ttl).await?;
        self.lease_acquired.store(true, Ordering::SeqCst);
        Ok(token)
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

/// **P2 — satisfy_signal_waits must guard the execution status under the lease**
/// (regression guard for the Codex finding on commit `d8814879`):
///
/// `dispatch_resume` reads `Paused` BEFORE acquiring the execution lease. If a
/// concurrent Cancel/Terminate commits `Cancelled` in the window before
/// `satisfy_signal_waits` reloads under the lease, the per-node `Waiting →
/// Completed` CAS (which guards only the node version, not the execution status)
/// would flip — and durably commit — a wait node on an already-cancelled
/// execution, corrupting its terminal audit state. `drive()` then only observes
/// the terminal status and acks the Resume.
///
/// The fix: `satisfy_signal_waits` re-checks `exec_state.status` under the lease
/// and returns `ExecutionNotResumable` (idempotent no-op, no commit) when the
/// execution is terminal/`Cancelling`; `dispatch_resume` acks without driving.
///
/// Test mechanics:
/// 1. Park with a real store (`Paused` persisted, signal node `Waiting`).
/// 2. Wire a second engine against a `CancelDuringSatisfyInterceptor`: the
///    pre-lease `read_status` sees `Paused`, but the under-lease reload inside
///    `satisfy_signal_waits` sees an injected `Cancelled` status.
/// 3. Call `dispatch_resume` → must return `Ok` (idempotent ack).
/// 4. Assert NO commit was attempted (the durable-write invariant).
/// 5. Assert the signal node is still `Waiting` in the real row (not flipped).
/// 6. Assert downstream ran 0 times.
///
/// **Falsifiability**: remove the under-lease status guard in
/// `satisfy_signal_waits` → satisfy collects the `Waiting` node and commits it
/// `Completed` → step 4 (`commit_attempted == false`) and step 5 (node still
/// `Waiting`) both fail → RED.
#[tokio::test]
async fn satisfy_signal_waits_skips_when_execution_cancelled_under_lease() {
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
        "execution must be Paused before the cancel-under-lease test"
    );

    // ── Phase 2: wire a second engine with the cancel-injecting interceptor ───
    let interceptor = Arc::new(CancelDuringSatisfyInterceptor::new(Arc::clone(
        &harness.stores.execution,
    )));
    let interceptor_as_store: Arc<dyn ExecutionStore> = Arc::clone(&interceptor) as _;

    let downstream_invocations_2 = Arc::new(AtomicU32::new(0));
    let registry2 = Arc::new(ActionRegistry::new());
    registry2.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.signal.webhook_wait"),
            "WebhookWaitNode",
            "signal_resume cancel-under-lease test stub",
        ),
        WebhookWaitNode,
    );
    registry2.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.signal.counting_echo"),
            "CountingEchoNode",
            "signal_resume cancel-under-lease test stub",
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
        resume_tokens: Arc::new(harness.stores.execution.resume_token_store()),
    };

    let engine2 = Arc::new(
        WorkflowEngine::new(runtime2, metrics2)
            .unwrap()
            .with_execution_stores(execution_stores_with_interceptor)
            .with_workflow_stores(harness.stores.workflow_stores()),
    );
    let dispatch2 = EngineControlDispatch::new(Arc::clone(&engine2), interceptor_as_store.clone());

    // ── Phase 3: arm the injection and call dispatch_resume ───────────────────
    interceptor.arm();

    let result = dispatch2.dispatch_resume(&scope, execution_id, None).await;
    assert!(
        result.is_ok(),
        "dispatch_resume must ack (Ok) when the execution was cancelled before satisfy; \
         got {result:?}"
    );

    // ── Phase 4: the durable-write invariant — NO commit on a terminal exec ───
    assert!(
        !interceptor.commit_attempted(),
        "satisfy_signal_waits must NOT commit any transition once the under-lease reload \
         observes a terminal status (would corrupt a cancelled execution's audit state)"
    );

    // ── Phase 5: the signal node must remain Waiting (not flipped) ────────────
    let record = harness
        .stores
        .execution
        .get(&scope, &execution_id.to_string())
        .await
        .unwrap()
        .expect("execution row must exist");
    // `from_value` cannot satisfy ExecutionState's borrowed-string fields; go
    // through a string the same way the engine's reload does.
    let state_str = serde_json::to_string(&record.state).unwrap();
    let state: ExecutionState = serde_json::from_str(&state_str).unwrap();
    let waiting_nodes = state
        .node_states
        .values()
        .filter(|ns| ns.state.is_waiting())
        .count();
    assert_eq!(
        waiting_nodes, 1,
        "the signal node must remain Waiting after a cancel-under-lease race \
         (the satisfy must not have flipped it to Completed)"
    );

    // ── Phase 6: downstream gate stays closed ─────────────────────────────────
    assert_eq!(
        downstream_invocations_2.load(Ordering::SeqCst),
        0,
        "downstream must not run when the satisfy was skipped on a cancelled execution"
    );
}

/// **P2 — a satisfied signal wait routes on the `main` port only** (regression
/// guard for the Codex finding on commit `9b1c2457`):
///
/// A signal-wait node wired with both a `main` and an `error` outgoing port must,
/// on a normal Resume, activate ONLY the `main` branch — the `error` branch must
/// be `Skipped`, exactly as a timer wait's Phase-0b completion routes via the
/// port-aware `process_outgoing_edges(None)` → main.
///
/// The bug: `satisfy_signal_waits` used to transition the node `Waiting →
/// Completed`, after which `resume_execution`'s rebuild activated EVERY outgoing
/// edge of a `Completed` node (port-blind), firing the `error` branch too. The
/// fix arms `next_attempt_at = now` and leaves the node `Waiting`, so Phase-0b
/// completes it through the main-port-only path.
///
/// **Falsifiability**: revert `satisfy_signal_waits` to transition the node
/// `Waiting → Completed` (so the port-blind resume rebuild routes its edges) →
/// the `error` branch runs → `error_invocations == 1` and the node is
/// `Completed` not `Skipped` → the assertions fail → RED.
#[tokio::test]
async fn satisfied_signal_wait_activates_main_port_only_not_error_branch() {
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let stores = SignalStores::new();

    let main_invocations = Arc::new(AtomicU32::new(0));
    let error_invocations = Arc::new(AtomicU32::new(0));

    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.signal.webhook_wait"),
            "WebhookWaitNode",
            "multi-port signal-wait test stub",
        ),
        WebhookWaitNode,
    );
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.signal.counting_echo"),
            "CountingEchoNode",
            "main-port downstream probe",
        ),
        CountingEchoNode {
            invocation_count: Arc::clone(&main_invocations),
        },
    );
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.signal.counting_error"),
            "CountingErrorNode",
            "error-port downstream probe",
        ),
        CountingErrorNode {
            invocation_count: Arc::clone(&error_invocations),
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
    let engine = Arc::new(stores.attach(WorkflowEngine::new(runtime, metrics).unwrap()));
    let dispatch = EngineControlDispatch::new(Arc::clone(&engine), stores.execution.clone());

    // Workflow: wait_node ──main──> main_node
    //                    └─error──> error_node
    let workflow_id = nebula_core::WorkflowId::new();
    let now = Utc::now();
    let wait_node = node_key!("signal_node");
    let main_node = node_key!("main_node");
    let error_node = node_key!("error_node");
    let wf = WorkflowDefinition {
        id: workflow_id,
        name: "signal-resume-multiport-test".into(),
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
                main_node.clone(),
                "MainNode",
                "core",
                "test.signal.counting_echo",
            )
            .unwrap(),
            NodeDefinition::new(
                error_node.clone(),
                "ErrorNode",
                "core",
                "test.signal.counting_error",
            )
            .unwrap(),
        ],
        connections: vec![
            // main port (default) → main_node
            Connection::new(wait_node.clone(), main_node.clone()),
            // error port → error_node
            Connection::new(wait_node.clone(), error_node.clone()).with_from_port("error"),
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

    let execution_id = ExecutionId::new();
    let mut exec_state = ExecutionState::new(execution_id, workflow_id, &[]);
    exec_state.set_workflow_input(serde_json::json!(null));
    stores
        .execution
        .create(
            &scope,
            &execution_id.to_string(),
            &workflow_id.to_string(),
            serde_json::to_value(&exec_state).unwrap(),
        )
        .await
        .unwrap();

    // Park the signal node.
    dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park the signal node");
    assert_eq!(
        stores
            .execution
            .get(&scope, &execution_id.to_string())
            .await
            .unwrap()
            .map(|r| serde_json::from_value::<ExecutionStatus>(
                r.state.get("status").cloned().unwrap()
            )
            .unwrap()),
        Some(ExecutionStatus::Paused),
        "execution must be Paused after parking the signal node"
    );

    // Resume: satisfy arms the wait, Phase-0b completes it on the main port.
    dispatch
        .dispatch_resume(&scope, execution_id, None)
        .await
        .expect("dispatch_resume must succeed");

    // The main branch ran exactly once; the error branch never ran.
    assert_eq!(
        main_invocations.load(Ordering::SeqCst),
        1,
        "the main-port downstream must run exactly once on Resume"
    );
    assert_eq!(
        error_invocations.load(Ordering::SeqCst),
        0,
        "the error-port branch must NOT run on a normal Resume (port-blind routing bug)"
    );

    // Durable state: wait + main Completed, error Skipped, execution Completed.
    let record = stores
        .execution
        .get(&scope, &execution_id.to_string())
        .await
        .unwrap()
        .expect("execution row must exist");
    let state_str = serde_json::to_string(&record.state).unwrap();
    let state: ExecutionState = serde_json::from_str(&state_str).unwrap();
    let node_display = |k: &nebula_core::NodeKey| {
        state
            .node_states
            .get(k)
            .map(|ns| ns.state.to_string())
            .unwrap_or_else(|| "<missing>".to_owned())
    };
    assert_eq!(
        node_display(&main_node),
        "completed",
        "main_node must be Completed"
    );
    assert_eq!(
        node_display(&error_node),
        "skipped",
        "error_node must be Skipped (its edge was never activated on the main-port completion)"
    );
    assert_eq!(
        serde_json::from_value::<ExecutionStatus>(record.state.get("status").cloned().unwrap())
            .unwrap(),
        ExecutionStatus::Completed,
        "execution must be Completed after the main branch runs"
    );
}

/// Wraps an [`InMemoryExecutionStore`] and, once armed, lets the next
/// `acquire_lease` succeed and fails the one after that (returns `Ok(None)` =
/// lease held elsewhere), then self-disarms. Used to fail the `drive`'s lease
/// acquisition that follows a successful `satisfy_signal_waits` arm, leaving the
/// wait node armed-but-not-completed so a later reclaim can finish it.
#[derive(Debug)]
struct LeaseFailAfterArmInterceptor {
    inner: Arc<InMemoryExecutionStore>,
    armed: AtomicBool,
    /// While armed, this many `acquire_lease` calls succeed before one fails.
    successes_before_fail: AtomicU32,
}

impl LeaseFailAfterArmInterceptor {
    fn new(inner: Arc<InMemoryExecutionStore>) -> Self {
        Self {
            inner,
            armed: AtomicBool::new(false),
            successes_before_fail: AtomicU32::new(0),
        }
    }

    /// Arm so the NEXT lease acquisition succeeds (satisfy's) and the one after
    /// it fails (the drive's), then disarms.
    fn arm_to_fail_the_drive(&self) {
        self.successes_before_fail.store(1, Ordering::SeqCst);
        self.armed.store(true, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl ExecutionStore for LeaseFailAfterArmInterceptor {
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
        self.inner.commit(batch).await
    }

    async fn acquire_lease(
        &self,
        scope: &Scope,
        id: &str,
        holder: &str,
        ttl: Duration,
    ) -> Result<Option<FencingToken>, StorageError> {
        if self.armed.load(Ordering::SeqCst) {
            if self.successes_before_fail.load(Ordering::SeqCst) == 0 {
                // Fail this acquisition (the drive's, after satisfy's arm).
                self.armed.store(false, Ordering::SeqCst);
                return Ok(None);
            }
            self.successes_before_fail.fetch_sub(1, Ordering::SeqCst);
        }
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

/// **Q2c — an armed signal wait survives a failed Resume drive and is completed
/// by a later reclaim drive, never lost** (concurrency hardening for the
/// satisfy/lease seam, per the W-S2 fix-A review):
///
/// `satisfy_signal_waits` arms the wait (`next_attempt_at = now`) under its own
/// lease and commits. If the genuine Resume's follow-up `drive` then fails to
/// acquire the lease (a concurrent runner grabbed it), the control-queue row is
/// acked but the node is durably `Waiting{next_attempt_at = Some}`. A later
/// reclaim drive (`dispatch_start`, which does NOT call satisfy) re-seeds the
/// armed node into the `wait_heap` and Phase-0b completes it — the Resume's work
/// is guaranteed, not lost. This is correct recovery, not a discriminator
/// breach: only `satisfy_signal_waits` ever arms `next_attempt_at` on a
/// `Waiting{None}` node (a never-armed wait stays parked under any reclaim — see
/// `dispatch_start_redelivery_does_not_satisfy_signal_wait`).
///
/// **Falsifiability**: if a reclaim drive did not complete an armed wait (e.g.
/// the wait-heap resume-seed at `resume_execution` excluded armed nodes), the
/// downstream would never run → the `== 1` assertion fails → RED.
#[tokio::test]
async fn armed_signal_wait_is_completed_by_reclaim_drive_not_lost() {
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let stores = SignalStores::new();
    let interceptor = Arc::new(LeaseFailAfterArmInterceptor::new(Arc::clone(
        &stores.execution,
    )));
    let interceptor_as_store: Arc<dyn ExecutionStore> = Arc::clone(&interceptor) as _;

    let downstream = Arc::new(AtomicU32::new(0));
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.signal.webhook_wait"),
            "WebhookWaitNode",
            "armed-wait reclaim test stub",
        ),
        WebhookWaitNode,
    );
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.signal.counting_echo"),
            "CountingEchoNode",
            "armed-wait reclaim downstream probe",
        ),
        CountingEchoNode {
            invocation_count: Arc::clone(&downstream),
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

    let execution_stores = nebula_engine::ExecutionStores {
        execution: Arc::clone(&interceptor) as _,
        journal: stores.journal.clone(),
        node_results: stores.node_results.clone(),
        checkpoints: stores.checkpoints.clone(),
        idempotency: stores.idempotency.clone(),
        resume_tokens: Arc::new(stores.execution.resume_token_store()),
    };
    let engine = Arc::new(
        WorkflowEngine::new(runtime, metrics)
            .unwrap()
            .with_execution_stores(execution_stores)
            .with_workflow_stores(stores.workflow_stores()),
    );
    let dispatch = EngineControlDispatch::new(Arc::clone(&engine), interceptor_as_store.clone());

    // Two-node workflow: webhook_wait → counting_echo.
    let workflow_id = nebula_core::WorkflowId::new();
    let now = Utc::now();
    let wait_node = node_key!("signal_node");
    let downstream_node = node_key!("downstream_node");
    let wf = WorkflowDefinition {
        id: workflow_id,
        name: "armed-wait-reclaim-test".into(),
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
        connections: vec![Connection::new(wait_node.clone(), downstream_node.clone())],
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

    let execution_id = ExecutionId::new();
    let mut exec_state = ExecutionState::new(execution_id, workflow_id, &[]);
    exec_state.set_workflow_input(serde_json::json!(null));
    stores
        .execution
        .create(
            &scope,
            &execution_id.to_string(),
            &workflow_id.to_string(),
            serde_json::to_value(&exec_state).unwrap(),
        )
        .await
        .unwrap();

    // Park (interceptor not armed → normal lease behaviour).
    dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park");

    let read_state = |stores: &SignalStores, eid: ExecutionId| {
        let stores = stores.execution.clone();
        async move {
            let rec = stores
                .get(
                    &nebula_engine::store_seam::single_tenant_scope(),
                    &eid.to_string(),
                )
                .await
                .unwrap()
                .unwrap();
            let s = serde_json::to_string(&rec.state).unwrap();
            serde_json::from_str::<ExecutionState>(&s).unwrap()
        }
    };

    let parked = read_state(&stores, execution_id).await;
    assert_eq!(
        parked.status,
        ExecutionStatus::Paused,
        "execution must be Paused after park"
    );

    // Resume: satisfy arms (lease #1 succeeds), the drive's lease (#2) fails →
    // node left armed-but-not-completed. The post-satisfy drive must DEFER (not
    // ack) so the Resume is redelivered — acking here would strand the paused
    // execution when the lease holder is a crashed runner whose TTL has not
    // expired yet (P1: keep Resume redeliverable after drive lease contention).
    interceptor.arm_to_fail_the_drive();
    let deferred = dispatch.dispatch_resume(&scope, execution_id, None).await;
    assert!(
        matches!(deferred, Err(ControlDispatchError::Deferred(_))),
        "post-satisfy drive that fails to acquire the lease must Defer (keep the Resume \
         redeliverable), not ack; got {deferred:?}"
    );

    let armed = read_state(&stores, execution_id).await;
    assert_eq!(
        armed.status,
        ExecutionStatus::Paused,
        "execution stays Paused — the drive deferred before completing the armed wait"
    );
    let wait_ns = armed.node_states.get(&wait_node).unwrap();
    assert!(
        wait_ns.state.is_waiting(),
        "the wait node must still be Waiting (armed, not yet completed)"
    );
    assert!(
        wait_ns.next_attempt_at.is_some(),
        "satisfy must have armed next_attempt_at on the wait node"
    );
    assert_eq!(
        downstream.load(Ordering::SeqCst),
        0,
        "downstream must not run while the armed wait is uncompleted"
    );

    // Reclaim drive (dispatch_start, no satisfy) completes the armed wait.
    dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("reclaim drive must succeed");

    let done = read_state(&stores, execution_id).await;
    assert_eq!(
        done.status,
        ExecutionStatus::Completed,
        "the reclaim drive must complete the armed wait via Phase-0b"
    );
    assert_eq!(
        downstream.load(Ordering::SeqCst),
        1,
        "downstream must run exactly once — the armed Resume's work was recovered, not lost"
    );
}

/// Wraps an [`InMemoryExecutionStore`] and records, for every `commit`, a pair
/// `(status, has_signal_waiting_node)` parsed from the committed state snapshot.
/// Lets a test assert the SHIP-C invariant: no durable commit ever pairs
/// `status == "running"` with a signal-`Waiting{next_attempt_at: null}` node.
#[derive(Debug)]
struct CommitStatusRecorder {
    inner: Arc<InMemoryExecutionStore>,
    /// `(status_string, any signal-Waiting node present)` per commit.
    commits: Mutex<Vec<(String, bool)>>,
}

impl CommitStatusRecorder {
    fn new(inner: Arc<InMemoryExecutionStore>) -> Self {
        Self {
            inner,
            commits: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl ExecutionStore for CommitStatusRecorder {
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
        let state = batch.new_state();
        let status = state
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("<missing>")
            .to_owned();
        // A signal wait is a node `state == "waiting"` with `next_attempt_at` null.
        let has_signal_waiting = state
            .get("node_states")
            .and_then(|m| m.as_object())
            .is_some_and(|m| {
                m.values().any(|ns| {
                    ns.get("state").and_then(|s| s.as_str()) == Some("waiting")
                        && ns
                            .get("next_attempt_at")
                            .is_some_and(serde_json::Value::is_null)
                })
            });
        self.commits
            .lock()
            .unwrap()
            .push((status, has_signal_waiting));
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

/// **P1 SHIP-C — a signal park persists `Paused` atomically, never a durable
/// `Running` + signal-`Waiting{None}` window** (regression guard for the Codex P1
/// on commit `fb39ef08`):
///
/// When a signal park leaves no other active frontier work, the park's
/// `checkpoint_node` batch must already carry `status == Paused` — otherwise the
/// row sits durably `Running` + `Waiting{next_attempt_at: None}` until the
/// frontier-exit `persist_final_state`, and a crash in that window is
/// unrecoverable (`dispatch_start`/`dispatch_resume` short-circuit on `Running`,
/// and a Resume only satisfies a `Paused` execution).
///
/// Test mechanics: a single signal-wait node (the park IS the last work), driven
/// through a `CommitStatusRecorder`. Assert NO commit ever pairs
/// `status == "running"` with a signal-`Waiting{None}` node present.
///
/// **Falsifiability**: drop the `transition_status(Paused)` from the signal-park
/// branch → the park checkpoint commits `("running", true)` → the invariant
/// assertion fails → RED.
#[tokio::test]
async fn signal_park_persists_paused_atomically_no_running_waiting_window() {
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let stores = SignalStores::new();
    let recorder = Arc::new(CommitStatusRecorder::new(Arc::clone(&stores.execution)));
    let recorder_as_store: Arc<dyn ExecutionStore> = Arc::clone(&recorder) as _;

    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.signal.webhook_wait"),
            "WebhookWaitNode",
            "SHIP-C atomic-Paused test stub",
        ),
        WebhookWaitNode,
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

    let execution_stores = nebula_engine::ExecutionStores {
        execution: Arc::clone(&recorder) as _,
        journal: stores.journal.clone(),
        node_results: stores.node_results.clone(),
        checkpoints: stores.checkpoints.clone(),
        idempotency: stores.idempotency.clone(),
        resume_tokens: Arc::new(stores.execution.resume_token_store()),
    };
    let engine = Arc::new(
        WorkflowEngine::new(runtime, metrics)
            .unwrap()
            .with_execution_stores(execution_stores)
            .with_workflow_stores(stores.workflow_stores()),
    );
    let dispatch = EngineControlDispatch::new(Arc::clone(&engine), recorder_as_store.clone());

    // Single signal-wait node — the park is the last (and only) frontier work.
    let workflow_id = nebula_core::WorkflowId::new();
    let now = Utc::now();
    let wait_node = node_key!("signal_node");
    let wf = WorkflowDefinition {
        id: workflow_id,
        name: "ship-c-atomic-paused-test".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![
            NodeDefinition::new(wait_node, "SignalNode", "core", "test.signal.webhook_wait")
                .unwrap(),
        ],
        connections: vec![],
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

    let execution_id = ExecutionId::new();
    let mut exec_state = ExecutionState::new(execution_id, workflow_id, &[]);
    exec_state.set_workflow_input(serde_json::json!(null));
    stores
        .execution
        .create(
            &scope,
            &execution_id.to_string(),
            &workflow_id.to_string(),
            serde_json::to_value(&exec_state).unwrap(),
        )
        .await
        .unwrap();

    dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park the signal node");

    // The execution ends Paused.
    let record = stores
        .execution
        .get(&scope, &execution_id.to_string())
        .await
        .unwrap()
        .expect("execution row must exist");
    assert_eq!(
        serde_json::from_value::<ExecutionStatus>(record.state.get("status").cloned().unwrap())
            .unwrap(),
        ExecutionStatus::Paused,
        "execution must be Paused after the signal park"
    );

    // SHIP-C invariant: no durable commit pairs `Running` with a signal-`Waiting{None}` node.
    let commits = recorder.commits.lock().unwrap().clone();
    assert!(
        commits.iter().any(|(_, waiting)| *waiting),
        "the park must have committed a signal-Waiting node (invariant is not vacuous); \
         commits = {commits:?}"
    );
    assert!(
        !commits
            .iter()
            .any(|(status, waiting)| status == "running" && *waiting),
        "SHIP-C: no durable commit may pair status=running with a signal-Waiting{{None}} node \
         (that would be the unrecoverable crash window); commits = {commits:?}"
    );
}

/// **P1 — cancelling a no-live-runner `Paused` execution terminalizes its parked
/// nodes** (regression guard for the Codex P1 on commit `f8bce5c3`):
///
/// A signal-`Paused` execution has no live frontier. The API cancel path writes
/// `status = Cancelled` but cannot terminalize the engine-owned node states;
/// `dispatch_cancel`'s `cancel_execution` signal returns false (no live runner),
/// so the in-loop teardown never runs. Without the engine's `cancel_dangling_nodes`
/// the `Cancelled` execution would keep a non-terminal `Waiting` node — a
/// terminal-execution ⇒ all-nodes-terminal invariant violation.
///
/// Test mechanics: park a signal node (`Paused`), simulate the API's `Cancelled`
/// status write (`force_cancelled`), then drain `Cancel` via `dispatch_cancel`.
/// Assert every node is terminal (`Cancelled`), not left `Waiting`.
///
/// **Falsifiability**: drop the `cancel_dangling_nodes` call from `dispatch_cancel`
/// → the parked `Waiting` node stays non-terminal → the all-terminal assertion
/// fails → RED.
#[tokio::test]
async fn cancel_of_paused_signal_execution_terminalizes_parked_nodes() {
    let harness = SignalHarness::new().await;
    let workflow_id = harness.persist_signal_workflow().await;
    let execution_id = harness.persist_created_execution(workflow_id).await;
    let scope = nebula_engine::store_seam::single_tenant_scope();

    // Park the signal node → execution Paused, no live runner.
    harness
        .dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park the signal node");
    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Paused
    );

    // The API cancel path writes the terminal status (but not the node states).
    harness
        .force_status(execution_id, ExecutionStatus::Cancelled)
        .await;

    // Drain the Cancel command. No live runner → engine terminalizes the parked
    // nodes under the lease.
    harness
        .dispatch
        .dispatch_cancel(&scope, execution_id)
        .await
        .expect("dispatch_cancel must succeed (no live runner; durable node cleanup)");

    // Every node must be terminal — no dangling `Waiting` under a `Cancelled` execution.
    let record = harness
        .stores
        .execution
        .get(&scope, &execution_id.to_string())
        .await
        .unwrap()
        .expect("execution row must exist");
    let state_str = serde_json::to_string(&record.state).unwrap();
    let state: ExecutionState = serde_json::from_str(&state_str).unwrap();
    let non_terminal: Vec<String> = state
        .node_states
        .iter()
        .filter(|(_, ns)| !ns.state.is_terminal())
        .map(|(k, ns)| format!("{k}={}", ns.state))
        .collect();
    assert!(
        non_terminal.is_empty(),
        "a cancelled no-live-runner execution must have NO non-terminal nodes; found: {non_terminal:?}"
    );
    assert!(
        state
            .node_states
            .values()
            .any(|ns| ns.state.to_string() == "cancelled"),
        "the parked signal node must have been transitioned to Cancelled"
    );
}

/// **C1 — `dispatch_cancel` DEFERS (does not silently ack) when the cancel is
/// not yet durably recorded** (concurrency-hardening for the cancel-of-paused
/// fix): the API writes `status = Cancelled` BEFORE enqueuing `Cancel`, so by
/// drain time the status is `Cancelled`. If a producer-ordering regression ever
/// delivered the `Cancel` while the execution were still `Paused`,
/// `cancel_dangling_nodes` must return `StatusNotCancelled` and `dispatch_cancel`
/// must `Deferred` (so B1 reclaim redelivers) rather than ack-and-drop the node
/// cleanup.
///
/// **Falsifiability**: make `cancel_dangling_nodes` return `NothingToCancel`
/// (ack) for a non-terminal non-cancel status → `dispatch_cancel` returns `Ok`
/// → the `Deferred` assertion fails → RED.
#[tokio::test]
async fn dispatch_cancel_defers_when_cancel_not_yet_recorded() {
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
        ExecutionStatus::Paused
    );

    // NOTE: deliberately do NOT call `force_cancelled` — the status is still
    // `Paused` (the cancel has not been durably recorded).
    let result = harness.dispatch.dispatch_cancel(&scope, execution_id).await;
    assert!(
        matches!(result, Err(ControlDispatchError::Deferred(_))),
        "dispatch_cancel must Defer when the cancel is not yet durably recorded; got {result:?}"
    );
    // No node cleanup happened — the wait node is still Waiting, execution Paused.
    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Paused
    );
}

/// **Lease held by a live owner — `dispatch_cancel` ACKS (does not churn) and
/// the owner cleans up; once the lease frees, a Cancel terminalizes the parked
/// nodes.** A held lease (acquire fails ⇒ TTL not expired) means a live runner
/// owns the execution: it observes the durable `Cancelled` status via its next
/// checkpoint CAS and tears its own frontier down. `dispatch_cancel` must ACK
/// rather than Defer — there is no targeted cross-runner cancel delivery, so
/// deferring would only churn the row through budget-capped reclaim and mark it
/// failed without reaching the owner. A genuinely no-live-runner Paused
/// execution has a FREE lease, so `cancel_dangling_nodes` acquires it and
/// terminalizes (proven by the second half + by
/// `cancel_of_paused_signal_execution_terminalizes_parked_nodes`).
///
/// **Falsifiability**: revert the `Leased` arm to `Deferred` → the held-lease
/// `dispatch_cancel` returns `Err(Deferred)` → the `Ok` assertion fails → RED.
#[tokio::test]
async fn dispatch_cancel_acks_when_lease_held_then_terminalizes_once_free() {
    let harness = SignalHarness::new().await;
    let workflow_id = harness.persist_signal_workflow().await;
    let execution_id = harness.persist_created_execution(workflow_id).await;
    let scope = nebula_engine::store_seam::single_tenant_scope();

    harness
        .dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park the signal node");
    // The API cancel path records the terminal status.
    harness
        .force_status(execution_id, ExecutionStatus::Cancelled)
        .await;

    // A live owner holds the execution lease.
    let blocker = harness
        .stores
        .execution
        .acquire_lease(
            &scope,
            &execution_id.to_string(),
            "live-owner-runner",
            Duration::from_secs(30),
        )
        .await
        .unwrap()
        .expect("lease must be free before the contention test");

    // dispatch_cancel must ACK (the live owner handles its own teardown), NOT
    // churn the row through reclaim.
    harness
        .dispatch
        .dispatch_cancel(&scope, execution_id)
        .await
        .expect("dispatch_cancel must ACK when a live owner holds the lease (no reclaim churn)");

    // This dispatcher did not touch node state (the owner owns cleanup) — the
    // wait node is still Waiting from its perspective.
    let record = harness
        .stores
        .execution
        .get(&scope, &execution_id.to_string())
        .await
        .unwrap()
        .unwrap();
    let state: ExecutionState =
        serde_json::from_str(&serde_json::to_string(&record.state).unwrap()).unwrap();
    assert!(
        state.node_states.values().any(|ns| ns.state.is_waiting()),
        "the non-owner dispatcher must not have mutated node state while the owner holds the lease"
    );

    // Once the lease frees (the owner is gone / done), a Cancel terminalizes the
    // parked nodes via the lease-free `cancel_dangling_nodes` path.
    harness
        .stores
        .execution
        .release_lease(&scope, &execution_id.to_string(), blocker)
        .await
        .unwrap();
    harness
        .dispatch
        .dispatch_cancel(&scope, execution_id)
        .await
        .expect("dispatch_cancel must terminalize the parked nodes once the lease is free");

    let record2 = harness
        .stores
        .execution
        .get(&scope, &execution_id.to_string())
        .await
        .unwrap()
        .unwrap();
    let state2: ExecutionState =
        serde_json::from_str(&serde_json::to_string(&record2.state).unwrap()).unwrap();
    assert!(
        state2.node_states.values().all(|ns| ns.state.is_terminal()),
        "once the lease is free, the Cancel must terminalize every node"
    );
}

/// **CodeRabbit — a `Cancelling` no-live-runner execution is FINALIZED to
/// `Cancelled`** (not left non-terminal): a runner that began a cancel
/// (`Running → Cancelling`) and then crashed leaves the execution `Cancelling`
/// with no live frontier. A redelivered `Cancel` must terminalize the parked
/// nodes AND move the execution `Cancelling → Cancelled` in the same cleanup
/// commit, or the execution stays non-terminal `Cancelling` forever.
///
/// **Falsifiability**: drop the `Cancelling → Cancelled` finalize from
/// `cancel_dangling_nodes` → the execution stays `Cancelling` → the
/// `== Cancelled` assertion fails → RED.
#[tokio::test]
async fn cancel_of_cancelling_no_live_runner_finalizes_to_cancelled() {
    let harness = SignalHarness::new().await;
    let workflow_id = harness.persist_signal_workflow().await;
    let execution_id = harness.persist_created_execution(workflow_id).await;
    let scope = nebula_engine::store_seam::single_tenant_scope();

    harness
        .dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park the signal node");

    // Simulate a runner that began the cancel then crashed: status `Cancelling`,
    // parked Waiting node intact, no live frontier.
    harness
        .force_status(execution_id, ExecutionStatus::Cancelling)
        .await;

    harness
        .dispatch
        .dispatch_cancel(&scope, execution_id)
        .await
        .expect("dispatch_cancel must finalize a Cancelling no-live-runner execution");

    let record = harness
        .stores
        .execution
        .get(&scope, &execution_id.to_string())
        .await
        .unwrap()
        .unwrap();
    let state: ExecutionState =
        serde_json::from_str(&serde_json::to_string(&record.state).unwrap()).unwrap();
    assert_eq!(
        state.status,
        ExecutionStatus::Cancelled,
        "a Cancelling no-live-runner execution must be finalized to Cancelled"
    );
    assert!(
        state.node_states.values().all(|ns| ns.state.is_terminal()),
        "all parked nodes must be terminal after the cancel finalization"
    );
}
