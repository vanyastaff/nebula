//! ADR-0095 D3/D5 — "first real trigger dispatch" vertical slice.
//!
//! Tests verify the full path: trigger fires → `DurableExecutionEmitter` →
//! `WorkflowStartService` atomically materializes the contract → exact-flavor
//! `ControlConsumer` claims → `EngineControlDispatch` drives the execution.
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
//!   control consumer → engine runs to Completed; redelivery of same event_id asserts exactly
//!   one execution.
//!
//! This engine integration fixture uses InMemory. Storage's shared materialization
//! oracle separately runs against SQLite and required live PostgreSQL.

use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use nebula_action::{
    ActionError, ExecutionEmitter, IdempotencyKey, action::Action, result::ActionResult,
    stateless::StatelessAction,
};
use nebula_core::{Dependencies, PluginKey, action_key, id::ExecutionId, node_key};
use nebula_engine::{
    ActionRegistry, ActionRuntime, DataPassingPolicy, EngineExecutionSink, InProcessRunner,
    ResourceFanoutCoordinator, ResourceFanoutFailureKind, WorkflowEngine,
    WorkflowTriggerConsumerCodec, WorkflowTriggerTarget,
};
use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_metrics::MetricsRegistry;
use nebula_orchestrator::{DispatchedTurn, ExecutionSink};
use nebula_storage::inmem::InMemoryTurnHandoff;
use nebula_storage::{
    InMemoryExecutionStore, InMemoryWorkflowVersionStore, inmem::InMemoryJobDispatchQueue,
};
use nebula_storage_port::{
    Scope, StorageError,
    dto::{
        AcceptResourceEventRequest, AcquireResourceSourceLeaseOutcome,
        AcquireResourceSourceLeaseRequest, ClaimResourceHandoffsRequest,
        ClaimResourceRuntimeWorkRequest, ClaimedResourceHandoff, ControlCommand, EventEnvelope,
        EventOccurrenceKey, EventOccurrenceNamespace, HeartbeatResourceHandoffRequest,
        JobDispatchMsg, PutResourceSubscriptionRequest, ResolveSharedResourceOutcome,
        ResolveSharedResourceRequest, ResourceCompatibilityVersion, ResourceConfigurationIdentity,
        ResourceHandoffClaimRequest, ResourceKind, ResourceLeaseHolder, ResourceLeaseTtl,
        ResourcePageSize, ResourceSlotIdentity, SharedResourceIdentity, TriggerStartKey,
        WorkflowVersionRecord,
    },
    store::{
        ControlQueue, ExecutionStore, JobDispatchQueue, ResourceEventFanoutStore,
        ResourceExecutionHandoffStore, ResourceRuntimeRecovery, ResourceSourceLeaseStore,
        ResourceSubscriptionStore, SharedResourceStore, StartAcceptanceStore, WorkflowStore,
        WorkflowVersionStore,
    },
};
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, TriggerBinding, ValidatedWorkflow, Version,
    WorkflowConfig, WorkflowDefinition,
};
use tokio_util::sync::CancellationToken;

// ── shared harness ────────────────────────────────────────────────────────────

mod exact_fixture;

/// In-memory storage adapters for one isolated test tenant.
#[derive(Clone)]
struct TestStores {
    frozen: OnceLock<Arc<nebula_plugin::FrozenPluginRegistry>>,
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
        let workflow =
            nebula_storage::InMemoryWorkflowStore::new_with_versions(&versions, &execution);
        Self {
            frozen: OnceLock::new(),
            checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new(&execution)),
            execution,
            journal,
            node_results: Arc::new(nebula_storage::InMemoryNodeResultStore::new()),
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
            operation_ledger: Arc::new(nebula_storage::inmem::InMemoryOperationLedger::new(
                &self.execution,
            )),
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
            .with_plan_flavor_runtime(
                Arc::new(nebula_engine::PlanFlavorRevisionLoader::new(Arc::new(
                    self.execution.plan_flavor_catalog(),
                ))),
                Arc::clone(self.frozen.get().unwrap()),
                Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                    &self.execution,
                )),
            )
            .with_execution_stores(self.execution_stores())
    }
}

/// Return the test scope via the public re-export.
fn scope() -> Scope {
    static SCOPE: OnceLock<Scope> = OnceLock::new();
    SCOPE
        .get_or_init(|| {
            Scope::new(
                nebula_core::WorkspaceId::new().to_string(),
                nebula_core::OrgId::new().to_string(),
            )
        })
        .clone()
}

/// The test injects trigger events through its emitter; lifecycle has no external resource.
struct SliceTrigger;
struct SliceTriggerSource;

impl nebula_action::TriggerSource for SliceTriggerSource {
    type Event = serde_json::Value;
}

impl Action for SliceTrigger {
    type Input = serde_json::Value;
    type Output = serde_json::Value;
    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("test.dispatch.plugin.trigger"),
            nebula_action::metadata_name!("Slice trigger"),
            "Explicit test event source",
        )
        .with_effect_contract(nebula_action::effect::ActionEffectContract::ReadOnly)
    }
    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl nebula_action::FromWorkflowNode for SliceTrigger {
    type Error = ActionError;
    async fn from_workflow_node(
        _: &NodeDefinition,
        _: &dyn nebula_action::ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(Self)
    }
}

