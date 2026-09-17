//! Unit tests for `EngineControlDispatch`.
//!
//! These tests mirror the API → consumer → engine seam without running the
//! full `ControlConsumer` polling loop: they invoke `dispatch_start` /
//! `dispatch_resume` / `dispatch_restart` / `dispatch_cancel` /
//! `dispatch_terminate` directly against an engine wired to in-memory repos,
//! and assert both the happy-path transitions (Created → Completed,
//! Running → Cancelled) and the idempotency contract
//! (re-delivery does not re-run or double-signal).

use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use nebula_action::{
    ActionError, ActionMetadataDraft, action::Action, result::ActionResult,
    stateless::StatelessAction,
};
use nebula_core::{ActionKey, Dependencies, action_key, id::ExecutionId, node_key};
use nebula_engine::{
    ActionRegistry, ActionRuntime, ControlConsumer, ControlDispatch, ControlDispatchError,
    DataPassingPolicy, EngineControlDispatch, InProcessRunner, WorkflowEngine,
};
use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_metrics::MetricsRegistry;
use nebula_storage::inmem::InMemoryTurnHandoff;
use nebula_storage::{InMemoryControlQueue, InMemoryExecutionStore, InMemoryWorkflowVersionStore};
use nebula_storage_port::dto::{ControlCommand, ControlMsg, WorkflowVersionRecord};
use nebula_storage_port::store::{ControlQueue, ExecutionStore, WorkflowVersionStore};
use nebula_storage_port::{TransitionBatch, TransitionOutcome};
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition,
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

mod exact_fixture;

/// Text a pre-envelope execution row can hold where this build expects a
/// `status` discriminant. Structured so a substring match cannot false-positive
/// on unrelated framework output.
const MARKER: &str = "MARKER-9f3a-secret";

/// Bundled port adapters for one shared in-memory tenant (mirrors the
/// in-source `TestStores` pattern). All store calls use `single_tenant_scope()`
/// so the raw adapters behave as one coherent tenant.
#[derive(Clone)]
struct DispatchStores {
    execution: Arc<InMemoryExecutionStore>,
    journal: Arc<nebula_storage::InMemoryJournalReader>,
    node_results: Arc<nebula_storage::InMemoryNodeResultStore>,
    checkpoints: Arc<nebula_storage::InMemoryCheckpointStore>,
    idempotency: Arc<nebula_storage::InMemoryIdempotencyGuard>,
    versions: Arc<InMemoryWorkflowVersionStore>,
}

impl DispatchStores {
    fn new() -> Self {
        let execution = Arc::new(InMemoryExecutionStore::new());
        let journal = Arc::new(nebula_storage::InMemoryJournalReader::new(&execution));
        let versions = InMemoryWorkflowVersionStore::new();
        Self {
            execution,
            journal,
            node_results: Arc::new(nebula_storage::InMemoryNodeResultStore::new()),
            checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new()),
            idempotency: Arc::new(nebula_storage::InMemoryIdempotencyGuard::new()),
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
            operation_ledger: Arc::new(nebula_storage::inmem::InMemoryOperationLedger::new(
                &self.execution,
            )),
        }
    }

    fn attach(&self, engine: WorkflowEngine) -> WorkflowEngine {
        engine.with_execution_stores(self.execution_stores())
    }

    /// Persist a workflow definition as published version 0.
    async fn save_workflow(&self, wf: &WorkflowDefinition) {
        self.versions
            .create(
                &nebula_engine::store_seam::single_tenant_scope(),
                WorkflowVersionRecord {
                    workflow_id: wf.id.to_string(),
                    number: 0,
                    published: true,
                    pinned: false,
                    activation: None,
                    definition: serde_json::to_value(wf).unwrap(),
                },
            )
            .await
            .unwrap();
    }
}

// ── Test handlers (Variant A) ─────────────────────────────────────────────

/// Echo handler that counts invocations so idempotency tests can assert no
/// second dispatch happened.
struct CountingEchoHandler {
    count: Arc<AtomicU32>,
}

