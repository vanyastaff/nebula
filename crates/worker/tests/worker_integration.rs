//! Integration tests for `nebula-worker` durable control processing and for
//! the separately retained job-dispatch technical contract.
//!
//! Verifies that exact-flavor control starts reach the engine and that the
//! worker runtime does not attach a competing `JobDispatchQueue` poller.
//!
//! ## Test plan
//!
//! `worker_runtime_does_not_poll_the_technical_job_queue` asserts the component
//! boundary directly. `control_start_waits_for_worker_with_retained_exact_flavor`
//! covers the production command path.
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
    ActionError, action::Action, result::ActionResult, stateless::StatelessAction,
};
use nebula_core::{Dependencies, PluginKey, action_key, id::ExecutionId, node_key};
use nebula_engine::{
    ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessRunner, ResourceFanoutCoordinator,
    WorkflowEngine, WorkflowStartService,
};
use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_metrics::MetricsRegistry;
use nebula_storage::{
    InMemoryControlQueue, InMemoryExecutionStore, InMemoryWorkflowVersionStore,
    inmem::InMemoryJobDispatchQueue,
};
use nebula_storage_port::{
    Scope, StorageError,
    dto::{
        ClaimResourceRuntimeWorkRequest, ControlCommand, JobDispatchMsg, ResourceLeaseHolder,
        ResourceLeaseTtl, ResourcePageSize, ScopedClaimedResourceDelivery,
        ScopedClaimedResourceHandoff,
    },
    store::{ExecutionStore, JobDispatchQueue, ResourceRuntimeRecovery, WorkflowVersionStore},
};
use nebula_worker::WorkerRuntimeBuilder;
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, TriggerBinding, ValidatedWorkflow, Version,
    WorkflowConfig, WorkflowDefinition,
};
use tokio_util::sync::CancellationToken;

// ── Plugin key used across all test helpers ───────────────────────────────────

const TEST_PLUGIN_KEY: &str = "test";

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
        let workflow =
            nebula_storage::InMemoryWorkflowStore::new_with_versions(&versions, &execution);
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
        engine.with_execution_stores(self.execution_stores())
    }

    /// Handoff over the SAME shared core the queue and execution store use —
    /// the lease write and the queue acknowledgement must land under one
    /// boundary.
    fn turn_handoff(&self) -> Arc<nebula_storage::inmem::InMemoryTurnHandoff> {
        Arc::new(nebula_storage::inmem::InMemoryTurnHandoff::new(
            &self.execution,
        ))
    }

    fn resource_fanout(&self, plugins: &[PluginKey]) -> Arc<ResourceFanoutCoordinator> {
        let runtime = Arc::new(nebula_storage::inmem::InMemoryResourceRuntime::new());
        self.resource_fanout_with_recovery(plugins, runtime.clone(), runtime, 3)
    }

    fn resource_fanout_with_recovery(
        &self,
        plugins: &[PluginKey],
        recovery: Arc<dyn ResourceRuntimeRecovery>,
        runtime: Arc<nebula_storage::inmem::InMemoryResourceRuntime>,
        max_consecutive_failures: u32,
    ) -> Arc<ResourceFanoutCoordinator> {
        let starts = Arc::new(
            WorkflowStartService::new(
                self.workflow_stores(),
                self.execution.clone(),
                Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                    &self.execution,
                )),
                nebula_engine::PlanFlavorRevisionLoader::new(Arc::new(
                    nebula_storage::InMemoryPlanFlavorCatalog::new(&self.execution),
                )),
                Arc::new(frozen_fixture(plugins, Arc::new(AtomicU32::new(0)))),
                Arc::new(nebula_core::accessor::SystemClock),
                nebula_execution::ExecutionBudget::default(),
            )
            .expect("test workflow-start service must accept the default budget"),
        );
        let holder = ResourceLeaseHolder::new("worker-test-resource-fanout")
            .expect("test resource fanout holder is bounded");
        let ttl = ResourceLeaseTtl::new(Duration::from_secs(30))
            .expect("test resource fanout TTL is bounded");
        let batch_size =
            ResourcePageSize::new(32).expect("test resource fanout batch size is bounded");
        Arc::new(
            ResourceFanoutCoordinator::new(
                recovery,
                runtime.clone(),
                runtime.clone(),
                runtime,
                starts,
                ClaimResourceRuntimeWorkRequest::new(holder, ttl, batch_size),
                Duration::from_millis(100),
                max_consecutive_failures,
            )
            .expect("test resource fanout configuration is valid"),
        )
    }
}

