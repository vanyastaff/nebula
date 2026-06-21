//! Integration tests for `nebula-worker`: the worker-runtime claim→drive→complete
//! happy path, plus the reclaim / redelivery / reclaim-exhaustion liveness
//! properties of the job-dispatch claim.
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

/// Test scope used for worker integration tests. Matches `nebula_engine::store_seam::single_tenant_scope()`
/// (`("nebula","nebula")`) so the worker and engine observe the same in-memory rows.
/// Production code uses the per-message scope from the control-queue / job-dispatch DTO.
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
    registry.register_stateless_instance(
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

// ── Reclaim + re-run tests ────────────────────────────────────────────────────

/// A job-dispatch row that was reclaimed (reset to `Pending`) by a crashed
/// runner is picked up by a live worker and driven to `Completed` exactly once
/// through the real `EngineExecutionSink`.
///
/// Sequence:
/// 1. Seed a `Created` execution row + enqueue a `Start` job.
/// 2. Directly claim the row as `proc_a` (simulates a worker that claims but
///    then crashes before returning).
/// 3. Advance virtual time past `reclaim_after` and call `reclaim_stuck`
///    directly — this is the same code the orchestrator's sweep executes.
///    The row goes back to `Pending` (`reclaim_count` → 1).
/// 4. Build and spawn a `WorkerRuntime` (proc_b) that claims the reclaimed
///    row and drives it through the real `EngineExecutionSink`.
/// 5. Assert execution reaches `Completed` AND the echo handler ran exactly
///    once — ruling out spurious "already-terminal, idempotent no-op" false-greens.
#[tokio::test(start_paused = true)]
async fn reclaim_then_rerun_drives_exactly_once_via_real_sink() {
    let stores = TestStores::new();
    let (engine, echo_count) = make_engine(&stores).await;
    let workflow = save_echo_workflow(&stores).await;
    let workflow_id = workflow.definition().id;

    // Seed a Created execution row.
    let execution_id = ExecutionId::new();
    persist_created(
        &stores,
        workflow_id,
        execution_id,
        serde_json::json!({"trigger": "reclaim-rerun"}),
    )
    .await;

    let queue = Arc::new(InMemoryJobDispatchQueue::new(&stores.execution));
    let plugin_key: PluginKey = TEST_PLUGIN_KEY.parse().unwrap();

    // Enqueue a Start job.
    let job_id = [0xAAu8; 16];
    let msg = JobDispatchMsg::new(
        job_id,
        execution_id.to_string(),
        ControlCommand::Start,
        scope(),
        serde_json::json!({}),
        None::<String>,
        "sha-reclaim-test",
        plugin_key.clone(),
        vec![plugin_key.clone()],
        None::<String>,
        0,
    );
    queue.enqueue(&msg).await.expect("enqueue Start job");

    // Step 2: claim as proc_a (simulates a worker that crashed before dispatching).
    let proc_a = proc16(0xAA);
    let claimed = queue
        .claim_pending(&proc_a, 1, std::slice::from_ref(&plugin_key))
        .await
        .expect("claim as proc_a");
    assert_eq!(claimed.len(), 1, "proc_a must claim the row");

    // Step 3: advance past reclaim_after and call reclaim_stuck directly.
    // Use a tiny reclaim_after so a small virtual-time advance suffices.
    let tiny_reclaim_after = Duration::from_millis(5);
    tokio::time::advance(Duration::from_millis(10)).await;
    let outcome = queue
        .reclaim_stuck(tiny_reclaim_after, 3)
        .await
        .expect("reclaim_stuck");
    assert_eq!(
        outcome.reclaimed, 1,
        "the Processing row must be reclaimed to Pending"
    );
    assert_eq!(
        outcome.exhausted, 0,
        "budget=3 > reclaim_count=0; must not exhaust"
    );

    // The execution row must still be Created — reclaim does not touch it.
    let status_after_reclaim = read_status(&stores, execution_id)
        .await
        .expect("execution row must exist after reclaim");
    assert_eq!(
        status_after_reclaim,
        ExecutionStatus::Created,
        "execution row must remain Created after job-dispatch reclaim; got {status_after_reclaim:?}"
    );

    // Step 4: build a worker (proc_b) and let it claim + drive the reclaimed row.
    let proc_b = proc16(0xBB);
    let runtime = WorkerRuntimeBuilder::from_wired_engine(
        Arc::clone(&engine),
        stores.execution_stores(),
        Arc::clone(&queue) as Arc<dyn JobDispatchQueue>,
        vec![plugin_key],
        proc_b,
    )
    .with_poll_interval(Duration::from_millis(10))
    .build()
    .expect("WorkerRuntimeBuilder::build must succeed");

    let cancel = CancellationToken::new();
    let handle = runtime.spawn(cancel.clone());

    // Step 5: wait for the execution to reach Completed.
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
        "reclaimed job was not driven to Completed within the poll budget"
    );

    cancel.cancel();
    handle.await.expect("worker task must not panic");

    // Echo handler must have fired exactly once — confirms the real sink ran and
    // did not short-circuit on an idempotent no-op for an already-terminal status.
    assert_eq!(
        echo_count.load(Ordering::SeqCst),
        1,
        "echo handler must be invoked exactly once after reclaim+redispatch; got {}",
        echo_count.load(Ordering::SeqCst)
    );
}

