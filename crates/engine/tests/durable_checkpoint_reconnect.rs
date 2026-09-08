//! Real cold and warm durable turns with a newly constructed engine per turn.

use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use nebula_action::{
    Action, ActionMetadata, ActionOutput, ActionResult, StatelessAction,
    effect::ActionEffectContract, result::WaitCondition,
};
use nebula_core::{Dependencies, ExecutionId, WorkflowId, action_key, node_key};
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, ControlDispatch, DataPassingPolicy,
    EngineControlDispatch, ExecutionStores, InProcessRunner, PlanFlavorRevisionInstaller,
    PlanFlavorRevisionLoader, WorkflowEngine,
};
use nebula_execution::{
    ExecutionBudget, ExecutionContractBundle, ExecutionRevisions, ExecutionState,
};
use nebula_metrics::MetricsRegistry;
use nebula_plugin::{FrozenPluginRegistry, Plugin, PluginManifest, PluginRegistry, ResolvedPlugin};
use nebula_storage_port::{
    Scope,
    dto::{ContractBundleRecord, ControlCommand, ControlMsg, MaterializedStart, NewExecution},
    store::{
        ControlQueue, PlanFlavorCatalog, PlanFlavorCatalogWriter, StartAcceptanceStore,
        StartContractIdentity, StartMaterialization,
    },
};
use nebula_workflow::{Connection, NodeDefinition, WorkflowDefinition};

#[path = "support/postgres_schema.rs"]
mod postgres_schema;
#[path = "durable_checkpoint_reconnect/recovery.rs"]
mod recovery;

struct Echo(Arc<AtomicU32>, Option<Arc<ActionBarrier>>);
#[derive(Default)]
struct ActionBarrier {
    block_at_call: u32,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
struct Park;

impl Action for Echo {
    type Input = serde_json::Value;
    type Output = serde_json::Value;
    fn metadata() -> ActionMetadata {
        ActionMetadata::new(action_key!("checkpoint.echo"), "Echo", "pure payload echo")
            .with_effect_contract(ActionEffectContract::NoExternalEffects)
    }
    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}
impl StatelessAction for Echo {
    async fn execute(
        &self,
        input: serde_json::Value,
        _context: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<serde_json::Value>, nebula_action::ActionError> {
        let call = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(barrier) = &self.1
            && (barrier.block_at_call == 0 || barrier.block_at_call == call)
        {
            barrier.entered.notify_one();
            barrier.release.notified().await;
        }
        Ok(ActionResult::success(input))
    }
}
impl Action for Park {
    type Input = serde_json::Value;
    type Output = serde_json::Value;
    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("checkpoint.park"),
            "Park",
            "durable signal wait",
        )
        .with_effect_contract(ActionEffectContract::NoExternalEffects)
    }
    fn dependencies() -> &'static Dependencies {
        Echo::dependencies()
    }
}
impl StatelessAction for Park {
    async fn execute(
        &self,
        input: serde_json::Value,
        _context: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<serde_json::Value>, nebula_action::ActionError> {
        Ok(ActionResult::Wait {
            condition: WaitCondition::Execution {
                execution_id: ExecutionId::from_bytes([0x44; 16]),
            },
            timeout: None,
            partial_output: Some(ActionOutput::Value(input)),
        })
    }
}

struct FixturePlugin {
    manifest: PluginManifest,
    actions: Vec<Arc<dyn nebula_action::ActionFactory>>,
}
impl std::fmt::Debug for FixturePlugin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CheckpointFixture")
            .finish_non_exhaustive()
    }
}
impl Plugin for FixturePlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
    fn actions(&self) -> Vec<Arc<dyn nebula_action::ActionFactory>> {
        self.actions.clone()
    }
}
fn frozen_registry(count: &Arc<AtomicU32>) -> (Arc<ActionRegistry>, Arc<FrozenPluginRegistry>) {
    frozen_registry_with_barrier(count, None)
}
fn frozen_registry_with_barrier(
    count: &Arc<AtomicU32>,
    barrier: Option<Arc<ActionBarrier>>,
) -> (Arc<ActionRegistry>, Arc<FrozenPluginRegistry>) {
    let actions = Arc::new(ActionRegistry::new());
    actions.register_stateless_instance(Echo::metadata(), Echo(Arc::clone(count), barrier));
    actions.register_stateless_instance(Park::metadata(), Park);
    let plugin = FixturePlugin {
        manifest: PluginManifest::builder("checkpoint", "Checkpoint")
            .build()
            .unwrap(),
        actions: [
            action_key!("checkpoint.echo"),
            action_key!("checkpoint.park"),
        ]
        .iter()
        .map(|key| actions.get_factory(key).unwrap().1)
        .collect(),
    };
    let mut plugins = PluginRegistry::new();
    plugins
        .register(Arc::new(ResolvedPlugin::from(plugin).unwrap()))
        .unwrap();
    let frozen = plugins
        .freeze(
            nebula_core::ArtifactSetDigest::from_bytes([0x74; 32]),
            "1.0.0".parse().unwrap(),
        )
        .unwrap();
    (actions, Arc::new(frozen))
}

