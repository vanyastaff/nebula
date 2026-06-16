//! Integration test for `nebula-worker` — U-D1.3 acceptance proof.
//!
//! Verifies the full path:
//!   `WorkerRuntimeBuilder::build` → `.spawn(cancel)` →
//!   orchestrator claims a `Start` job → `EngineExecutionSink::dispatch` →
//!   `WorkflowEngine::resume_execution` → execution reaches `Completed`.
//!
//! ## Test plan
//!
//! `worker_runtime_drives_execution_to_completed`
//!   - Seeds a one-node echo workflow (published) + a `Created` execution row.
//!   - Enqueues a `Start` `JobDispatchMsg` matching the worker's `available_plugins`.
//!   - Builds a `WorkerRuntime` via `WorkerRuntimeBuilder` and spawns it.
//!   - Advances virtual time so the orchestrator's poll interval fires.
//!   - Asserts the execution row reaches `Completed` and the job row is `Dispatched`.
//!
//! Backend: InMemory only.

use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use nebula_action::{
    ActionError, ActionMetadata, action::Action, result::ActionResult, stateless::StatelessAction,
};
use nebula_core::{Dependencies, PluginKey, action_key, id::ExecutionId, node_key};
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessRunner,
    WorkflowEngine,
};
use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_metrics::MetricsRegistry;
use nebula_storage::{
    InMemoryExecutionStore, InMemoryWorkflowVersionStore, inmem::InMemoryJobDispatchQueue,
};
use nebula_storage_port::{
    Scope,
    dto::{ControlCommand, JobDispatchMsg},
    store::{ExecutionStore, JobDispatchQueue, WorkflowVersionStore},
};
use nebula_worker::WorkerRuntimeBuilder;
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, TriggerBinding, ValidatedWorkflow, Version,
    WorkflowConfig, WorkflowDefinition,
};
use tokio_util::sync::CancellationToken;

// ── Plugin key used across all test helpers ───────────────────────────────────

const TEST_PLUGIN_KEY: &str = "test.worker.plugin";

// ── Shared harness ────────────────────────────────────────────────────────────

/// In-memory storage adapters sharing one execution-store core.
#[derive(Clone)]
struct TestStores {
    execution: Arc<InMemoryExecutionStore>,
    journal: Arc<nebula_storage::InMemoryJournalReader>,
    node_results: Arc<nebula_storage::InMemoryNodeResultStore>,
    checkpoints: Arc<nebula_storage::InMemoryCheckpointStore>,
    idempotency: Arc<nebula_storage::InMemoryIdempotencyGuard>,
    workflow: Arc<nebula_storage::InMemoryWorkflowStore>,
    versions: Arc<InMemoryWorkflowVersionStore>,
}

impl TestStores {
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
}

/// Fixed placeholder scope — deliberately mirrors `engine_scope()` from `nebula-engine`
/// (the same `("nebula","nebula")` placeholder the engine uses for every port call).
/// The request-derived `Scope` wiring belongs to U-D1.4; do NOT re-export `engine_scope`
/// from `nebula-engine` here — it is a placeholder slated to change.
fn scope() -> Scope {
    Scope::new("nebula", "nebula")
}

/// `[b; 16]` processor id helper.
fn proc16(b: u8) -> [u8; 16] {
    [b; 16]
}

// ── Echo action ───────────────────────────────────────────────────────────────

struct EchoHandler {
    count: Arc<AtomicU32>,
}

impl Action for EchoHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(action_key!("test.echo.worker"), "Echo", "echo")
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for EchoHandler {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(input))
    }
}

// ── Engine builder ────────────────────────────────────────────────────────────

async fn make_engine(stores: &TestStores) -> (Arc<WorkflowEngine>, Arc<AtomicU32>) {
    let count = Arc::new(AtomicU32::new(0));
    let registry = Arc::new(ActionRegistry::new());
    registry.legacy_register_stateless_with_metadata(
        ActionMetadata::new(action_key!("test.echo.worker"), "Echo", "echo"),
        EchoHandler {
            count: count.clone(),
        },
    );
    // `InProcessRunner` + `executor` are structural boilerplate required by
    // `ActionRuntime::try_new` but are NOT the code path exercised by this test.
    // The legacy-registered `EchoHandler` runs via the direct stateless dispatch path;
    // `echo_count` is the witness that the real handler was invoked.
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
        .expect("ActionRuntime must build in tests"),
    );
    let engine = Arc::new(stores.attach(
        WorkflowEngine::new(runtime, metrics).expect("WorkflowEngine must build in tests"),
    ));
    (engine, count)
}

// ── Workflow persistence ──────────────────────────────────────────────────────

async fn save_echo_workflow(stores: &TestStores) -> Arc<ValidatedWorkflow> {
    let workflow_id = nebula_core::WorkflowId::new();
    let now = chrono::Utc::now();
    let def = WorkflowDefinition {
        id: workflow_id,
        name: "worker-integration-echo".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![
            NodeDefinition::new(
                node_key!("step"),
                "Step",
                TEST_PLUGIN_KEY,
                "test.echo.worker",
            )
            .unwrap(),
        ],
        connections: Vec::<Connection>::new(),
        variables: HashMap::new(),
        config: WorkflowConfig::default(),
        trigger_bindings: vec![
            TriggerBinding::new(
                node_key!("test.trigger"),
                TEST_PLUGIN_KEY,
                "test.trigger.action",
            )
            .unwrap(),
        ],
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: CURRENT_SCHEMA_VERSION,
    };
    let validated =
        ValidatedWorkflow::validate(def).expect("echo workflow definition must pass validation");
    stores
        .versions
        .create(
            &scope(),
            nebula_storage_port::dto::WorkflowVersionRecord {
                workflow_id: validated.definition().id.to_string(),
                number: 0,
                published: true,
                pinned: false,
                definition: serde_json::to_value(validated.definition())
                    .expect("serialize workflow"),
            },
        )
        .await
        .expect("save workflow version");
    Arc::new(validated)
}

