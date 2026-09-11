use super::*;

pub(super) struct SnapshotHandler {
    output: &'static str,
    executions: Option<Arc<AtomicU32>>,
}

impl SnapshotHandler {
    pub(super) const fn new(output: &'static str) -> Self {
        Self {
            output,
            executions: None,
        }
    }

    fn counted(output: &'static str, executions: Arc<AtomicU32>) -> Self {
        Self {
            output,
            executions: Some(executions),
        }
    }
}

impl Action for SnapshotHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            action_key!("exact.run"),
            nebula_action::metadata_name!("Run"),
            "snapshot fixture",
        )
    }

    fn dependencies() -> &'static Dependencies {
        EchoHandler::dependencies()
    }
}

impl StatelessAction for SnapshotHandler {
    async fn execute(
        &self,
        _: Self::Input,
        _: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        if let Some(executions) = &self.executions {
            executions.fetch_add(1, Ordering::SeqCst);
        }
        Ok(ActionResult::success(serde_json::json!(self.output)))
    }
}

pub(super) struct SnapshotPlugin {
    pub(super) manifest: nebula_plugin::PluginManifest,
    pub(super) factory: Arc<dyn nebula_action::ActionFactory>,
}

impl std::fmt::Debug for SnapshotPlugin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SnapshotPlugin")
            .finish_non_exhaustive()
    }
}

impl nebula_plugin::Plugin for SnapshotPlugin {
    fn manifest(&self) -> &nebula_plugin::PluginManifest {
        &self.manifest
    }

    fn actions(&self) -> Vec<Arc<dyn nebula_action::ActionFactory>> {
        vec![Arc::clone(&self.factory)]
    }
}

pub(super) fn snapshot_registry(
    runtime_registry: &ActionRegistry,
    output: &'static str,
) -> Arc<FrozenPluginRegistry> {
    snapshot_registry_counted(runtime_registry, output).0
}

pub(super) fn snapshot_registry_counted(
    runtime_registry: &ActionRegistry,
    output: &'static str,
) -> (Arc<FrozenPluginRegistry>, Arc<AtomicU32>) {
    let metadata = SnapshotHandler::metadata()
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects);
    let executions = Arc::new(AtomicU32::new(0));
    runtime_registry
        .register_stateless_instance(
            metadata,
            SnapshotHandler::counted(output, Arc::clone(&executions)),
        )
        .expect("valid snapshot fixture factory");
    let frozen = frozen_registered_snapshot(runtime_registry);
    (frozen, executions)
}

pub(super) fn frozen_registered_snapshot(
    runtime_registry: &ActionRegistry,
) -> Arc<FrozenPluginRegistry> {
    let (_, factory) = runtime_registry
        .get_factory(&action_key!("exact.run"))
        .unwrap();
    let plugin = SnapshotPlugin {
        manifest: nebula_plugin::PluginManifest::builder("exact", "Exact")
            .build()
            .unwrap(),
        factory,
    };
    let mut registry = PluginRegistry::new();
    registry
        .register(Arc::new(
            nebula_plugin::ResolvedPlugin::from(plugin).unwrap(),
        ))
        .unwrap();
    Arc::new(
        registry
            .freeze(
                nebula_core::ArtifactSetDigest::from_bytes([0x93; 32]),
                "1.0.0".parse().unwrap(),
            )
            .unwrap(),
    )
}

pub(super) async fn install_snapshot_execution(
    stores: &TestStores,
    registry: &Arc<FrozenPluginRegistry>,
) -> (ExecutionId, WorkflowDefinition) {
    install_snapshot_execution_with_bundle(stores, registry, true).await
}

pub(super) async fn install_snapshot_execution_with_bundle(
    stores: &TestStores,
    registry: &Arc<FrozenPluginRegistry>,
    materialize_bundle: bool,
) -> (ExecutionId, WorkflowDefinition) {
    let workflow = make_workflow(
        vec![NodeDefinition::new(node_key!("run"), "Run", "exact", "run").unwrap()],
        vec![],
    );
    let plan = registry
        .compile_graph_v1(nebula_core::WorkflowVersionId::new(), &workflow)
        .unwrap();
    let catalog = Arc::new(stores.execution.plan_flavor_catalog());
    crate::PlanFlavorRevisionInstaller::new(catalog)
        .install(registry, &plan)
        .await
        .unwrap();
    let execution_id = ExecutionId::new();
    let mut state = ExecutionState::new(execution_id, workflow.id, &[]);
    state.set_revision_ids(plan.id(), registry.revision().id());
    state.set_budget(ExecutionBudget::default());
    state.set_workflow_input(serde_json::json!("admitted-input"));
    if materialize_bundle {
        stores.materialize_exact_state(&plan, state).await;
    } else {
        stores
            .inject_state(
                execution_id,
                workflow.id,
                serde_json::to_value(state).unwrap(),
            )
            .await;
    }
    (execution_id, workflow)
}