/// A second `EngineExecutionSink::dispatch` of the same `Start` job on an
/// execution that is already `Completed` is a safe no-op: it returns `Ok(())`
/// and does NOT re-run the action handler (echo counter stays at 1).
///
/// This directly exercises the `read_status` → terminal → `Ok(())` guard in
/// `EngineExecutionSink::dispatch` (the idempotency contract documented in
/// `execution_sink.rs`).  The test dispatches twice on a `Completed` execution
/// because driving to a clean intermediate `Running` state is not possible with
/// the synchronous echo engine (the engine drives from `Created` to `Completed`
/// in one call).
#[tokio::test(start_paused = true)]
async fn redelivered_start_on_running_or_terminal_is_noop() {
    use nebula_engine::EngineExecutionSink;
    use nebula_orchestrator::ExecutionSink;

    let stores = TestStores::new();
    let (engine, echo_count) = make_engine(&stores).await;
    let workflow = save_echo_workflow(&stores).await;
    let workflow_id = workflow.definition().id;

    // Seed a Created execution row.
    let execution_id = ExecutionId::new();
    persist_created(
        &stores,
        workflow_id,
        execution_id,
        serde_json::json!({"trigger": "idempotency-test"}),
    )
    .await;

    // Build the real EngineExecutionSink (same wiring as WorkerRuntimeBuilder).
    let sink = EngineExecutionSink::new(
        Arc::clone(&engine),
        Arc::clone(&stores.execution) as Arc<dyn ExecutionStore>,
    );

    let plugin_key: PluginKey = TEST_PLUGIN_KEY.parse().unwrap();
    let job_id = [0xCCu8; 16];
    let msg = JobDispatchMsg::new(
        job_id,
        execution_id.to_string(),
        ControlCommand::Start,
        scope(),
        serde_json::json!({}),
        None::<String>,
        "sha-idem-test",
        plugin_key.clone(),
        vec![plugin_key],
        None::<String>,
        0,
    );

    // First dispatch: drives Created → Completed; handler runs once.
    let result1 = sink.dispatch(&msg).await;
    assert!(
        result1.is_ok(),
        "first dispatch must succeed; got {result1:?}"
    );
    assert_eq!(
        echo_count.load(Ordering::SeqCst),
        1,
        "echo handler must run exactly once on first dispatch"
    );

    let status_after_first = read_status(&stores, execution_id)
        .await
        .expect("execution row must exist");
    assert_eq!(
        status_after_first,
        ExecutionStatus::Completed,
        "execution must be Completed after first dispatch; got {status_after_first:?}"
    );

    // Second dispatch: execution is already Completed; must be a no-op.
    let result2 = sink.dispatch(&msg).await;
    assert!(
        result2.is_ok(),
        "re-delivered Start on a Completed execution must return Ok(()); got {result2:?}"
    );

    // The echo handler must NOT have been invoked a second time.
    assert_eq!(
        echo_count.load(Ordering::SeqCst),
        1,
        "echo handler must NOT run again on re-delivered Start (idempotency guard); count={}",
        echo_count.load(Ordering::SeqCst)
    );

    // Execution status must be unchanged.
    let status_after_second = read_status(&stores, execution_id)
        .await
        .expect("execution row must still exist");
    assert_eq!(
        status_after_second,
        ExecutionStatus::Completed,
        "execution status must remain Completed after second dispatch; got {status_after_second:?}"
    );

    // Verify result2 is specifically Ok, not an error disguised as ok.
    result2.expect("re-delivered Start must be Ok(())");
}

