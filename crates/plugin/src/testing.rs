//! Factories for admission-boundary integration tests.

use std::sync::{Arc, OnceLock};

use nebula_action::effect::{
    ActionEffectContract, EffectPreparationContext, EffectPreparationError, PreparedRemoteEffect,
    RemoteDestinationGuarantee, RemoteEffectAction, RemoteEffectDeclarationError,
    RemoteEffectDescriptor, RemoteEffectPolicy,
};
use nebula_action::{
    Action, ActionContext, ActionError, ActionFactory, ActionMetadata, ActionResult,
    FromWorkflowNode, GenericTriggerFactory, InstanceFactory, RemoteEffectInstanceFactory,
    StatelessAction, TriggerAction, TriggerContext, TriggerEventOutcome, TriggerSource,
};
use nebula_core::{ArtifactSetDigest, Dependencies};
use nebula_schema::{HasSchema, Schema, SecretField, ValidSchema, field_key};
use nebula_workflow::NodeDefinition;
use serde_json::Value;

use crate::{FrozenPluginRegistry, Plugin, PluginManifest, PluginRegistry, ResolvedPlugin};

/// Declare a stateless fixture action that fails closed if it ever executes.
///
/// Each fixture is its own type, so its key and metadata are the type's own.
macro_rules! diagnostic_stateless {
    ($name:ident, $input:ty, $dependencies:expr, $key:literal, $metadata:expr $(,)?) => {
        struct $name;

        impl Action for $name {
            const KEY: &'static str = $key;
            type Input = $input;
            type Output = Value;

            fn metadata() -> ActionMetadata {
                $metadata
            }

            fn dependencies() -> &'static Dependencies {
                $dependencies
            }
        }

        impl StatelessAction for $name {
            async fn execute(
                &self,
                _input: $input,
                _context: &(impl ActionContext + ?Sized),
            ) -> Result<ActionResult<Value>, ActionError> {
                Err(ActionError::fatal(
                    "diagnostic admission fixture cannot execute",
                ))
            }
        }
    };
}

fn instance<A>(action: A) -> Arc<dyn ActionFactory>
where
    A: StatelessAction,
{
    Arc::new(InstanceFactory::new(action).expect("static diagnostic action contract must admit"))
}

fn read_only(name: nebula_action::MetadataName, description: &str) -> ActionMetadata {
    ActionMetadata::new(name, description).with_effect_contract(ActionEffectContract::ReadOnly)
}

fn port_fixture(name: nebula_action::MetadataName) -> ActionMetadata {
    let mut port = nebula_action::InputPort::support(
        nebula_action::port_key!("support"),
        "Support",
        "Fixture support",
    );
    if let nebula_action::InputPort::Support(support) = &mut port {
        support.required = name.as_str() == "support_required";
        match name.as_str() {
            "tag_filter" => support.filter.allowed_tags = Some(vec!["fixture".to_owned()]),
            "node_filter" => {
                support.filter.allowed_node_types = Some(vec!["core.absent".to_owned()]);
            },
            "invalid_projection" => support.filter.allowed_tags = Some(vec![]),
            _ => {},
        }
    }
    read_only(name, "Port fixture").with_inputs(vec![
        nebula_action::InputPort::flow(nebula_action::port_key!("in")),
        port,
    ])
}

