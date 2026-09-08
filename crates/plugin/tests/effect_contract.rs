use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use nebula_action::{
    ActionContext, ActionError, ActionFactory, ActionHandle, ActionMetadata,
    effect::{
        ActionEffectContract, EffectPreparationContext, EffectPreparationError,
        PreparedRemoteEffect, RemoteDestinationGuarantee, RemoteEffectDescriptor,
        RemoteEffectFactory, RemoteEffectPolicy,
    },
};
use nebula_core::{ActionKey, ArtifactSetDigest, Dependencies, WorkflowVersionId, node_key};
use nebula_metadata::PluginManifest;
use nebula_plugin::{
    FrozenPluginRegistry, PlanActionEffectContract, Plugin, PluginError, PluginRegistry,
    ResolvedPlugin,
};
use nebula_workflow::{NodeDefinition, WorkflowBuilder};

struct RemoteFactory {
    metadata: ActionMetadata,
    dependencies: Dependencies,
    descriptor: RemoteEffectDescriptor,
    changed_descriptor: RemoteEffectDescriptor,
    changed: AtomicBool,
    expose_capability: bool,
    instantiations: AtomicUsize,
}

fn policy(
    destination_guarantee: RemoteDestinationGuarantee,
    maximum_invocations: u32,
    maximum_queries: u32,
    recovery_window_ms: u64,
) -> RemoteEffectPolicy {
    RemoteEffectPolicy::builder(destination_guarantee)
        .maximum_invocations(maximum_invocations)
        .maximum_queries(maximum_queries)
        .recovery_window(std::time::Duration::from_millis(recovery_window_ms))
        .build()
        .expect("the fixture policy must be coherent")
}

fn descriptor(contract_id: &str, invocations: u32) -> RemoteEffectDescriptor {
    RemoteEffectDescriptor::new(
        contract_id,
        1,
        policy(
            RemoteDestinationGuarantee::stable_key(60_000).unwrap(),
            invocations,
            2,
            60_000,
        ),
    )
    .unwrap()
}

impl RemoteFactory {
    fn new(contract: RemoteEffectDescriptor) -> Self {
        Self {
            metadata: ActionMetadata::new(
                ActionKey::new("demo.remote").unwrap(),
                "Remote",
                "No-I/O capability fixture",
            )
            .with_effect_contract(ActionEffectContract::Remote(Box::new(contract.clone()))),
            dependencies: Dependencies::new(),
            descriptor: contract,
            changed_descriptor: descriptor("changed.contract", 1),
            changed: AtomicBool::new(false),
            expose_capability: true,
            instantiations: AtomicUsize::new(0),
        }
    }
}

impl ActionFactory for RemoteFactory {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
    fn dependencies(&self) -> &Dependencies {
        &self.dependencies
    }
    fn remote_effect_factory(&self) -> Option<&dyn RemoteEffectFactory> {
        self.expose_capability.then_some(self)
    }
    fn instantiate<'a>(
        &'a self,
        _: &'a NodeDefinition,
        _: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
        self.instantiations.fetch_add(1, Ordering::Relaxed);
        Box::pin(async { Err(ActionError::fatal("must not instantiate during admission")) })
    }
}

#[async_trait::async_trait]
impl RemoteEffectFactory for RemoteFactory {
    fn descriptor(&self) -> &RemoteEffectDescriptor {
        if self.changed.load(Ordering::Relaxed) {
            &self.changed_descriptor
        } else {
            &self.descriptor
        }
    }
    async fn prepare(
        &self,
        _: serde_json::Value,
        _: &EffectPreparationContext,
    ) -> Result<PreparedRemoteEffect, EffectPreparationError> {
        Err(EffectPreparationError::InvalidRequest)
    }
}

struct FixturePlugin(Arc<RemoteFactory>);
impl std::fmt::Debug for FixturePlugin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FixturePlugin")
            .finish_non_exhaustive()
    }
}
impl Plugin for FixturePlugin {
    fn manifest(&self) -> &PluginManifest {
        static MANIFEST: std::sync::OnceLock<PluginManifest> = std::sync::OnceLock::new();
        MANIFEST.get_or_init(|| PluginManifest::builder("demo", "Demo").build().unwrap())
    }
    fn actions(&self) -> Vec<Arc<dyn ActionFactory>> {
        vec![self.0.clone()]
    }
}
fn freeze(factory: Arc<RemoteFactory>) -> FrozenPluginRegistry {
    let mut registry = PluginRegistry::new();
    registry
        .register(Arc::new(
            ResolvedPlugin::from(FixturePlugin(factory)).unwrap(),
        ))
        .unwrap();
    registry
        .freeze(
            ArtifactSetDigest::from_bytes([0x91; 32]),
            "1.0.0".parse().unwrap(),
        )
        .unwrap()
}

