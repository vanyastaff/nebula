//! Real owner adapters and pure action fixtures for worker restart tests.
use nebula_action::{
    Action, ActionError, ActionFactory, ActionResult, RemoteEffectInstanceFactory, StatelessAction,
    effect::{
        ActionEffectContract, EffectFailureCode, EffectInvocationContext, EffectInvocationOutcome,
        EffectPreparationContext, EffectPreparationError, PreparedEffectAdapter,
        PreparedRemoteEffect, RemoteDestinationGuarantee, RemoteEffectAction,
        RemoteEffectDescriptor, RemoteEffectPolicy,
    },
};
use nebula_core::{Dependencies, ExecutionId, OperationId, WorkflowId, action_key, node_key};
use nebula_engine::{
    ActionRegistry, ActionRuntime, DataPassingPolicy, ExecutionStores, InProcessRunner,
    PlanFlavorRevisionInstaller, PlanFlavorRevisionLoader, ResourceFanoutCoordinator,
    WorkflowEngine, WorkflowStartService,
};
use nebula_execution::{
    ExecutionBudget, ExecutionContractBundle, ExecutionRevisions, ExecutionState,
};
use nebula_metrics::MetricsRegistry;
use nebula_plugin::{FrozenPluginRegistry, Plugin, PluginManifest, PluginRegistry, ResolvedPlugin};
use nebula_storage_port::{
    FencingToken, Scope,
    dto::{
        ClaimResourceRuntimeWorkRequest, ContractBundleRecord, ControlCommand, ControlMsg,
        MaterializedStart, NewExecution, ResourceLeaseHolder, ResourceLeaseTtl, ResourcePageSize,
    },
    store::{
        ControlQueue, PlanFlavorCatalog, PlanFlavorCatalogWriter, StartAcceptanceStore,
        StartContractIdentity, StartMaterialization,
    },
};
use nebula_workflow::{Connection, NodeDefinition, WorkflowDefinition};
use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

#[derive(Debug, Default)]
pub(super) struct EffectObservations {
    operation_ids: Mutex<Vec<OperationId>>,
    applied_requests: Mutex<HashSet<Vec<u8>>>,
    business_effects: AtomicU32,
}

impl EffectObservations {
    pub(super) fn operation_ids(&self) -> Vec<OperationId> {
        self.operation_ids.lock().unwrap().clone()
    }

    pub(super) fn business_effects(&self) -> u32 {
        self.business_effects.load(Ordering::SeqCst)
    }
}

struct RecoveryEffectAction {
    descriptor: RemoteEffectDescriptor,
    observations: Arc<EffectObservations>,
}

impl Action for RecoveryEffectAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("recovery.send"),
            nebula_action::metadata_name!("Send"),
            "durable recovery effect fixture",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl RemoteEffectAction for RecoveryEffectAction {
    fn descriptor(&self) -> &RemoteEffectDescriptor {
        &self.descriptor
    }

    async fn prepare(
        &self,
        input: serde_json::Value,
        _: &EffectPreparationContext,
    ) -> Result<PreparedRemoteEffect, EffectPreparationError> {
        let canonical_request =
            serde_json::to_vec(&input).map_err(|_| EffectPreparationError::InvalidRequest)?;
        PreparedRemoteEffect::new(
            canonical_request.clone().into_boxed_slice(),
            b"worker-recovery-provider/account-a"
                .to_vec()
                .into_boxed_slice(),
            Box::new(RecoveryEffectAdapter {
                canonical_request,
                observations: self.observations.clone(),
            }),
        )
    }
}

struct RecoveryEffectAdapter {
    canonical_request: Vec<u8>,
    observations: Arc<EffectObservations>,
}