#[derive(Debug)]
struct FailingResourceRecovery;

#[async_trait::async_trait]
impl ResourceRuntimeRecovery for FailingResourceRecovery {
    async fn claim_deliveries_globally(
        &self,
        _request: ClaimResourceRuntimeWorkRequest,
    ) -> Result<Vec<ScopedClaimedResourceDelivery>, StorageError> {
        Err(StorageError::AcknowledgementUnknown {
            operation: "resource-fanout-test",
        })
    }

    async fn claim_handoffs_globally(
        &self,
        _request: ClaimResourceRuntimeWorkRequest,
    ) -> Result<Vec<ScopedClaimedResourceHandoff>, StorageError> {
        Err(StorageError::AcknowledgementUnknown {
            operation: "resource-fanout-test",
        })
    }
}

/// Test scope used for worker integration tests. Matches `nebula_engine::store_seam::single_tenant_scope()`
/// so the worker and engine observe the same in-memory rows.
/// Production code uses the per-message scope from the control-queue / job-dispatch DTO.
fn scope() -> Scope {
    nebula_engine::store_seam::single_tenant_scope()
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

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("test.echo.worker"),
            nebula_action::metadata_name!("Echo"),
            "echo",
        )
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
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
    make_engine_with_exact_configuration(stores, true).await
}

async fn make_engine_with_exact_configuration(
    stores: &TestStores,
    exact: bool,
) -> (Arc<WorkflowEngine>, Arc<AtomicU32>) {
    make_engine_with_plugins(stores, exact, &[TEST_PLUGIN_KEY.parse().unwrap()]).await
}

async fn make_engine_with_plugins(
    stores: &TestStores,
    exact: bool,
    plugins: &[PluginKey],
) -> (Arc<WorkflowEngine>, Arc<AtomicU32>) {
    let count = Arc::new(AtomicU32::new(0));
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            nebula_action::ActionMetadataDraft::new(
                action_key!("test.echo.worker"),
                nebula_action::metadata_name!("Echo"),
                "echo",
            )
            .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects),
            EchoHandler {
                count: count.clone(),
            },
        )
        .expect("valid test catalog definition");
    // `InProcessRunner` + `executor` are structural boilerplate required by
    // `ActionRuntime::try_new` but are NOT the code path exercised by this test.
    // The legacy-registered `EchoHandler` runs via the direct stateless dispatch path;
    // `echo_count` is the witness that the real handler was invoked.
    let runner = Arc::new(InProcessRunner::new());
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
    let engine = stores
        .attach(WorkflowEngine::new(runtime, metrics).expect("WorkflowEngine must build in tests"));
    let engine = if exact {
        engine.with_plan_flavor_runtime(
            Arc::new(nebula_engine::PlanFlavorRevisionLoader::new(Arc::new(
                nebula_storage::InMemoryPlanFlavorCatalog::new(&stores.execution),
            ))),
            Arc::new(frozen_fixture(plugins, count.clone())),
            Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                &stores.execution,
            )),
        )
    } else {
        engine
    };
    (Arc::new(engine), count)
}

// ── Workflow persistence ──────────────────────────────────────────────────────

async fn save_echo_workflow(stores: &TestStores) -> Arc<ValidatedWorkflow> {
    save_echo_workflow_scoped(stores, &scope()).await
}