struct Ports {
    handoff: Arc<dyn nebula_storage_port::ExecutionTurnHandoff>,
    recovery: Arc<dyn nebula_storage_port::TurnRecovery>,
    stores: ExecutionStores,
    bundles: Arc<dyn StartAcceptanceStore>,
    queue: Arc<dyn ControlQueue>,
    catalog: Arc<dyn PlanFlavorCatalog>,
    writer: Arc<dyn PlanFlavorCatalogWriter>,
}
fn in_memory(core: &Arc<nebula_storage::InMemoryExecutionStore>) -> Ports {
    let catalog = Arc::new(core.plan_flavor_catalog());
    let turn_store = Arc::new(nebula_storage::inmem::InMemoryTurnHandoff::new(core));
    Ports {
        handoff: turn_store.clone(),
        recovery: turn_store,
        stores: ExecutionStores {
            execution: core.clone(),
            journal: Arc::new(nebula_storage::InMemoryJournalReader::new(core)),
            node_results: Arc::new(nebula_storage::InMemoryNodeResultStore::new()),
            checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new()),
            idempotency: Arc::new(nebula_storage::InMemoryIdempotencyGuard::new()),
            resume_tokens: Arc::new(core.resume_token_store()),
            operation_ledger: Arc::new(nebula_storage::inmem::InMemoryOperationLedger::new(core)),
        },
        bundles: Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
            core,
        )),
        queue: Arc::new(nebula_storage::InMemoryControlQueue::new(core)),
        catalog: catalog.clone(),
        writer: catalog,
    }
}
fn sqlite(pool: sqlx::SqlitePool) -> Ports {
    use nebula_storage::sqlite::*;
    let catalog = Arc::new(SqlitePlanFlavorCatalog::new(
        pool.clone(),
        &MetricsRegistry::new(),
    ));
    let turn_store = Arc::new(SqliteTurnHandoff::new(pool.clone()));
    Ports {
        handoff: turn_store.clone(),
        recovery: turn_store,
        stores: ExecutionStores {
            execution: Arc::new(SqliteExecutionStore::new(pool.clone())),
            journal: Arc::new(SqliteJournalReader::new(pool.clone())),
            node_results: Arc::new(nebula_storage::InMemoryNodeResultStore::new()),
            checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new()),
            idempotency: Arc::new(SqliteIdempotencyGuard::new(pool.clone())),
            resume_tokens: Arc::new(SqliteResumeTokenStore::new(pool.clone())),
            operation_ledger: Arc::new(SqliteOperationLedger::new(pool.clone())),
        },
        bundles: Arc::new(SqliteStartAcceptanceStore::new(pool.clone())),
        queue: Arc::new(SqliteControlQueue::new(pool)),
        catalog: catalog.clone(),
        writer: catalog,
    }
}
fn postgres(pool: sqlx::PgPool) -> Ports {
    use nebula_storage::postgres::*;
    let catalog = Arc::new(PgPlanFlavorCatalog::new(
        pool.clone(),
        &MetricsRegistry::new(),
    ));
    let turn_store = Arc::new(PgTurnHandoff::new(pool.clone()));
    Ports {
        handoff: turn_store.clone(),
        recovery: turn_store,
        stores: ExecutionStores {
            execution: Arc::new(PgExecutionStore::new(pool.clone())),
            journal: Arc::new(PgJournalReader::new(pool.clone())),
            node_results: Arc::new(nebula_storage::InMemoryNodeResultStore::new()),
            checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new()),
            idempotency: Arc::new(PgIdempotencyGuard::new(pool.clone())),
            resume_tokens: Arc::new(PgResumeTokenStore::new(pool.clone())),
            operation_ledger: Arc::new(PgOperationLedger::new(pool.clone())),
        },
        bundles: Arc::new(PgStartAcceptanceStore::new(pool.clone())),
        queue: Arc::new(PgControlQueue::new(pool)),
        catalog: catalog.clone(),
        writer: catalog,
    }
}

