//! Factories for admission-boundary integration tests.

use std::{
    marker::PhantomData,
    sync::{Arc, OnceLock},
};

use nebula_action::effect::{
    ActionEffectContract, EffectPreparationContext, EffectPreparationError, PreparedRemoteEffect,
    RemoteDestinationGuarantee, RemoteEffectAction, RemoteEffectDescriptor, RemoteEffectPolicy,
};
use nebula_action::{
    Action, ActionContext, ActionError, ActionFactory, ActionMetadataDraft, ActionResult,
    FromWorkflowNode, GenericTriggerFactory, InstanceFactory, RemoteEffectInstanceFactory,
    StatelessAction, TriggerAction, TriggerContext, TriggerEventOutcome, TriggerSource,
};
use nebula_core::{ArtifactSetDigest, Dependencies};
use nebula_schema::{HasSchema, Schema, SecretField, ValidSchema, field_key};
use nebula_workflow::NodeDefinition;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{FrozenPluginRegistry, Plugin, PluginManifest, PluginRegistry, ResolvedPlugin};

struct DiagnosticAction<Input>(PhantomData<fn() -> Input>);

impl<Input> DiagnosticAction<Input> {
    const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<Input> Action for DiagnosticAction<Input>
where
    Input: HasSchema + DeserializeOwned + Send + Sync + 'static,
{
    type Input = Input;
    type Output = Value;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            nebula_core::action_key!("fixture.instance_backing_type"),
            nebula_action::metadata_name!("Instance backing type"),
            "Typed backing action for per-registration diagnostic metadata",
        )
    }

    fn dependencies() -> &'static Dependencies {
        empty_dependencies()
    }
}

impl<Input> StatelessAction for DiagnosticAction<Input>
where
    Input: HasSchema + DeserializeOwned + Send + Sync + 'static,
{
    async fn execute(
        &self,
        _input: Input,
        _context: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        Err(ActionError::fatal(
            "diagnostic admission fixture cannot execute",
        ))
    }
}

fn stateless_fixture<Input>(draft: ActionMetadataDraft) -> Arc<dyn ActionFactory>
where
    Input: HasSchema + DeserializeOwned + Send + Sync + 'static,
{
    Arc::new(
        InstanceFactory::new(draft, DiagnosticAction::<Input>::new())
            .expect("static diagnostic action contract must admit"),
    )
}

trait FixtureDependencies: Send + Sync + 'static {
    fn get() -> &'static Dependencies;
}

struct DependencyAction<Provider>(PhantomData<fn() -> Provider>);

impl<Provider> Action for DependencyAction<Provider>
where
    Provider: FixtureDependencies,
{
    type Input = Value;
    type Output = Value;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            nebula_core::action_key!("fixture.dependency_backing_type"),
            nebula_action::metadata_name!("Dependency backing type"),
            "Typed backing action for dependency diagnostics",
        )
    }

    fn dependencies() -> &'static Dependencies {
        Provider::get()
    }
}

impl<Provider> StatelessAction for DependencyAction<Provider>
where
    Provider: FixtureDependencies,
{
    async fn execute(
        &self,
        _input: Value,
        _context: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        Err(ActionError::fatal(
            "dependency diagnostic fixture cannot execute",
        ))
    }
}

fn dependency_fixture<Provider>(draft: ActionMetadataDraft) -> Arc<dyn ActionFactory>
where
    Provider: FixtureDependencies,
{
    Arc::new(
        InstanceFactory::new(draft, DependencyAction::<Provider>(PhantomData))
            .expect("static dependency diagnostic contract must admit"),
    )
}

struct ResourceSlotDependencies;

impl FixtureDependencies for ResourceSlotDependencies {
    fn get() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(|| {
            Dependencies::new().slot_field(nebula_core::SlotField {
                slot_key: "binding",
                default_id: "fixture",
                kind: nebula_core::SlotKind::Resource {
                    type_id: std::any::TypeId::of::<()>(),
                    type_name: "FixtureResource",
                    key: "core.absent".parse().unwrap(),
                },
                required: true,
                lazy: false,
                purpose: None,
            })
        })
    }
}

struct CredentialSlotDependencies;