impl Action for CountingEchoHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            action_key!("test.counting_echo.static"),
            nebula_action::metadata_name!("CountingEcho"),
            "static",
        )
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for CountingEchoHandler {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(input))
    }
}

/// Cooperatively-cancellable handler. Notifies when it enters the sleep so
/// tests know the frontier loop is live before delivering a `Cancel`.
struct SlowCancellableHandler {
    started: Arc<Notify>,
    count: Arc<AtomicU32>,
}

impl Action for SlowCancellableHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            action_key!("test.slow_cancellable.static"),
            nebula_action::metadata_name!("SlowCancellable"),
            "static",
        )
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for SlowCancellableHandler {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(30)) => Ok(ActionResult::success(input)),
            () = ctx.cancellation().cancelled() => Err(ActionError::Cancelled),
        }
    }
}

fn meta(key: ActionKey) -> ActionMetadataDraft {
    let name = key.clone().into();
    ActionMetadataDraft::new(key, name, "control_dispatch test handler")
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
}

// ── Harness ───────────────────────────────────────────────────────────────

struct Harness {
    dispatch: EngineControlDispatch,
    engine: Arc<WorkflowEngine>,
    stores: DispatchStores,
    action_count: Arc<AtomicU32>,
    slow_count: Arc<AtomicU32>,
    slow_started: Arc<Notify>,
    frozen: Arc<nebula_plugin::FrozenPluginRegistry>,
}

