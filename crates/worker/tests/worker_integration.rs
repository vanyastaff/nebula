//! Integration tests for `nebula-worker` durable control processing.
//!
//! Verifies that exact-flavor control starts reach the engine.
//!
//! ## Test plan
//!
//! `control_start_waits_for_worker_with_retained_exact_flavor` covers the
//! production command path.
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
use nebula_core::{Dependencies, PluginKey, action_key, node_key};
use nebula_engine::{
    ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessRunner, ResourceFanoutCoordinator,
    WorkflowEngine, WorkflowStartService,
};
use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_metrics::MetricsRegistry;
use nebula_storage::{InMemoryControlQueue, InMemoryExecutionStore, InMemoryWorkflowVersionStore};
use nebula_storage_port::{
    Scope, StorageError,
    dto::{
        ClaimResourceRuntimeWorkRequest, ResourceLeaseHolder, ResourceLeaseTtl, ResourcePageSize,
        ScopedClaimedResourceDelivery, ScopedClaimedResourceHandoff,
    },
    store::{ControlQueue, ExecutionStore, ResourceRuntimeRecovery, WorkflowVersionStore},
};
use nebula_worker::WorkerRuntimeBuilder;
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, TriggerBinding, ValidatedWorkflow, Version,
    WorkflowConfig, WorkflowDefinition,
};
use tokio_util::sync::CancellationToken;

// ── Plugin key used across all test helpers ───────────────────────────────────

const TEST_PLUGIN_KEY: &str = "test";

mod resource_maintenance;

#[tokio::test(start_paused = true)]
async fn worker_resource_reconciliation_missing_inputs_refuses_startup() {
    let stores = TestStores::new();
    let (engine, echo_count) = make_engine(&stores).await;
    let engine = Arc::new(
        Arc::try_unwrap(engine)
            .unwrap_or_else(|_| panic!("fixture owns its engine"))
            .with_resource_manager(Arc::new(nebula_engine::resource::Manager::new())),
    );
    let queue = Arc::new(ControlPollingWitness::new(&stores));
    let runtime =
        WorkerRuntimeBuilder::from_wired_engine(engine, stores.execution_stores(), proc16(0x91))
            .with_control_queue(queue.clone())
            .with_turn_handoff(stores.turn_handoff())
            .with_turn_recovery(stores.turn_handoff())
            .with_resource_fanout(stores.resource_fanout(&[TEST_PLUGIN_KEY.parse().unwrap()]))
            .build()
            .expect("durable worker ports are wired");

    let outcome = tokio::time::timeout(
        Duration::from_secs(1),
        runtime.run(CancellationToken::new()),
    )
    .await;
    assert!(
        outcome.is_ok(),
        "missing reconciliation inputs must refuse startup instead of polling indefinitely"
    );
    let error = outcome
        .unwrap()
        .expect_err("a manager with no reconciliation input is a startup error");
    let nebula_worker::WorkerRuntimeError::ResourceReconciliationStartup(
        nebula_engine::ResourceReconciliationStartupError::Driver(_),
    ) = error
    else {
        panic!("missing driver inputs must preserve the typed startup failure");
    };
    assert_eq!(echo_count.load(Ordering::SeqCst), 0);
    assert_eq!(
        queue.poll_calls.load(Ordering::SeqCst),
        0,
        "startup rejection must happen before control consumption"
    );
}

/// Decorates the existing queue to witness the first production control poll.
#[derive(Debug)]
struct ControlPollingWitness {
    inner: InMemoryControlQueue,
    poll_calls: AtomicU32,
    first_poll: tokio::sync::Notify,
}

impl ControlPollingWitness {
    fn new(stores: &TestStores) -> Self {
        Self {
            inner: InMemoryControlQueue::new(&stores.execution),
            poll_calls: AtomicU32::new(0),
            first_poll: tokio::sync::Notify::new(),
        }
    }

    fn record_poll(&self) {
        if self.poll_calls.fetch_add(1, Ordering::SeqCst) == 0 {
            self.first_poll.notify_one();
        }
    }
}

#[async_trait::async_trait]
impl ControlQueue for ControlPollingWitness {
    async fn enqueue(
        &self,
        msg: &nebula_storage_port::dto::ControlMsg,
    ) -> Result<(), StorageError> {
        self.inner.enqueue(msg).await
    }

