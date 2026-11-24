//! Factories for admission-boundary integration tests.

use std::sync::Arc;

use nebula_action::effect::{
    ActionEffectContract, EffectPreparationContext, EffectPreparationError, PreparedRemoteEffect,
    RemoteDestinationGuarantee, RemoteEffectAction, RemoteEffectDeclarationError,
    RemoteEffectDescriptor, RemoteEffectPolicy,
};
use nebula_action::{
    Action, ActionContext, ActionError, ActionFactory, ActionResult, Component, FromWorkflowNode,
    GenericTriggerFactory, InstanceFactory, Metadata, RemoteEffectInstanceFactory, StatelessAction,
    TriggerAction, TriggerContext, TriggerEventOutcome, TriggerSource, kind,
};
use nebula_core::ArtifactSetDigest;
use nebula_schema::{HasSchema, Schema, SecretField, ValidSchema, field_key};
use nebula_workflow::NodeDefinition;
use serde_json::Value;

use crate::{FrozenPluginRegistry, Plugin, PluginManifest, PluginRegistry, ResolvedPlugin};

/// Declare a stateless fixture action that fails closed if it ever executes.
///
/// Each fixture is its own type, so its key and metadata are the type's own.
macro_rules! diagnostic_stateless {
    ($name:ident, $input:ty, $dependencies:expr, $key:literal, $metadata:expr $(,)?) => {
        diagnostic_stateless!(
            $name,
            $input,
            $dependencies,
            $key,
            $metadata,
            nebula_action::port::default_input_ports(),
        );
    };
    (
        $name:ident,
        $input:ty,
        $dependencies:expr,
        $key:literal,
        $metadata:expr,
        $inputs:expr $(,)?
    ) => {
        struct $name;

        impl Component for $name {
            type Kind = kind::Action;
            type Config = $input;

            const KEY: &'static str = $key;

            fn metadata() -> Metadata {
                $metadata
            }

            fn dependencies() -> nebula_action::Dependencies {
                $dependencies
            }
        }

        impl Action for $name {
            type Output = Value;

            const EFFECT_CONTRACT: ActionEffectContract = ActionEffectContract::ReadOnly;

            fn inputs() -> Vec<nebula_action::InputPort> {
                $inputs
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

fn read_only(name: nebula_action::MetadataName, description: &str) -> Metadata {
    Metadata::new(name, description)
}

/// The input ports of the port fixture named `name`.
fn port_fixture(name: &str) -> Vec<nebula_action::InputPort> {
    let mut port = nebula_action::InputPort::support(
        nebula_action::port_key!("support"),
        "Support",
        "Fixture support",
    );
    if let nebula_action::InputPort::Support(support) = &mut port {
        support.required = name == "support_required";
        match name {
            "tag_filter" => support.filter.allowed_tags = Some(vec!["fixture".to_owned()]),
            "node_filter" => {
                support.filter.allowed_node_types = Some(vec!["core.absent".to_owned()]);
            },
            "invalid_projection" => support.filter.allowed_tags = Some(vec![]),
            _ => {},
        }
    }
    vec![
        nebula_action::InputPort::flow(nebula_action::port_key!("in")),
        port,
    ]
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
    resource_slot_dependencies(),
    "core.resource_slot",
    read_only(
        nebula_action::metadata_name!("resource_slot"),
        "Unresolved resource slot fixture"
    ),
);
diagnostic_stateless!(
    CredentialSlotFixture,
    Value,
    credential_slot_dependencies(),
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
    read_only(nebula_action::metadata_name!("tag_filter"), "Port fixture"),
    port_fixture("tag_filter"),
);
diagnostic_stateless!(
    NodeFilterFixture,
    Value,
    empty_dependencies(),
    "core.node_filter",
    read_only(nebula_action::metadata_name!("node_filter"), "Port fixture"),
    port_fixture("node_filter"),
);
diagnostic_stateless!(
    SupportRequiredFixture,
    Value,
    empty_dependencies(),
    "core.support_required",
    read_only(
        nebula_action::metadata_name!("support_required"),
        "Port fixture"
    ),
    port_fixture("support_required"),
);
diagnostic_stateless!(
    InvalidProjectionFixture,
    Value,
    empty_dependencies(),
    "core.invalid_projection",
    read_only(
        nebula_action::metadata_name!("invalid_projection"),
        "Port fixture"
    ),
    port_fixture("invalid_projection"),
);
diagnostic_stateless!(
    WrongDependencyTypeFixture,
    Value,
    wrong_resource_type_dependencies(),
    "core.wrong_dependency_type",
    read_only(
        nebula_action::metadata_name!("wrong_dependency_type"),
        "Wrong dependency type fixture"
    ),
);
diagnostic_stateless!(
    UndeclaredDependencyFixture,
    Value,
    undeclared_resource_dependencies(),
    "core.undeclared_dependency",
    read_only(
        nebula_action::metadata_name!("undeclared_dependency"),
        "Undeclared dependency fixture"
    ),
);

/// A slot value pointing at the component `C`, for fixtures that declare a
/// slot without holding a real handle.
struct SlotOn<C>(std::marker::PhantomData<fn() -> C>);

impl<C> nebula_component::SlotValue for SlotOn<C>
where
    C: Component<Kind: nebula_component::DependencyKind> + 'static,
{
    type Target = C;
}

/// Declares the component `$name` of kind `$kind` under `$key`, with no
/// behaviour: a slot target the registry does not provide, or provides under
/// another type.
macro_rules! slot_target {
    ($name:ident, $kind:ty, $key:literal) => {
        struct $name;

        impl Component for $name {
            type Kind = $kind;
            type Config = ();

            const KEY: &'static str = $key;

            fn metadata() -> Metadata {
                Metadata::new(nebula_action::metadata_name!("slot_target"), "")
            }
        }
    };
}

slot_target!(AbsentResource, kind::Resource, "core.absent");
slot_target!(AbsentCredential, kind::Credential, "core.absent");
slot_target!(ImpostorResident, kind::Resource, "provider.resident");

fn resource_slot_dependencies() -> nebula_action::Dependencies {
    nebula_action::Dependencies::none()
        .slot(nebula_action::Slot::of::<SlotOn<AbsentResource>>("binding"))
}

fn credential_slot_dependencies() -> nebula_action::Dependencies {
    nebula_action::Dependencies::none().slot(nebula_action::Slot::of::<SlotOn<AbsentCredential>>(
        "binding",
    ))
}

/// A slot on the provider's resource key with another Rust type behind it.
fn wrong_resource_type_dependencies() -> nebula_action::Dependencies {
    nebula_action::Dependencies::none().slot(nebula_action::Slot::of::<SlotOn<ImpostorResident>>(
        "resident",
    ))
}

/// A slot on the provider's resource, from a plugin that does not declare
/// the provider as a dependency.
fn undeclared_resource_dependencies() -> nebula_action::Dependencies {
    nebula_action::Dependencies::none().slot(nebula_action::Slot::of::<SlotOn<FixtureResource>>(
        "resident",
    ))
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

        impl Component for $trigger {
            type Kind = kind::Action;
            type Config = $input;

            const KEY: &'static str = $key;

            fn metadata() -> Metadata {
                Metadata::new(
                    nebula_action::metadata_name!($name),
                    "Activation diagnostic trigger fixture",
                )
            }
        }

        impl Action for $trigger {
            type Output = Value;

            const EFFECT_CONTRACT: nebula_action::ActionEffectContract =
                ActionEffectContract::ReadOnly;
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

impl Component for DiagnosticRemoteAction {
    type Kind = kind::Action;
    type Config = Value;

    const KEY: &'static str = "core.remote_control";

    fn metadata() -> Metadata {
        Metadata::new(
            nebula_action::metadata_name!("remote_control"),
            "Remote effect diagnostic fixture",
        )
    }
}

impl Action for DiagnosticRemoteAction {
    type Output = Value;
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
        _input: Self::Config,
        _context: &EffectPreparationContext,
    ) -> Result<PreparedRemoteEffect, EffectPreparationError> {
        Err(EffectPreparationError::Unavailable)
    }
}

const fn empty_dependencies() -> nebula_action::Dependencies {
    nebula_action::Dependencies::none()
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

impl Component for FixtureResource {
    type Kind = kind::Resource;
    type Config = ();

    const KEY: &'static str = "provider.resident";

    fn metadata() -> Metadata {
        Metadata::new(
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
