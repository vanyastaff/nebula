//! ADR-0095 D3/D5 — "first real trigger dispatch" vertical slice.
//!
//! Tests verify the full path: trigger fires → `DurableExecutionEmitter` →
//! `TriggerDedupInbox::claim_and_materialize_start` → `Orchestrator` claims →
//! `EngineExecutionSink::dispatch` drives `resume_execution` → execution runs.
//!
//! ## Test plan
//!
//! **B-series (EngineExecutionSink unit tests)**
//! - `sink_dispatch_drives_resume_execution` — Created row → sink → engine runs → Completed.
//! - `sink_dispatch_redelivery_is_idempotent` — same JobDispatchMsg twice → Ok both times, one run.
//!
//! **C-series (DurableExecutionEmitter unit tests)**
//! - `emitter_dispatched_creates_row_and_enqueues_start` — emit with Some(event_id) → Dispatched,
//!   Created row exists, Start row in queue with exact routing fields.
//! - `emitter_duplicate_event_id_no_second_row` — emit same event_id twice → id unchanged,
//!   no second Created row, no second Start row.
//!
//! **Acceptance test**
//! - `trigger_dispatch_end_to_end_real_engine_resume` — trigger fires via adapter → emitter →
//!   orchestrator → sink → engine runs to Completed; redelivery of same event_id asserts exactly
//!   one execution.
//!
//! Backend: InMemory ONLY.  Postgres is ROADMAP-M7 (no DATABASE_URL).

use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use nebula_action::{
    ActionError, ActionMetadata, ExecutionEmitter, IdempotencyKey, action::Action,
    result::ActionResult, stateless::StatelessAction,
};
use nebula_core::{Dependencies, PluginKey, action_key, id::ExecutionId, node_key};
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, EngineExecutionSink,
    InProcessRunner, WorkflowEngine,
};
use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_metrics::MetricsRegistry;
use nebula_orchestrator::{ExecutionSink, Orchestrator};
use nebula_storage::{
    InMemoryExecutionStore, InMemoryWorkflowVersionStore,
    inmem::{InMemoryJobDispatchQueue, InMemoryTriggerDedupInbox},
};
use nebula_storage_port::{
    Scope,
    dto::{ControlCommand, JobDispatchMsg, WorkflowVersionRecord},
    store::{ExecutionStore, JobDispatchQueue, TriggerDedupInbox, WorkflowVersionStore},
};
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, TriggerBinding, ValidatedWorkflow, Version,
    WorkflowConfig, WorkflowDefinition,
};
use tokio_util::sync::CancellationToken;

// ── shared harness ────────────────────────────────────────────────────────────

/// In-memory storage adapters for one isolated test tenant.
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
}

/// Return the test scope via the public re-export.
fn scope() -> Scope {
    nebula_engine::store_seam::single_tenant_scope()
}

/// One-node echo workflow (StatelessAction that returns its input).
struct EchoHandler {
    count: Arc<AtomicU32>,
}

impl Action for EchoHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(action_key!("test.echo.dispatch_slice"), "Echo", "echo")
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

/// Build an engine wired with the given `TestStores` and the `EchoHandler`.
async fn make_engine(stores: &TestStores) -> (Arc<WorkflowEngine>, Arc<AtomicU32>) {
    let count = Arc::new(AtomicU32::new(0));
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("test.echo.dispatch_slice"), "Echo", "echo"),
        EchoHandler {
            count: count.clone(),
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
        .expect("ActionRuntime::try_new must succeed in tests"),
    );
    let engine = Arc::new(stores.attach(
        WorkflowEngine::new(runtime, metrics).expect("WorkflowEngine::new must succeed in tests"),
    ));
    (engine, count)
}