impl nebula_action::TriggerAction for SliceTrigger {
    type Source = SliceTriggerSource;
    type Error = ActionError;
    async fn start(
        &self,
        _: &(impl nebula_action::TriggerContext + ?Sized),
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    async fn stop(
        &self,
        _: &(impl nebula_action::TriggerContext + ?Sized),
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn handle(
        &self,
        _: &(impl nebula_action::TriggerContext + ?Sized),
        _: serde_json::Value,
    ) -> Result<nebula_action::TriggerEventOutcome, Self::Error> {
        Err(ActionError::fatal(
            "test events enter through the dedicated emitter",
        ))
    }
}

/// One-node echo workflow (StatelessAction that returns its input).
struct EchoHandler {
    count: Arc<AtomicU32>,
}

impl Action for EchoHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("test.dispatch.plugin.echo"),
            nebula_action::metadata_name!("Echo"),
            "echo",
        )
        .with_effect_contract(nebula_action::effect::ActionEffectContract::ReadOnly)
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
    registry
        .register_stateless_instance(
            nebula_action::ActionMetadataDraft::new(
                action_key!("test.dispatch.plugin.echo"),
                nebula_action::metadata_name!("Echo"),
                "echo",
            )
            .with_effect_contract(nebula_action::effect::ActionEffectContract::ReadOnly),
            EchoHandler {
                count: count.clone(),
            },
        )
        .expect("valid test catalog definition");
    registry
        .register_trigger_factory::<SliceTrigger>()
        .expect("valid test catalog definition");
    stores.frozen.get_or_init(|| {
        exact_fixture::freeze_registry(
            &registry,
            &[
                (TEST_PLUGIN_KEY, "test.dispatch.plugin.echo"),
                (TEST_PLUGIN_KEY, "test.dispatch.plugin.trigger"),
            ],
        )
    });
    let runner = Arc::new(InProcessRunner::new());
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
                "test.dispatch.plugin.echo",
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
                "test.dispatch.plugin.trigger",
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
    // Revision 1 of the row is what activation later advances.
    stores
        .workflow
        .create(
            &scope(),
            nebula_storage_port::dto::WorkflowRecord {
                id: validated.definition().id.to_string(),
                scope: scope(),
                version: 1,
                slug: "trigger-fixture".into(),
            },
        )
        .await
        .expect("create workflow row");
    stores
        .versions
        .create(
            &scope(),
            WorkflowVersionRecord {
                activation: None,
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
    let record = stores
        .versions
        .get(&scope(), &workflow_id.to_string(), 0)
        .await
        .unwrap()
        .unwrap();
    let encoded = serde_json::to_string(&record.definition).unwrap();
    let workflow = serde_json::from_str(&encoded).unwrap();
    exact_fixture::materialize_state(
        &stores.execution,
        &scope(),
        stores.frozen.get().unwrap(),
        &workflow,
        &mut exec_state,
    )
    .await;
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

/// Mint a turn fence the way the durable handoff does: acquire the
/// execution lease and hand the fence to the sink, which adopts it instead of
/// acquiring a second lease.
async fn mint_turn_fence(
    stores: &TestStores,
    execution_id: ExecutionId,
) -> nebula_storage_port::FencingToken {
    stores
        .execution
        .acquire_lease(
            &scope(),
            &execution_id.to_string(),
            "test-driver",
            Duration::from_secs(30),
        )
        .await
        .expect("acquire lease for the test turn")
        .expect("no live lease yet")
}

/// `EngineExecutionSink::dispatch` on a `Created` row drives
/// `resume_execution_leased` under the handoff fence and the execution
/// reaches `Completed`.
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
        test_plugin_key.clone(),
        vec![test_plugin_key],
        None::<String>,
        0,
        test_flavor(&[TEST_PLUGIN_KEY.parse().unwrap()]).revision_id(),
    );

    let fence = mint_turn_fence(&stores, execution_id).await;
    let turn = DispatchedTurn { msg: &msg, fence };
    let result = sink.dispatch(&turn).await;
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
        test_plugin_key.clone(),
        vec![test_plugin_key],
        None::<String>,
        0,
        test_flavor(&[TEST_PLUGIN_KEY.parse().unwrap()]).revision_id(),
    );

    // First dispatch — drives execution to Completed under the handoff fence.
    let fence = mint_turn_fence(&stores, execution_id).await;
    let turn = DispatchedTurn { msg: &msg, fence };
    sink.dispatch(&turn)
        .await
        .expect("first dispatch must succeed");

    // Second dispatch (redelivery) — must be Ok without re-running the
    // handler. The idempotency guard short-circuits before the fence is
    // adopted, so the same (now released) fence value is fine.
    let redelivery = DispatchedTurn { msg: &msg, fence };
    let second = sink.dispatch(&redelivery).await;
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

const TEST_PLUGIN_KEY: &str = "test.dispatch.plugin";

/// Build an emitter and control queue sharing one execution-store core.
async fn make_emitter(
    stores: &TestStores,
    workflow: Arc<ValidatedWorkflow>,
) -> (
    DurableExecutionEmitter,
    Arc<nebula_storage::InMemoryControlQueue>,
) {
    let queue = Arc::new(nebula_storage::InMemoryControlQueue::new(&stores.execution));
    let service = activated_start_service(stores, &workflow).await;
    let emitter = DurableExecutionEmitter::new(
        service,
        workflow.definition().id,
        node_key!("test.trigger"),
        scope(),
    );
    (emitter, queue)
}

async fn activated_start_service(
    stores: &TestStores,
    workflow: &ValidatedWorkflow,
) -> Arc<nebula_engine::WorkflowStartService> {
    if stores.frozen.get().is_none() {
        let _ = make_engine(stores).await;
    }
    let registry = Arc::clone(stores.frozen.get().unwrap());
    nebula_engine::WorkflowActivationService::new(
        stores.workflow.clone(),
        stores.versions.clone(),
        registry.clone(),
        nebula_engine::PlanFlavorRevisionInstaller::new(Arc::new(
            stores.execution.plan_flavor_catalog(),
        )),
        Arc::new(nebula_core::accessor::SystemClock),
    )
    .activate(
        &scope(),
        workflow.definition().id,
        1,
        workflow.definition().clone(),
    )
    .await
    .unwrap();
    Arc::new(
        nebula_engine::WorkflowStartService::new(
            stores.workflow_stores(),
            stores.execution.clone(),
            Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                &stores.execution,
            )),
            nebula_engine::PlanFlavorRevisionLoader::new(Arc::new(
                stores.execution.plan_flavor_catalog(),
            )),
            registry,
            Arc::new(nebula_core::accessor::SystemClock),
            nebula_execution::context::ExecutionBudget::default(),
        )
        .unwrap(),
    )
}

/// `DurableExecutionEmitter::emit` with `Some(event_id)` produces:
///  - the accepted execution id
///  - a `Created` execution row in the store
///  - exactly one Start row in the control queue
#[tokio::test(start_paused = true)]
async fn emitter_dispatched_creates_row_and_enqueues_start() {
    let stores = TestStores::new();
    let workflow = save_echo_workflow(&stores).await;
    let (emitter, queue) = make_emitter(&stores, Arc::clone(&workflow)).await;

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
    let jobs = queue
        .claim_pending_for_flavor(
            &proc16(1),
            10,
            nebula_plugin::WorkerFlavorContext::from_registry(stores.frozen.get().unwrap())
                .revision_id(),
        )
        .await
        .expect("claim_pending must succeed");
    assert_eq!(
        jobs.len(),
        1,
        "exactly one Start job must be enqueued after Dispatched emit; got {}",
        jobs.len()
    );
    assert_eq!(
        jobs[0].msg.execution_id,
        execution_id.to_string(),
        "enqueued job execution_id must match the returned id"
    );
    assert!(
        matches!(jobs[0].msg.command, ControlCommand::Start),
        "job command must be Start"
    );

    // Routing now uses the persisted contract's exact flavor rather than a
    // separate plugin list copied onto a job. Verify the authoritative pin.
    let row = stores
        .execution
        .get(&scope(), &execution_id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.state["worker_flavor_revision_id"],
        serde_json::to_value(
            nebula_plugin::WorkerFlavorContext::from_registry(stores.frozen.get().unwrap())
                .revision_id()
        )
        .unwrap()
    );
    assert!(
        InMemoryJobDispatchQueue::new(&stores.execution)
            .claim_pending(
                &proc16(9),
                10,
                &[TEST_PLUGIN_KEY.parse().unwrap()],
                nebula_plugin::WorkerFlavorContext::from_registry(stores.frozen.get().unwrap())
                    .revision_id()
            )
            .await
            .unwrap()
            .is_empty(),
        "one drive identity, with no second Job row"
    );
}