struct Admitted {
    scope: Scope,
    id: ExecutionId,
    input: serde_json::Value,
}
async fn admit(ports: Ports, count: &Arc<AtomicU32>) -> Admitted {
    let (_, frozen) = frozen_registry(count);
    let predecessor = node_key!("predecessor");
    let wait = node_key!("wait");
    let successor = node_key!("successor");
    let workflow = WorkflowDefinition {
        id: WorkflowId::new(),
        name: "Durable checkpoint reconnect".to_owned(),
        nodes: vec![
            NodeDefinition::new(predecessor.clone(), "Predecessor", "checkpoint", "echo").unwrap(),
            NodeDefinition::new(wait.clone(), "Wait", "checkpoint", "park").unwrap(),
            NodeDefinition::new(successor.clone(), "Successor", "checkpoint", "echo").unwrap(),
        ],
        connections: vec![
            Connection::new(predecessor, wait.clone()),
            Connection::new(wait, successor),
        ],
        description: None,
        version: nebula_workflow::Version::new(0, 1, 0),
        variables: std::collections::HashMap::new(),
        config: nebula_workflow::WorkflowConfig::default(),
        trigger_bindings: Vec::new(),
        tags: Vec::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        owner_id: None,
        ui_metadata: None,
        schema_version: nebula_workflow::CURRENT_SCHEMA_VERSION,
    };
    let plan = frozen
        .compile_graph_v1(nebula_core::WorkflowVersionId::new(), &workflow)
        .unwrap();
    PlanFlavorRevisionInstaller::new(ports.writer)
        .install(&frozen, &plan)
        .await
        .unwrap();
    let scope = Scope::new(
        nebula_core::WorkspaceId::new().to_string(),
        nebula_core::OrgId::new().to_string(),
    );
    let id = ExecutionId::new();
    let input =
        serde_json::json!({"payload": "persisted predecessor output", "identity": id.to_string()});
    let mut state = ExecutionState::new(id, workflow.id, &[]);
    state.set_revision_ids(plan.id(), frozen.revision().id());
    state.set_workflow_version_number(1);
    state.set_budget(ExecutionBudget::default());
    state.set_workflow_input(input.clone());
    let bundle = ExecutionContractBundle::new_graph_v1(
        nebula_core::ExecutionContractBundleId::new(),
        scope.org_id.parse().unwrap(),
        scope.workspace_id.parse().unwrap(),
        plan.id(),
        plan.plugin_set_id(),
        ExecutionRevisions::new(plan.workflow_version_id(), frozen.revision().id()),
        [],
    );
    let bundle = ContractBundleRecord::v1_json(
        StartContractIdentity::new(
            bundle.bundle_id(),
            nebula_storage_port::PlanFlavorRevisionIds::new(plan.id(), frozen.revision().id()),
        ),
        serde_json::to_vec(&bundle).unwrap(),
    )
    .unwrap();
    let command = ControlMsg {
        id: ulid::Ulid::new().to_bytes(),
        execution_id: id.to_string(),
        command: ControlCommand::Start,
        scope: scope.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    assert!(matches!(
        ports
            .bundles
            .materialize_start(&MaterializedStart::new(
                &scope,
                None,
                &id.to_string(),
                NewExecution::new(
                    &workflow.id.to_string(),
                    &serde_json::to_value(state).unwrap()
                ),
                &command,
                &bundle
            ))
            .await
            .unwrap(),
        StartMaterialization::Accepted { .. }
    ));
    Admitted { scope, id, input }
}

async fn drive(
    ports: Ports,
    admitted: &Admitted,
    count: &Arc<AtomicU32>,
    warm: bool,
) -> serde_json::Value {
    let (registry, frozen) = frozen_registry(count);
    let metrics = MetricsRegistry::new();
    let executor: ActionExecutor =
        Arc::new(|_, _, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            Arc::new(InProcessRunner::new(executor)),
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .unwrap(),
    );
    let execution = ports.stores.execution.clone();
    let handoff = ports.handoff.clone();
    let flavor = frozen.revision().id();
    let engine = Arc::new(
        WorkflowEngine::new(runtime, metrics)
            .unwrap()
            .with_execution_stores(ports.stores)
            .with_plan_flavor_runtime(
                Arc::new(PlanFlavorRevisionLoader::new(ports.catalog)),
                frozen,
                ports.bundles,
            ),
    );
    let prior_engine = Arc::downgrade(&engine);
    let dispatch = EngineControlDispatch::new(
        engine,
        execution.clone(),
        handoff,
        "checkpoint-reconnect-control".to_owned(),
        Duration::from_secs(1),
    );
    if warm {
        dispatch
            .dispatch_resume(&admitted.scope, admitted.id, None)
            .await
            .unwrap();
    } else {
        let claims = ports
            .queue
            .claim_pending_for_flavor(&[0x74; 16], 1, flavor)
            .await
            .unwrap();
        assert_eq!(claims.len(), 1);
        dispatch
            .dispatch_start(&admitted.scope, admitted.id)
            .await
            .unwrap();
        ports.queue.mark_completed(&claims[0].token).await.unwrap();
    }
    let record = execution
        .get(&admitted.scope, &admitted.id.to_string())
        .await
        .unwrap()
        .unwrap();
    let state: ExecutionState =
        serde_json::from_slice(&serde_json::to_vec(&record.state).unwrap()).unwrap();
    assert_eq!(
        count.load(Ordering::SeqCst),
        if warm { 2 } else { 1 },
        "a committed predecessor must never rerun"
    );
    assert_eq!(
        state.status,
        if warm {
            nebula_execution::ExecutionStatus::Completed
        } else {
            nebula_execution::ExecutionStatus::Paused
        }
    );
    let node = if warm {
        node_key!("successor")
    } else {
        node_key!("predecessor")
    };
    let evidence = state
        .checkpoint
        .as_ref()
        .unwrap()
        .nodes()
        .get(&node)
        .unwrap();
    let nebula_execution::NodeCheckpoint::ActionResult { value, .. } = evidence else {
        panic!("actual action result expected")
    };
    let result: ActionResult<serde_json::Value> =
        serde_json::from_slice(&serde_json::to_vec(value).unwrap()).unwrap();
    let ActionResult::Success { output } = result else {
        panic!("echo must record Success")
    };
    assert_eq!(output.as_value(), Some(&admitted.input));
    drop(dispatch);
    let engine_recreated = prior_engine.upgrade().is_none();
    assert!(
        engine_recreated,
        "each reconnect turn must release the previous engine instance"
    );
    serde_json::json!({
        "turn": if warm { "warm-resume" } else { "cold-start" },
        "execution_id": admitted.id.to_string(),
        "execution_version": record.version,
        "status": format!("{:?}", state.status),
        "action_calls_total": count.load(Ordering::SeqCst),
        "checkpoint_node": node.as_str(),
        "checkpoint_output": output.as_value().unwrap(),
        "executable_plan_revision_id": state.executable_plan_revision_id.unwrap().to_string(),
        "worker_flavor_revision_id": state.worker_flavor_revision_id.unwrap().to_string(),
        "engine_recreated": engine_recreated
    })
}

fn write_checkpoint_report(backend: &str, env: &str, turns: Vec<serde_json::Value>) {
    let Ok(path) = std::env::var(env) else {
        return;
    };
    let path = std::path::Path::new(&path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .unwrap();
    serde_json::to_writer_pretty(
        std::io::BufWriter::new(file),
        &serde_json::json!({
            "producer_version": 1,
            "contract": "durable-checkpoint-reconnect",
            "scenario_inventory_version": 1,
            "backend": backend,
            "turns": turns
        }),
    )
    .unwrap();
}

#[tokio::test]
async fn in_memory_checkpoint_reconnect() {
    let core = Arc::new(nebula_storage::InMemoryExecutionStore::new());
    let count = Arc::new(AtomicU32::new(0));
    let admitted = admit(in_memory(&core), &count).await;
    let cold = drive(in_memory(&core), &admitted, &count, false).await;
    let warm = drive(in_memory(&core), &admitted, &count, true).await;
    write_checkpoint_report(
        "in-memory",
        "NEBULA_CHECKPOINT_RECONNECT_IN_MEMORY_OBSERVATIONS_PATH",
        vec![cold, warm],
    );
}

async fn claimed_start_ends_delivery(
    ports: Ports,
    admitted: &Admitted,
    count: &Arc<AtomicU32>,
) -> serde_json::Value {
    let barrier = Arc::new(ActionBarrier::default());
    let (registry, frozen) = frozen_registry_with_barrier(count, Some(Arc::clone(&barrier)));
    let flavor = frozen.revision().id();
    let metrics = MetricsRegistry::new();
    let executor: ActionExecutor =
        Arc::new(|_, _, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            Arc::new(InProcessRunner::new(executor)),
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .unwrap(),
    );
    let execution = ports.stores.execution.clone();
    let engine = Arc::new(
        WorkflowEngine::new(runtime, metrics)
            .unwrap()
            .with_lease_ttl(Duration::from_secs(1))
            .with_execution_stores(ports.stores)
            .with_plan_flavor_runtime(
                Arc::new(PlanFlavorRevisionLoader::new(ports.catalog)),
                frozen,
                ports.bundles,
            ),
    );
    let dispatch = Arc::new(EngineControlDispatch::new(
        engine,
        execution.clone(),
        ports.handoff,
        "control-start-worker".to_owned(),
        Duration::from_secs(1),
    ));
    let shutdown = tokio_util::sync::CancellationToken::new();
    let consumer = nebula_engine::ControlConsumer::for_flavor(
        ports.queue.clone(),
        dispatch,
        [0x75; 16],
        flavor,
    )
    .with_poll_interval(Duration::from_millis(5))
    .spawn(shutdown.clone());
    tokio::time::timeout(Duration::from_secs(10), barrier.entered.notified())
        .await
        .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    // The action is still suspended: delivery must already be terminal when a
    // competing worker immediately sweeps all processing claims.
    let swept = ports.queue.reclaim_stuck(Duration::ZERO, 3).await.unwrap();
    assert_eq!(
        swept.reclaimed, 0,
        "action duration must not retain the Start delivery claim"
    );
    assert_eq!(swept.exhausted, 0);
    let competing_claims = ports
        .queue
        .claim_pending_for_flavor(&[0x76; 16], 1, flavor)
        .await
        .unwrap();
    assert!(competing_claims.is_empty());
    let competing_execution_lease_acquired = execution
        .acquire_lease(
            &admitted.scope,
            &admitted.id.to_string(),
            "competing-worker",
            Duration::from_secs(30),
        )
        .await
        .unwrap()
        .is_some();
    assert!(!competing_execution_lease_acquired);
    barrier.release.notify_one();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let record = execution
                .get(&admitted.scope, &admitted.id.to_string())
                .await
                .unwrap()
                .unwrap();
            let state: ExecutionState =
                serde_json::from_slice(&serde_json::to_vec(&record.state).unwrap()).unwrap();
            if state.status == nebula_execution::ExecutionStatus::Paused {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(10), consumer)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let final_sweep = ports.queue.reclaim_stuck(Duration::ZERO, 3).await.unwrap();
    assert_eq!((final_sweep.reclaimed, final_sweep.exhausted), (0, 0));
    serde_json::json!({
        "command": "Start",
        "action_calls_while_delivery_terminal": count.load(Ordering::SeqCst),
        "claim_reclaimed_while_action_blocked": swept.reclaimed,
        "claim_exhausted_while_action_blocked": swept.exhausted,
        "competing_claim_count": competing_claims.len(),
        "competing_execution_lease_acquired": competing_execution_lease_acquired,
        "final_reclaimed": final_sweep.reclaimed,
        "final_exhausted": final_sweep.exhausted
    })
}

#[tokio::test]
async fn in_memory_control_start_ends_delivery_before_action() {
    let core = Arc::new(nebula_storage::InMemoryExecutionStore::new());
    let count = Arc::new(AtomicU32::new(0));
    let admitted = admit(in_memory(&core), &count).await;
    claimed_start_ends_delivery(in_memory(&core), &admitted, &count).await;
}

#[derive(Debug)]
struct LostHandoffAcknowledgement(Arc<dyn nebula_storage_port::ExecutionTurnHandoff>);

#[async_trait::async_trait]
impl nebula_storage_port::ExecutionTurnHandoff for LostHandoffAcknowledgement {
    async fn commit_control_turn(
        &self,
        request: &nebula_storage_port::store::ControlTurnCommit<'_>,
    ) -> Result<
        nebula_storage_port::store::ControlTurnCommitOutcome,
        nebula_storage_port::StorageError,
    > {
        self.0.commit_control_turn(request).await
    }

    async fn accept_control_start(
        &self,
        request: &nebula_storage_port::store::ControlStartHandoff<'_>,
    ) -> Result<nebula_storage_port::store::ControlStartAcceptance, nebula_storage_port::StorageError>
    {
        assert!(matches!(
            self.0.accept_control_start(request).await.unwrap(),
            nebula_storage_port::store::ControlStartAcceptance::Accepted { .. }
        ));
        Err(nebula_storage_port::StorageError::AcknowledgementUnknown {
            operation: "control_start_handoff",
        })
    }

    async fn accept_turn(
        &self,
        request: &nebula_storage_port::TurnHandoff<'_>,
    ) -> Result<nebula_storage_port::TurnAcceptance, nebula_storage_port::StorageError> {
        self.0.accept_turn(request).await
    }
}

#[tokio::test]
async fn control_start_preflight_and_unknown_acceptance_never_invoke_actions() {
    let core = Arc::new(nebula_storage::InMemoryExecutionStore::new());
    let count = Arc::new(AtomicU32::new(0));
    let admitted = admit(in_memory(&core), &count).await;
    let ports = in_memory(&core);
    let (registry, frozen) = frozen_registry(&count);
    let metrics = MetricsRegistry::new();
    let executor: ActionExecutor =
        Arc::new(|_, _, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            Arc::new(InProcessRunner::new(executor)),
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .unwrap(),
    );
    let engine = WorkflowEngine::new(runtime, metrics)
        .unwrap()
        .with_execution_stores(ports.stores);
    let claims = ports
        .queue
        .claim_pending_for_flavor(&[0x77; 16], 1, frozen.revision().id())
        .await
        .unwrap();
    assert_eq!(claims.len(), 1);
    let uncertain = LostHandoffAcknowledgement(ports.handoff);
    let request = nebula_engine::ClaimedStartRequest {
        claim: claims[0].token.clone(),
        handoff: &uncertain,
        holder: "unknown-ack-worker",
        lease_ttl: Duration::from_secs(30),
    };
    assert!(matches!(
        engine
            .resume_control_start(&admitted.scope, admitted.id, request.clone())
            .await,
        nebula_engine::ClaimedStartOutcome::NotAccepted(
            nebula_engine::EngineError::MissingExactRuntime
        )
    ));
    assert_eq!(count.load(Ordering::SeqCst), 0);
    let engine = engine.with_plan_flavor_runtime(
        Arc::new(PlanFlavorRevisionLoader::new(ports.catalog)),
        frozen,
        ports.bundles,
    );
    assert!(matches!(
        engine
            .resume_control_start(&admitted.scope, admitted.id, request)
            .await,
        nebula_engine::ClaimedStartOutcome::AcceptanceUnknown(
            nebula_engine::EngineError::ControlStartHandoff {
                source: nebula_storage_port::StorageError::AcknowledgementUnknown { .. },
            }
        )
    ));
    assert_eq!(count.load(Ordering::SeqCst), 0);
    let swept = ports.queue.reclaim_stuck(Duration::ZERO, 3).await.unwrap();
    assert_eq!(
        (swept.reclaimed, swept.exhausted),
        (0, 0),
        "unknown acknowledgement may already have completed Start"
    );
    let record = nebula_storage_port::store::ExecutionStore::get(
        core.as_ref(),
        &admitted.scope,
        &admitted.id.to_string(),
    )
    .await
    .unwrap()
    .unwrap();
    let state: ExecutionState =
        serde_json::from_slice(&serde_json::to_vec(&record.state).unwrap()).unwrap();
    assert_eq!(state.status, nebula_execution::ExecutionStatus::Created);
    assert!(
        nebula_storage_port::store::ExecutionStore::acquire_lease(
            core.as_ref(),
            &admitted.scope,
            &admitted.id.to_string(),
            "competitor",
            Duration::from_secs(30)
        )
        .await
        .unwrap()
        .is_none()
    );
}

async fn claimed_control_ends_delivery_before_action(
    ports: Ports,
    admitted: &Admitted,
    command: ControlCommand,
) -> serde_json::Value {
    let count = Arc::new(AtomicU32::new(0));
    let barrier = Arc::new(ActionBarrier {
        block_at_call: 1,
        ..ActionBarrier::default()
    });
    let (registry, frozen) = frozen_registry_with_barrier(&count, Some(barrier.clone()));
    let start = ports
        .queue
        .claim_pending_for_flavor(&[0x91; 16], 1, frozen.revision().id())
        .await
        .unwrap();
    assert_eq!(start.len(), 1);
    ports.queue.mark_completed(&start[0].token).await.unwrap();
    let message = ControlMsg {
        id: ExecutionId::new().as_bytes(),
        execution_id: admitted.id.to_string(),
        command,
        scope: admitted.scope.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    ports.queue.enqueue(&message).await.unwrap();
    let claim = ports
        .queue
        .claim_pending_for_flavor(&[0x92; 16], 1, frozen.revision().id())
        .await
        .unwrap();
    assert_eq!(claim.len(), 1);
    let metrics = MetricsRegistry::new();
    let executor: ActionExecutor =
        Arc::new(|_, _, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            Arc::new(InProcessRunner::new(executor)),
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .unwrap(),
    );
    let engine = WorkflowEngine::new(runtime, metrics)
        .unwrap()
        .with_execution_stores(ports.stores)
        .with_plan_flavor_runtime(
            Arc::new(PlanFlavorRevisionLoader::new(ports.catalog)),
            frozen,
            ports.bundles,
        );
    let request = nebula_engine::ClaimedControlTurnRequest {
        claim: claim[0].token.clone(),
        handoff: ports.handoff,
        command: match command {
            ControlCommand::Resume => {
                nebula_storage_port::store::ControlTurnCommand::Resume { target: None }
            },
            ControlCommand::Restart => nebula_storage_port::store::ControlTurnCommand::Restart,
            _ => panic!("fixture supports only driving control commands"),
        },
    };
    let drive = engine.resume_claimed_control_turn(&admitted.scope, admitted.id, request);
    tokio::pin!(drive);
    tokio::select! {
        result = &mut drive => panic!("action must remain blocked after command acceptance: {result:?}"),
        () = barrier.entered.notified() => {},
        () = tokio::time::sleep(Duration::from_secs(10)) => panic!("claimed command did not reach the action"),
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let swept = ports.queue.reclaim_stuck(Duration::ZERO, 3).await.unwrap();
    assert_eq!(
        swept.reclaimed, 0,
        "accepted command must leave claim ownership before the action returns"
    );
    let competing = ports.queue.claim_pending(&[0x93; 16], 1).await.unwrap();
    assert!(
        competing.is_empty(),
        "accepted delivery cannot remain pending"
    );
    barrier.release.notify_one();
    assert!(matches!(
        drive.await,
        nebula_engine::ClaimedControlTurnOutcome::Accepted(Ok(()))
    ));
    serde_json::json!({
        "command": format!("{command:?}"),
        "claim_generation": claim[0].token.generation().get(),
        "action_calls_while_delivery_terminal": count.load(Ordering::SeqCst),
        "claim_reclaimed_while_action_blocked": swept.reclaimed,
        "claim_exhausted_while_action_blocked": swept.exhausted,
        "competing_claim_count": competing.len(),
        "durable_handoff_outcome": "Accepted"
    })
}

#[tokio::test]
async fn in_memory_resume_and_restart_end_delivery_before_action() {
    for command in [ControlCommand::Resume, ControlCommand::Restart] {
        let core = Arc::new(nebula_storage::InMemoryExecutionStore::new());
        let count = Arc::new(AtomicU32::new(0));
        let admitted = admit(in_memory(&core), &count).await;
        claimed_control_ends_delivery_before_action(in_memory(&core), &admitted, command).await;
    }
}

#[tokio::test]
async fn sqlite_resume_and_restart_end_delivery_before_action() {
    for command in [ControlCommand::Resume, ControlCommand::Restart] {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(4)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        nebula_storage::sqlite::init_schema(&pool).await.unwrap();
        let count = Arc::new(AtomicU32::new(0));
        let admitted = admit(sqlite(pool.clone()), &count).await;
        claimed_control_ends_delivery_before_action(sqlite(pool.clone()), &admitted, command).await;
        pool.close().await;
    }
}

#[tokio::test]
async fn postgres_resume_and_restart_end_delivery_before_action() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        assert!(std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none());
        return;
    };
    for command in [ControlCommand::Resume, ControlCommand::Restart] {
        let pool = postgres_schema::connect_with_private_schema(&url, "control_turn_runtime")
            .await
            .unwrap();
        nebula_storage::postgres::init_schema(&pool).await.unwrap();
        let count = Arc::new(AtomicU32::new(0));
        let admitted = admit(postgres(pool.clone()), &count).await;
        claimed_control_ends_delivery_before_action(postgres(pool.clone()), &admitted, command)
            .await;
        pool.close().await;
    }
}

#[tokio::test]
async fn sqlite_control_start_ends_delivery_before_action() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    nebula_storage::sqlite::init_schema(&pool).await.unwrap();
    let count = Arc::new(AtomicU32::new(0));
    let admitted = admit(sqlite(pool.clone()), &count).await;
    claimed_start_ends_delivery(sqlite(pool.clone()), &admitted, &count).await;
    pool.close().await;
}

#[tokio::test]
async fn postgres_control_start_ends_delivery_before_action() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        assert!(std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none());
        return;
    };
    let pool = postgres_schema::connect_with_private_schema(&url, "control_start_runtime")
        .await
        .unwrap();
    nebula_storage::postgres::init_schema(&pool).await.unwrap();
    let count = Arc::new(AtomicU32::new(0));
    let admitted = admit(postgres(pool.clone()), &count).await;
    claimed_start_ends_delivery(postgres(pool.clone()), &admitted, &count).await;
    pool.close().await;
}