#[test]
fn incoherent_remote_capabilities_are_rejected_before_instantiation() {
    for mode in 0..3 {
        let mut factory = RemoteFactory::new(descriptor("demo.contract", 1));
        match mode {
            0 => factory.expose_capability = false,
            1 => factory.metadata.effect_contract = ActionEffectContract::NoExternalEffects,
            _ => factory.changed.store(true, Ordering::Relaxed),
        }
        let factory = Arc::new(factory);
        assert!(matches!(
            ResolvedPlugin::from(FixturePlugin(factory.clone())),
            Err(PluginError::InvalidEffectContract { .. })
        ));
        assert_eq!(factory.instantiations.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn exact_loading_rechecks_actual_remote_capability_without_instantiating() {
    let factory = Arc::new(RemoteFactory::new(descriptor("demo.contract", 1)));
    let registry = freeze(factory.clone());
    let workflow = WorkflowBuilder::new("Remote")
        .add_node(NodeDefinition::new(node_key!("remote"), "Remote", "demo", "remote").unwrap())
        .build()
        .unwrap();
    let plan = registry
        .compile_graph_v1(WorkflowVersionId::from_bytes([0x92; 16]), &workflow)
        .unwrap();
    plan.validate_against(&registry).unwrap();
    assert_eq!(
        plan.action_effect_contract(&ActionKey::new("demo.remote").unwrap())
            .unwrap(),
        PlanActionEffectContract::Declared(ActionEffectContract::Remote(Box::new(
            factory.descriptor.clone()
        )))
    );
    factory.changed.store(true, Ordering::Relaxed);
    assert!(plan.validate_against(&registry).is_err());
    assert_eq!(factory.instantiations.load(Ordering::Relaxed), 0);
}

#[test]
fn remote_non_stateless_actions_are_not_durably_compilable() {
    for kind in [
        nebula_action::ActionKind::Stateful,
        nebula_action::ActionKind::Control,
    ] {
        let mut factory = RemoteFactory::new(descriptor("demo.contract", 1));
        factory.metadata = factory.metadata.with_kind(kind);
        let registry = freeze(Arc::new(factory));
        let workflow = WorkflowBuilder::new("Remote")
            .add_node(NodeDefinition::new(node_key!("remote"), "Remote", "demo", "remote").unwrap())
            .build()
            .unwrap();
        let failure = registry
            .compile_graph_v1(WorkflowVersionId::from_bytes([0x94; 16]), &workflow)
            .expect_err("remote occurrences require a stateless action");
        assert!(
            failure
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code()
                    == "PLUGIN_PLAN_GRAPH_V1:UNSUPPORTED_EFFECT_KIND")
        );
    }
}

#[test]
fn complete_static_effect_policy_changes_plan_identity() {
    let workflow = WorkflowBuilder::new("Remote")
        .add_node(NodeDefinition::new(node_key!("remote"), "Remote", "demo", "remote").unwrap())
        .build()
        .unwrap();
    let mut identities = std::collections::HashSet::new();
    for contract in [
        descriptor("demo.contract", 1),
        descriptor("demo.contract", 2),
        descriptor("demo.other", 1),
        RemoteEffectDescriptor::new(
            "demo.contract",
            2,
            descriptor("demo.contract", 1).policy().clone(),
        )
        .unwrap(),
        RemoteEffectDescriptor::new(
            "demo.contract",
            1,
            policy(
                RemoteDestinationGuarantee::stable_key(60_000).unwrap(),
                1,
                3,
                60_000,
            ),
        )
        .unwrap(),
        RemoteEffectDescriptor::new(
            "demo.contract",
            1,
            policy(
                RemoteDestinationGuarantee::stable_key(60_000).unwrap(),
                1,
                2,
                70_000,
            ),
        )
        .unwrap(),
        RemoteEffectDescriptor::new(
            "demo.contract",
            1,
            policy(
                RemoteDestinationGuarantee::stable_key(50_000).unwrap(),
                1,
                2,
                60_000,
            ),
        )
        .unwrap(),
        RemoteEffectDescriptor::new(
            "demo.contract",
            1,
            policy(RemoteDestinationGuarantee::Reconcilable, 1, 2, 60_000),
        )
        .unwrap(),
        RemoteEffectDescriptor::new(
            "demo.contract",
            1,
            policy(RemoteDestinationGuarantee::Opaque, 1, 0, 60_000),
        )
        .unwrap(),
    ] {
        let registry = freeze(Arc::new(RemoteFactory::new(contract)));
        let plan = registry
            .compile_graph_v1(WorkflowVersionId::from_bytes([0x93; 16]), &workflow)
            .unwrap();
        assert!(identities.insert(plan.id()));
    }
}