impl Harness {
    async fn new() -> Self {
        let action_count = Arc::new(AtomicU32::new(0));
        let slow_count = Arc::new(AtomicU32::new(0));
        let slow_started = Arc::new(Notify::new());
        let registry = Arc::new(ActionRegistry::new());
        registry
            .register_stateless_instance(
                meta(action_key!("core.echo")),
                CountingEchoHandler {
                    count: Arc::clone(&action_count),
                },
            )
            .expect("valid test catalog definition");
        registry
            .register_stateless_instance(
                meta(action_key!("core.slow")),
                SlowCancellableHandler {
                    started: Arc::clone(&slow_started),
                    count: Arc::clone(&slow_count),
                },
            )
            .expect("valid test catalog definition");
        let frozen = exact_fixture::freeze_registry(
            &registry,
            &[("core", "core.echo"), ("core", "core.slow")],
        );
        let runner = Arc::new(InProcessRunner::new());
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

        let stores = DispatchStores::new();
        let engine = Arc::new(
            stores
                .attach(WorkflowEngine::new(runtime, metrics).unwrap())
                .with_plan_flavor_runtime(
                    Arc::new(nebula_engine::PlanFlavorRevisionLoader::new(Arc::new(
                        stores.execution.plan_flavor_catalog(),
                    ))),
                    Arc::clone(&frozen),
                    Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                        &stores.execution,
                    )),
                ),
        );
        let dispatch = EngineControlDispatch::new(
            Arc::clone(&engine),
            stores.execution.clone(),
            Arc::new(InMemoryTurnHandoff::new(&stores.execution)),
            "control-dispatch-test".to_owned(),
            Duration::from_secs(30),
        );

        Self {
            dispatch,
            engine,
            stores,
            action_count,
            slow_count,
            slow_started,
            frozen,
        }
    }

    /// Persist a single-node echo workflow and return its id.
    async fn persist_echo_workflow(&self) -> nebula_core::WorkflowId {
        let workflow_id = nebula_core::WorkflowId::new();
        let now = chrono::Utc::now();
        let wf = WorkflowDefinition {
            id: workflow_id,
            name: "a2-dispatch-test".into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes: vec![
                NodeDefinition::new(node_key!("step"), "Step", "core", "core.echo").unwrap(),
            ],
            connections: Vec::<Connection>::new(),
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

    /// Persist a pristine `Created` execution row, mirroring how the API
    /// `start_execution` handler writes the row before enqueueing `Start`.
    async fn persist_created_execution(
        &self,
        workflow_id: nebula_core::WorkflowId,
        input: serde_json::Value,
    ) -> ExecutionId {
        let execution_id = ExecutionId::new();
        let mut exec_state = ExecutionState::new(execution_id, workflow_id, &[]);
        exec_state.set_workflow_input(input);
        let record = self
            .stores
            .versions
            .get(
                &nebula_engine::store_seam::single_tenant_scope(),
                &workflow_id.to_string(),
                0,
            )
            .await
            .unwrap()
            .expect("fixture workflow was persisted");
        let workflow: WorkflowDefinition =
            serde_json::from_str(&serde_json::to_string(&record.definition).unwrap()).unwrap();
        exact_fixture::materialize_state(
            &self.stores.execution,
            &nebula_engine::store_seam::single_tenant_scope(),
            &self.frozen,
            &workflow,
            &mut exec_state,
        )
        .await;
        execution_id
    }

    /// Persist a single-node `slow` workflow — the node sleeps until cancelled.
    async fn persist_slow_workflow(&self) -> nebula_core::WorkflowId {
        let workflow_id = nebula_core::WorkflowId::new();
        let now = chrono::Utc::now();
        let wf = WorkflowDefinition {
            id: workflow_id,
            name: "a3-cancel-test".into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes: vec![
                NodeDefinition::new(node_key!("step"), "Step", "core", "core.slow").unwrap(),
            ],
            connections: Vec::<Connection>::new(),
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

    async fn status(&self, id: ExecutionId) -> ExecutionStatus {
        let record = self
            .stores
            .execution
            .get(
                &nebula_engine::store_seam::single_tenant_scope(),
                &id.to_string(),
            )
            .await
            .unwrap()
            .expect("execution exists");
        serde_json::from_value(record.state.get("status").cloned().unwrap()).unwrap()
    }

    /// Overwrite the persisted `status` discriminant with `value` through the
    /// store's own fenced commit — the shape a row written by an older build,
    /// or a partially migrated one, has on disk.
    async fn overwrite_persisted_status(
        &self,
        execution_id: ExecutionId,
        value: serde_json::Value,
    ) {
        let scope = nebula_engine::store_seam::single_tenant_scope();
        let id = execution_id.to_string();
        let fencing = self
            .stores
            .execution
            .acquire_lease(&scope, &id, "test-corrupt-status", Duration::from_secs(30))
            .await
            .unwrap()
            .expect("lease must be free for the simulated corrupt-status write");
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
            .expect("a pinned execution state is a JSON object")
            .insert("status".to_owned(), value);
        let batch = TransitionBatch::builder()
            .scope(scope.clone())
            .execution_id(&id)
            .expected_version(record.version)
            .fencing(fencing)
            .new_state(state)
            .build()
            .unwrap();
        assert!(matches!(
            self.stores.execution.commit(batch).await.unwrap(),
            TransitionOutcome::Applied { .. }
        ));
        self.stores
            .execution
            .release_lease(&scope, &id, fencing)
            .await
            .unwrap();
    }

    /// Poll the execution row until `status.is_terminal()`, bounded by `deadline`.
    async fn wait_terminal(&self, id: ExecutionId, deadline: Duration) -> ExecutionStatus {
        tokio::time::timeout(deadline, async {
            loop {
                let status = self.status(id).await;
                if status.is_terminal() {
                    return status;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("execution reached terminal within deadline")
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

/// Happy path: dispatch_start on a fresh `Created` execution row drives the
/// engine to completion. This is the A2 -step-3 invariant — a POST to
/// `/executions` ends with the workflow actually running, not stranded at
/// `Created`.
#[tokio::test]
async fn dispatch_start_drives_created_execution_to_completion() {
    let harness = Harness::new().await;
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("hello"))
        .await;

    assert_eq!(harness.status(execution_id).await, ExecutionStatus::Created);

    harness
        .dispatch
        .dispatch_start(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("dispatch_start succeeds");

    assert_eq!(
        harness.status(execution_id).await,
        ExecutionStatus::Completed,
        "engine transitioned the execution all the way to Completed"
    );
    assert_eq!(
        harness.action_count.load(Ordering::SeqCst),
        1,
        "echo action was dispatched exactly once"
    );
}

/// idempotency: a re-delivered `Start` for an execution the
/// engine already completed is a no-op — no second run of the workflow.
/// This is the load-bearing guard against at-least-once redelivery
/// double-running the work.
#[tokio::test]
async fn dispatch_start_is_idempotent_on_redelivery() {
    let harness = Harness::new().await;
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!(42))
        .await;

    harness
        .dispatch
        .dispatch_start(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .unwrap();
    assert_eq!(harness.action_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        harness.status(execution_id).await,
        ExecutionStatus::Completed
    );

    // Re-deliver Start. `EngineControlDispatch` must read the terminal
    // status and short-circuit; the engine must not be entered a second time.
    harness
        .dispatch
        .dispatch_start(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("redelivered Start is idempotent");

    assert_eq!(
        harness.action_count.load(Ordering::SeqCst),
        1,
        "re-delivered Start must NOT run the workflow again "
    );
}

/// `Resume` converges on the same engine entry as `Start` today. Delivered
/// against a pristine `Created` row (e.g. an operator-issued resume of a
/// run that was never started), it must drive the execution to completion
/// just like `Start`.
#[tokio::test]
async fn dispatch_resume_drives_created_execution_to_completion() {
    let harness = Harness::new().await;
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("resume"))
        .await;

    harness
        .dispatch
        .dispatch_resume(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
            None,
        )
        .await
        .expect("dispatch_resume succeeds");

    assert_eq!(
        harness.status(execution_id).await,
        ExecutionStatus::Completed
    );
    assert_eq!(harness.action_count.load(Ordering::SeqCst), 1);
}

/// Redelivered `Resume` on an already-completed execution is a no-op per
/// — symmetric with the `Start` idempotency guard.
#[tokio::test]
async fn dispatch_resume_is_idempotent_on_completed_execution() {
    let harness = Harness::new().await;
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("x"))
        .await;

    harness
        .dispatch
        .dispatch_resume(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
            None,
        )
        .await
        .unwrap();
    assert_eq!(harness.action_count.load(Ordering::SeqCst), 1);

    harness
        .dispatch
        .dispatch_resume(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
            None,
        )
        .await
        .expect("second resume is idempotent");

    assert_eq!(
        harness.action_count.load(Ordering::SeqCst),
        1,
        "re-delivered Resume must NOT re-run the workflow"
    );
}

/// `Restart` on a pristine `Created` execution behaves like `Start` — it
/// drives the engine to completion. This is the non-terminal arm of the
/// A2 restart path.
#[tokio::test]
async fn dispatch_restart_drives_created_execution_to_completion() {
    let harness = Harness::new().await;
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("restart"))
        .await;

    harness
        .dispatch
        .dispatch_restart(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("dispatch_restart on a Created execution drives it to completion");

    assert_eq!(
        harness.status(execution_id).await,
        ExecutionStatus::Completed
    );
    assert_eq!(harness.action_count.load(Ordering::SeqCst), 1);
}

/// `Restart` on an already-terminal execution surfaces a typed reject
/// rather than silently succeeding — full rewind-from-input requires
/// durable output purge and a restart counter, neither of which exists in
/// A2. This keeps the capability gap honest on the
/// `execution_control_queue.error_message` column.
#[tokio::test]
async fn dispatch_restart_rejects_terminal_execution() {
    let harness = Harness::new().await;
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("done"))
        .await;

    // Drive it to Completed first.
    harness
        .dispatch
        .dispatch_start(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .unwrap();
    assert_eq!(
        harness.status(execution_id).await,
        ExecutionStatus::Completed
    );
    assert_eq!(harness.action_count.load(Ordering::SeqCst), 1);

    // A real restart-from-input would reset everything and re-run; A2 does not
    // support that yet — the dispatch must reject so operators can see the gap.
    let err = harness
        .dispatch
        .dispatch_restart(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect_err("restart of terminal execution rejects for A2");
    match err {
        ControlDispatchError::Rejected(msg) => {
            assert!(
                msg.contains("durable output purge") || msg.contains(" follow-up"),
                "reject message must name the A2 gap, got: {msg}"
            );
        },
        other => panic!("expected Rejected, got {other:?}"),
    }
    assert_eq!(
        harness.action_count.load(Ordering::SeqCst),
        1,
        "rejected restart must NOT re-run the workflow"
    );
}

/// `Start` for an execution id that was never persisted (producer bug —
/// queue row written without the execution row) surfaces a typed reject
/// so operators see the diagnosis on the row instead of the consumer
/// quietly acking a broken command.
#[tokio::test]
async fn dispatch_start_rejects_nonexistent_execution() {
    let harness = Harness::new().await;
    let orphan = ExecutionId::new();

    let err = harness
        .dispatch
        .dispatch_start(&nebula_engine::store_seam::single_tenant_scope(), orphan)
        .await
        .expect_err("missing execution rejects");
    match err {
        ControlDispatchError::Rejected(msg) => {
            assert!(
                msg.contains("not found") && msg.contains(&orphan.to_string()),
                "reject message must identify the orphan, got: {msg}"
            );
        },
        other => panic!("expected Rejected, got {other:?}"),
    }
    assert_eq!(harness.action_count.load(Ordering::SeqCst), 0);
}

// ── A3 tests — dispatch_cancel / dispatch_terminate ───────────────────────

/// Happy path: a `Cancel` delivered while the frontier loop is inside a
/// live node aborts the execution cooperatively. The spawned run returns,
/// the node surfaces its typed `ActionError::Cancelled`, and the execution
/// row lands on a terminal state. This is the -step-5 invariant that A3
/// closes — the durable `Cancel` signal actually reaches the engine.
#[tokio::test]
async fn dispatch_cancel_aborts_running_execution() {
    let harness = Harness::new().await;
    let workflow_id = harness.persist_slow_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("cancel-me"))
        .await;

    // Spawn the engine on a separate task so the test thread can drive the
    // `Cancel` dispatch while the frontier loop is live.
    let engine = Arc::clone(&harness.engine);
    let run_handle = tokio::spawn(async move {
        engine
            .resume_execution(
                &nebula_engine::store_seam::single_tenant_scope(),
                execution_id,
            )
            .await
    });

    // Wait for the slow handler to confirm it entered the `select!` — the
    // frontier loop is now observing the cancel token.
    tokio::time::timeout(Duration::from_secs(5), harness.slow_started.notified())
        .await
        .expect("slow handler started within 5s");

    // Deliver the Cancel. A3: signal reaches the live token, the
    // handler exits via `ActionError::Cancelled`, frontier loop tears down.
    harness
        .dispatch
        .dispatch_cancel(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("dispatch_cancel succeeds");

    // The spawned run must complete quickly once the cancel fires. The
    // InMemoryExecutionStore does not enforce a separate Cancelled status on
    // CAS without a prior external transition, so we assert the broader
    // invariant: the run finishes in a terminal state promptly (no 30s sleep).
    let result = tokio::time::timeout(Duration::from_secs(5), run_handle)
        .await
        .expect("spawned run returns within 5s of cancel")
        .expect("join ok");
    let _run_result = result.expect("engine run returns Ok (node cancelled counts as finished)");

    // The A3 invariant: the frontier loop exited promptly on cancel (not
    // the full 30s sleep the slow handler would otherwise wait for). The
    // exact terminal label (`Cancelled` on the abort-select path vs
    // `Failed` on the node-error path) depends on a scheduler race between
    // the cancel-token select arm and the handler's own error return; both
    // are terminal per `ExecutionStatus::is_terminal`, so we assert the
    // broad invariant here.
    let terminal = harness
        .wait_terminal(execution_id, Duration::from_secs(5))
        .await;
    assert!(
        terminal.is_terminal(),
        "execution reached a terminal state after Cancel, got: {terminal:?}"
    );
    assert_eq!(
        harness.slow_count.load(Ordering::SeqCst),
        1,
        "slow handler entered exactly once — cancel did not cause a re-dispatch"
    );
}

/// idempotency on terminal: a `Cancel` re-delivered for an
/// already-terminal execution must be `Ok(())` without disturbing state or
/// triggering a second dispatch of the workflow.
///
/// The A3 contract makes this property hold through **two
/// layers**, not a status short-circuit:
///
///   1. `dispatch_cancel` signals the engine on every non-orphan delivery, including terminal.
///      `engine.cancel_execution(id)` looks up the registry — by the time the run is terminal, its
///      `RunningRegistration` guard has already removed the entry, so the lookup returns `false`
///      and the call is a no-op.
///   2. The underlying `CancellationToken::cancel` is idempotent per token, so even a racy delivery
///      where the registry entry is still live cannot re-run the workflow (the frontier loop
///      already observed the original cancel or completed naturally).
///
/// This test asserts the observable effect: no second dispatch of the
/// echo handler, terminal row untouched.
#[tokio::test]
async fn dispatch_cancel_is_idempotent_on_terminal() {
    let harness = Harness::new().await;
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("already-done"))
        .await;

    // Drive the run to Completed first.
    harness
        .dispatch
        .dispatch_start(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .unwrap();
    assert_eq!(
        harness.status(execution_id).await,
        ExecutionStatus::Completed
    );
    assert_eq!(harness.action_count.load(Ordering::SeqCst), 1);

    // Re-deliver Cancel. The A3 body signals `engine.cancel_execution`
    // unconditionally — but the registry entry was removed when the run
    // finished, so the lookup is a no-op and the workflow is not
    // re-dispatched. The return value is `Ok(())`.
    harness
        .dispatch
        .dispatch_cancel(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("re-delivered Cancel on terminal execution is Ok");

    assert_eq!(
        harness.status(execution_id).await,
        ExecutionStatus::Completed,
        "terminal state must not be disturbed by a re-delivered Cancel"
    );
    assert_eq!(
        harness.action_count.load(Ordering::SeqCst),
        1,
        "no second dispatch happened — registry entry was already cleared \
         when the run finished, so the engine signal was a no-op"
    );
    // Confirm the registry is indeed empty for this id — the observable
    // truth behind point (1) above.
    assert!(
        !harness.engine.cancel_execution(execution_id),
        "registry entry was removed on run completion"
    );
}

/// Cross-runner case: a Cancel arrives at an engine instance that never
/// held this execution's frontier loop. `cancel_execution` returns `false`
/// (nothing to signal locally), but `dispatch_cancel` must still return
/// `Ok(())` — the durable CAS transition to `Cancelled` happens on the API
/// handler side, and the holding runner will observe it on its next state
/// transition. Returning an error here would mark the control-queue row
/// `Failed` on a path that is actually healthy.
#[tokio::test]
async fn dispatch_cancel_ok_when_execution_not_held_locally() {
    let harness = Harness::new().await;
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("elsewhere"))
        .await;

    // The execution row sits at `Created`; this engine never started it.
    // The registry does not know about this id.
    assert_eq!(harness.status(execution_id).await, ExecutionStatus::Created);
    assert!(
        !harness.engine.cancel_execution(execution_id),
        "precondition: engine's registry does not hold this id"
    );

    harness
        .dispatch
        .dispatch_cancel(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("cross-runner Cancel is Ok — no local token is not an error");

    assert_eq!(
        harness.action_count.load(Ordering::SeqCst),
        0,
        "no side effect: echo handler was never dispatched"
    );
}

/// Orphan `Cancel` — producer bug, row enqueued without the execution row.
/// Must surface a typed reject so the diagnosis lands on the queue row's
/// `error_message`, matching the A2 symmetric path for `Start`.
#[tokio::test]
async fn dispatch_cancel_rejects_nonexistent_execution() {
    let harness = Harness::new().await;
    let orphan = ExecutionId::new();

    let err = harness
        .dispatch
        .dispatch_cancel(&nebula_engine::store_seam::single_tenant_scope(), orphan)
        .await
        .expect_err("missing execution rejects");
    match err {
        ControlDispatchError::Rejected(msg) => {
            assert!(
                msg.contains("not found")
                    && msg.contains(&orphan.to_string())
                    && msg.contains("cancel"),
                "reject message must name the orphan + command, got: {msg}"
            );
        },
        other => panic!("expected Rejected, got {other:?}"),
    }
}

/// Invariant #7 — fail-closed cross-tenant isolation on the CONTROL-DISPATCH
/// path (`EngineControlDispatch::read_status` → `dispatch_start`).
///
/// This is the symmetric companion to `cross_tenant_dispatch_is_rejected` in
/// `execution_sink.rs`, which covers the orchestrator-dispatch path.
///
/// The row is seeded under `single_tenant_scope()` = `("nebula","nebula")`,
/// the exact value the old `engine_scope()` constant returned. A `dispatch_start`
/// call carrying `scope_b = ("wsB","orgB")` is then issued.
///
/// **Why this is RED on the old `read_status` (constant scope):**
/// `read_status` calls `self.execution.get(single_tenant_scope(), id)` — the row
/// IS there, so `Some(Created)` is returned. The method then calls `drive`, which
/// calls `engine.resume_execution(single_tenant_scope(), id)`. The harness has a
/// full workflow store attached, so the engine runs the workflow and returns `Ok`.
/// The overall dispatch returns `Ok(())` — NOT `Rejected`. The assertion
/// `expect_err("cross-tenant dispatch must be rejected")` therefore panics → RED.
///
/// **Why this is GREEN on the new `read_status` (per-message scope):**
/// `read_status` calls `self.execution.get(scope_b, id)`. The store holds no row
/// under `("wsB","orgB")` → `None` → `Rejected("… not found …")` immediately,
/// never reaching `drive` → assertion passes → GREEN.
#[tokio::test]
async fn cross_tenant_control_dispatch_is_rejected() {
    use nebula_storage_port::Scope;

    let harness = Harness::new().await;

    // Seed a workflow definition and execution row under single_tenant_scope().
    // This is the SAME value the old engine_scope() constant returned, so the
    // old code's read_status finds the row and proceeds to drive the workflow —
    // returning Ok(()) instead of Rejected (see doc comment above).
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("cross-tenant"))
        .await;

    // Deliver dispatch_start under a DIFFERENT tenant scope.
    let scope_b = Scope::new("wsB", "orgB");
    let err = harness
        .dispatch
        .dispatch_start(&scope_b, execution_id)
        .await
        .expect_err("cross-tenant dispatch must be rejected");

    // New code: read_status reads under scope_b → None → Rejected.
    // Old code: read_status reads under ("nebula","nebula") → finds the row →
    //   drive → resume_execution → Ok(()) — NOT Rejected → RED.
    match err {
        ControlDispatchError::Rejected(msg) => {
            assert!(
                msg.contains("not found"),
                "rejection message must name the missing row, got: {msg}"
            );
        },
        other => panic!("expected Rejected, got {other:?}"),
    }

    // No workflow action was dispatched — the engine was never entered.
    assert_eq!(
        harness.action_count.load(Ordering::SeqCst),
        0,
        "cross-tenant rejection must not dispatch any action"
    );
}

/// `Terminate` is a synonym for `Cancel` until a forced-shutdown path is
/// wired ( and the module doc). Same idempotency / orphan /
/// cross-runner contracts apply — this smoke test just asserts the
/// delegation is wired, not a separate code path.
#[tokio::test]
async fn dispatch_terminate_behaves_like_cancel() {
    let harness = Harness::new().await;
    let orphan = ExecutionId::new();

    // Orphan -> Rejected (same code path as dispatch_cancel).
    let err = harness
        .dispatch
        .dispatch_terminate(&nebula_engine::store_seam::single_tenant_scope(), orphan)
        .await
        .expect_err("orphan terminate rejects");
    assert!(matches!(err, ControlDispatchError::Rejected(_)));

    // Terminal -> Ok.
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("t"))
        .await;
    harness
        .dispatch
        .dispatch_start(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .unwrap();
    assert_eq!(
        harness.status(execution_id).await,
        ExecutionStatus::Completed
    );
    harness
        .dispatch
        .dispatch_terminate(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("terminal terminate is Ok");
}

/// Poll the queue until `row_id` leaves `Pending`/`Processing`, returning its
/// `(message, status, error_message)` row.
async fn await_settled_control_row(
    queue: &InMemoryControlQueue,
    row_id: [u8; 16],
    deadline: Duration,
) -> (ControlMsg, String, Option<String>) {
    tokio::time::timeout(deadline, async {
        loop {
            let settled = queue
                .snapshot_detailed()
                .into_iter()
                .find(|(msg, _, _)| msg.id == row_id)
                .filter(|(_, status, _)| status != "Pending" && status != "Processing");
            if let Some(row) = settled {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the consumer must settle the control row within the deadline")
}

/// **A control command for a row whose persisted `status` does not decode must
/// not re-publish the stored value into the durable control-queue row.**
///
/// `read_status` decodes `state["status"]` directly — not through
/// `StorageError` — and on a row written before the typed failure envelope that
/// field can be the failed action's free-text provider error. `serde_json`'s
/// decode error renders the offending value **quoted and uncapped**, and
/// `ControlDispatchError::Internal` is deliberately not `Deferred`: the consumer
/// ack-fails the row and persists the dispatch error's `Display` into
/// `execution_control_queue.error_message`, which an operator querying the row
/// and a post-mortem read both see. The refusal must therefore be a
/// framework-authored phrase naming the execution and the shape mismatch.
///
/// This drives the whole durable path — a real `InMemoryControlQueue` drained by
/// the production `ControlConsumer` over the production
/// `EngineControlDispatch` — so the assertion reads what `mark_failed` actually
/// persisted, not a log line or a returned value. `Cancel` is the command that
/// reaches the non-discriminated `read_status` from the consumer; the `Start`
/// claim path discriminates a corrupt row and ack-drops it.
///
/// **Falsifiability**: restore `{e}` in the `read_status` decode `map_err` → the
/// persisted `error_message` quotes the marker → the absence assertion flips →
/// RED.
#[tokio::test]
async fn undecodable_persisted_status_never_reaches_the_durable_queue_row() {
    let harness = Harness::new().await;
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("corrupt-status"))
        .await;
    assert_eq!(
        harness.status(execution_id).await,
        ExecutionStatus::Created,
        "the fixture row must start decodable, so the corruption below is what fails the read"
    );
    harness
        .overwrite_persisted_status(
            execution_id,
            serde_json::json!(format!("provider rejected token {MARKER}")),
        )
        .await;

    let queue = Arc::new(InMemoryControlQueue::new(&harness.stores.execution));
    let cancel_row_id = [0x5Au8; 16];
    queue
        .enqueue(&ControlMsg {
            id: cancel_row_id,
            execution_id: execution_id.to_string(),
            command: ControlCommand::Cancel,
            scope: nebula_engine::store_seam::single_tenant_scope(),
            w3c_traceparent: None,
            reclaim_count: 0,
            resume_target: None,
        })
        .await
        .expect("the control queue must accept the Cancel row");

    let shutdown = CancellationToken::new();
    let consumer_queue: Arc<dyn ControlQueue> = queue.clone();
    let consumer_task = ControlConsumer::for_flavor(
        consumer_queue,
        Arc::new(harness.dispatch.clone()),
        *b"corrupt-status-p",
        nebula_plugin::WorkerFlavorContext::from_registry(&harness.frozen).revision_id(),
    )
    .with_poll_interval(Duration::from_millis(10))
    .spawn(shutdown.clone());

    let (_, status, error_message) =
        await_settled_control_row(&queue, cancel_row_id, Duration::from_secs(5)).await;
    shutdown.cancel();
    consumer_task
        .await
        .expect("the consumer task must join after shutdown");

    // Non-vacuity: the row was ack-FAILED, so the text read next is this
    // dispatch's own refusal rather than a default (an ack-dropped corrupt row
    // would read `Completed` with no message, and a redelivering one would read
    // `Pending`).
    assert_eq!(
        status, "Failed",
        "a corrupt persisted status is a permanent dispatch failure: the consumer must \
         ack-fail the row, not complete or redeliver it"
    );
    let persisted_error =
        error_message.expect("an ack-failed control-queue row must carry the dispatch error text");
    assert!(
        persisted_error.contains("persisted `status` field does not decode as this build's shape"),
        "the persisted text must be the framework-authored shape-mismatch phrase: {persisted_error}"
    );
    assert!(
        !persisted_error.contains(MARKER),
        "a decode failure must not re-publish the stored value into the durable queue row: \
         {persisted_error}"
    );
}