async fn collect_claim_handoff_observations(
    make_ports: impl Fn() -> Ports,
) -> Vec<serde_json::Value> {
    let count = Arc::new(AtomicU32::new(0));
    let admitted = admit(make_ports(), &count).await;
    let start = claimed_start_ends_delivery(make_ports(), &admitted, &count).await;
    let mut observations = vec![start];
    for command in [ControlCommand::Resume, ControlCommand::Restart] {
        let count = Arc::new(AtomicU32::new(0));
        let admitted = admit(make_ports(), &count).await;
        observations.push(
            claimed_control_ends_delivery_before_action(make_ports(), &admitted, command).await,
        );
    }
    observations
}

fn write_claim_handoff_report(backend: &str, env: &str, observations: Vec<serde_json::Value>) {
    let Ok(path) = std::env::var(env) else {
        return;
    };
    let path = std::path::Path::new(&path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .unwrap();
    serde_json::to_writer_pretty(
        std::io::BufWriter::new(file),
        &serde_json::json!({
            "producer_version": 1,
            "contract": "claim-handoff",
            "scenario_inventory_version": 1,
            "backend": backend,
            "commands": observations
        }),
    )
    .unwrap();
}

#[tokio::test]
async fn in_memory_claim_handoff_raw_observations() {
    let core = Arc::new(nebula_storage::InMemoryExecutionStore::new());
    let observations = collect_claim_handoff_observations(|| in_memory(&core)).await;
    write_claim_handoff_report(
        "in-memory",
        "NEBULA_CLAIM_HANDOFF_IN_MEMORY_OBSERVATIONS_PATH",
        observations,
    );
}

#[tokio::test]
async fn sqlite_claim_handoff_raw_observations() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    nebula_storage::sqlite::init_schema(&pool).await.unwrap();
    let observations = collect_claim_handoff_observations(|| sqlite(pool.clone())).await;
    write_claim_handoff_report(
        "sqlite",
        "NEBULA_CLAIM_HANDOFF_SQLITE_OBSERVATIONS_PATH",
        observations,
    );
    pool.close().await;
}