impl FixtureDependencies for CredentialSlotDependencies {
    fn get() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(|| {
            Dependencies::new().slot_field(nebula_core::SlotField {
                slot_key: "binding",
                default_id: "fixture",
                kind: nebula_core::SlotKind::Credential {
                    type_id: std::any::TypeId::of::<()>(),
                    type_name: "FixtureCredential",
                    key: "core.absent".parse().unwrap(),
                },
                required: true,
                lazy: false,
                purpose: None,
            })
        })
    }
}

struct WrongResourceTypeDependencies;

impl FixtureDependencies for WrongResourceTypeDependencies {
    fn get() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(|| {
            Dependencies::new().resource(nebula_core::ResourceRequirement::new(
                "provider.resident".parse().unwrap(),
                std::any::TypeId::of::<u8>(),
                "FixtureResource",
            ))
        })
    }
}

struct UndeclaredResourceDependencies;

impl FixtureDependencies for UndeclaredResourceDependencies {
    fn get() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(|| {
            Dependencies::new().resource(nebula_core::ResourceRequirement::new(
                "provider.resident".parse().unwrap(),
                std::any::TypeId::of::<FixtureResource>(),
                "FixtureResource",
            ))
        })
    }
}

#[derive(serde::Deserialize)]
struct StringInput {
    #[serde(rename = "value")]
    _value: String,
}

impl HasSchema for StringInput {
    fn schema() -> Result<ValidSchema, nebula_schema::ValidationReport> {
        Schema::builder()
            .add(nebula_schema::Field::string(field_key!("value")).required())
            .build()
    }
}

#[derive(serde::Deserialize)]
struct SecretInput {
    #[serde(rename = "token")]
    _token: String,
}

impl HasSchema for SecretInput {
    fn schema() -> Result<ValidSchema, nebula_schema::ValidationReport> {
        Schema::builder()
            .add(SecretField::new(field_key!("token")))
            .build()
    }
}

struct DiagnosticSource;

impl TriggerSource for DiagnosticSource {
    type Event = Value;
}

macro_rules! diagnostic_trigger {
    ($trigger:ident, $input:ty, $key:literal, $name:literal) => {
        struct $trigger;

        impl Action for $trigger {
            type Input = $input;
            type Output = Value;

            fn metadata() -> ActionMetadataDraft {
                ActionMetadataDraft::new(
                    nebula_core::action_key!($key),
                    nebula_action::metadata_name!($name),
                    "Activation diagnostic trigger fixture",
                )
                .with_effect_contract(ActionEffectContract::NoExternalEffects)
            }

            fn dependencies() -> &'static Dependencies {
                empty_dependencies()
            }
        }

        impl FromWorkflowNode for $trigger {
            type Error = ActionError;

            async fn from_workflow_node<'a>(
                _node: &'a NodeDefinition,
                _context: &'a dyn ActionContext,
            ) -> Result<Self, Self::Error> {
                Ok(Self)
            }
        }

        impl TriggerAction for $trigger {
            type Source = DiagnosticSource;
            type Error = ActionError;

            async fn start(
                &self,
                _context: &(impl TriggerContext + ?Sized),
            ) -> Result<(), Self::Error> {
                Ok(())
            }

            async fn stop(
                &self,
                _context: &(impl TriggerContext + ?Sized),
            ) -> Result<(), Self::Error> {
                Ok(())
            }

            async fn handle(
                &self,
                _context: &(impl TriggerContext + ?Sized),
                _event: Value,
            ) -> Result<TriggerEventOutcome, Self::Error> {
                Err(ActionError::fatal(
                    "diagnostic trigger does not accept external events",
                ))
            }
        }
    };
}

diagnostic_trigger!(DiagnosticTrigger, Value, "core.trigger", "trigger");
diagnostic_trigger!(
    SecretDiagnosticTrigger,
    SecretInput,
    "core.secret_trigger",
    "secret_trigger"
);

struct DiagnosticRemoteAction {
    descriptor: RemoteEffectDescriptor,
}