async fn save_echo_workflow_scoped(stores: &TestStores, scope: &Scope) -> Arc<ValidatedWorkflow> {
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
        trigger_bindings: Vec::<TriggerBinding>::new(),
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
            scope,
            nebula_storage_port::dto::WorkflowVersionRecord {
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

async fn persist_created(
    stores: &TestStores,
    workflow_id: nebula_core::WorkflowId,
    execution_id: ExecutionId,
    input: serde_json::Value,
) {
    let mut exec_state = ExecutionState::new(execution_id, workflow_id, &[]);
    let record = stores
        .versions
        .get_published(&scope(), &workflow_id.to_string())
        .await
        .unwrap()
        .unwrap();
    let workflow: WorkflowDefinition =
        serde_json::from_str(&serde_json::to_string(&record.definition).unwrap()).unwrap();
    let registry = frozen_fixture(
        &[TEST_PLUGIN_KEY.parse().unwrap()],
        Arc::new(AtomicU32::new(0)),
    );
    let plan = registry
        .compile_graph_v1(nebula_core::WorkflowVersionId::new(), &workflow)
        .unwrap();
    nebula_engine::PlanFlavorRevisionInstaller::new(Arc::new(
        nebula_storage::InMemoryPlanFlavorCatalog::new(&stores.execution),
    ))
    .install(&registry, &plan)
    .await
    .unwrap();
    exec_state.set_revision_ids(plan.id(), plan.worker_flavor_revision_id());
    exec_state.set_workflow_version_number(1);
    exec_state.set_budget(nebula_execution::ExecutionBudget::default());
    exec_state.set_workflow_input(input);
    let state_json = serde_json::to_value(&exec_state).expect("serialize execution state");
    use nebula_storage_port::{
        dto::{
            ContractBundleRecord, ControlMsg, MaterializedStart, NewExecution,
            PlanFlavorRevisionIds,
        },
        store::{ControlQueue, StartAcceptanceStore, StartContractIdentity},
    };
    let scope = scope();
    let bundle = nebula_execution::ExecutionContractBundle::new_graph_v1(
        nebula_core::ExecutionContractBundleId::new(),
        scope.org_id.parse().unwrap(),
        scope.workspace_id.parse().unwrap(),
        plan.id(),
        plan.plugin_set_id(),
        nebula_execution::ExecutionRevisions::new(
            plan.workflow_version_id(),
            plan.worker_flavor_revision_id(),
        ),
        [],
    );
    let record = ContractBundleRecord::v1_json(
        StartContractIdentity::new(
            bundle.bundle_id(),
            PlanFlavorRevisionIds::new(plan.id(), plan.worker_flavor_revision_id()),
        ),
        serde_json::to_vec(&bundle).unwrap(),
    )
    .unwrap();
    let execution_key = execution_id.to_string();
    let workflow_key = workflow_id.to_string();
    let command = ControlMsg {
        id: execution_id.as_bytes(),
        execution_id: execution_key.clone(),
        command: ControlCommand::Start,
        scope: scope.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    let starts = nebula_storage::inmem::InMemoryStartAcceptanceStore::new(&stores.execution);
    assert!(matches!(
        starts
            .materialize_start(&MaterializedStart::new(
                &scope,
                None,
                &execution_key,
                NewExecution::new(&workflow_key, &state_json),
                &command,
                &record
            ))
            .await
            .unwrap(),
        nebula_storage_port::store::StartMaterialization::Accepted { .. }
    ));
    // These tests drive the execution through the job queue; acknowledge the
    // admission outbox command explicitly before enqueuing their observed job.
    let queue = InMemoryControlQueue::new(&stores.execution);
    let claims = queue
        .claim_pending_for_flavor(&[0x74; 16], 1, plan.worker_flavor_revision_id())
        .await
        .unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].msg.id, command.id);
    queue.mark_completed(&claims[0].token).await.unwrap();
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
async fn worker_runtime_does_not_poll_the_technical_job_queue() {
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
        plugin_key.clone(),
        vec![plugin_key.clone()],
        None::<String>,
        0,
        test_flavor(&[TEST_PLUGIN_KEY.parse().unwrap()]).revision_id(),
    );
    queue.enqueue(&msg).await.expect("enqueue Start job");

    // Build the WorkerRuntime via the builder.
    let execution_stores = stores.execution_stores();
    let runtime = WorkerRuntimeBuilder::from_wired_engine(
        Arc::clone(&engine),
        execution_stores,
        proc16(0xBB),
    )
    .with_control_queue(Arc::new(InMemoryControlQueue::new(&stores.execution)))
    .with_turn_handoff(stores.turn_handoff())
    .with_turn_recovery(stores.turn_handoff())
    .with_resource_fanout(stores.resource_fanout(std::slice::from_ref(&plugin_key)))
    .build()
    .expect("WorkerRuntimeBuilder::build must succeed with non-empty plugin set");

    // Spawn the runtime.
    let cancel = CancellationToken::new();
    let handle = runtime.spawn(cancel.clone());

    for _ in 0..10 {
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(100)).await;
    }
    assert_eq!(
        read_status(&stores, execution_id).await,
        Some(ExecutionStatus::Created),
        "the worker runtime must not consume the standalone job-dispatch contract"
    );

    // Cancel the worker and wait for clean shutdown.
    cancel.cancel();
    handle
        .await
        .expect("worker task must not panic")
        .expect("every supervised worker component must stop cleanly");

    assert_eq!(
        echo_count.load(Ordering::SeqCst),
        0,
        "a technical job-dispatch row must not invoke the runtime action path"
    );
    let claimed = queue
        .claim_pending(
            &proc16(0xBC),
            1,
            std::slice::from_ref(&plugin_key),
            test_flavor(std::slice::from_ref(&plugin_key)).revision_id(),
        )
        .await
        .expect("technical queue remains independently claimable");
    assert_eq!(claimed.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn control_start_waits_for_worker_with_retained_exact_flavor() {
    use nebula_storage_port::store::{ControlQueue, WorkflowStore};
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
    let stores = TestStores::new();
    let workflow = save_echo_workflow_scoped(&stores, &scope()).await;
    let workflow_id = workflow.definition().id;
    stores
        .workflow
        .create(
            &scope(),
            nebula_storage_port::dto::WorkflowRecord {
                id: workflow_id.to_string(),
                scope: scope(),
                version: 1,
                slug: "exact-control".into(),
                deleted: false,
            },
        )
        .await
        .unwrap();
    let registry = Arc::new(frozen_fixture(
        &[TEST_PLUGIN_KEY.parse().unwrap()],
        Arc::new(AtomicU32::new(0)),
    ));
    nebula_engine::WorkflowActivationService::new(
        stores.workflow.clone(),
        stores.versions.clone(),
        registry.clone(),
        nebula_engine::PlanFlavorRevisionInstaller::new(Arc::new(
            stores.execution.plan_flavor_catalog(),
        )),
        Arc::new(nebula_core::accessor::SystemClock),
    )
    .activate(&scope(), workflow_id, 1, workflow.definition().clone())
    .await
    .unwrap();
    let starts = WorkflowStartService::new(
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
        nebula_execution::ExecutionBudget::default(),
    )
    .unwrap();
    let receipt = starts
        .start(
            &scope(),
            workflow_id,
            Some(serde_json::json!({"value":17})),
            Some("exact-control"),
            None,
        )
        .await
        .unwrap();
    let execution_id = receipt.state().execution_id;
    let before = stores
        .execution
        .get(&scope(), &execution_id.to_string())
        .await
        .unwrap()
        .unwrap();
    let queue = Arc::new(InMemoryControlQueue::new(&stores.execution));
    let (wrong_engine, wrong_count) = make_engine_with_plugins(
        &stores,
        true,
        &[
            TEST_PLUGIN_KEY.parse().unwrap(),
            "test.extra".parse().unwrap(),
        ],
    )
    .await;
    let wrong = WorkerRuntimeBuilder::from_wired_engine(
        wrong_engine,
        stores.execution_stores(),
        proc16(0xD1),
    )
    .with_control_queue(queue.clone())
    .with_turn_handoff(stores.turn_handoff())
    .with_turn_recovery(stores.turn_handoff())
    .with_resource_fanout(stores.resource_fanout(&[
        TEST_PLUGIN_KEY.parse().unwrap(),
        "test.extra".parse().unwrap(),
    ]))
    .build()
    .unwrap();
    let cancel = CancellationToken::new();
    let handle = wrong.spawn(cancel.clone());
    for _ in 0..20 {
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(100)).await;
    }
    cancel.cancel();
    handle.await.unwrap().unwrap();
    assert_eq!(wrong_count.load(Ordering::SeqCst), 0);
    assert_eq!(
        stores
            .execution
            .get(&scope(), &execution_id.to_string())
            .await
            .unwrap()
            .unwrap(),
        before
    );
    let (engine, count) = make_engine(&stores).await;
    let flavor = engine.worker_flavor_context().unwrap().revision_id();
    let claims = queue
        .claim_pending_for_flavor(&proc16(0xD2), 1, flavor)
        .await
        .unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(
        claims[0].token.generation().get(),
        1,
        "wrong worker must never claim the Start command"
    );
    // Reclaim the deliberate observation claim, then exercise the matching worker.
    tokio::time::advance(Duration::from_secs(2)).await;
    queue
        .reclaim_stuck(Duration::from_secs(1), 10)
        .await
        .unwrap();
    let correct =
        WorkerRuntimeBuilder::from_wired_engine(engine, stores.execution_stores(), proc16(0xD3))
            .with_control_queue(queue)
            .with_turn_handoff(stores.turn_handoff())
            .with_turn_recovery(stores.turn_handoff())
            .with_resource_fanout(stores.resource_fanout(&[TEST_PLUGIN_KEY.parse().unwrap()]))
            .build()
            .unwrap();
    let cancel = CancellationToken::new();
    let handle = correct.spawn(cancel.clone());
    let mut completed = false;
    for _ in 0..200 {
        tokio::task::yield_now().await;
        let record = stores
            .execution
            .get(&scope(), &execution_id.to_string())
            .await
            .unwrap()
            .unwrap();
        let state: ExecutionState =
            serde_json::from_slice(&serde_json::to_vec(&record.state).unwrap()).unwrap();
        if state.status == ExecutionStatus::Completed {
            completed = true;
            break;
        }
        tokio::time::advance(Duration::from_millis(100)).await;
    }
    cancel.cancel();
    handle.await.unwrap().unwrap();
    assert!(
        completed,
        "matching worker must consume the retained Start command"
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn builder_rejects_an_engine_without_exact_runtime_configuration() {
    let stores = TestStores::new();
    let (engine, _) = make_engine_with_exact_configuration(&stores, false).await;
    let result =
        WorkerRuntimeBuilder::from_wired_engine(engine, stores.execution_stores(), proc16(0x09))
            .with_control_queue(Arc::new(InMemoryControlQueue::new(&stores.execution)))
            .with_turn_handoff(stores.turn_handoff())
            .with_turn_recovery(stores.turn_handoff())
            .with_resource_fanout(stores.resource_fanout(&[TEST_PLUGIN_KEY.parse().unwrap()]))
            .build();
    let error = result.expect_err("an independently advertised flavor cannot authorize an engine");
    assert_eq!(error.to_string(), "exact runtime configuration is required");
}

#[tokio::test]
async fn builder_requires_resource_fanout() {
    let stores = TestStores::new();
    let (engine, _) = make_engine(&stores).await;
    let result =
        WorkerRuntimeBuilder::from_wired_engine(engine, stores.execution_stores(), proc16(0x08))
            .with_control_queue(Arc::new(InMemoryControlQueue::new(&stores.execution)))
            .with_turn_handoff(stores.turn_handoff())
            .with_turn_recovery(stores.turn_handoff())
            .build();

    assert!(
        matches!(
            result,
            Err(nebula_worker::WorkerBuildError::NoResourceFanout)
        ),
        "missing resource fanout must be rejected; got {result:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn resource_fanout_failure_stops_worker_with_typed_error() {
    let stores = TestStores::new();
    let (engine, _) = make_engine(&stores).await;
    let runtime_store = Arc::new(nebula_storage::inmem::InMemoryResourceRuntime::new());
    let resource_fanout = stores.resource_fanout_with_recovery(
        &[TEST_PLUGIN_KEY.parse().unwrap()],
        Arc::new(FailingResourceRecovery),
        runtime_store,
        1,
    );
    let runtime =
        WorkerRuntimeBuilder::from_wired_engine(engine, stores.execution_stores(), proc16(0x07))
            .with_control_queue(Arc::new(InMemoryControlQueue::new(&stores.execution)))
            .with_turn_handoff(stores.turn_handoff())
            .with_turn_recovery(stores.turn_handoff())
            .with_resource_fanout(resource_fanout)
            .build()
            .expect("fully wired worker runtime must build");

    let error = runtime
        .run(CancellationToken::new())
        .await
        .expect_err("bounded resource fanout failure must stop the worker");
    assert!(
        matches!(
            error,
            nebula_worker::WorkerRuntimeError::ResourceFanout {
                attempts: 1,
                error_code: "RESOURCE_FANOUT:CLAIM_DELIVERIES",
                ..
            }
        ),
        "worker must preserve the bounded fanout error classification; got {error:?}"
    );
}

/// A zero timer-scan interval is rejected rather than deferred to a panic.
///
/// `tokio::time::interval` panics on a zero period, so accepting it would let a
/// plausible-looking configuration kill the scanner the moment it starts —
/// inside a supervised task, where the only symptom is that parked executions
/// quietly stop waking.
#[tokio::test]
async fn builder_rejects_a_zero_timer_scan_interval() {
    use nebula_worker::WorkerBuildError;
    let stores = TestStores::new();
    let (engine, _) = make_engine(&stores).await;
    let result =
        WorkerRuntimeBuilder::from_wired_engine(engine, stores.execution_stores(), proc16(0x01))
            .with_control_queue(Arc::new(InMemoryControlQueue::new(&stores.execution)))
            .with_turn_handoff(stores.turn_handoff())
            .with_turn_recovery(stores.turn_handoff())
            .with_resource_fanout(stores.resource_fanout(&[TEST_PLUGIN_KEY.parse().unwrap()]))
            .with_timer_scan_interval(Duration::ZERO)
            .build();

    assert!(
        matches!(result, Err(WorkerBuildError::ZeroTimerScanInterval)),
        "a zero timer scan interval must be rejected at build time; got {result:?}"
    );
}

#[test]
fn empty_plugins_cannot_construct_a_worker_flavor() {
    let registry = nebula_plugin::PluginRegistry::new();
    let result = registry.freeze(
        nebula_core::ArtifactSetDigest::from_bytes([0x31; 32]),
        "1.0.0".parse().unwrap(),
    );
    assert!(
        matches!(
            result,
            Err(nebula_plugin::RegistryFreezeError::EmptyRegistry)
        ),
        "empty registry must fail before a worker context can be constructed"
    );
}

// ── Reclaim + re-run tests ────────────────────────────────────────────────────

/// A reclaimed technical job-dispatch row remains outside `WorkerRuntime`.
///
/// Queue conformance retains its own reclaim behavior while the first-party
/// worker graph proves it does not attach a second competing command source.
#[tokio::test(start_paused = true)]
async fn reclaimed_job_dispatch_row_remains_outside_worker_runtime() {
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
        plugin_key.clone(),
        vec![plugin_key.clone()],
        None::<String>,
        0,
        test_flavor(&[TEST_PLUGIN_KEY.parse().unwrap()]).revision_id(),
    );
    queue.enqueue(&msg).await.expect("enqueue Start job");

    // Step 2: claim as proc_a (simulates a worker that crashed before dispatching).
    let proc_a = proc16(0xAA);
    let claimed = queue
        .claim_pending(
            &proc_a,
            1,
            std::slice::from_ref(&plugin_key),
            test_flavor(std::slice::from_ref(&plugin_key)).revision_id(),
        )
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

    // Start the worker runtime and prove it does not claim the standalone row.
    let proc_b = proc16(0xBB);
    let runtime = WorkerRuntimeBuilder::from_wired_engine(
        Arc::clone(&engine),
        stores.execution_stores(),
        proc_b,
    )
    .with_control_queue(Arc::new(InMemoryControlQueue::new(&stores.execution)))
    .with_turn_handoff(stores.turn_handoff())
    .with_turn_recovery(stores.turn_handoff())
    .with_resource_fanout(stores.resource_fanout(&[TEST_PLUGIN_KEY.parse().unwrap()]))
    .build()
    .expect("WorkerRuntimeBuilder::build must succeed");

    let cancel = CancellationToken::new();
    let handle = runtime.spawn(cancel.clone());

    for _ in 0..10 {
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(100)).await;
    }
    assert_eq!(
        read_status(&stores, execution_id).await,
        Some(ExecutionStatus::Created),
        "worker runtime must not consume reclaimed technical job-dispatch rows"
    );

    cancel.cancel();
    handle
        .await
        .expect("worker task must not panic")
        .expect("every supervised worker component must stop cleanly");

    assert_eq!(
        echo_count.load(Ordering::SeqCst),
        0,
        "job-dispatch reclaim must not enter the worker action path"
    );
    let reclaimed = queue
        .claim_pending(
            &proc_b,
            1,
            std::slice::from_ref(&plugin_key),
            test_flavor(std::slice::from_ref(&plugin_key)).revision_id(),
        )
        .await
        .expect("reclaimed technical row remains claimable");
    assert_eq!(reclaimed.len(), 1);
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
    use nebula_orchestrator::{DispatchedTurn, ExecutionSink};

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

    // Mint the turn the way the durable handoff does: lease acquired under
    // this test's driver identity, with its fence threaded into the sink.
    let fence = stores
        .execution
        .acquire_lease(
            &scope(),
            &execution_id.to_string(),
            "test-driver",
            Duration::from_secs(30),
        )
        .await
        .expect("acquire lease for the test turn")
        .expect("no live lease yet");

    let plugin_key: PluginKey = TEST_PLUGIN_KEY.parse().unwrap();
    let job_id = [0xCCu8; 16];
    let msg = JobDispatchMsg::new(
        job_id,
        execution_id.to_string(),
        ControlCommand::Start,
        scope(),
        serde_json::json!({}),
        None::<String>,
        plugin_key.clone(),
        vec![plugin_key],
        None::<String>,
        0,
        test_flavor(&[TEST_PLUGIN_KEY.parse().unwrap()]).revision_id(),
    );

    // First dispatch: drives Created → Completed under the handoff fence;
    // handler runs once.
    let turn = DispatchedTurn { msg: &msg, fence };
    let result1 = sink.dispatch(&turn).await;
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

    // Second dispatch: execution is already Completed; the idempotency guard
    // short-circuits before the fence is ever adopted, so the same (now
    // released) fence value is fine.
    let redelivery = DispatchedTurn { msg: &msg, fence };
    let result2 = sink.dispatch(&redelivery).await;
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
        plugin_key.clone(),
        vec![plugin_key.clone()],
        None::<String>,
        0,
        test_flavor(&[TEST_PLUGIN_KEY.parse().unwrap()]).revision_id(),
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
            .claim_pending(&crasher, 1, &tags, test_flavor(&tags).revision_id())
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
        .claim_pending(&proc16(0xEE), 8, &tags, test_flavor(&tags).revision_id())
        .await
        .expect("probe claim");
    assert!(
        leftover.is_empty(),
        "exhausted (Failed) job-dispatch row must not be returned by claim_pending"
    );
}

fn test_flavor(plugins: &[PluginKey]) -> nebula_plugin::WorkerFlavorContext {
    nebula_plugin::WorkerFlavorContext::from_registry(&frozen_fixture(
        plugins,
        Arc::new(AtomicU32::new(0)),
    ))
}

fn frozen_fixture(
    plugins: &[PluginKey],
    count: Arc<AtomicU32>,
) -> nebula_plugin::FrozenPluginRegistry {
    struct FixturePlugin(nebula_plugin::PluginManifest, Arc<AtomicU32>);
    impl std::fmt::Debug for FixturePlugin {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("FixturePlugin")
                .field("manifest", &self.0)
                .finish_non_exhaustive()
        }
    }
    impl nebula_plugin::Plugin for FixturePlugin {
        fn manifest(&self) -> &nebula_plugin::PluginManifest {
            &self.0
        }
        fn actions(&self) -> Vec<Arc<dyn nebula_action::ActionFactory>> {
            if self.0.key().as_str() == TEST_PLUGIN_KEY {
                vec![Arc::new(
                    nebula_action::factory::InstanceFactory::new(
                        EchoHandler::metadata(),
                        EchoHandler {
                            count: self.1.clone(),
                        },
                    )
                    .expect("valid test catalog definition"),
                )]
            } else {
                Vec::new()
            }
        }
    }
    let mut registry = nebula_plugin::PluginRegistry::new();
    for key in plugins {
        let plugin = FixturePlugin(
            nebula_plugin::PluginManifest::builder(key.as_str(), key.as_str())
                .build()
                .unwrap(),
            count.clone(),
        );
        registry
            .register(Arc::new(
                nebula_plugin::ResolvedPlugin::from(plugin).unwrap(),
            ))
            .unwrap();
    }
    registry
        .freeze(
            nebula_core::ArtifactSetDigest::from_bytes([0x31; 32]),
            "1.0.0".parse().unwrap(),
        )
        .unwrap()
}