#[async_trait::async_trait]
impl PreparedEffectAdapter for RecoveryEffectAdapter {
    async fn invoke(&self, context: &dyn EffectInvocationContext) -> EffectInvocationOutcome {
        let invocation_count = {
            let mut operation_ids = self.observations.operation_ids.lock().unwrap();
            operation_ids.push(context.operation_id());
            operation_ids.len()
        };
        if invocation_count == 1 {
            return EffectInvocationOutcome::BeforeBoundary(
                EffectFailureCode::UnavailableBeforeBoundary,
            );
        }

        if self
            .observations
            .applied_requests
            .lock()
            .unwrap()
            .insert(self.canonical_request.clone())
        {
            self.observations
                .business_effects
                .fetch_add(1, Ordering::SeqCst);
        }
        EffectInvocationOutcome::Applied(Box::new(ActionResult::success(
            serde_json::json!({"receipt":"worker-recovery-provider"}),
        )))
    }
}

#[derive(Default)]
pub(super) struct Calls {
    pub predecessor: AtomicU32,
    pub successor: AtomicU32,
    pub entered: tokio::sync::Notify,
}
struct Echo {
    calls: Arc<Calls>,
    block_successor: bool,
}
impl Action for Echo {
    type Input = serde_json::Value;
    type Output = serde_json::Value;
    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("recovery.echo"),
            nebula_action::metadata_name!("Echo"),
            "pure recovery fixture",
        )
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
        context: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        if context.node_key() == &node_key!("predecessor") {
            self.calls.predecessor.fetch_add(1, Ordering::SeqCst);
        } else {
            self.calls.successor.fetch_add(1, Ordering::SeqCst);
            if self.block_successor {
                self.calls.entered.notify_one();
                std::future::pending::<()>().await;
            }
        }
        Ok(ActionResult::success(input))
    }
}
struct FixturePlugin {
    manifest: PluginManifest,
    factory: Arc<dyn ActionFactory>,
}
impl std::fmt::Debug for FixturePlugin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RecoveryFixturePlugin")
    }
}
impl Plugin for FixturePlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
    fn actions(&self) -> Vec<Arc<dyn ActionFactory>> {
        vec![self.factory.clone()]
    }
}
fn registry(
    calls: &Arc<Calls>,
    block_successor: bool,
) -> (Arc<ActionRegistry>, Arc<FrozenPluginRegistry>) {
    let actions = Arc::new(ActionRegistry::new());
    actions
        .register_stateless_instance(
            Echo::metadata(),
            Echo {
                calls: calls.clone(),
                block_successor,
            },
        )
        .expect("valid test catalog definition");
    let plugin = FixturePlugin {
        manifest: PluginManifest::builder("recovery", "Recovery")
            .build()
            .unwrap(),
        factory: actions
            .get_factory(&action_key!("recovery.echo"))
            .unwrap()
            .1,
    };
    let mut plugins = PluginRegistry::new();
    plugins
        .register(Arc::new(ResolvedPlugin::from(plugin).unwrap()))
        .unwrap();
    let frozen = plugins
        .freeze(
            nebula_core::ArtifactSetDigest::from_bytes([0x79; 32]),
            "1.0.0".parse().unwrap(),
        )
        .unwrap();
    (actions, Arc::new(frozen))
}

fn effect_registry(
    observations: &Arc<EffectObservations>,
) -> (Arc<ActionRegistry>, Arc<FrozenPluginRegistry>) {
    let policy =
        RemoteEffectPolicy::builder(RemoteDestinationGuarantee::stable_key(60_000).unwrap())
            .maximum_invocations(2)
            .maximum_queries(0)
            .recovery_window(Duration::from_mins(1))
            .build()
            .unwrap();
    let descriptor = RemoteEffectDescriptor::new("worker.recovery/v1", 1, policy).unwrap();
    let metadata = nebula_action::ActionMetadataDraft::new(
        action_key!("recovery.send"),
        nebula_action::metadata_name!("Send"),
        "durable recovery effect fixture",
    )
    .with_effect_contract(ActionEffectContract::Remote(Box::new(descriptor.clone())));
    let factory = Arc::new(
        RemoteEffectInstanceFactory::new(
            metadata,
            RecoveryEffectAction {
                descriptor,
                observations: observations.clone(),
            },
        )
        .expect("coherent recovery effect factory"),
    );
    let plugin = FixturePlugin {
        manifest: PluginManifest::builder("recovery", "Recovery")
            .build()
            .unwrap(),
        factory,
    };
    let mut plugins = PluginRegistry::new();
    plugins
        .register(Arc::new(ResolvedPlugin::from(plugin).unwrap()))
        .unwrap();
    let frozen = plugins
        .freeze(
            nebula_core::ArtifactSetDigest::from_bytes([0x79; 32]),
            "1.0.0".parse().unwrap(),
        )
        .unwrap();
    (Arc::new(ActionRegistry::new()), Arc::new(frozen))
}