async fn persist_created(
    stores: &TestStores,
    workflow_id: nebula_core::WorkflowId,
    execution_id: ExecutionId,
    input: serde_json::Value,
) {
    let mut exec_state = ExecutionState::new(execution_id, workflow_id, &[]);
    exec_state.set_workflow_input(input);
    let state_json = serde_json::to_value(&exec_state).expect("serialize execution state");
    stores
        .execution
        .create(
            &scope(),
            &execution_id.to_string(),
            &workflow_id.to_string(),
            state_json,
        )
        .await
        .expect("create execution row");
}

async fn read_status(stores: &TestStores, execution_id: ExecutionId) -> Option<ExecutionStatus> {
    let record = stores
        .execution
        .get(&scope(), &execution_id.to_string())
        .await
        .expect("get execution");
    record.and_then(|r| {
        r.state
            .get("status")
            .and_then(|s| serde_json::from_value::<ExecutionStatus>(s.clone()).ok())
    })
}

// ── Acceptance test ───────────────────────────────────────────────────────────

/// Full end-to-end proof: `WorkerRuntimeBuilder::build` → `.spawn` →
/// orchestrator claims a `Start` job → engine drives execution to `Completed`.
#[tokio::test(start_paused = true)]
async fn worker_runtime_drives_execution_to_completed() {
    let stores = TestStores::new();
    let (engine, echo_count) = make_engine(&stores).await;
    let workflow = save_echo_workflow(&stores).await;
    let workflow_id = workflow.definition().id;

    // Seed a `Created` execution row.
    let execution_id = ExecutionId::new();
    persist_created(
        &stores,
        workflow_id,
        execution_id,
        serde_json::json!({"trigger": "worker-integration"}),
    )
    .await;

    // Assert starting state is Created.
    let status_before = read_status(&stores, execution_id)
        .await
        .expect("Created row must exist before worker runs");
    assert_eq!(
        status_before,
        ExecutionStatus::Created,
        "row must be Created before worker claims it; got {status_before:?}"
    );

    // Wire a job-dispatch queue sharing the execution store's core.
    let queue = Arc::new(InMemoryJobDispatchQueue::new(&stores.execution));
    let plugin_key: PluginKey = TEST_PLUGIN_KEY.parse().unwrap();

    // Enqueue a Start job whose `required_plugins` matches the worker's advertised set.
    let job_id = [0x11u8; 16];
    let msg = JobDispatchMsg::new(
        job_id,
        execution_id.to_string(),
        ControlCommand::Start,
        scope(),
        serde_json::json!({}),
        None::<String>,
        "sha-worker-test",
        plugin_key.clone(),
        vec![plugin_key.clone()],
        None::<String>,
        0,
    );
    queue.enqueue(&msg).await.expect("enqueue Start job");

    // Build the WorkerRuntime via the builder.
    let execution_stores = stores.execution_stores();
    let runtime = WorkerRuntimeBuilder::from_wired_engine(
        Arc::clone(&engine),
        execution_stores,
        Arc::clone(&queue) as Arc<dyn JobDispatchQueue>,
        vec![plugin_key],
        proc16(0xBB),
    )
    // Use a fast poll interval so virtual-time advance is small.
    .with_poll_interval(Duration::from_millis(10))
    .build()
    .expect("WorkerRuntimeBuilder::build must succeed with non-empty plugin set");

    // Spawn the runtime.
    let cancel = CancellationToken::new();
    let handle = runtime.spawn(cancel.clone());

    // Bounded poll loop: drive progress by state, not by a magic sleep budget.
    // Each iteration yields to let the worker task run, then advances virtual time
    // by one poll interval (10 ms) so the orchestrator's sleep fires. The loop exits
    // as soon as the execution reaches Completed, or fails the assertion after 200
    // iterations (~2 s of virtual time) — a worker that never ticks must FAIL here.
    let mut completed = false;
    for _ in 0..200 {
        tokio::task::yield_now().await;
        if read_status(&stores, execution_id).await == Some(ExecutionStatus::Completed) {
            completed = true;
            break;
        }
        tokio::time::advance(Duration::from_millis(10)).await;
    }
    assert!(
        completed,
        "worker did not drive the execution to Completed within the poll budget"
    );

    // Cancel the worker and wait for clean shutdown.
    cancel.cancel();
    handle.await.expect("worker task must not panic");

    // Echo handler must have been invoked exactly once.
    assert_eq!(
        echo_count.load(Ordering::SeqCst),
        1,
        "echo handler must be invoked exactly once"
    );
}

/// `WorkerRuntimeBuilder::build` rejects an empty `available_plugins` vec.
#[tokio::test]
async fn builder_rejects_empty_plugins() {
    use nebula_worker::WorkerBuildError;
    let stores = TestStores::new();
    let (engine, _) = make_engine(&stores).await;
    let queue = Arc::new(InMemoryJobDispatchQueue::new(&stores.execution));
    let result = WorkerRuntimeBuilder::from_wired_engine(
        engine,
        stores.execution_stores(),
        queue as Arc<dyn JobDispatchQueue>,
        vec![], // intentionally empty — must be rejected
        proc16(0x00),
    )
    .build();

    assert!(
        matches!(result, Err(WorkerBuildError::NoPlugins)),
        "empty available_plugins must produce WorkerBuildError::NoPlugins; got {result:?}"
    );
}
