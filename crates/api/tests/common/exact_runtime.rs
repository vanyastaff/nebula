//! Explicit frozen action contracts for the HTTP workflow fixtures.

use std::sync::Arc;

use nebula_action::{Action, ActionMetadata, ActionResult, InstanceFactory, StatelessAction};
use nebula_engine::{
    PlanFlavorRevisionInstaller, PlanFlavorRevisionLoader, WorkflowActivationService,
    WorkflowStartService, WorkflowStores,
};
use nebula_plugin::{Plugin, PluginManifest, PluginRegistry, ResolvedPlugin};
use nebula_storage::{InMemoryExecutionStore, InMemoryWorkflowStore};
use nebula_storage_port::store::WorkflowVersionStore;

#[derive(Debug)]
struct Echo;

impl Action for Echo {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            nebula_core::action_key!("core.echo"),
            "Echo",
            "HTTP fixture echo",
        )
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
        .with_kind(nebula_action::ActionKind::Stateless)
    }

    fn dependencies() -> &'static nebula_core::Dependencies {
        static DEPENDENCIES: std::sync::OnceLock<nebula_core::Dependencies> =
            std::sync::OnceLock::new();
        DEPENDENCIES.get_or_init(nebula_core::Dependencies::new)
    }
}

impl StatelessAction for Echo {
    async fn execute(
        &self,
        input: Self::Input,
        _context: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, nebula_action::ActionError> {
        Ok(ActionResult::success(input))
    }
}

#[derive(Debug)]
struct FixturePlugin {
    manifest: PluginManifest,
    slow_started: Arc<tokio::sync::Notify>,
}

impl Plugin for FixturePlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn actions(&self) -> Vec<Arc<dyn nebula_action::ActionFactory>> {
        vec![
            Arc::new(InstanceFactory::new(Echo::metadata(), Echo)),
            Arc::new(InstanceFactory::new(
                super::engine_seam::SlowAction::metadata(),
                super::engine_seam::SlowAction {
                    started: Arc::clone(&self.slow_started),
                },
            )),
        ]
    }
}

pub(crate) struct RuntimeFixture {
    pub(crate) registry: Arc<nebula_plugin::FrozenPluginRegistry>,
    pub(crate) loader: Arc<PlanFlavorRevisionLoader>,
    pub(crate) slow_started: Arc<tokio::sync::Notify>,
}

pub(super) fn services(
    workflows: &InMemoryWorkflowStore,
    versions: Arc<dyn WorkflowVersionStore>,
    executions: &InMemoryExecutionStore,
) -> (
    Arc<WorkflowActivationService>,
    Arc<WorkflowStartService>,
    RuntimeFixture,
) {
    let mut registry = PluginRegistry::new();
    let slow_started = Arc::new(tokio::sync::Notify::new());
    registry
        .register(Arc::new(
            ResolvedPlugin::from(FixturePlugin {
                manifest: PluginManifest::builder("core", "HTTP fixture")
                    .build()
                    .unwrap(),
                slow_started: Arc::clone(&slow_started),
            })
            .unwrap(),
        ))
        .unwrap();
    let frozen = Arc::new(
        registry
            .freeze(
                nebula_core::ArtifactSetDigest::from_bytes([0x63; 32]),
                "1.0.0".parse().unwrap(),
            )
            .unwrap(),
    );
    let activation = Arc::new(WorkflowActivationService::new(
        Arc::new(workflows.clone()),
        Arc::clone(&versions),
        Arc::clone(&frozen),
        PlanFlavorRevisionInstaller::new(Arc::new(executions.plan_flavor_catalog())),
        Arc::new(nebula_core::accessor::SystemClock),
    ));
    let start = Arc::new(
        WorkflowStartService::new(
            WorkflowStores {
                workflow: Arc::new(workflows.clone()),
                versions,
            },
            Arc::new(executions.clone()),
            Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                executions,
            )),
            PlanFlavorRevisionLoader::new(Arc::new(executions.plan_flavor_catalog())),
            Arc::clone(&frozen),
            Arc::new(nebula_core::accessor::SystemClock),
            Default::default(),
        )
        .unwrap(),
    );
    (
        activation,
        start,
        RuntimeFixture {
            registry: frozen,
            loader: Arc::new(PlanFlavorRevisionLoader::new(Arc::new(
                executions.plan_flavor_catalog(),
            ))),
            slow_started,
        },
    )
}