/// A second `emit` with the same `event_id` returns the WINNER's id (same as
/// `id1`), writes NO second `Created` row, and enqueues NO second Start job.
/// Atomic start acceptance returns the original winner's id in-transaction,
/// so duplicate callers always hold a valid execution id.
#[tokio::test(start_paused = true)]
async fn emitter_duplicate_event_id_no_second_row() {
    let stores = TestStores::new();
    let workflow = save_echo_workflow(&stores).await;
    let (emitter, queue) = make_emitter(&stores, Arc::clone(&workflow)).await;

    let event_id = IdempotencyKey::new("evt-dup");

    let id1 = emitter
        .emit(serde_json::json!({"n": 1}), Some(event_id.clone()))
        .await
        .expect("first emit (Dispatched) must succeed");

    let id2 = emitter
        .emit(serde_json::json!({"n": 2}), Some(event_id.clone()))
        .await
        .expect("second emit (Duplicate) must succeed");

    // On duplicate acceptance the emitter returns the winner's id from the
    // same transaction. Both calls therefore return the same id.
    assert_eq!(
        id1, id2,
        "Duplicate emit must return the original winner's id — both calls must return the same id"
    );

    // Only one Start job must be in the queue (the duplicate write is a no-op).
    let jobs = queue
        .claim_pending_for_flavor(
            &proc16(2),
            10,
            nebula_plugin::WorkerFlavorContext::from_registry(stores.frozen.get().unwrap())
                .revision_id(),
        )
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

/// Full vertical slice: trigger fires, atomic start acceptance persists the
/// execution and control delivery, then `ControlConsumer` claims and drives
/// `resume_execution` to completion. Redelivery of the same `event_id`
/// produces exactly one execution.
#[tokio::test(start_paused = true)]
async fn trigger_dispatch_end_to_end_real_engine_resume() {
    let stores = TestStores::new();
    let (engine, echo_count) = make_engine(&stores).await;
    let workflow = save_echo_workflow(&stores).await;

    let (emitter, queue) = make_emitter(&stores, Arc::clone(&workflow)).await;

    // 1. Trigger fires → emitter creates Created row + enqueues Start.
    let event_id = IdempotencyKey::new("evt-e2e-001");
    let execution_id = emitter
        .emit(serde_json::json!({"event": "tick"}), Some(event_id.clone()))
        .await
        .expect("emit must succeed");

    // Assert Created row seeded correctly.
    let status_before = read_status(&stores, execution_id)
        .await
        .expect("Created row must exist before the control consumer runs");
    assert_eq!(
        status_before,
        ExecutionStatus::Created,
        "row must be Created before the control consumer claims it; got {status_before:?}"
    );

    // 2. Wire the execution dispatch and let the control consumer claim,
    //    hand off, and dispatch the accepted start.
    let dispatch = Arc::new(nebula_engine::EngineControlDispatch::new(
        Arc::clone(&engine),
        stores.execution.clone() as Arc<dyn ExecutionStore>,
        Arc::new(InMemoryTurnHandoff::new(&stores.execution)),
        "trigger-dispatch-control".to_owned(),
        Duration::from_secs(30),
    ));
    let cancel = CancellationToken::new();
    let consumer = nebula_engine::ControlConsumer::for_flavor(
        queue,
        dispatch,
        proc16(0xAA),
        engine.worker_flavor_context().unwrap().revision_id(),
    );

    let cancel_clone = cancel.clone();
    let consumer_handle = tokio::spawn(async move { consumer.run(cancel_clone).await });

    // Yield so the consumer spawns and enters its poll loop, then advance
    // virtual time past the poll interval so it claims the pending job.
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_secs(2)).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }

    cancel.cancel();
    consumer_handle
        .await
        .expect("control consumer task must not panic");

    // 3. Assert execution reached Completed.
    let status_after = read_status(&stores, execution_id)
        .await
        .expect("execution row must exist after the control consumer ran");
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

fn test_flavor(plugins: &[PluginKey]) -> nebula_plugin::WorkerFlavorContext {
    nebula_plugin::WorkerFlavorContext::from_registry(&test_registry(plugins))
}

fn test_registry(plugins: &[PluginKey]) -> Arc<nebula_plugin::FrozenPluginRegistry> {
    #[derive(Debug)]
    struct FixturePlugin(nebula_plugin::PluginManifest);
    impl nebula_plugin::Plugin for FixturePlugin {
        fn manifest(&self) -> &nebula_plugin::PluginManifest {
            &self.0
        }
    }
    let mut registry = nebula_plugin::PluginRegistry::new();
    for key in plugins {
        let plugin = FixturePlugin(
            nebula_plugin::PluginManifest::builder(key.as_str(), key.as_str())
                .build()
                .unwrap(),
        );
        registry
            .register(Arc::new(
                nebula_plugin::ResolvedPlugin::from(plugin).unwrap(),
            ))
            .unwrap();
    }
    let frozen = registry
        .freeze(
            nebula_core::ArtifactSetDigest::from_bytes([0x31; 32]),
            "1.0.0".parse().unwrap(),
        )
        .unwrap();
    Arc::new(frozen)
}

#[tokio::test]
async fn unsupported_selected_flavor_writes_no_execution_dispatch_or_dedup() {
    let stores = TestStores::new();
    let workflow = save_echo_workflow(&stores).await;
    let starts = nebula_storage::inmem::InMemoryStartAcceptanceStore::new(&stores.execution);
    let queue = InMemoryJobDispatchQueue::new(&stores.execution);
    let _ = activated_start_service(&stores, &workflow).await;
    let registry = test_registry(&["unrelated.plugin".parse().unwrap()]);
    let flavor_id = nebula_plugin::WorkerFlavorContext::from_registry(&registry).revision_id();
    let service = Arc::new(
        nebula_engine::WorkflowStartService::new(
            stores.workflow_stores(),
            stores.execution.clone(),
            Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                &stores.execution,
            )),
            nebula_engine::PlanFlavorRevisionLoader::new(Arc::new(
                stores.execution.plan_flavor_catalog(),
            )),
            registry,
            Arc::new(nebula_core::accessor::SystemClock),
            nebula_execution::context::ExecutionBudget::default(),
        )
        .unwrap(),
    );
    let emitter = DurableExecutionEmitter::new(
        service,
        workflow.definition().id,
        node_key!("test.trigger"),
        scope(),
    );
    let result = emitter
        .emit(
            serde_json::json!({}),
            Some(IdempotencyKey::new("unsupported-flavor")),
        )
        .await;
    assert!(
        result.is_err(),
        "an unsupported selected flavor must fail before materialization"
    );
    assert_eq!(stores.execution.count(&scope(), None).await.unwrap(), 0);
    assert!(
        starts
            .lookup_trigger_start(
                &scope(),
                &TriggerStartKey::new("test.trigger", "unsupported-flavor")
            )
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        queue
            .claim_pending(
                &proc16(0xAB),
                10,
                &[TEST_PLUGIN_KEY.parse().unwrap()],
                flavor_id
            )
            .await
            .unwrap()
            .is_empty()
    );
}

#[derive(Debug)]
struct FailFirstHandoffAcknowledgement {
    inner: Arc<nebula_storage::InMemoryResourceRuntime>,
    remaining_failures: AtomicU32,
    heartbeat_calls: AtomicU32,
}

#[derive(Debug)]
struct AlwaysFailResourceRecovery;

