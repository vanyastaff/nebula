use std::assert_matches;
use std::cell::RefCell;
use std::sync::{Arc, OnceLock};

use nebula_action::{
    Action, ActionFactory, ActionMetadata, RemoteEffectInstanceFactory,
    effect::{
        ActionEffectContract, EffectPreparationContext, EffectPreparationError,
        PreparedRemoteEffect, RemoteDestinationGuarantee, RemoteEffectAction,
        RemoteEffectDeclarationError, RemoteEffectDescriptor, RemoteEffectPolicy,
    },
};
use nebula_core::{ActionKey, ArtifactSetDigest, Dependencies, WorkflowVersionId, node_key};
use nebula_metadata::PluginManifest;
use nebula_plugin::{
    FrozenPluginRegistry, PlanActionEffectContract, Plugin, PluginRegistry, ResolvedPlugin,
};
use nebula_workflow::{NodeDefinition, WorkflowBuilder};

struct RemoteAction;

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
    const KEY: &'static str = "demo.remote";
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        let metadata = ActionMetadata::new(
            nebula_action::metadata_name!("Remote"),
            "No-I/O capability fixture",
        );
        match METADATA_CONTRACT.with_borrow(Clone::clone) {
            Some(contract) => metadata.with_effect_contract(contract),
            None => metadata,
        }
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl RemoteEffectAction for RemoteAction {
    fn descriptor() -> Result<RemoteEffectDescriptor, RemoteEffectDeclarationError> {
        Ok(DESCRIPTOR
            .with_borrow(Clone::clone)
            .expect("a descriptor is installed during fixture admission"))
    }

    async fn prepare(
        &self,
        _: serde_json::Value,
        _: &EffectPreparationContext,
    ) -> Result<PreparedRemoteEffect, EffectPreparationError> {
        Err(EffectPreparationError::InvalidRequest)
    }
}

thread_local! {
    /// The descriptor `RemoteAction::descriptor` returns.
    ///
    /// Admission reads the remote contract only from the type; these tests
    /// vary it, so they install it before building a factory.
    static DESCRIPTOR: RefCell<Option<RemoteEffectDescriptor>> = const { RefCell::new(None) };
    /// An effect contract `RemoteAction::metadata` declares, which admission
    /// must refuse.
    static METADATA_CONTRACT: RefCell<Option<ActionEffectContract>> = const { RefCell::new(None) };
}

fn admit(
    descriptor: RemoteEffectDescriptor,
    metadata_contract: Option<ActionEffectContract>,
) -> Result<RemoteEffectInstanceFactory<RemoteAction>, nebula_action::ActionMetadataAdmissionError>
{
    DESCRIPTOR.set(Some(descriptor));
    METADATA_CONTRACT.set(metadata_contract);
    let factory = RemoteEffectInstanceFactory::new(RemoteAction);
    DESCRIPTOR.set(None);
    METADATA_CONTRACT.set(None);
    factory
}

fn remote_factory(contract: RemoteEffectDescriptor) -> RemoteEffectInstanceFactory<RemoteAction> {
    admit(contract, None).expect("coherent remote fixture admits")
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
fn remote_contract_declared_in_metadata_is_rejected_before_instantiation() {
    let contract = descriptor("demo.contract", 1);
    assert_matches!(
        admit(
            contract.clone(),
            Some(ActionEffectContract::Remote(Box::new(contract.clone())))
        )
        .map(drop),
        Err(nebula_action::ActionMetadataAdmissionError::RemoteEffectContractInMetadata)
    );
    assert_matches!(
        admit(contract, Some(ActionEffectContract::ReadOnly)).map(drop),
        Err(nebula_action::ActionMetadataAdmissionError::EffectContractOnRemoteAction)
    );
}

#[test]
fn exact_loading_rechecks_actual_remote_capability_without_instantiating() {
    let contract = descriptor("demo.contract", 1);
    let registry = freeze(Arc::new(remote_factory(contract.clone())));
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
    // The same action type built with another descriptor is a different
    // catalog entry: the plan no longer loads against it.
    let changed = freeze(Arc::new(remote_factory(descriptor("changed.contract", 1))));
    assert!(plan.validate_against(&changed).is_err());
}

#[test]
fn remote_non_stateless_actions_are_not_durably_compilable() {
    let factory = remote_factory(descriptor("demo.contract", 1));
    assert_eq!(
        ActionFactory::definition(&factory).kind(),
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
        let factory = remote_factory(contract);
        let registry = freeze(Arc::new(factory));
        let plan = registry
            .compile_graph_v1(WorkflowVersionId::from_bytes([0x93; 16]), &workflow)
            .unwrap();
        assert!(identities.insert(plan.id()));
    }
}