impl Action for DiagnosticRemoteAction {
    type Input = Value;
    type Output = Value;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            nebula_core::action_key!("fixture.remote_backing_type"),
            nebula_action::metadata_name!("Remote backing type"),
            "Typed remote action for diagnostic registration",
        )
    }

    fn dependencies() -> &'static Dependencies {
        empty_dependencies()
    }
}

impl RemoteEffectAction for DiagnosticRemoteAction {
    fn descriptor(&self) -> &RemoteEffectDescriptor {
        &self.descriptor
    }

    async fn prepare(
        &self,
        _input: Self::Input,
        _context: &EffectPreparationContext,
    ) -> Result<PreparedRemoteEffect, EffectPreparationError> {
        Err(EffectPreparationError::Unavailable)
    }
}

fn empty_dependencies() -> &'static Dependencies {
    static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
    DEPENDENCIES.get_or_init(Dependencies::new)
}

struct FixturePlugin {
    manifest: PluginManifest,
    actions: Vec<Arc<dyn ActionFactory>>,
}

impl std::fmt::Debug for FixturePlugin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

    fn resources(&self) -> Vec<Arc<dyn nebula_resource::ResourceFactory>> {
        if self.manifest.key().as_str() == "provider" {
            vec![Arc::new(nebula_resource::KindActivator::<
                FixtureResource,
                _,
                _,
            >::new(
                || FixtureResource,
                || nebula_resource::Resident::new(nebula_resource::ResidentConfig::default()),
            ))]
        } else {
            vec![]
        }
    }
}

#[derive(Clone)]
struct FixtureResource;

impl nebula_resource::HasCredentialSlots for FixtureResource {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }

    fn declares_credential_slots() -> bool {
        false
    }
}

impl nebula_core::DeclaresDependencies for FixtureResource {}

#[async_trait::async_trait]
impl nebula_resource::Provider for FixtureResource {
    type Config = ();
    type Instance = ();
    type Topology = nebula_resource::Resident<Self>;

    fn key() -> nebula_core::ResourceKey {
        "provider.resident".parse().unwrap()
    }

    fn metadata() -> nebula_resource::ResourceMetadataDraft {
        nebula_resource::ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("Resident"),
            "Admission-only resource",
        )
    }

    async fn create(
        &self,
        _config: &(),
        _context: &nebula_resource::ResourceContext,
    ) -> Result<(), nebula_resource::Error> {
        panic!("diagnostic admission must not construct resources")
    }
}

#[async_trait::async_trait]
impl nebula_resource::ResidentProvider for FixtureResource {}