diagnostic_stateless!(
    EchoFixture,
    Value,
    empty_dependencies(),
    "core.echo",
    read_only(nebula_action::metadata_name!("echo"), "Admission fixture"),
);
diagnostic_stateless!(
    StringInputFixture,
    StringInput,
    empty_dependencies(),
    "core.string_input",
    read_only(
        nebula_action::metadata_name!("string_input"),
        "Schema admission fixture"
    ),
);
diagnostic_stateless!(
    ResourceSlotFixture,
    Value,
    ResourceSlotDependencies::get(),
    "core.resource_slot",
    read_only(
        nebula_action::metadata_name!("resource_slot"),
        "Unresolved resource slot fixture"
    ),
);
diagnostic_stateless!(
    CredentialSlotFixture,
    Value,
    CredentialSlotDependencies::get(),
    "core.credential_slot",
    read_only(
        nebula_action::metadata_name!("credential_slot"),
        "Unresolved credential slot fixture"
    ),
);
diagnostic_stateless!(
    TagFilterFixture,
    Value,
    empty_dependencies(),
    "core.tag_filter",
    port_fixture(nebula_action::metadata_name!("tag_filter")),
);
diagnostic_stateless!(
    NodeFilterFixture,
    Value,
    empty_dependencies(),
    "core.node_filter",
    port_fixture(nebula_action::metadata_name!("node_filter")),
);
diagnostic_stateless!(
    SupportRequiredFixture,
    Value,
    empty_dependencies(),
    "core.support_required",
    port_fixture(nebula_action::metadata_name!("support_required")),
);
diagnostic_stateless!(
    InvalidProjectionFixture,
    Value,
    empty_dependencies(),
    "core.invalid_projection",
    port_fixture(nebula_action::metadata_name!("invalid_projection")),
);
diagnostic_stateless!(
    WrongDependencyTypeFixture,
    Value,
    WrongResourceTypeDependencies::get(),
    "core.wrong_dependency_type",
    read_only(
        nebula_action::metadata_name!("wrong_dependency_type"),
        "Wrong dependency type fixture"
    ),
);
diagnostic_stateless!(
    UndeclaredDependencyFixture,
    Value,
    UndeclaredResourceDependencies::get(),
    "core.undeclared_dependency",
    read_only(
        nebula_action::metadata_name!("undeclared_dependency"),
        "Undeclared dependency fixture"
    ),
);

trait FixtureDependencies: Send + Sync + 'static {
    fn get() -> &'static Dependencies;
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
            .property(nebula_schema::Property::string(field_key!("value")).required())
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
            .property(SecretField::new(field_key!("token")))
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
            const KEY: &'static str = $key;
            type Input = $input;
            type Output = Value;

            fn metadata() -> ActionMetadata {
                ActionMetadata::new(
                    nebula_action::metadata_name!($name),
                    "Activation diagnostic trigger fixture",
                )
                .with_effect_contract(ActionEffectContract::ReadOnly)
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

struct DiagnosticRemoteAction;

impl Action for DiagnosticRemoteAction {
    const KEY: &'static str = "core.remote_control";
    type Input = Value;
    type Output = Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            nebula_action::metadata_name!("remote_control"),
            "Remote effect diagnostic fixture",
        )
    }

    fn dependencies() -> &'static Dependencies {
        empty_dependencies()
    }
}

impl RemoteEffectAction for DiagnosticRemoteAction {
    fn descriptor() -> Result<RemoteEffectDescriptor, RemoteEffectDeclarationError> {
        Ok(RemoteEffectDescriptor::new(
            "activation.remote",
            1,
            RemoteEffectPolicy::builder(RemoteDestinationGuarantee::Opaque)
                .maximum_invocations(1)
                .maximum_queries(0)
                .recovery_window(std::time::Duration::from_mins(1))
                .build()?,
        )?)
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
                nebula_resource::topology::fixed(|| {
                    nebula_resource::Resident::new(nebula_resource::ResidentConfig::default())
                }),
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

impl nebula_resource::Component for FixtureResource {
    type Kind = nebula_resource::kind::Resource;
    type Config = ();

    const KEY: &'static str = "provider.resident";

    fn metadata() -> nebula_resource::Metadata {
        nebula_resource::Metadata::new(
            nebula_resource::metadata_name!("Resident"),
            "Admission-only resource",
        )
    }
}

#[async_trait::async_trait]
impl nebula_resource::Provider for FixtureResource {
    type Instance = ();
    type Topology = nebula_resource::Resident<Self>;

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
    let remote = RemoteEffectInstanceFactory::new(DiagnosticRemoteAction)
        .expect("static remote diagnostic contract must admit");

    let actions: Vec<Arc<dyn ActionFactory>> = vec![
        instance(EchoFixture),
        Arc::new(remote),
        Arc::new(
            GenericTriggerFactory::<DiagnosticTrigger>::new()
                .expect("static trigger diagnostic contract must admit"),
        ),
        instance(StringInputFixture),
        Arc::new(
            GenericTriggerFactory::<SecretDiagnosticTrigger>::new()
                .expect("static secret trigger diagnostic contract must admit"),
        ),
        instance(ResourceSlotFixture),
        instance(CredentialSlotFixture),
        instance(TagFilterFixture),
        instance(NodeFilterFixture),
        instance(SupportRequiredFixture),
        instance(InvalidProjectionFixture),
        instance(WrongDependencyTypeFixture),
        instance(UndeclaredDependencyFixture),
    ];

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
