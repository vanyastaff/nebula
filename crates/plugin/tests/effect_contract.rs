use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

use nebula_action::{
    Action, ActionFactory, ActionMetadataDraft, RemoteEffectInstanceFactory,
    effect::{
        ActionEffectContract, EffectPreparationContext, EffectPreparationError,
        PreparedRemoteEffect, RemoteDestinationGuarantee, RemoteEffectAction,
        RemoteEffectDescriptor, RemoteEffectPolicy,
    },
};
use nebula_core::{ActionKey, ArtifactSetDigest, Dependencies, WorkflowVersionId, node_key};
use nebula_metadata::PluginManifest;
use nebula_plugin::{
    FrozenPluginRegistry, PlanActionEffectContract, Plugin, PluginRegistry, ResolvedPlugin,
};
use nebula_workflow::{NodeDefinition, WorkflowBuilder};

struct RemoteAction {
    descriptor: RemoteEffectDescriptor,
    changed_descriptor: RemoteEffectDescriptor,
    changed: Arc<AtomicBool>,
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

impl Action for RemoteAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        remote_draft(&descriptor("demo.contract", 1))
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl RemoteEffectAction for RemoteAction {
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

fn remote_draft(contract: &RemoteEffectDescriptor) -> ActionMetadataDraft {
    ActionMetadataDraft::new(
        ActionKey::new("demo.remote").unwrap(),
        nebula_action::metadata_name!("Remote"),
        "No-I/O capability fixture",
    )
    .with_effect_contract(ActionEffectContract::Remote(Box::new(contract.clone())))
}

fn remote_factory(
    contract: RemoteEffectDescriptor,
) -> (RemoteEffectInstanceFactory<RemoteAction>, Arc<AtomicBool>) {
    let changed = Arc::new(AtomicBool::new(false));
    let action = RemoteAction {
        descriptor: contract.clone(),
        changed_descriptor: descriptor("changed.contract", 1),
        changed: Arc::clone(&changed),
    };
    let factory = RemoteEffectInstanceFactory::new(remote_draft(&contract), action)
        .expect("coherent remote fixture admits");
    (factory, changed)
}

struct FixturePlugin(Arc<dyn ActionFactory>);
impl std::fmt::Debug for FixturePlugin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FixturePlugin")
            .finish_non_exhaustive()
    }
}
impl Plugin for FixturePlugin {
    fn manifest(&self) -> &PluginManifest {
        static MANIFEST: OnceLock<PluginManifest> = OnceLock::new();
        MANIFEST.get_or_init(|| PluginManifest::builder("demo", "Demo").build().unwrap())
    }
    fn actions(&self) -> Vec<Arc<dyn ActionFactory>> {
        vec![self.0.clone()]
    }
}
fn freeze(factory: Arc<dyn ActionFactory>) -> FrozenPluginRegistry {
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
    let contract = descriptor("demo.contract", 1);
    let missing_contract = ActionMetadataDraft::new(
        ActionKey::new("demo.remote").unwrap(),
        nebula_action::metadata_name!("Remote"),
        "No-I/O capability fixture",
    );
    let action = RemoteAction {
        descriptor: contract.clone(),
        changed_descriptor: descriptor("changed.contract", 1),
        changed: Arc::new(AtomicBool::new(false)),
    };
    assert!(RemoteEffectInstanceFactory::new(missing_contract, action).is_err());

    let mismatched = descriptor("different.contract", 1);
    let action = RemoteAction {
        descriptor: contract,
        changed_descriptor: descriptor("changed.contract", 1),
        changed: Arc::new(AtomicBool::new(false)),
    };
    assert!(RemoteEffectInstanceFactory::new(remote_draft(&mismatched), action).is_err());
}

#[test]
fn exact_loading_rechecks_actual_remote_capability_without_instantiating() {
    let contract = descriptor("demo.contract", 1);
    let (factory, changed) = remote_factory(contract.clone());
    let factory = Arc::new(factory);
    let registry = freeze(factory);
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
        PlanActionEffectContract::Declared(ActionEffectContract::Remote(Box::new(contract)))
    );
    changed.store(true, Ordering::Relaxed);
    assert!(plan.validate_against(&registry).is_err());
}

#[test]
fn remote_non_stateless_actions_are_not_durably_compilable() {
    let (factory, _) = remote_factory(descriptor("demo.contract", 1));
    assert_eq!(
        ActionFactory::metadata(&factory).kind(),
        nebula_action::ActionKind::Stateless
    );
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
        let (factory, _) = remote_factory(contract);
        let registry = freeze(Arc::new(factory));
        let plan = registry
            .compile_graph_v1(WorkflowVersionId::from_bytes([0x93; 16]), &workflow)
            .unwrap();
        assert!(identities.insert(plan.id()));
    }
}