/// Builds a frozen registry covering activation-diagnostic rejection paths.
///
/// The factories fail closed if compilation crosses into runtime construction,
/// keeping this fixture limited to admission behavior.
#[must_use]
pub fn activation_diagnostic_registry() -> Arc<FrozenPluginRegistry> {
    let descriptor = RemoteEffectDescriptor::new(
        "activation.remote",
        1,
        RemoteEffectPolicy::builder(RemoteDestinationGuarantee::Opaque)
            .maximum_invocations(1)
            .maximum_queries(0)
            .recovery_window(std::time::Duration::from_mins(1))
            .build()
            .unwrap(),
    )
    .unwrap();
    let remote = RemoteEffectInstanceFactory::new(
        ActionMetadataDraft::new(
            nebula_core::action_key!("core.remote_control"),
            nebula_action::metadata_name!("remote_control"),
            "Remote effect diagnostic fixture",
        )
        .with_effect_contract(ActionEffectContract::Remote(Box::new(descriptor.clone()))),
        DiagnosticRemoteAction { descriptor },
    )
    .expect("static remote diagnostic contract must admit");

    let mut actions: Vec<Arc<dyn ActionFactory>> = vec![
        stateless_fixture::<Value>(
            ActionMetadataDraft::new(
                nebula_core::action_key!("core.echo"),
                nebula_action::metadata_name!("echo"),
                "Admission fixture",
            )
            .with_effect_contract(ActionEffectContract::NoExternalEffects),
        ),
        stateless_fixture::<Value>(ActionMetadataDraft::new(
            nebula_core::action_key!("core.undeclared"),
            nebula_action::metadata_name!("undeclared"),
            "Admission fixture",
        )),
        Arc::new(remote),
        Arc::new(
            GenericTriggerFactory::<DiagnosticTrigger>::new()
                .expect("static trigger diagnostic contract must admit"),
        ),
        stateless_fixture::<StringInput>(
            ActionMetadataDraft::new(
                nebula_core::action_key!("core.string_input"),
                nebula_action::metadata_name!("string_input"),
                "Schema admission fixture",
            )
            .with_effect_contract(ActionEffectContract::NoExternalEffects),
        ),
        Arc::new(
            GenericTriggerFactory::<SecretDiagnosticTrigger>::new()
                .expect("static secret trigger diagnostic contract must admit"),
        ),
    ];

    actions.push(dependency_fixture::<ResourceSlotDependencies>(
        ActionMetadataDraft::new(
            nebula_core::action_key!("core.resource_slot"),
            nebula_action::metadata_name!("resource_slot"),
            "Unresolved resource slot fixture",
        )
        .with_effect_contract(ActionEffectContract::NoExternalEffects),
    ));
    actions.push(dependency_fixture::<CredentialSlotDependencies>(
        ActionMetadataDraft::new(
            nebula_core::action_key!("core.credential_slot"),
            nebula_action::metadata_name!("credential_slot"),
            "Unresolved credential slot fixture",
        )
        .with_effect_contract(ActionEffectContract::NoExternalEffects),
    ));

    for key in [
        nebula_action::metadata_name!("tag_filter"),
        nebula_action::metadata_name!("node_filter"),
        nebula_action::metadata_name!("support_required"),
        nebula_action::metadata_name!("invalid_projection"),
    ] {
        let mut port = nebula_action::InputPort::support(
            nebula_action::port_key!("support"),
            "Support",
            "Fixture support",
        );
        if let nebula_action::InputPort::Support(support) = &mut port {
            support.required = key.as_str() == "support_required";
            match key.as_str() {
                "tag_filter" => support.filter.allowed_tags = Some(vec!["fixture".to_owned()]),
                "node_filter" => {
                    support.filter.allowed_node_types = Some(vec!["core.absent".to_owned()]);
                },
                "invalid_projection" => support.filter.allowed_tags = Some(vec![]),
                _ => {},
            }
        }
        actions.push(stateless_fixture::<Value>(
            ActionMetadataDraft::new(format!("core.{key}").parse().unwrap(), key, "Port fixture")
                .with_effect_contract(ActionEffectContract::NoExternalEffects)
                .with_inputs(vec![
                    nebula_action::InputPort::flow(nebula_action::port_key!("in")),
                    port,
                ]),
        ));
    }

    actions.push(dependency_fixture::<WrongResourceTypeDependencies>(
        ActionMetadataDraft::new(
            nebula_core::action_key!("core.wrong_dependency_type"),
            nebula_action::metadata_name!("wrong_dependency_type"),
            "Wrong dependency type fixture",
        )
        .with_effect_contract(ActionEffectContract::NoExternalEffects),
    ));
    actions.push(dependency_fixture::<UndeclaredResourceDependencies>(
        ActionMetadataDraft::new(
            nebula_core::action_key!("core.undeclared_dependency"),
            nebula_action::metadata_name!("undeclared_dependency"),
            "Undeclared dependency fixture",
        )
        .with_effect_contract(ActionEffectContract::NoExternalEffects),
    ));

    let mut registry = PluginRegistry::new();
    registry
        .register(Arc::new(
            ResolvedPlugin::from(FixturePlugin {
                manifest: PluginManifest::builder("provider", "Resource provider")
                    .build()
                    .unwrap(),
                actions: vec![],
            })
            .unwrap(),
        ))
        .unwrap();
    registry
        .register(Arc::new(
            ResolvedPlugin::from(FixturePlugin {
                manifest: PluginManifest::builder("core", "activation diagnostic fixture")
                    .build()
                    .unwrap(),
                actions,
            })
            .unwrap(),
        ))
        .unwrap();
    Arc::new(
        registry
            .freeze(
                ArtifactSetDigest::from_bytes([0x14; 32]),
                "1.0.0".parse().unwrap(),
            )
            .unwrap(),
    )
}