/// Build, validate, persist, and return the echo workflow.
///
/// The workflow carries:
/// - one node `"step"` with `plugin_key = TEST_PLUGIN_KEY`
/// - one trigger binding `id = node_key!("test.trigger")` with
///   `plugin_key = TEST_PLUGIN_KEY`
///
/// Both emitter tests and the acceptance test fire `node_key!("test.trigger")`,
/// so the resolver will find this binding and produce `required_plugin_key ==
/// TEST_PLUGIN_KEY`.
async fn save_echo_workflow(stores: &TestStores) -> Arc<ValidatedWorkflow> {
    let workflow_id = nebula_core::WorkflowId::new();
    let now = chrono::Utc::now();
    let def = WorkflowDefinition {
        id: workflow_id,
        name: "dispatch-slice-echo".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![
            NodeDefinition::new(
                node_key!("step"),
                "Step",
                TEST_PLUGIN_KEY,
                "test.echo.dispatch_slice",
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
            WorkflowVersionRecord {
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

/// Persist a pristine `Created` execution row, mirroring the API handler.
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

/// Read the persisted execution status from the store.
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

/// Helper: `[b; 16]` processor id.
fn proc16(b: u8) -> [u8; 16] {
    [b; 16]
}

// ── B-series: EngineExecutionSink unit tests ─────────────────────────────────

/// `EngineExecutionSink::dispatch` on a `Created` row drives `resume_execution`
/// and the execution reaches `Completed`.
#[tokio::test(start_paused = true)]
async fn sink_dispatch_drives_resume_execution() {
    let stores = TestStores::new();
    let (engine, echo_count) = make_engine(&stores).await;
    let workflow = save_echo_workflow(&stores).await;
    let workflow_id = workflow.definition().id;

    // Seed a Created row (the emitter does this in prod; we seed it directly
    // for the isolated sink unit test).
    let execution_id = ExecutionId::new();
    persist_created(
        &stores,
        workflow_id,
        execution_id,
        serde_json::json!({"x": 1}),
    )
    .await;

    let sink = EngineExecutionSink::new(Arc::clone(&engine), stores.execution.clone());
    let test_plugin_key: PluginKey = "test.plugin".parse().unwrap();
    let msg = JobDispatchMsg::new(
        [42u8; 16],
        execution_id.to_string(),
        ControlCommand::Start,
        scope(),
        serde_json::json!({}),
        None::<String>,
        "sha-abc",
        test_plugin_key.clone(),
        vec![test_plugin_key],
        None::<String>,
        0,
    );

    let result = sink.dispatch(&msg).await;
    assert!(
        result.is_ok(),
        "EngineExecutionSink::dispatch must succeed on a Created row: {result:?}"
    );

    // Status must be Completed — not just Created, which would mean
    // resume_execution was never driven.
    let status = read_status(&stores, execution_id)
        .await
        .expect("execution row must exist after dispatch");
    assert_eq!(
        status,
        ExecutionStatus::Completed,
        "execution must reach Completed after sink dispatch — got {status:?}"
    );
    assert_eq!(
        echo_count.load(Ordering::SeqCst),
        1,
        "echo handler must be invoked exactly once"
    );
}

/// Re-delivering the same `JobDispatchMsg` to `EngineExecutionSink` returns
/// `Ok(())` without driving the engine a second time (idempotency contract).
#[tokio::test(start_paused = true)]
async fn sink_dispatch_redelivery_is_idempotent() {
    let stores = TestStores::new();
    let (engine, echo_count) = make_engine(&stores).await;
    let workflow = save_echo_workflow(&stores).await;
    let workflow_id = workflow.definition().id;

    let execution_id = ExecutionId::new();
    persist_created(&stores, workflow_id, execution_id, serde_json::json!({})).await;

    let sink = EngineExecutionSink::new(Arc::clone(&engine), stores.execution.clone());
    let test_plugin_key: PluginKey = "test.plugin".parse().unwrap();
    let msg = JobDispatchMsg::new(
        [7u8; 16],
        execution_id.to_string(),
        ControlCommand::Start,
        scope(),
        serde_json::json!({}),
        None::<String>,
        "sha-abc",
        test_plugin_key.clone(),
        vec![test_plugin_key],
        None::<String>,
        0,
    );

    // First dispatch — drives execution to Completed.
    sink.dispatch(&msg)
        .await
        .expect("first dispatch must succeed");

    // Second dispatch (redelivery) — must be Ok without re-running the handler.
    let second = sink.dispatch(&msg).await;
    assert!(
        second.is_ok(),
        "redelivery must return Ok (idempotency contract): {second:?}"
    );

    // Handler invoked exactly once across both dispatches.
    assert_eq!(
        echo_count.load(Ordering::SeqCst),
        1,
        "echo handler must not be invoked on re-delivery"
    );
}

// ── C-series: DurableExecutionEmitter unit tests ─────────────────────────────

use nebula_engine::daemon::durable_emitter::DurableExecutionEmitter;
use nebula_engine::daemon::routing::{DefinitionRoutingResolver, SLICE_FLAVOR_SHA};

const TEST_PLUGIN_KEY: &str = "test.dispatch.plugin";

/// Build InMemory dedup + queue + emitter sharing one execution-store core.
async fn make_emitter(
    stores: &TestStores,
    workflow: Arc<ValidatedWorkflow>,
) -> (
    DurableExecutionEmitter,
    Arc<InMemoryTriggerDedupInbox>,
    Arc<InMemoryJobDispatchQueue>,
) {
    let dedup = Arc::new(InMemoryTriggerDedupInbox::new(&stores.execution));
    let queue = Arc::new(InMemoryJobDispatchQueue::new(&stores.execution));
    let resolver = Arc::new(DefinitionRoutingResolver::new(SLICE_FLAVOR_SHA));
    let emitter = DurableExecutionEmitter::new(
        Arc::clone(&dedup) as Arc<dyn TriggerDedupInbox>,
        resolver,
        workflow,
        node_key!("test.trigger"),
        scope(),
    );
    (emitter, dedup, queue)
}

/// `DurableExecutionEmitter::emit` with `Some(event_id)` produces:
///  - `DispatchKind::Dispatched` (returned as Ok(execution_id))
///  - A `Created` execution row in the store
///  - Exactly one Start row in the job-dispatch queue with the correct routing
///    fields (`required_plugin_key` and exact `required_plugins`)
#[tokio::test(start_paused = true)]
async fn emitter_dispatched_creates_row_and_enqueues_start() {
    let stores = TestStores::new();
    let workflow = save_echo_workflow(&stores).await;
    let (emitter, _dedup, queue) = make_emitter(&stores, Arc::clone(&workflow)).await;

    let event_id = IdempotencyKey::new("evt-001");
    let execution_id = emitter
        .emit(
            serde_json::json!({"trigger": "first"}),
            Some(event_id.clone()),
        )
        .await
        .expect("first emit must succeed (Dispatched)");

    // A Created row must exist.
    let status = read_status(&stores, execution_id)
        .await
        .expect("Created row must be present after Dispatched emit");
    assert_eq!(
        status,
        ExecutionStatus::Created,
        "row must be in Created state immediately after emit; got {status:?}"
    );

    // Exactly one Start job in the queue, claimable by the test plugin key.
    let plugin_key: PluginKey = TEST_PLUGIN_KEY.parse().unwrap();
    let jobs = queue
        .claim_pending(&proc16(1), 10, &[plugin_key])
        .await
        .expect("claim_pending must succeed");
    assert_eq!(
        jobs.len(),
        1,
        "exactly one Start job must be enqueued after Dispatched emit; got {}",
        jobs.len()
    );
    assert_eq!(
        jobs[0].execution_id,
        execution_id.to_string(),
        "enqueued job execution_id must match the returned id"
    );
    assert!(
        matches!(jobs[0].command, ControlCommand::Start),
        "job command must be Start"
    );

    // The route written into the job must match the exact expected values.
    // The echo workflow has one trigger binding (TEST_PLUGIN_KEY) and one
    // enabled node (TEST_PLUGIN_KEY), so required_plugins = [TEST_PLUGIN_KEY]
    // (deduplicated).
    assert_eq!(
        jobs[0].required_plugin_key.as_str(),
        TEST_PLUGIN_KEY,
        "required_plugin_key on enqueued job must equal TEST_PLUGIN_KEY"
    );
    let expected_key: PluginKey = TEST_PLUGIN_KEY.parse().unwrap();
    let expected_plugins = vec![expected_key];
    assert_eq!(
        jobs[0].required_plugins, expected_plugins,
        "required_plugins on enqueued job must equal exactly {{TEST_PLUGIN_KEY}}; \
         trigger and node share the same plugin key so dedup yields one entry. \
         got: {:?}",
        jobs[0].required_plugins
    );
}

/// A second `emit` with the same `event_id` returns the WINNER's id (same as
/// `id1`), writes NO second `Created` row, and enqueues NO second Start job.
/// The dedup-inbox read-back contract: `Duplicate` returns the original
/// winner's id in-transaction, so callers always hold a valid execution id.
#[tokio::test(start_paused = true)]
async fn emitter_duplicate_event_id_no_second_row() {
    let stores = TestStores::new();
    let workflow = save_echo_workflow(&stores).await;
    let (emitter, _dedup, queue) = make_emitter(&stores, Arc::clone(&workflow)).await;

    let event_id = IdempotencyKey::new("evt-dup");

    let id1 = emitter
        .emit(serde_json::json!({"n": 1}), Some(event_id.clone()))
        .await
        .expect("first emit (Dispatched) must succeed");

    let id2 = emitter
        .emit(serde_json::json!({"n": 2}), Some(event_id.clone()))
        .await
        .expect("second emit (Duplicate) must succeed");

    // On Duplicate the emitter returns the WINNER's id (id1), read back
    // from the dedup inbox in-transaction.  Both calls return the same id.
    assert_eq!(
        id1, id2,
        "Duplicate emit must return the original winner's id — both calls must return the same id"
    );

    // Only one Start job must be in the queue (the duplicate write is a no-op).
    let plugin_key2: PluginKey = TEST_PLUGIN_KEY.parse().unwrap();
    let jobs = queue
        .claim_pending(&proc16(2), 10, &[plugin_key2])
        .await
        .expect("claim_pending must succeed");
    assert_eq!(
        jobs.len(),
        1,
        "Duplicate emit must not enqueue a second Start; got {} jobs",
        jobs.len()
    );

    // The execution row must exist (it was created on the first Dispatched emit).
    // Both id1 and id2 are the same id so we only need one get.
    let row = stores
        .execution
        .get(&scope(), &id1.to_string())
        .await
        .expect("get execution row");
    assert!(
        row.is_some(),
        "winner's execution row must exist (id1 == id2 after Duplicate read-back)"
    );
}

// ── Acceptance test ───────────────────────────────────────────────────────────

/// Full vertical slice: trigger fires → `DurableExecutionEmitter` → dedup inbox
/// → orchestrator claims → `EngineExecutionSink` drives `resume_execution` →
/// execution reaches `Completed`.  Redelivery of the same `event_id` produces
/// exactly one execution (dedup guard).
#[tokio::test(start_paused = true)]
async fn trigger_dispatch_end_to_end_real_engine_resume() {
    let stores = TestStores::new();
    let (engine, echo_count) = make_engine(&stores).await;
    let workflow = save_echo_workflow(&stores).await;

    // Wire: dedup + queue share the execution store's core so all three writes
    // (dedup guard + execution row + Start job) are atomic under one lock.
    let dedup = Arc::new(InMemoryTriggerDedupInbox::new(&stores.execution));
    let queue = Arc::new(InMemoryJobDispatchQueue::new(&stores.execution));

    let resolver = Arc::new(DefinitionRoutingResolver::new(SLICE_FLAVOR_SHA));
    let emitter = DurableExecutionEmitter::new(
        Arc::clone(&dedup) as Arc<dyn TriggerDedupInbox>,
        resolver,
        Arc::clone(&workflow),
        node_key!("test.trigger"),
        scope(),
    );

    // 1. Trigger fires → emitter creates Created row + enqueues Start.
    let event_id = IdempotencyKey::new("evt-e2e-001");
    let execution_id = emitter
        .emit(serde_json::json!({"event": "tick"}), Some(event_id.clone()))
        .await
        .expect("emit must succeed");

    // Assert Created row seeded correctly.
    let status_before = read_status(&stores, execution_id)
        .await
        .expect("Created row must exist before orchestrator runs");
    assert_eq!(
        status_before,
        ExecutionStatus::Created,
        "row must be Created before orchestrator claims it; got {status_before:?}"
    );

    // 2. Orchestrator: wire sink, start pull loop, let it claim + dispatch.
    let sink = Arc::new(EngineExecutionSink::new(
        Arc::clone(&engine),
        stores.execution.clone() as Arc<dyn ExecutionStore>,
    ));
    let cancel = CancellationToken::new();
    let orch_plugin_key: PluginKey = TEST_PLUGIN_KEY.parse().unwrap();
    let orch = Orchestrator::new(
        Arc::clone(&queue) as Arc<dyn JobDispatchQueue>,
        sink as Arc<dyn ExecutionSink>,
        proc16(0xAA),
        vec![orch_plugin_key],
    );

    let cancel_clone = cancel.clone();
    let orch_handle = tokio::spawn(async move { orch.run(cancel_clone).await });

    // Yield so the orchestrator spawns and enters its poll loop, then advance
    // virtual time past the poll interval so it claims the pending job.
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_secs(2)).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }

    cancel.cancel();
    orch_handle.await.expect("orchestrator task must not panic");

    // 3. Assert execution reached Completed.
    let status_after = read_status(&stores, execution_id)
        .await
        .expect("execution row must exist after orchestrator ran");
    assert_eq!(
        status_after,
        ExecutionStatus::Completed,
        "execution must reach Completed after full dispatch slice; got {status_after:?}"
    );
    assert_eq!(
        echo_count.load(Ordering::SeqCst),
        1,
        "echo handler must be invoked exactly once end-to-end"
    );

    // 4. Redelivery of the same event_id must NOT create a second execution.
    // On Duplicate the emitter returns the WINNER's execution id (same as
    // execution_id from step 1) — the dedup guard read-back is in-transaction.
    let id2 = emitter
        .emit(
            serde_json::json!({"event": "tick-dup"}),
            Some(event_id.clone()),
        )
        .await
        .expect("duplicate emit must return Ok");
    assert_eq!(
        execution_id, id2,
        "Duplicate emit must return the original winner's id — both calls must return the same id"
    );
    // The winner's row still exists (the Duplicate did not create a second one).
    let winner_row = stores
        .execution
        .get(&scope(), &id2.to_string())
        .await
        .expect("get winner row after duplicate emit");
    assert!(
        winner_row.is_some(),
        "winner's row must still exist after Duplicate emit; got {winner_row:?}"
    );

    // Handler was not invoked a second time.
    assert_eq!(
        echo_count.load(Ordering::SeqCst),
        1,
        "echo handler must NOT be invoked on duplicate event_id"
    );
}