pub(super) struct Ports {
    pub stores: ExecutionStores,
    pub control: Arc<dyn ControlQueue>,
    pub handoff: Arc<dyn nebula_storage_port::ExecutionTurnHandoff>,
    pub recovery: Arc<dyn nebula_storage_port::TurnRecovery>,
    pub bundles: Arc<dyn StartAcceptanceStore>,
    pub catalog: Arc<dyn PlanFlavorCatalog>,
    pub writer: Arc<dyn PlanFlavorCatalogWriter>,
}
pub(super) enum Backend {
    Memory(Arc<nebula_storage::InMemoryExecutionStore>),
    Sqlite {
        pool: sqlx::SqlitePool,
        options: sqlx::sqlite::SqliteConnectOptions,
        _directory: tempfile::TempDir,
    },
    Postgres {
        pool: sqlx::PgPool,
        options: sqlx::postgres::PgConnectOptions,
    },
}
impl Backend {
    /// Make the abandoned execution immediately recoverable through the
    /// storage-owned, fence-checked lease operation. Waiting for a database
    /// wall clock would make this restart test depend on scheduler load.
    pub(super) async fn release_abandoned_lease(&self, admitted: &Admitted) {
        let execution = self
            .ports()
            .stores
            .execution
            .get(&admitted.scope, &admitted.id.to_string())
            .await
            .unwrap()
            .unwrap();
        assert!(
            execution.lease_holder.is_some(),
            "the stopped runtime must leave an owned execution lease"
        );
        let fencing = FencingToken::from_generation(
            execution
                .fencing
                .expect("an owned execution lease must carry a fencing generation"),
        );
        assert!(
            self.ports()
                .stores
                .execution
                .release_lease(&admitted.scope, &admitted.id.to_string(), fencing)
                .await
                .unwrap(),
            "the abandoned owner must still hold the recorded fencing generation"
        );
    }