    async fn claim_pending(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
    ) -> Result<Vec<nebula_storage_port::store::ControlClaim>, StorageError> {
        self.record_poll();
        self.inner.claim_pending(processor, batch_size).await
    }

    async fn claim_pending_for_flavor(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
        flavor: nebula_core::WorkerFlavorRevisionId,
    ) -> Result<Vec<nebula_storage_port::store::ControlClaim>, StorageError> {
        self.record_poll();
        self.inner
            .claim_pending_for_flavor(processor, batch_size, flavor)
            .await
    }

    async fn claim_pending_interrupts_for_flavor(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
        flavor: nebula_core::WorkerFlavorRevisionId,
    ) -> Result<Vec<nebula_storage_port::store::ControlClaim>, StorageError> {
        self.inner
            .claim_pending_interrupts_for_flavor(processor, batch_size, flavor)
            .await
    }

    async fn mark_completed(
        &self,
        claim: &nebula_storage_port::store::ControlClaimToken,
    ) -> Result<(), StorageError> {
        self.inner.mark_completed(claim).await
    }

    async fn mark_failed(
        &self,
        claim: &nebula_storage_port::store::ControlClaimToken,
        error: &str,
    ) -> Result<(), StorageError> {
        self.inner.mark_failed(claim, error).await
    }

    async fn release_claim(
        &self,
        claim: &nebula_storage_port::store::ControlClaimToken,
    ) -> Result<(), StorageError> {
        self.inner.release_claim(claim).await
    }

    async fn reclaim_stuck(
        &self,
        age: Duration,
        max: u32,
    ) -> Result<nebula_storage_port::store::ReclaimOutcome, StorageError> {
        self.inner.reclaim_stuck(age, max).await
    }

    async fn cleanup(&self, retention: Duration) -> Result<u64, StorageError> {
        self.inner.cleanup(retention).await
    }
}

/// No rows are bound in this fixture; resolution is never used as a startup fallback.
struct EmptyDeploymentResolver;

impl nebula_credential::CredentialSlotResolver for EmptyDeploymentResolver {
    fn resolve_slot<'a>(
        &'a self,
        _scope: &'a nebula_credential::TenantScope,
        _credential_id: nebula_credential::CredentialId,
        _expected_key: nebula_core::CredentialKey,
        _capabilities: nebula_credential::Capabilities,
        _cancel: CancellationToken,
    ) -> std::pin::Pin<
        Box<
            dyn Future<
                    Output = Result<
                        nebula_credential::ErasedCredentialGuard,
                        nebula_credential::CredentialSlotResolveError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(std::future::pending())
    }

    fn as_availability_observer(
        &self,
    ) -> Option<&dyn nebula_credential::CredentialAvailabilityObserver> {
        Some(self)
    }

    fn into_availability_observer(
        self: Arc<Self>,
    ) -> Option<Arc<dyn nebula_credential::CredentialAvailabilityObserver>> {
        Some(self)
    }
}

impl nebula_credential::CredentialAvailabilityObserver for EmptyDeploymentResolver {
    fn observe_availability<'a>(
        &'a self,
        _scope: &'a nebula_credential::TenantScope,
        _credential_id: nebula_credential::CredentialId,
        _expected_key: nebula_core::CredentialKey,
        _cancel: CancellationToken,
    ) -> std::pin::Pin<
        Box<
            dyn Future<
                    Output = Result<
                        nebula_credential::CredentialAvailabilityObservation,
                        nebula_credential::CredentialObserveError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async { Err(nebula_credential::CredentialObserveError::Absent) })
    }
}

async fn reconciliation_engine(stores: &TestStores) -> Arc<WorkflowEngine> {
    let (engine, _) = make_engine(stores).await;
    Arc::new(
        Arc::try_unwrap(engine)
            .unwrap_or_else(|_| panic!("fixture owns its engine"))
            .with_resource_manager(Arc::new(nebula_engine::resource::Manager::new()))
            .with_credential_resolver(Arc::new(EmptyDeploymentResolver)),
    )
}

