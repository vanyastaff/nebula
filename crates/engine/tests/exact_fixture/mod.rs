//! Real compile/install/pin fixtures shared by durable engine integration tests.

use std::{collections::BTreeMap, fmt, future::Future, pin::Pin, sync::Arc};

use nebula_action::{ActionContext, ActionError, ActionFactory, ActionHandle, ActionMetadata};
use nebula_core::{ActionKey, ArtifactSetDigest, Dependencies, WorkflowVersionId};
use nebula_engine::{ActionRegistry, PlanFlavorRevisionInstaller};
use nebula_execution::{ExecutionBudget, ExecutionState};
use nebula_plugin::{FrozenPluginRegistry, Plugin, PluginManifest, PluginRegistry, ResolvedPlugin};
use nebula_storage_port::{
    Scope, TransitionBatch,
    dto::{ContractBundleRecord, ControlCommand, ControlMsg, MaterializedStart, NewExecution},
    store::{ControlQueue, ExecutionStore, StartAcceptanceStore, StartContractIdentity},
};
use nebula_workflow::{NodeDefinition, WorkflowDefinition};

struct QualifiedFactory {
    metadata: ActionMetadata,
    inner: Arc<dyn ActionFactory>,
}

impl ActionFactory for QualifiedFactory {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
    fn dependencies(&self) -> &Dependencies {
        self.inner.dependencies()
    }
    fn instantiate<'a>(
        &'a self,
        node: &'a NodeDefinition,
        context: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
        self.inner.instantiate(node, context)
    }
}

struct FixturePlugin {
    manifest: PluginManifest,
    actions: Vec<Arc<dyn ActionFactory>>,
}

impl fmt::Debug for FixturePlugin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FixturePlugin")
            .finish_non_exhaustive()
    }
}

impl Plugin for FixturePlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
    fn actions(&self) -> Vec<Arc<dyn ActionFactory>> {
        self.actions.clone()
    }
}

/// Freeze actual registered factories after supplying their fixture namespace.
pub(crate) fn freeze_registry(
    registry: &ActionRegistry,
    actions: &[(&str, &str)],
) -> Arc<FrozenPluginRegistry> {
    let mut plugins: BTreeMap<&str, Vec<Arc<dyn ActionFactory>>> = BTreeMap::new();
    for &(plugin, local) in actions {
        let key = ActionKey::new(local).expect("fixture action key");
        let (mut metadata, inner) = registry
            .get_factory(&key)
            .expect("fixture factory registered");
        metadata.base.key = if local.starts_with(&format!("{plugin}.")) {
            key
        } else {
            ActionKey::new(format!("{plugin}.{local}")).expect("qualified fixture key")
        };
        if matches!(
            metadata.effect_contract,
            nebula_action::effect::ActionEffectContract::Undeclared
        ) {
            metadata.effect_contract =
                nebula_action::effect::ActionEffectContract::NoExternalEffects;
        }
        plugins
            .entry(plugin)
            .or_default()
            .push(Arc::new(QualifiedFactory { metadata, inner }));
    }
    let mut registry = PluginRegistry::new();
    for (plugin, actions) in plugins {
        registry
            .register(Arc::new(
                ResolvedPlugin::from(FixturePlugin {
                    manifest: PluginManifest::builder(plugin, plugin)
                        .build()
                        .expect("fixture manifest"),
                    actions,
                })
                .expect("fixture contracts resolve"),
            ))
            .expect("fixture registers once");
    }
    Arc::new(
        registry
            .freeze(
                ArtifactSetDigest::from_bytes([0x95; 32]),
                "1.0.0".parse().expect("runtime version"),
            )
            .expect("fixture registry freezes"),
    )
}

/// Admit a real cold execution, then install any requested recovery snapshot
/// through the execution owner's fenced commit. These fixtures drive manually,
/// so they acknowledge their admitted Start before testing later commands.
pub(crate) async fn materialize_state(
    execution: &nebula_storage::InMemoryExecutionStore,
    scope: &Scope,
    frozen: &FrozenPluginRegistry,
    workflow: &WorkflowDefinition,
    state: &mut ExecutionState,
) {
    let plan = frozen
        .compile_graph_v1(WorkflowVersionId::new(), workflow)
        .unwrap_or_else(|error| panic!("fixture compile diagnostics: {:?}", error.diagnostics()));
    PlanFlavorRevisionInstaller::new(Arc::new(execution.plan_flavor_catalog()))
        .install(frozen, &plan)
        .await
        .expect("fixture exact catalog install");
    state.set_revision_ids(plan.id(), frozen.revision().id());
    if state.budget.is_none() {
        state.set_budget(ExecutionBudget::default());
    }
    state.set_workflow_version_number(1);
    let bundle = nebula_execution::ExecutionContractBundle::new_graph_v1(
        nebula_core::ExecutionContractBundleId::new(),
        scope.org_id.parse().unwrap(),
        scope.workspace_id.parse().unwrap(),
        plan.id(),
        plan.plugin_set_id(),
        nebula_execution::ExecutionRevisions::new(
            plan.workflow_version_id(),
            frozen.revision().id(),
        ),
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
    let mut cold = ExecutionState::new(state.execution_id, state.workflow_id, &[]);
    cold.created_at = state.created_at;
    cold.updated_at = state.created_at;
    cold.set_revision_ids(plan.id(), frozen.revision().id());
    cold.set_workflow_version_number(1);
    cold.set_budget(ExecutionBudget::default());
    cold.workflow_input = state.workflow_input.clone();
    let cold_json = serde_json::to_value(&cold).unwrap();
    let workflow_id = state.workflow_id.to_string();
    let execution_id = state.execution_id.to_string();
    let command = ControlMsg {
        id: ulid::Ulid::new().to_bytes(),
        execution_id: execution_id.clone(),
        command: ControlCommand::Start,
        scope: scope.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    let starts = nebula_storage::inmem::InMemoryStartAcceptanceStore::new(execution);
    assert!(matches!(
        starts
            .materialize_start(&MaterializedStart::new(
                scope,
                None,
                &execution_id,
                NewExecution::new(&workflow_id, &cold_json),
                &command,
                &record
            ))
            .await
            .unwrap(),
        nebula_storage_port::store::StartMaterialization::Accepted { .. }
    ));
    let queue = nebula_storage::InMemoryControlQueue::new(execution);
    let claims = queue.claim_pending(&[0xF1; 16], 1).await.unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(
        claims[0].msg.id, command.id,
        "fixture must not consume another pending command"
    );
    queue.mark_completed(&claims[0].token).await.unwrap();
    let desired = serde_json::to_value(&state).unwrap();
    if desired != cold_json {
        let fencing = execution
            .acquire_lease(
                scope,
                &execution_id,
                "fixture-snapshot",
                std::time::Duration::from_secs(30),
            )
            .await
            .unwrap()
            .unwrap();
        let version = execution
            .get(scope, &execution_id)
            .await
            .unwrap()
            .unwrap()
            .version;
        execution
            .commit(
                TransitionBatch::builder()
                    .scope(scope.clone())
                    .execution_id(&execution_id)
                    .expected_version(version)
                    .fencing(fencing)
                    .new_state(desired)
                    .build()
                    .unwrap(),
            )
            .await
            .unwrap();
        execution
            .release_lease(scope, &execution_id, fencing)
            .await
            .unwrap();
    }
}