    /// SQL retention fault injection. The public cleanup port is currently a
    /// no-op, so deleting the completed row explicitly avoids fictitious proof.
    pub(super) async fn delete_completed_delivery(&self) {
        let deleted = match self {
            Self::Memory(_) => return,
            Self::Sqlite { pool, .. } => {
                sqlx::query("DELETE FROM port_control_queue WHERE status = 'Completed'")
                    .execute(pool)
                    .await
                    .unwrap()
                    .rows_affected()
            },
            Self::Postgres { pool, .. } => {
                sqlx::query("DELETE FROM port_control_queue WHERE status = 'Completed'")
                    .execute(pool)
                    .await
                    .unwrap()
                    .rows_affected()
            },
        };
        assert!(deleted > 0, "the actual accepted queue row must be deleted");
    }
    pub(super) async fn new(kind: &str) -> Option<Self> {
        match kind {
            "memory" => Some(Self::Memory(Arc::new(
                nebula_storage::InMemoryExecutionStore::new(),
            ))),
            "sqlite" => {
                let directory = tempfile::tempdir().unwrap();
                let options = sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(directory.path().join("worker.db"))
                    .create_if_missing(true)
                    .busy_timeout(Duration::from_secs(10));
                let pool = sqlx::sqlite::SqlitePoolOptions::new()
                    .max_connections(8)
                    .connect_with(options.clone())
                    .await
                    .unwrap();
                nebula_storage::sqlite::init_schema(&pool).await.unwrap();
                Some(Self::Sqlite {
                    pool,
                    options,
                    _directory: directory,
                })
            },
            "postgres" => {
                let Ok(url) = std::env::var("DATABASE_URL") else {
                    assert!(std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none());
                    return None;
                };
                let admin = sqlx::postgres::PgPoolOptions::new()
                    .max_connections(1)
                    .connect(&url)
                    .await
                    .unwrap();
                let schema = format!(
                    "worker_recovery_{}",
                    ulid::Ulid::new().to_string().to_lowercase()
                );
                sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
                    .execute(&admin)
                    .await
                    .unwrap();
                admin.close().await;
                let options = url
                    .parse::<sqlx::postgres::PgConnectOptions>()
                    .unwrap()
                    .options([("search_path", schema)]);
                let pool = sqlx::postgres::PgPoolOptions::new()
                    .max_connections(8)
                    .connect_with(options.clone())
                    .await
                    .unwrap();
                nebula_storage::postgres::init_schema(&pool).await.unwrap();
                Some(Self::Postgres { pool, options })
            },
            _ => panic!("unknown backend"),
        }
    }
    pub(super) async fn reconnect(&mut self) {
        match self {
            Self::Memory(_) => {},
            Self::Sqlite { pool, options, .. } => {
                pool.close().await;
                *pool = sqlx::sqlite::SqlitePoolOptions::new()
                    .max_connections(8)
                    .connect_with(options.clone())
                    .await
                    .unwrap();
            },
            Self::Postgres { pool, options } => {
                pool.close().await;
                *pool = sqlx::postgres::PgPoolOptions::new()
                    .max_connections(8)
                    .connect_with(options.clone())
                    .await
                    .unwrap();
            },
        }
    }
    pub(super) fn ports(&self) -> Ports {
        let node_results = Arc::new(nebula_storage::InMemoryNodeResultStore::new());
        let checkpoints = Arc::new(nebula_storage::InMemoryCheckpointStore::new());
        match self {
            Self::Memory(core) => {
                use nebula_storage::inmem::*;
                let catalog = Arc::new(core.plan_flavor_catalog());
                Ports {
                    stores: ExecutionStores {
                        execution: core.clone(),
                        journal: Arc::new(InMemoryJournalReader::new(core)),
                        node_results,
                        checkpoints,
                        idempotency: Arc::new(InMemoryIdempotencyGuard::new()),
                        resume_tokens: Arc::new(core.resume_token_store()),
                        operation_ledger: Arc::new(InMemoryOperationLedger::new(core)),
                    },
                    control: Arc::new(InMemoryControlQueue::new(core)),
                    handoff: Arc::new(InMemoryTurnHandoff::new(core)),
                    recovery: Arc::new(InMemoryTurnHandoff::new(core)),
                    bundles: Arc::new(InMemoryStartAcceptanceStore::new(core)),
                    catalog: catalog.clone(),
                    writer: catalog,
                }
            },
            Self::Sqlite { pool, .. } => {
                use nebula_storage::sqlite::*;
                let catalog = Arc::new(SqlitePlanFlavorCatalog::new(
                    pool.clone(),
                    &MetricsRegistry::new(),
                ));
                Ports {
                    stores: ExecutionStores {
                        execution: Arc::new(SqliteExecutionStore::new(pool.clone())),
                        journal: Arc::new(SqliteJournalReader::new(pool.clone())),
                        node_results,
                        checkpoints,
                        idempotency: Arc::new(SqliteIdempotencyGuard::new(pool.clone())),
                        resume_tokens: Arc::new(SqliteResumeTokenStore::new(pool.clone())),
                        operation_ledger: Arc::new(SqliteOperationLedger::new(pool.clone())),
                    },
                    control: Arc::new(SqliteControlQueue::new(pool.clone())),
                    handoff: Arc::new(SqliteTurnHandoff::new(pool.clone())),
                    recovery: Arc::new(SqliteTurnHandoff::new(pool.clone())),
                    bundles: Arc::new(SqliteStartAcceptanceStore::new(pool.clone())),
                    catalog: catalog.clone(),
                    writer: catalog,
                }
            },
            Self::Postgres { pool, .. } => {
                use nebula_storage::postgres::*;
                let catalog = Arc::new(PgPlanFlavorCatalog::new(
                    pool.clone(),
                    &MetricsRegistry::new(),
                ));
                Ports {
                    stores: ExecutionStores {
                        execution: Arc::new(PgExecutionStore::new(pool.clone())),
                        journal: Arc::new(PgJournalReader::new(pool.clone())),
                        node_results,
                        checkpoints,
                        idempotency: Arc::new(PgIdempotencyGuard::new(pool.clone())),
                        resume_tokens: Arc::new(PgResumeTokenStore::new(pool.clone())),
                        operation_ledger: Arc::new(PgOperationLedger::new(pool.clone())),
                    },
                    control: Arc::new(PgControlQueue::new(pool.clone())),
                    handoff: Arc::new(PgTurnHandoff::new(pool.clone())),
                    recovery: Arc::new(PgTurnHandoff::new(pool.clone())),
                    bundles: Arc::new(PgStartAcceptanceStore::new(pool.clone())),
                    catalog: catalog.clone(),
                    writer: catalog,
                }
            },
        }
    }
}