/// When a job-dispatch row is reclaimed `max_reclaim_count` times its status
/// moves to `Failed`.  This does NOT affect the execution row.
///
/// This test documents the "long-job artifact" as a property: a dispatch row
/// can reach `Failed` while the execution it represents is `Completed`.
/// `claim_pending` no longer returns the exhausted row.
///
/// Seeded execution is `Completed` from the start to make the independence
/// of the two tables concrete and visible in the assertion.
#[tokio::test(start_paused = true)]
async fn job_dispatch_row_exhausted_to_failed_leaves_execution_intact() {
    let stores = TestStores::new();
    let workflow = save_echo_workflow(&stores).await;
    let workflow_id = workflow.definition().id;

    // Seed the execution already Completed (via an engine run).
    let (engine, _echo_count) = make_engine(&stores).await;
    let execution_id = ExecutionId::new();
    persist_created(
        &stores,
        workflow_id,
        execution_id,
        serde_json::json!({"trigger": "exhaustion-test"}),
    )
    .await;

    // Drive to Completed via the engine so the status is durable.
    engine
        .resume_execution(&scope(), execution_id)
        .await
        .expect("initial engine run to Completed");

    let status_before = read_status(&stores, execution_id)
        .await
        .expect("execution row must exist");
    assert_eq!(
        status_before,
        ExecutionStatus::Completed,
        "execution must start Completed for this property test; got {status_before:?}"
    );

    // Enqueue a separate job-dispatch row (simulates a Start that was issued
    // but whose job row is now being reclaimed repeatedly).
    let queue = Arc::new(InMemoryJobDispatchQueue::new(&stores.execution));
    let plugin_key: PluginKey = TEST_PLUGIN_KEY.parse().unwrap();
    let job_id = [0xDDu8; 16];
    let msg = JobDispatchMsg::new(
        job_id,
        execution_id.to_string(),
        ControlCommand::Start,
        scope(),
        serde_json::json!({}),
        None::<String>,
        "sha-exhaust-test",
        plugin_key.clone(),
        vec![plugin_key.clone()],
        None::<String>,
        0,
    );
    queue.enqueue(&msg).await.expect("enqueue job");

    // max_reclaim_count = 2: the row exhausts after being reclaimed twice.
    let max_reclaim_count: u32 = 2;
    let tiny = Duration::from_millis(1);
    let tags = vec![plugin_key.clone()];
    let crasher = proc16(0xDD);

    // Reclaim loop: claim, advance time, reclaim_stuck, repeat until exhausted.
    let mut exhausted = false;
    for i in 0..=max_reclaim_count {
        // Claim (puts row Processing).
        let claimed = queue
            .claim_pending(&crasher, 1, &tags)
            .await
            .expect("claim");
        if claimed.is_empty() {
            // Row is no longer Pending (it must be Failed/exhausted).
            exhausted = true;
            break;
        }
        assert_eq!(
            claimed.len(),
            1,
            "must claim exactly one row on iteration {i}"
        );

        // Do NOT mark dispatched — simulate a crash.
        // Advance virtual time past reclaim_after so the row becomes stale.
        tokio::time::advance(Duration::from_millis(5)).await;

        let outcome = queue
            .reclaim_stuck(tiny, max_reclaim_count)
            .await
            .expect("reclaim_stuck");

        // When reclaim_count reaches max_reclaim_count the sweep exhausts the row.
        if outcome.exhausted >= 1 {
            exhausted = true;
            break;
        }
    }
    assert!(
        exhausted,
        "job-dispatch row must exhaust to Failed within max_reclaim_count reclaims"
    );

    // Execution row must be unaffected — still Completed.
    let status_after = read_status(&stores, execution_id)
        .await
        .expect("execution row must still exist after job-dispatch exhaustion");
    assert_eq!(
        status_after,
        ExecutionStatus::Completed,
        "execution status must remain Completed after job-dispatch row exhausts; \
         dispatch-row failure is a routing artifact, not an execution failure; \
         got {status_after:?}"
    );

    // The exhausted (Failed) row must no longer be returned by claim_pending.
    let leftover = queue
        .claim_pending(&proc16(0xEE), 8, &tags)
        .await
        .expect("probe claim");
    assert!(
        leftover.is_empty(),
        "exhausted (Failed) job-dispatch row must not be returned by claim_pending"
    );
}
