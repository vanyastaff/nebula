//! Real backend ports and compile/install/materialize fixture for owner decisions.

use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU32, Ordering},
};

use nebula_action::{
    Action, ActionOutput, ActionResult, StatelessAction, effect::ActionEffectContract,
    result::WaitCondition,
};
use nebula_core::{Dependencies, ExecutionId, WorkflowId, action_key, node_key};
use nebula_engine::{ActionRegistry, ExecutionStores, PlanFlavorRevisionInstaller};
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

struct Echo(Arc<AtomicU32>);
struct Park;

impl Action for Echo {
    type Input = serde_json::Value;
    type Output = serde_json::Value;
    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("operator.echo"),
            nebula_action::metadata_name!("Echo"),
            "pure payload echo",
        )
        .with_effect_contract(ActionEffectContract::ReadOnly)
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
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(input))
    }
}
impl Action for Park {
    type Input = serde_json::Value;
    type Output = serde_json::Value;
    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("operator.park"),
            nebula_action::metadata_name!("Park"),
            "durable signal wait",
        )
        .with_effect_contract(ActionEffectContract::ReadOnly)
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
pub(super) fn frozen_registry(
    count: &Arc<AtomicU32>,
) -> (Arc<ActionRegistry>, Arc<FrozenPluginRegistry>) {
    frozen_registry_for_artifact(count, 0x74)
}
pub(super) fn frozen_registry_for_artifact(
    count: &Arc<AtomicU32>,
    artifact: u8,
) -> (Arc<ActionRegistry>, Arc<FrozenPluginRegistry>) {
    let actions = Arc::new(ActionRegistry::new());
    actions
        .register_stateless_instance(Echo::metadata(), Echo(Arc::clone(count)))
        .expect("valid test catalog definition");
    actions
        .register_stateless_instance(Park::metadata(), Park)
        .expect("valid test catalog definition");
    let plugin = FixturePlugin {
        manifest: PluginManifest::builder("operator", "Checkpoint")
            .build()
            .unwrap(),
        actions: [action_key!("operator.echo"), action_key!("operator.park")]
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
            nebula_core::ArtifactSetDigest::from_bytes([artifact; 32]),
            "1.0.0".parse().unwrap(),
        )
        .unwrap();
    (actions, Arc::new(frozen))
}

#[derive(Clone)]
pub(super) struct Ports {
    pub(super) handoff: Arc<dyn nebula_storage_port::ExecutionTurnHandoff>,
    pub(super) recovery: Arc<dyn nebula_storage_port::TurnRecovery>,
    pub(super) stores: ExecutionStores,
    pub(super) bundles: Arc<dyn StartAcceptanceStore>,
    pub(super) queue: Arc<dyn ControlQueue>,
    pub(super) catalog: Arc<dyn PlanFlavorCatalog>,
    pub(super) writer: Arc<dyn PlanFlavorCatalogWriter>,
}
pub(super) fn in_memory(core: &Arc<nebula_storage::InMemoryExecutionStore>) -> Ports {
    let catalog = Arc::new(core.plan_flavor_catalog());
    let turn_store = Arc::new(nebula_storage::inmem::InMemoryTurnHandoff::new(core));
    Ports {
        handoff: turn_store.clone(),
        recovery: turn_store,
        stores: ExecutionStores {
            execution: core.clone(),
            journal: Arc::new(nebula_storage::InMemoryJournalReader::new(core)),
            node_results: Arc::new(nebula_storage::InMemoryNodeResultStore::new()),
            checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new(core)),
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
pub(super) fn sqlite(pool: sqlx::SqlitePool) -> Ports {
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
            checkpoints: Arc::new(SqliteCheckpointStore::new(pool.clone())),
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
pub(super) fn postgres(pool: sqlx::PgPool) -> Ports {
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
            checkpoints: Arc::new(PgCheckpointStore::new(pool.clone())),
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

pub(super) struct Admitted {
    pub(super) scope: Scope,
    pub(super) id: ExecutionId,
}
pub(super) async fn admit(ports: Ports, count: &Arc<AtomicU32>, throttled: bool) -> Admitted {
    let (_, frozen) = frozen_registry(count);
    let predecessor = node_key!("predecessor");
    let wait = node_key!("wait");
    let successor = node_key!("successor");
    let mut workflow = WorkflowDefinition {
        id: WorkflowId::new(),
        name: "Operator control outcomes".to_owned(),
        nodes: vec![
            NodeDefinition::new(predecessor.clone(), "Predecessor", "operator", "echo").unwrap(),
            NodeDefinition::new(wait.clone(), "Wait", "operator", "park").unwrap(),
            NodeDefinition::new(successor.clone(), "Successor", "operator", "echo").unwrap(),
        ],
        connections: vec![
            Connection::new(predecessor.clone(), wait.clone()),
            Connection::new(wait.clone(), successor.clone()),
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
    if throttled {
        workflow.nodes.retain(|node| node.id != wait);
        for node in &mut workflow.nodes {
            node.rate_limit = Some(nebula_workflow::node::RateLimit {
                max_requests: 1,
                // One token, refilled far slower than the scenario runs; the
                // limiter refuses refill rates below 0.001 per second.
                window_secs: 600,
            });
        }
        workflow.connections = vec![Connection::new(predecessor, successor)];
    }
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
    Admitted { scope, id }
}