pub(super) struct Admitted {
    pub scope: Scope,
    pub id: ExecutionId,
    pub input: serde_json::Value,
}

async fn materialize(
    ports: Ports,
    frozen: Arc<FrozenPluginRegistry>,
    workflow: WorkflowDefinition,
) -> Admitted {
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
        serde_json::json!({"accepted_input": id.to_string(), "payload":"predecessor persisted"});
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
    let record = ContractBundleRecord::v1_json(
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
    std::assert_matches!(
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
                &record
            ))
            .await
            .unwrap(),
        StartMaterialization::Accepted { .. }
    );
    Admitted { scope, id, input }
}

pub(super) async fn admit(ports: Ports, calls: &Arc<Calls>) -> Admitted {
    let (_, frozen) = registry(calls, false);
    let workflow = WorkflowDefinition {
        id: WorkflowId::new(),
        name: "Worker recovery".to_owned(),
        description: None,
        version: nebula_workflow::Version::new(0, 1, 0),
        nodes: vec![
            NodeDefinition::new(node_key!("predecessor"), "Predecessor", "recovery", "echo")
                .unwrap(),
            NodeDefinition::new(node_key!("successor"), "Successor", "recovery", "echo").unwrap(),
        ],
        connections: vec![Connection::new(
            node_key!("predecessor"),
            node_key!("successor"),
        )],
        variables: std::collections::HashMap::new(),
        config: nebula_workflow::WorkflowConfig::default(),
        trigger_bindings: vec![],
        tags: vec![],
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        owner_id: None,
        ui_metadata: None,
        schema_version: nebula_workflow::CURRENT_SCHEMA_VERSION,
    };
    materialize(ports, frozen, workflow).await
}

pub(super) async fn admit_remote_effect(
    ports: Ports,
    observations: &Arc<EffectObservations>,
) -> Admitted {
    let (_, frozen) = effect_registry(observations);
    let workflow = WorkflowDefinition {
        id: WorkflowId::new(),
        name: "Worker effect recovery".to_owned(),
        description: None,
        version: nebula_workflow::Version::new(0, 1, 0),
        nodes: vec![
            NodeDefinition::new(node_key!("effect"), "Effect", "recovery", "send").unwrap(),
        ],
        connections: vec![],
        variables: std::collections::HashMap::new(),
        config: nebula_workflow::WorkflowConfig::default(),
        trigger_bindings: vec![],
        tags: vec![],
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        owner_id: None,
        ui_metadata: None,
        schema_version: nebula_workflow::CURRENT_SCHEMA_VERSION,
    };
    materialize(ports, frozen, workflow).await
}