#[tokio::test]
async fn postgres_claim_handoff_raw_observations() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        assert!(std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none());
        return;
    };
    let pool = postgres_schema::connect_with_private_schema(&url, "handoff_observations")
        .await
        .unwrap();
    nebula_storage::postgres::init_schema(&pool).await.unwrap();
    let observations = collect_claim_handoff_observations(|| postgres(pool.clone())).await;
    write_claim_handoff_report(
        "postgresql",
        "NEBULA_CLAIM_HANDOFF_POSTGRES_OBSERVATIONS_PATH",
        observations,
    );
    pool.close().await;
}

#[tokio::test]
async fn sqlite_checkpoint_reconnect() {
    let directory = tempfile::tempdir().unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(directory.path().join("checkpoint.db"))
        .create_if_missing(true)
        .busy_timeout(Duration::from_secs(10));
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options.clone())
        .await
        .unwrap();
    nebula_storage::sqlite::init_schema(&pool).await.unwrap();
    let count = Arc::new(AtomicU32::new(0));
    let admitted = admit(sqlite(pool.clone()), &count).await;
    pool.close().await;
    let mut turns = Vec::new();
    for warm in [false, true] {
        let reopened = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options.clone())
            .await
            .unwrap();
        turns.push(drive(sqlite(reopened.clone()), &admitted, &count, warm).await);
        reopened.close().await;
    }
    write_checkpoint_report(
        "sqlite",
        "NEBULA_CHECKPOINT_RECONNECT_SQLITE_OBSERVATIONS_PATH",
        turns,
    );
}