#[async_trait::async_trait]
impl ResourceRuntimeRecovery for AlwaysFailResourceRecovery {
    async fn claim_deliveries_globally(
        &self,
        _request: ClaimResourceRuntimeWorkRequest,
    ) -> Result<Vec<nebula_storage_port::dto::ScopedClaimedResourceDelivery>, StorageError> {
        Err(StorageError::Connection(
            "injected global delivery claim failure".to_owned(),
        ))
    }

    async fn claim_handoffs_globally(
        &self,
        _request: ClaimResourceRuntimeWorkRequest,
    ) -> Result<Vec<nebula_storage_port::dto::ScopedClaimedResourceHandoff>, StorageError> {
        Err(StorageError::Connection(
            "injected global handoff claim failure".to_owned(),
        ))
    }
}

#[async_trait::async_trait]
impl ResourceExecutionHandoffStore for FailFirstHandoffAcknowledgement {
    async fn claim_handoffs(
        &self,
        request: ClaimResourceHandoffsRequest,
    ) -> Result<Vec<ClaimedResourceHandoff>, StorageError> {
        self.inner.claim_handoffs(request).await
    }

    async fn heartbeat_handoff(
        &self,
        request: HeartbeatResourceHandoffRequest,
    ) -> Result<ClaimedResourceHandoff, StorageError> {
        self.heartbeat_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.heartbeat_handoff(request).await
    }

    async fn release_handoff(
        &self,
        request: ResourceHandoffClaimRequest,
    ) -> Result<(), StorageError> {
        self.inner.release_handoff(request).await
    }

    async fn acknowledge_handoff(
        &self,
        request: ResourceHandoffClaimRequest,
    ) -> Result<nebula_storage_port::dto::AcknowledgeResourceHandoffOutcome, StorageError> {
        if self.remaining_failures.swap(0, Ordering::SeqCst) > 0 {
            return Err(StorageError::Connection(
                "injected handoff acknowledgement failure".to_owned(),
            ));
        }
        self.inner.acknowledge_handoff(request).await
    }
}