pub(super) fn worker(
    ports: Ports,
    calls: &Arc<Calls>,
    blocked: bool,
) -> (
    nebula_worker::WorkerRuntime,
    std::sync::Weak<WorkflowEngine>,
) {
    let (registry, frozen) = registry(calls, blocked);
    build_worker(ports, registry, frozen)
}

pub(super) fn effect_worker(
    ports: Ports,
    observations: &Arc<EffectObservations>,
) -> (
    nebula_worker::WorkerRuntime,
    std::sync::Weak<WorkflowEngine>,
) {
    let (registry, frozen) = effect_registry(observations);
    build_worker(ports, registry, frozen)
}

fn build_worker(
    ports: Ports,
    registry: Arc<ActionRegistry>,
    frozen: Arc<FrozenPluginRegistry>,
) -> (
    nebula_worker::WorkerRuntime,
    std::sync::Weak<WorkflowEngine>,
) {
    let resource_fanout = test_resource_fanout(Arc::clone(&frozen));
    let metrics = MetricsRegistry::new();
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            Arc::new(InProcessRunner::new()),
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .unwrap(),
    );
    let engine = Arc::new(
        WorkflowEngine::new(runtime, metrics)
            .unwrap()
            .with_lease_ttl(Duration::from_secs(1))
            .with_execution_stores(ports.stores.clone())
            .with_plan_flavor_runtime(
                Arc::new(PlanFlavorRevisionLoader::new(ports.catalog)),
                frozen,
                ports.bundles,
            ),
    );
    let weak = Arc::downgrade(&engine);
    let worker =
        nebula_worker::WorkerRuntimeBuilder::from_wired_engine(engine, ports.stores, [0x79; 16])
            .with_control_queue(ports.control)
            .with_turn_handoff(ports.handoff)
            .with_turn_recovery(ports.recovery)
            .with_resource_fanout(resource_fanout)
            .with_handoff_lease_ttl(Duration::from_secs(1))
            .build()
            .unwrap();
    (worker, weak)
}

fn test_resource_fanout(frozen: Arc<FrozenPluginRegistry>) -> Arc<ResourceFanoutCoordinator> {
    let execution = Arc::new(nebula_storage::InMemoryExecutionStore::new());
    let versions = nebula_storage::InMemoryWorkflowVersionStore::new();
    let workflows = Arc::new(nebula_storage::InMemoryWorkflowStore::new_with_versions(
        &versions, &execution,
    ));
    let runtime = Arc::new(nebula_storage::inmem::InMemoryResourceRuntime::new());
    let starts = Arc::new(
        WorkflowStartService::new(
            nebula_engine::WorkflowStores {
                workflow: workflows,
                versions: Arc::new(versions),
            },
            execution.clone(),
            Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                &execution,
            )),
            PlanFlavorRevisionLoader::new(Arc::new(
                nebula_storage::InMemoryPlanFlavorCatalog::new(&execution),
            )),
            frozen,
            Arc::new(nebula_core::accessor::SystemClock),
            ExecutionBudget::default(),
        )
        .expect("test resource workflow-start service accepts the default budget"),
    );
    let claim = ClaimResourceRuntimeWorkRequest::new(
        ResourceLeaseHolder::new("accepted-turn-resource-fanout")
            .expect("test resource holder is bounded"),
        ResourceLeaseTtl::new(Duration::from_secs(30)).expect("test resource claim TTL is bounded"),
        ResourcePageSize::new(32).expect("test resource batch is bounded"),
    );
    Arc::new(
        ResourceFanoutCoordinator::new(
            runtime.clone(),
            runtime.clone(),
            runtime.clone(),
            runtime,
            starts,
            claim,
            Duration::from_millis(100),
            3,
        )
        .expect("test resource fanout configuration is valid"),
    )
}