fn reconciliation_runtime(
    stores: &TestStores,
    engine: Arc<WorkflowEngine>,
    queue: Arc<ControlPollingWitness>,
) -> nebula_worker::WorkerRuntime {
    WorkerRuntimeBuilder::from_wired_engine(engine, stores.execution_stores(), proc16(0x92))
        .with_control_queue(queue)
        .with_turn_handoff(stores.turn_handoff())
        .with_turn_recovery(stores.turn_handoff())
        .with_resource_fanout(stores.resource_fanout(&[TEST_PLUGIN_KEY.parse().unwrap()]))
        .build()
        .expect("all durable worker ports are wired")
}

#[tokio::test]
async fn worker_start_rejects_reconciliation_failure_before_acknowledgement() {
    let stores = TestStores::new();
    let engine = reconciliation_engine(&stores).await;
    let mut prior = engine
        .spawn_resource_rotation_fanout(None, None)
        .unwrap()
        .unwrap();
    let queue = Arc::new(ControlPollingWitness::new(&stores));
    let runtime = reconciliation_runtime(&stores, engine, queue.clone());
    let shutdown = CancellationToken::new();
    let result = runtime.start(shutdown.clone()).await;
    prior.abort();
    let _ = prior.wait().await;
    assert!(matches!(
        result,
        Err(nebula_worker::WorkerRuntimeError::ResourceReconciliationStartup(_))
    ));
    assert!(shutdown.is_cancelled());
    assert_eq!(queue.poll_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn worker_start_acknowledges_installed_reconciliation_and_joins_on_shutdown() {
    let stores = TestStores::new();
    let engine = reconciliation_engine(&stores).await;
    let queue = Arc::new(ControlPollingWitness::new(&stores));
    let runtime = reconciliation_runtime(&stores, engine.clone(), queue);
    let shutdown = CancellationToken::new();
    let running = runtime.start(shutdown.clone()).await.unwrap();
    assert!(matches!(
        engine.spawn_resource_rotation_fanout(None, None),
        Err(nebula_engine::ResourceReconciliationStartupError::AlreadyRunning)
    ));
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(2), running)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn worker_start_rejects_already_cancelled_startup() {
    let stores = TestStores::new();
    let engine = reconciliation_engine(&stores).await;
    let runtime = reconciliation_runtime(
        &stores,
        engine,
        Arc::new(ControlPollingWitness::new(&stores)),
    );
    let shutdown = CancellationToken::new();
    shutdown.cancel();
    assert!(matches!(
        runtime.start(shutdown).await,
        Err(nebula_worker::WorkerRuntimeError::StartupCancelled)
    ));
}

#[tokio::test]
async fn worker_resource_reconciliation_rejects_an_already_running_driver_before_controls() {
    let stores = TestStores::new();
    let engine = reconciliation_engine(&stores).await;
    let mut prior_driver = engine
        .spawn_resource_rotation_fanout(None, None)
        .expect("resolver-backed driver starts")
        .expect("resource manager owns a driver");
    let queue = Arc::new(ControlPollingWitness::new(&stores));
    let runtime = reconciliation_runtime(&stores, engine, queue.clone());
    let outcome = tokio::time::timeout(
        Duration::from_secs(1),
        runtime.run(CancellationToken::new()),
    )
    .await;
    prior_driver.abort();
    assert!(
        prior_driver
            .wait()
            .await
            .expect_err("aborted prior driver returns cancellation")
            .is_cancelled()
    );

    let error = outcome
        .expect("duplicate driver must refuse startup promptly")
        .expect_err("a worker cannot adopt another driver's authority");
    assert!(matches!(
        error,
        nebula_worker::WorkerRuntimeError::ResourceReconciliationStartup(
            nebula_engine::ResourceReconciliationStartupError::AlreadyRunning
        )
    ));
    assert_eq!(queue.poll_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn worker_resource_reconciliation_is_live_before_controls_and_joined_on_shutdown() {
    let stores = TestStores::new();
    let engine = reconciliation_engine(&stores).await;
    let queue = Arc::new(ControlPollingWitness::new(&stores));
    let runtime = reconciliation_runtime(&stores, engine.clone(), queue.clone());
    let shutdown = CancellationToken::new();
    let handle = runtime.spawn(shutdown.clone());
    let started = tokio::time::timeout(Duration::from_secs(1), queue.first_poll.notified()).await;
    let live_probe = started
        .is_ok()
        .then(|| engine.spawn_resource_rotation_fanout(None, None));
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("worker joins reconciliation and all siblings")
        .expect("worker task joins")
        .expect("normal shutdown is successful");
    started.expect("worker reaches the production control polling seam");
    assert!(
        matches!(
            live_probe,
            Some(Err(
                nebula_engine::ResourceReconciliationStartupError::AlreadyRunning
            ))
        ),
        "the worker must own reconciliation before consuming controls"
    );
    // Empty-index replacement proves the engine's generation was released; it
    // does not attempt resource work on the manager already shut down by the worker.
    let mut replacement = engine
        .spawn_resource_rotation_fanout(None, None)
        .expect("joined shutdown releases the engine generation")
        .expect("manager remains configured");
    replacement.abort();
    assert!(
        replacement
            .wait()
            .await
            .expect_err("aborted replacement returns cancellation")
            .is_cancelled()
    );
}

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

#[derive(Debug)]
struct CorruptResourceRecovery;

#[async_trait::async_trait]
impl ResourceRuntimeRecovery for CorruptResourceRecovery {
    async fn claim_deliveries_globally(
        &self,
        _request: ClaimResourceRuntimeWorkRequest,
    ) -> Result<Vec<ScopedClaimedResourceDelivery>, StorageError> {
        Err(StorageError::Corrupt("private-resource-value".to_owned()))
    }

    async fn claim_handoffs_globally(
        &self,
        _request: ClaimResourceRuntimeWorkRequest,
    ) -> Result<Vec<ScopedClaimedResourceHandoff>, StorageError> {
        Err(StorageError::Corrupt("private-resource-value".to_owned()))
    }
}

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
            .with_effect_contract(nebula_action::effect::ActionEffectContract::ReadOnly),
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
    // Revision 1 of the row is what activation later advances.
    nebula_storage_port::store::WorkflowStore::create(
        stores.workflow.as_ref(),
        scope,
        nebula_storage_port::dto::WorkflowRecord {
            id: validated.definition().id.to_string(),
            scope: scope.clone(),
            version: 1,
            slug: "exact-control".into(),
        },
    )
    .await
    .expect("create workflow row");
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

// ── Acceptance test ───────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn control_start_waits_for_worker_with_retained_exact_flavor() {
    use nebula_storage_port::store::ControlQueue;
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
            nebula_worker::WorkerRuntimeError::ResourceFanout(source)
                if source.attempts() == 1
                    && source.error_code() == "RESOURCE_FANOUT:CLAIM_DELIVERIES"
                    && source.failure_kind()
                        == nebula_engine::ResourceFanoutFailureKind::CommitUnknown
        ),
        "worker must preserve the bounded fanout error classification; got {error:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn resource_fanout_corruption_stops_worker_before_retry_exhaustion() {
    let stores = TestStores::new();
    let (engine, _) = make_engine(&stores).await;
    let fanout = stores.resource_fanout_with_recovery(
        &[TEST_PLUGIN_KEY.parse().unwrap()],
        Arc::new(CorruptResourceRecovery),
        Arc::new(nebula_storage::inmem::InMemoryResourceRuntime::new()),
        3,
    );
    let runtime =
        WorkerRuntimeBuilder::from_wired_engine(engine, stores.execution_stores(), proc16(0x08))
            .with_control_queue(Arc::new(InMemoryControlQueue::new(&stores.execution)))
            .with_turn_handoff(stores.turn_handoff())
            .with_turn_recovery(stores.turn_handoff())
            .with_resource_fanout(fanout)
            .build()
            .expect("fully wired worker runtime must build");

    let error = runtime
        .run(CancellationToken::new())
        .await
        .expect_err("permanent fanout failure must stop the worker");
    assert!(matches!(
        error,
        nebula_worker::WorkerRuntimeError::ResourceFanout(source)
            if source.attempts() == 1
                && source.error_code() == "RESOURCE_FANOUT:CLAIM_DELIVERIES"
                && source.failure_kind()
                    == nebula_engine::ResourceFanoutFailureKind::StoredDataInvalid
    ));
    assert!(std::error::Error::source(&error).is_some());
    assert!(!error.to_string().contains("private-resource-value"));
    assert!(!format!("{error:?}").contains("private-resource-value"));
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