#[tokio::test]
async fn postgres_checkpoint_reconnect() {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => {
            assert!(
                std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
                "required PostgreSQL runtime evidence needs DATABASE_URL"
            );
            return;
        },
        Err(std::env::VarError::NotUnicode(_)) => panic!("PostgreSQL URL must be Unicode"),
    };
    let pool = postgres_schema::connect_with_private_schema(&url, "checkpoint_runtime")
        .await
        .unwrap();
    nebula_storage::postgres::init_schema(&pool).await.unwrap();
    let schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(&pool)
        .await
        .unwrap();
    let options = url
        .parse::<sqlx::postgres::PgConnectOptions>()
        .unwrap()
        .options([("search_path", schema)]);
    let count = Arc::new(AtomicU32::new(0));
    let admitted = admit(postgres(pool.clone()), &count).await;
    pool.close().await;
    let mut turns = Vec::new();
    for warm in [false, true] {
        let reopened = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.clone())
            .await
            .unwrap();
        turns.push(drive(postgres(reopened.clone()), &admitted, &count, warm).await);
        reopened.close().await;
    }
    write_checkpoint_report(
        "postgresql",
        "NEBULA_CHECKPOINT_RECONNECT_POSTGRES_OBSERVATIONS_PATH",
        turns,
    );
}