#[tokio::test]
async fn resource_fanout_one_shot_replays_start_after_ack_failure_without_eventbus() {
    let stores = TestStores::new();
    let workflow = save_echo_workflow(&stores).await;
    let starts = activated_start_service(&stores, &workflow).await;
    let resource_runtime = Arc::new(nebula_storage::InMemoryResourceRuntime::new());
    let resource_scope = scope();
    let resource_id = match resource_runtime
        .resolve(ResolveSharedResourceRequest::new(
            resource_scope.clone(),
            SharedResourceIdentity::new(
                ResourceKind::new("test.resource").expect("valid resource kind"),
                ResourceCompatibilityVersion::new(1),
                ResourceConfigurationIdentity::try_from_vec(b"configuration".to_vec())
                    .expect("valid configuration identity"),
                ResourceSlotIdentity::try_from_vec(Vec::new()).expect("valid slot identity"),
            ),
        ))
        .await
        .expect("resource resolves")
    {
        ResolveSharedResourceOutcome::Created(record)
        | ResolveSharedResourceOutcome::Existing(record) => record.id(),
    };
    let (consumer_kind, consumer_identity) = WorkflowTriggerConsumerCodec::encode(
        &WorkflowTriggerTarget::new(workflow.definition().id, node_key!("test.trigger")),
    )
    .expect("workflow target encodes");
    resource_runtime
        .put(PutResourceSubscriptionRequest::new(
            resource_scope.clone(),
            resource_id,
            consumer_kind,
            consumer_identity,
        ))
        .await
        .expect("subscription stores");
    let lease = match resource_runtime
        .acquire(AcquireResourceSourceLeaseRequest::new(
            resource_scope.clone(),
            resource_id,
            ResourceLeaseHolder::new("resource-source").expect("valid source holder"),
            ResourceLeaseTtl::new(Duration::from_secs(30)).expect("valid source TTL"),
        ))
        .await
        .expect("source lease acquires")
    {
        AcquireResourceSourceLeaseOutcome::Acquired(lease) => lease,
        AcquireResourceSourceLeaseOutcome::Contended { .. } => {
            panic!("fresh resource source lease must be acquired")
        },
    };
    resource_runtime
        .accept(AcceptResourceEventRequest::new(
            resource_scope.clone(),
            resource_id,
            lease.token().clone(),
            EventOccurrenceNamespace::new("test.event").expect("valid namespace"),
            EventOccurrenceKey::try_from_vec(b"occurrence-1".to_vec()).expect("valid occurrence"),
            EventEnvelope::try_from_vec(1, br#"{"event":"tick"}"#.to_vec())
                .expect("valid envelope"),
        ))
        .await
        .expect("event accepts");

    let handoffs = Arc::new(FailFirstHandoffAcknowledgement {
        inner: Arc::clone(&resource_runtime),
        remaining_failures: AtomicU32::new(1),
        heartbeat_calls: AtomicU32::new(0),
    });
    let claim = ClaimResourceRuntimeWorkRequest::new(
        ResourceLeaseHolder::new("resource-fanout").expect("valid fanout holder"),
        ResourceLeaseTtl::new(Duration::from_secs(30)).expect("valid fanout TTL"),
        ResourcePageSize::new(10).expect("valid batch size"),
    );
    let coordinator = ResourceFanoutCoordinator::new(
        resource_runtime.clone(),
        resource_runtime.clone(),
        resource_runtime.clone(),
        handoffs.clone(),
        starts,
        claim.clone(),
        Duration::from_millis(10),
        3,
    )
    .expect("coordinator builds");

    let first = coordinator.drain_once().await.expect_err("first ack fails");
    assert_eq!(first.error_code(), "RESOURCE_FANOUT:ACK_HANDOFF");
    let replay = coordinator.drain_once().await.expect("retry drains");
    assert_eq!(replay.completed_deliveries, 0);
    assert_eq!(replay.acknowledged_handoffs, 1);
    assert!(
        resource_runtime
            .claim_handoffs_globally(claim)
            .await
            .expect("handoff query succeeds")
            .is_empty(),
        "the exact handoff must be acknowledged after replay"
    );
    assert_eq!(
        stores.execution.count(&resource_scope, None).await.unwrap(),
        1,
        "stable trigger and delivery keys must materialize one execution"
    );
    let queue = nebula_storage::InMemoryControlQueue::new(&stores.execution);
    let starts = queue
        .claim_pending_for_flavor(
            &proc16(0xDC),
            10,
            nebula_plugin::WorkerFlavorContext::from_registry(stores.frozen.get().unwrap())
                .revision_id(),
        )
        .await
        .expect("control queue claims");
    assert_eq!(starts.len(), 1, "replay must not enqueue a second start");
    assert_eq!(
        handoffs.heartbeat_calls.load(Ordering::SeqCst),
        2,
        "each potentially slow workflow start must refresh its exact handoff claim"
    );
}

#[tokio::test(start_paused = true)]
async fn resource_fanout_run_stops_after_bounded_persistent_infrastructure_failures() {
    let stores = TestStores::new();
    let workflow = save_echo_workflow(&stores).await;
    let starts = activated_start_service(&stores, &workflow).await;
    let resource_runtime = Arc::new(nebula_storage::InMemoryResourceRuntime::new());
    let coordinator = ResourceFanoutCoordinator::new(
        Arc::new(AlwaysFailResourceRecovery),
        resource_runtime.clone(),
        resource_runtime.clone(),
        resource_runtime,
        starts,
        ClaimResourceRuntimeWorkRequest::new(
            ResourceLeaseHolder::new("bounded-failure").expect("valid holder"),
            ResourceLeaseTtl::new(Duration::from_secs(30)).expect("valid TTL"),
            ResourcePageSize::new(10).expect("valid batch size"),
        ),
        Duration::from_millis(1),
        2,
    )
    .expect("coordinator builds");

    let error = coordinator
        .run(CancellationToken::new())
        .await
        .expect_err("persistent failure reaches the configured bound");
    assert_eq!(error.attempts(), 2);
    assert_eq!(error.error_code(), "RESOURCE_FANOUT:CLAIM_DELIVERIES");
}

const FANOUT_FAILURE_SECRET: &str = "fanout-private-storage-value";

#[derive(Debug, Clone, Copy)]
enum RecoveryFault {
    Connection,
    CommitUnknown,
    Corrupt,
    Configuration,
}

impl RecoveryFault {
    fn error(self) -> StorageError {
        match self {
            Self::Connection => StorageError::Connection(FANOUT_FAILURE_SECRET.to_owned()),
            Self::CommitUnknown => StorageError::AcknowledgementUnknown {
                operation: FANOUT_FAILURE_SECRET,
            },
            Self::Corrupt => StorageError::Corrupt(FANOUT_FAILURE_SECRET.to_owned()),
            Self::Configuration => StorageError::Configuration(FANOUT_FAILURE_SECRET.to_owned()),
        }
    }
}

/// Fault decorator for the coordinator's authoritative global claim port.
#[derive(Debug)]
struct FaultingResourceRecovery {
    fault: RecoveryFault,
    delivery_calls: AtomicU32,
    cancel_after_first_failure: Option<CancellationToken>,
}

#[async_trait::async_trait]
impl ResourceRuntimeRecovery for FaultingResourceRecovery {
    async fn claim_deliveries_globally(
        &self,
        _request: ClaimResourceRuntimeWorkRequest,
    ) -> Result<Vec<nebula_storage_port::dto::ScopedClaimedResourceDelivery>, StorageError> {
        let call = self.delivery_calls.fetch_add(1, Ordering::SeqCst);
        if call > 0
            && let Some(shutdown) = &self.cancel_after_first_failure
        {
            shutdown.cancel();
            return Ok(Vec::new());
        }
        Err(self.fault.error())
    }

    async fn claim_handoffs_globally(
        &self,
        _request: ClaimResourceRuntimeWorkRequest,
    ) -> Result<Vec<nebula_storage_port::dto::ScopedClaimedResourceHandoff>, StorageError> {
        Ok(Vec::new())
    }
}

async fn coordinator_with_recovery_fault(
    recovery: Arc<FaultingResourceRecovery>,
) -> ResourceFanoutCoordinator {
    let stores = TestStores::new();
    let workflow = save_echo_workflow(&stores).await;
    let starts = activated_start_service(&stores, &workflow).await;
    let runtime = Arc::new(nebula_storage::InMemoryResourceRuntime::new());
    ResourceFanoutCoordinator::new(
        recovery,
        runtime.clone(),
        runtime.clone(),
        runtime,
        starts,
        ClaimResourceRuntimeWorkRequest::new(
            ResourceLeaseHolder::new("failure-taxonomy").expect("valid holder"),
            ResourceLeaseTtl::new(Duration::from_secs(30)).expect("valid TTL"),
            ResourcePageSize::new(10).expect("valid batch size"),
        ),
        Duration::from_millis(1),
        3,
    )
    .expect("coordinator builds")
}

#[tokio::test(start_paused = true)]
async fn resource_fanout_permanent_storage_failures_stop_before_retrying() {
    for fault in [RecoveryFault::Corrupt, RecoveryFault::Configuration] {
        let recovery = Arc::new(FaultingResourceRecovery {
            fault,
            delivery_calls: AtomicU32::new(0),
            cancel_after_first_failure: None,
        });
        let coordinator = coordinator_with_recovery_fault(recovery.clone()).await;
        let error = coordinator
            .run(CancellationToken::new())
            .await
            .expect_err("permanent storage failure must stop the runtime");

        assert_eq!(
            error.attempts(),
            1,
            "{fault:?} must not consume retry budget"
        );
        assert_eq!(recovery.delivery_calls.load(Ordering::SeqCst), 1);
        assert_eq!(error.error_code(), "RESOURCE_FANOUT:CLAIM_DELIVERIES");
        assert_eq!(
            error.failure_kind(),
            match fault {
                RecoveryFault::Corrupt => ResourceFanoutFailureKind::StoredDataInvalid,
                RecoveryFault::Configuration => ResourceFanoutFailureKind::Misconfigured,
                _ => unreachable!("test only injects permanent failures"),
            }
        );
        assert!(!error.to_string().contains(FANOUT_FAILURE_SECRET));
        assert!(!format!("{error:?}").contains(FANOUT_FAILURE_SECRET));
    }
}

#[tokio::test(start_paused = true)]
async fn resource_fanout_transient_and_unknown_commit_failures_keep_retry_bound() {
    for fault in [RecoveryFault::Connection, RecoveryFault::CommitUnknown] {
        let recovery = Arc::new(FaultingResourceRecovery {
            fault,
            delivery_calls: AtomicU32::new(0),
            cancel_after_first_failure: None,
        });
        let coordinator = coordinator_with_recovery_fault(recovery.clone()).await;
        let error = coordinator
            .run(CancellationToken::new())
            .await
            .expect_err("retryable storage failure must reach the configured bound");

        assert_eq!(error.attempts(), 3);
        assert_eq!(recovery.delivery_calls.load(Ordering::SeqCst), 3);
        assert_eq!(error.error_code(), "RESOURCE_FANOUT:CLAIM_DELIVERIES");
        assert_eq!(
            error.failure_kind(),
            match fault {
                RecoveryFault::Connection => ResourceFanoutFailureKind::Unavailable,
                RecoveryFault::CommitUnknown => ResourceFanoutFailureKind::CommitUnknown,
                _ => unreachable!("test only injects retryable failures"),
            }
        );
        assert!(!error.to_string().contains(FANOUT_FAILURE_SECRET));
        assert!(!format!("{error:?}").contains(FANOUT_FAILURE_SECRET));
    }
}

#[tokio::test(start_paused = true)]
async fn resource_fanout_empty_drain_cannot_erase_a_permanent_failure() {
    let shutdown = CancellationToken::new();
    let recovery = Arc::new(FaultingResourceRecovery {
        fault: RecoveryFault::Corrupt,
        delivery_calls: AtomicU32::new(0),
        cancel_after_first_failure: Some(shutdown.clone()),
    });
    let coordinator = coordinator_with_recovery_fault(recovery.clone()).await;
    let error = coordinator
        .run(shutdown)
        .await
        .expect_err("corruption must remain fatal before a later empty drain and shutdown");

    assert_eq!(error.attempts(), 1);
    assert_eq!(recovery.delivery_calls.load(Ordering::SeqCst), 1);
    assert_eq!(error.error_code(), "RESOURCE_FANOUT:CLAIM_DELIVERIES");
    assert_eq!(
        error.failure_kind(),
        ResourceFanoutFailureKind::StoredDataInvalid
    );
}

#[derive(Debug)]
struct FaultingSubscriptions {
    inner: Arc<nebula_storage::InMemoryResourceRuntime>,
    connection_before_corrupt: bool,
    get_calls: AtomicU32,
}

#[async_trait::async_trait]
impl ResourceSubscriptionStore for FaultingSubscriptions {
    async fn put(
        &self,
        request: PutResourceSubscriptionRequest,
    ) -> Result<nebula_storage_port::dto::PutResourceSubscriptionOutcome, StorageError> {
        self.inner.put(request).await
    }

    async fn get(
        &self,
        _scope: &Scope,
        _id: nebula_storage_port::dto::ResourceSubscriptionId,
    ) -> Result<Option<nebula_storage_port::dto::ResourceSubscriptionRecord>, StorageError> {
        let call = self.get_calls.fetch_add(1, Ordering::SeqCst);
        if self.connection_before_corrupt && call == 0 {
            Err(StorageError::Connection(FANOUT_FAILURE_SECRET.to_owned()))
        } else {
            Err(StorageError::Corrupt(FANOUT_FAILURE_SECRET.to_owned()))
        }
    }

    async fn list_active_for_resource(
        &self,
        scope: &Scope,
        resource_id: nebula_storage_port::dto::SharedResourceId,
        after: Option<nebula_storage_port::dto::ReconciliationCursor>,
        page_size: ResourcePageSize,
    ) -> Result<nebula_storage_port::dto::ResourceSubscriptionPage, StorageError> {
        self.inner
            .list_active_for_resource(scope, resource_id, after, page_size)
            .await
    }

    async fn list_for_reconciliation(
        &self,
        scope: &Scope,
        after: Option<nebula_storage_port::dto::ReconciliationCursor>,
        page_size: ResourcePageSize,
    ) -> Result<nebula_storage_port::dto::ResourceSubscriptionPage, StorageError> {
        ResourceSubscriptionStore::list_for_reconciliation(&*self.inner, scope, after, page_size)
            .await
    }

    async fn count_active_for_resource(
        &self,
        scope: &Scope,
        resource_id: nebula_storage_port::dto::SharedResourceId,
    ) -> Result<u64, StorageError> {
        self.inner
            .count_active_for_resource(scope, resource_id)
            .await
    }

    async fn transition(
        &self,
        request: nebula_storage_port::dto::TransitionResourceSubscriptionRequest,
    ) -> Result<nebula_storage_port::dto::ResourceSubscriptionRecord, StorageError> {
        self.inner.transition(request).await
    }
}

#[derive(Debug, Clone, Copy)]
enum DeliveryFault {
    None,
    Fenced,
    CommitUnknownWithFailedRelease,
}

/// Exact-token fault decorator; counters record attempted mutations, not completed work.
#[derive(Debug)]
struct RecordingFanout {
    inner: Arc<nebula_storage::InMemoryResourceRuntime>,
    fault: DeliveryFault,
    complete_calls: AtomicU32,
    release_calls: AtomicU32,
}

#[async_trait::async_trait]
impl ResourceEventFanoutStore for RecordingFanout {
    async fn accept(
        &self,
        request: AcceptResourceEventRequest,
    ) -> Result<nebula_storage_port::dto::AcceptResourceEventOutcome, StorageError> {
        self.inner.accept(request).await
    }

    async fn get_event(
        &self,
        scope: &Scope,
        id: nebula_storage_port::dto::ResourceEventId,
    ) -> Result<Option<nebula_storage_port::dto::ResourceEventRecord>, StorageError> {
        self.inner.get_event(scope, id).await
    }

    async fn claim_deliveries(
        &self,
        request: nebula_storage_port::dto::ClaimResourceDeliveriesRequest,
    ) -> Result<Vec<nebula_storage_port::dto::ClaimedResourceDelivery>, StorageError> {
        self.inner.claim_deliveries(request).await
    }

    async fn heartbeat_delivery(
        &self,
        request: nebula_storage_port::dto::HeartbeatResourceDeliveryRequest,
    ) -> Result<nebula_storage_port::dto::ClaimedResourceDelivery, StorageError> {
        self.inner.heartbeat_delivery(request).await
    }

    async fn release_delivery(
        &self,
        request: nebula_storage_port::dto::ReleaseResourceDeliveryRequest,
    ) -> Result<(), StorageError> {
        self.release_calls.fetch_add(1, Ordering::SeqCst);
        if matches!(self.fault, DeliveryFault::CommitUnknownWithFailedRelease) {
            Err(StorageError::Connection(FANOUT_FAILURE_SECRET.to_owned()))
        } else {
            self.inner.release_delivery(request).await
        }
    }

    async fn complete_delivery(
        &self,
        request: nebula_storage_port::dto::CompleteResourceDeliveryRequest,
    ) -> Result<nebula_storage_port::dto::CompleteResourceDeliveryOutcome, StorageError> {
        self.complete_calls.fetch_add(1, Ordering::SeqCst);
        match self.fault {
            DeliveryFault::None => self.inner.complete_delivery(request).await,
            DeliveryFault::Fenced => Err(StorageError::FencedOut {
                entity: "resource delivery",
                id: FANOUT_FAILURE_SECRET.to_owned(),
            }),
            DeliveryFault::CommitUnknownWithFailedRelease => {
                Err(StorageError::AcknowledgementUnknown {
                    operation: FANOUT_FAILURE_SECRET,
                })
            },
        }
    }
}

async fn seed_resource_deliveries(
    runtime: &nebula_storage::InMemoryResourceRuntime,
    workflow: &ValidatedWorkflow,
    subscription_count: u32,
) {
    let resource_id = match runtime
        .resolve(ResolveSharedResourceRequest::new(
            scope(),
            SharedResourceIdentity::new(
                ResourceKind::new("test.failures").unwrap(),
                ResourceCompatibilityVersion::new(1),
                ResourceConfigurationIdentity::try_from_vec(b"failure-fixture".to_vec()).unwrap(),
                ResourceSlotIdentity::try_from_vec(Vec::new()).unwrap(),
            ),
        ))
        .await
        .unwrap()
    {
        ResolveSharedResourceOutcome::Created(record)
        | ResolveSharedResourceOutcome::Existing(record) => record.id(),
    };
    for index in 0..subscription_count {
        let (kind, identity) = WorkflowTriggerConsumerCodec::encode(&WorkflowTriggerTarget::new(
            workflow.definition().id,
            nebula_core::NodeKey::new(if index == 0 {
                "test.trigger".to_owned()
            } else {
                format!("test.trigger{index}")
            })
            .unwrap(),
        ))
        .unwrap();
        runtime
            .put(PutResourceSubscriptionRequest::new(
                scope(),
                resource_id,
                kind,
                identity,
            ))
            .await
            .unwrap();
    }
    let lease = match runtime
        .acquire(AcquireResourceSourceLeaseRequest::new(
            scope(),
            resource_id,
            ResourceLeaseHolder::new("failure-source").unwrap(),
            ResourceLeaseTtl::new(Duration::from_secs(30)).unwrap(),
        ))
        .await
        .unwrap()
    {
        AcquireResourceSourceLeaseOutcome::Acquired(lease) => lease,
        AcquireResourceSourceLeaseOutcome::Contended { .. } => panic!("fresh source must acquire"),
    };
    runtime
        .accept(AcceptResourceEventRequest::new(
            scope(),
            resource_id,
            lease.token().clone(),
            EventOccurrenceNamespace::new("test.failure").unwrap(),
            EventOccurrenceKey::try_from_vec(b"failure-event".to_vec()).unwrap(),
            EventEnvelope::try_from_vec(1, b"{}".to_vec()).unwrap(),
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn resource_fanout_fatal_subscription_corruption_preserves_claims() {
    for earlier_transient in [false, true] {
        let stores = TestStores::new();
        let workflow = save_echo_workflow(&stores).await;
        let starts = activated_start_service(&stores, &workflow).await;
        let runtime = Arc::new(nebula_storage::InMemoryResourceRuntime::new());
        seed_resource_deliveries(&runtime, &workflow, if earlier_transient { 2 } else { 1 }).await;
        let subscriptions = Arc::new(FaultingSubscriptions {
            inner: runtime.clone(),
            connection_before_corrupt: earlier_transient,
            get_calls: AtomicU32::new(0),
        });
        let fanout = Arc::new(RecordingFanout {
            inner: runtime.clone(),
            fault: DeliveryFault::None,
            complete_calls: AtomicU32::new(0),
            release_calls: AtomicU32::new(0),
        });
        let coordinator = ResourceFanoutCoordinator::new(
            runtime.clone(),
            subscriptions.clone(),
            fanout.clone(),
            runtime.clone(),
            starts,
            ClaimResourceRuntimeWorkRequest::new(
                ResourceLeaseHolder::new("fault-drain").unwrap(),
                ResourceLeaseTtl::new(Duration::from_secs(30)).unwrap(),
                ResourcePageSize::new(10).unwrap(),
            ),
            Duration::from_millis(1),
            3,
        )
        .unwrap();
        let error = coordinator
            .drain_once()
            .await
            .expect_err("corruption is fatal");

        assert_eq!(
            error.failure_kind(),
            ResourceFanoutFailureKind::StoredDataInvalid
        );
        assert_eq!(error.error_code(), "RESOURCE_FANOUT:LOAD_SUBSCRIPTION");
        assert_eq!(
            subscriptions.get_calls.load(Ordering::SeqCst),
            if earlier_transient { 2 } else { 1 }
        );
        assert_eq!(fanout.complete_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            fanout.release_calls.load(Ordering::SeqCst),
            u32::from(earlier_transient)
        );
        assert_eq!(stores.execution.count(&scope(), None).await.unwrap(), 0);
        assert!(!format!("{error:?}").contains(FANOUT_FAILURE_SECRET));
    }
}

#[tokio::test]
async fn resource_fanout_exact_fencing_does_not_release_or_count_completion() {
    let stores = TestStores::new();
    let workflow = save_echo_workflow(&stores).await;
    let starts = activated_start_service(&stores, &workflow).await;
    let runtime = Arc::new(nebula_storage::InMemoryResourceRuntime::new());
    seed_resource_deliveries(&runtime, &workflow, 1).await;
    let fanout = Arc::new(RecordingFanout {
        inner: runtime.clone(),
        fault: DeliveryFault::Fenced,
        complete_calls: AtomicU32::new(0),
        release_calls: AtomicU32::new(0),
    });
    let coordinator = ResourceFanoutCoordinator::new(
        runtime.clone(),
        runtime.clone(),
        fanout.clone(),
        runtime,
        starts,
        ClaimResourceRuntimeWorkRequest::new(
            ResourceLeaseHolder::new("fenced-drain").unwrap(),
            ResourceLeaseTtl::new(Duration::from_secs(30)).unwrap(),
            ResourcePageSize::new(10).unwrap(),
        ),
        Duration::from_millis(1),
        3,
    )
    .unwrap();
    let outcome = coordinator
        .drain_once()
        .await
        .expect("lost exact ownership is skipped");
    assert_eq!(outcome.completed_deliveries, 0);
    assert_eq!(outcome.acknowledged_handoffs, 0);
    assert_eq!(fanout.complete_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fanout.release_calls.load(Ordering::SeqCst), 0);
    assert_eq!(stores.execution.count(&scope(), None).await.unwrap(), 0);
}

#[tokio::test]
async fn resource_fanout_unknown_commit_survives_release_connectivity_failure() {
    let stores = TestStores::new();
    let workflow = save_echo_workflow(&stores).await;
    let starts = activated_start_service(&stores, &workflow).await;
    let runtime = Arc::new(nebula_storage::InMemoryResourceRuntime::new());
    seed_resource_deliveries(&runtime, &workflow, 1).await;
    let fanout = Arc::new(RecordingFanout {
        inner: runtime.clone(),
        fault: DeliveryFault::CommitUnknownWithFailedRelease,
        complete_calls: AtomicU32::new(0),
        release_calls: AtomicU32::new(0),
    });
    let coordinator = ResourceFanoutCoordinator::new(
        runtime.clone(),
        runtime.clone(),
        fanout.clone(),
        runtime,
        starts,
        ClaimResourceRuntimeWorkRequest::new(
            ResourceLeaseHolder::new("unknown-drain").unwrap(),
            ResourceLeaseTtl::new(Duration::from_secs(30)).unwrap(),
            ResourcePageSize::new(10).unwrap(),
        ),
        Duration::from_millis(1),
        3,
    )
    .unwrap();
    let error = coordinator
        .drain_once()
        .await
        .expect_err("commit diagnosis must remain unknown");
    assert_eq!(
        error.failure_kind(),
        ResourceFanoutFailureKind::CommitUnknown
    );
    assert_eq!(error.error_code(), "RESOURCE_FANOUT:COMPLETE_DELIVERY");
    assert_eq!(fanout.complete_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fanout.release_calls.load(Ordering::SeqCst), 1);
    assert!(!format!("{error:?}").contains(FANOUT_FAILURE_SECRET));
}

/// Records exact handoff mutations while optionally simulating lost heartbeat ownership.
#[derive(Debug)]
struct RecordingHandoffs {
    inner: Arc<nebula_storage::InMemoryResourceRuntime>,
    fence_heartbeat: bool,
    heartbeat_calls: AtomicU32,
    acknowledge_calls: AtomicU32,
    release_calls: AtomicU32,
}

#[async_trait::async_trait]
impl ResourceExecutionHandoffStore for RecordingHandoffs {
    async fn claim_handoffs(
        &self,
        request: ClaimResourceHandoffsRequest,
    ) -> Result<Vec<ClaimedResourceHandoff>, StorageError> {
        self.inner.claim_handoffs(request).await
    }

    async fn heartbeat_handoff(
        &self,
        request: HeartbeatResourceHandoffRequest,
    ) -> Result<ClaimedResourceHandoff, StorageError> {
        self.heartbeat_calls.fetch_add(1, Ordering::SeqCst);
        if self.fence_heartbeat {
            Err(StorageError::FencedOut {
                entity: "resource handoff",
                id: FANOUT_FAILURE_SECRET.to_owned(),
            })
        } else {
            self.inner.heartbeat_handoff(request).await
        }
    }

    async fn release_handoff(
        &self,
        request: ResourceHandoffClaimRequest,
    ) -> Result<(), StorageError> {
        self.release_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.release_handoff(request).await
    }

    async fn acknowledge_handoff(
        &self,
        request: ResourceHandoffClaimRequest,
    ) -> Result<nebula_storage_port::dto::AcknowledgeResourceHandoffOutcome, StorageError> {
        self.acknowledge_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.acknowledge_handoff(request).await
    }
}

struct HandoffFailureFixture {
    stores: TestStores,
    runtime: Arc<nebula_storage::InMemoryResourceRuntime>,
    starts: Arc<nebula_engine::WorkflowStartService>,
    claim: ClaimResourceRuntimeWorkRequest,
}

impl HandoffFailureFixture {
    async fn new() -> Self {
        let stores = TestStores::new();
        let workflow = save_echo_workflow(&stores).await;
        let starts = activated_start_service(&stores, &workflow).await;
        let runtime = Arc::new(nebula_storage::InMemoryResourceRuntime::new());
        seed_resource_deliveries(&runtime, &workflow, 1).await;
        let claim = ClaimResourceRuntimeWorkRequest::new(
            ResourceLeaseHolder::new("handoff-failure-fixture").unwrap(),
            ResourceLeaseTtl::new(Duration::from_secs(30)).unwrap(),
            ResourcePageSize::new(10).unwrap(),
        );
        for scoped in runtime
            .claim_deliveries_globally(claim.clone())
            .await
            .unwrap()
        {
            let (scope, delivery) = scoped.into_parts();
            runtime
                .complete_delivery(
                    nebula_storage_port::dto::CompleteResourceDeliveryRequest::new(
                        scope,
                        delivery.id(),
                        delivery.token().clone(),
                        nebula_storage_port::dto::ResourceDeliveryCompletion::Delivered,
                    ),
                )
                .await
                .unwrap();
        }
        Self {
            stores,
            runtime,
            starts,
            claim,
        }
    }

    fn coordinator(&self, handoffs: Arc<RecordingHandoffs>) -> ResourceFanoutCoordinator {
        ResourceFanoutCoordinator::new(
            self.runtime.clone(),
            self.runtime.clone(),
            self.runtime.clone(),
            handoffs,
            self.starts.clone(),
            self.claim.clone(),
            Duration::from_millis(1),
            3,
        )
        .unwrap()
    }

    fn handoffs(&self, fence_heartbeat: bool) -> Arc<RecordingHandoffs> {
        Arc::new(RecordingHandoffs {
            inner: self.runtime.clone(),
            fence_heartbeat,
            heartbeat_calls: AtomicU32::new(0),
            acknowledge_calls: AtomicU32::new(0),
            release_calls: AtomicU32::new(0),
        })
    }
}

#[tokio::test]
async fn resource_fanout_fenced_handoff_heartbeat_cannot_start_or_acknowledge() {
    let fixture = HandoffFailureFixture::new().await;
    let handoffs = fixture.handoffs(true);
    let coordinator = fixture.coordinator(handoffs.clone());
    let outcome = coordinator
        .drain_once()
        .await
        .expect("lost exact ownership is skipped");

    assert_eq!(outcome.completed_deliveries, 0);
    assert_eq!(outcome.acknowledged_handoffs, 0);
    assert_eq!(handoffs.heartbeat_calls.load(Ordering::SeqCst), 1);
    assert_eq!(handoffs.acknowledge_calls.load(Ordering::SeqCst), 0);
    assert_eq!(handoffs.release_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture
            .stores
            .execution
            .count(&scope(), None)
            .await
            .unwrap(),
        0
    );
}

/// Injects the owner's typed materialization invariant while retaining real reads.
#[derive(Debug)]
struct FingerprintMismatchStartStore {
    inner: nebula_storage::inmem::InMemoryStartAcceptanceStore,
    materialize_calls: AtomicU32,
}

#[async_trait::async_trait]
impl StartAcceptanceStore for FingerprintMismatchStartStore {
    async fn lookup_trigger_start(
        &self,
        scope: &Scope,
        key: &TriggerStartKey<'_>,
    ) -> Result<Option<String>, StorageError> {
        self.inner.lookup_trigger_start(scope, key).await
    }

    async fn materialize_start(
        &self,
        _start: &nebula_storage_port::dto::MaterializedStart<'_>,
    ) -> Result<
        nebula_storage_port::store::StartMaterialization,
        nebula_storage_port::store::StartMaterializationError,
    > {
        self.materialize_calls.fetch_add(1, Ordering::SeqCst);
        Ok(nebula_storage_port::store::StartMaterialization::FingerprintMismatch)
    }

    async fn lookup_start(
        &self,
        scope: &Scope,
        key: &str,
    ) -> Result<Option<nebula_storage_port::dto::StartReservation>, StorageError> {
        self.inner.lookup_start(scope, key).await
    }

    async fn read_contract_bundle(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<Option<nebula_storage_port::dto::StoredContractBundle>, StorageError> {
        self.inner.read_contract_bundle(scope, execution_id).await
    }
}

#[tokio::test(start_paused = true)]
async fn resource_fanout_injected_materialization_invariant_is_fatal_without_acknowledgement() {
    let mut fixture = HandoffFailureFixture::new().await;
    let starts = Arc::new(FingerprintMismatchStartStore {
        inner: nebula_storage::inmem::InMemoryStartAcceptanceStore::new(&fixture.stores.execution),
        materialize_calls: AtomicU32::new(0),
    });
    fixture.starts = Arc::new(
        nebula_engine::WorkflowStartService::new(
            fixture.stores.workflow_stores(),
            fixture.stores.execution.clone(),
            starts.clone(),
            nebula_engine::PlanFlavorRevisionLoader::new(Arc::new(
                fixture.stores.execution.plan_flavor_catalog(),
            )),
            fixture.stores.frozen.get().unwrap().clone(),
            Arc::new(nebula_core::accessor::SystemClock),
            nebula_execution::context::ExecutionBudget::default(),
        )
        .unwrap(),
    );
    let handoffs = fixture.handoffs(false);
    let coordinator = fixture.coordinator(handoffs.clone());
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        coordinator.run(CancellationToken::new()),
    )
    .await
    .expect("invariant must stop run before it can poll indefinitely")
    .expect_err("fingerprint mismatch is permanent");

    assert_eq!(
        error.failure_kind(),
        ResourceFanoutFailureKind::InvariantViolation
    );
    assert_eq!(error.error_code(), "WORKFLOW_START:FINGERPRINT_MISMATCH");
    assert_eq!(error.attempts(), 1);
    assert_eq!(handoffs.heartbeat_calls.load(Ordering::SeqCst), 1);
    assert_eq!(handoffs.acknowledge_calls.load(Ordering::SeqCst), 0);
    assert_eq!(handoffs.release_calls.load(Ordering::SeqCst), 0);
    assert_eq!(starts.materialize_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture
            .stores
            .execution
            .count(&scope(), None)
            .await
            .unwrap(),
        0,
        "rejected materialization must not create an execution"
    );
}
