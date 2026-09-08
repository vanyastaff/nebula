//! Factories for admission-boundary integration tests.
use std::{future::Future, pin::Pin, sync::Arc};

use nebula_action::effect::{
    ActionEffectContract, EffectPreparationContext, EffectPreparationError, PreparedRemoteEffect,
    RemoteDestinationGuarantee, RemoteEffectDescriptor, RemoteEffectFactory, RemoteEffectPolicy,
};
use nebula_action::{
    ActionContext, ActionError, ActionFactory, ActionHandle, ActionKind, ActionMetadata,
};
use nebula_core::{ArtifactSetDigest, Dependencies};
use nebula_workflow::NodeDefinition;

use crate::{FrozenPluginRegistry, Plugin, PluginManifest, PluginRegistry, ResolvedPlugin};

#[derive(Debug)]
struct Factory {
    metadata: ActionMetadata,
    dependencies: Dependencies,
}
impl ActionFactory for Factory {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
    fn dependencies(&self) -> &Dependencies {
        &self.dependencies
    }
    fn remote_effect_factory(&self) -> Option<&dyn RemoteEffectFactory> {
        matches!(
            self.metadata.effect_contract,
            ActionEffectContract::Remote(_)
        )
        .then_some(self)
    }
    fn instantiate<'a>(
        &'a self,
        _: &'a NodeDefinition,
        _: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
        panic!("diagnostic admission must not construct actions")
    }
}
#[async_trait::async_trait]
impl RemoteEffectFactory for Factory {
    fn descriptor(&self) -> &RemoteEffectDescriptor {
        match &self.metadata.effect_contract {
            ActionEffectContract::Remote(descriptor) => descriptor,
            _ => panic!("capability only exists for a declared remote action"),
        }
    }
    async fn prepare(
        &self,
        _: serde_json::Value,
        _: &EffectPreparationContext,
    ) -> Result<PreparedRemoteEffect, EffectPreparationError> {
        panic!("diagnostic admission must not prepare provider requests")
    }
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
            vec![Arc::new(ResourceFactory {
                dependencies: Dependencies::new(),
            })]
        } else {
            vec![]
        }
    }
}

struct ResourceFactory {
    dependencies: Dependencies,
}
impl nebula_resource::ResourceFactory for ResourceFactory {
    fn key(&self) -> nebula_core::ResourceKey {
        "provider.resident".parse().unwrap()
    }
    fn dependencies(&self) -> &Dependencies {
        &self.dependencies
    }
    fn resource_type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<()>()
    }
    fn metadata(&self) -> nebula_resource::ResourceMetadata {
        nebula_resource::ResourceMetadata::new(
            self.key(),
            "Resident",
            "Admission-only resource",
            nebula_schema::ValidSchema::empty(),
        )
    }
    fn validate(&self, _: serde_json::Value) -> Result<(), nebula_resource::Error> {
        panic!("compilation must not validate concrete resource config")
    }
    fn register<'a>(
        &'a self,
        _: &'a nebula_resource::Manager,
        _: nebula_resource::RegisterRequest<'a>,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<nebula_resource::SlotIdentity, nebula_resource::Error>>
                + Send
                + 'a,
        >,
    > {
        panic!("compilation must not register resources")
    }
}
/// Builds a frozen registry covering activation-diagnostic rejection paths.
///
/// The factories panic if compilation crosses into runtime construction or
/// resource registration, keeping this fixture limited to admission behavior.
#[must_use]
pub fn activation_diagnostic_registry() -> Arc<FrozenPluginRegistry> {
    let remote = ActionEffectContract::Remote(Box::new(
        RemoteEffectDescriptor::new(
            "activation.remote",
            1,
            RemoteEffectPolicy::builder(RemoteDestinationGuarantee::Opaque)
                .maximum_invocations(1)
                .maximum_queries(0)
                .recovery_window(std::time::Duration::from_mins(1))
                .build()
                .unwrap(),
        )
        .unwrap(),
    ));
    let mut actions: Vec<Arc<dyn ActionFactory>> = [
        (
            "echo",
            ActionKind::Stateless,
            ActionEffectContract::NoExternalEffects,
        ),
        (
            "undeclared",
            ActionKind::Stateless,
            ActionEffectContract::Undeclared,
        ),
        ("remote_control", ActionKind::Control, remote),
        (
            "trigger",
            ActionKind::Trigger,
            ActionEffectContract::NoExternalEffects,
        ),
    ]
    .into_iter()
    .map(|(key, kind, effect)| {
        Arc::new(Factory {
            metadata: ActionMetadata::new(
                format!("core.{key}").parse().unwrap(),
                key,
                "Admission fixture",
            )
            .with_kind(kind)
            .with_effect_contract(effect),
            dependencies: Dependencies::new(),
        }) as Arc<dyn ActionFactory>
    })
    .collect();
    let string_schema = nebula_schema::Schema::builder()
        .add(nebula_schema::Field::string(nebula_schema::field_key!("value")).required())
        .build()
        .unwrap();
    let secret_schema = nebula_schema::Schema::builder()
        .add(nebula_schema::SecretField::new(nebula_schema::field_key!(
            "token"
        )))
        .build()
        .unwrap();
    for (key, kind, schema) in [
        ("string_input", ActionKind::Stateless, string_schema),
        ("secret_trigger", ActionKind::Trigger, secret_schema),
    ] {
        actions.push(Arc::new(Factory {
            metadata: ActionMetadata::new(
                format!("core.{key}").parse().unwrap(),
                key,
                "Schema admission fixture",
            )
            .with_kind(kind)
            .with_effect_contract(ActionEffectContract::NoExternalEffects)
            .with_schema(schema),
            dependencies: Dependencies::new(),
        }));
    }
    for (key, kind) in [
        (
            "resource_slot",
            nebula_core::SlotKind::Resource {
                type_id: std::any::TypeId::of::<()>(),
                type_name: "FixtureResource",
                key: "core.absent".parse().unwrap(),
            },
        ),
        (
            "credential_slot",
            nebula_core::SlotKind::Credential {
                type_id: std::any::TypeId::of::<()>(),
                type_name: "FixtureCredential",
                key: "core.absent".parse().unwrap(),
            },
        ),
    ] {
        actions.push(Arc::new(Factory {
            metadata: ActionMetadata::new(
                format!("core.{key}").parse().unwrap(),
                key,
                "Unresolved slot fixture",
            )
            .with_effect_contract(ActionEffectContract::NoExternalEffects),
            dependencies: Dependencies::new().slot_field(nebula_core::SlotField {
                slot_key: "binding",
                default_id: "fixture",
                kind,
                required: true,
                lazy: false,
                purpose: None,
            }),
        }));
    }
    for key in [
        "tag_filter",
        "node_filter",
        "support_required",
        "invalid_projection",
    ] {
        let mut port = nebula_action::InputPort::support(
            nebula_action::port_key!("support"),
            "Support",
            "Fixture support",
        );
        if let nebula_action::InputPort::Support(support) = &mut port {
            support.required = key == "support_required";
            match key {
                "tag_filter" => support.filter.allowed_tags = Some(vec!["fixture".to_owned()]),
                "node_filter" => {
                    support.filter.allowed_node_types = Some(vec!["core.absent".to_owned()]);
                },
                "invalid_projection" => support.filter.allowed_tags = Some(vec![]),
                _ => {},
            }
        }
        actions.push(Arc::new(Factory {
            metadata: ActionMetadata::new(
                format!("core.{key}").parse().unwrap(),
                key,
                "Port fixture",
            )
            .with_effect_contract(ActionEffectContract::NoExternalEffects)
            .with_inputs(vec![
                nebula_action::InputPort::flow(nebula_action::port_key!("in")),
                port,
            ]),
            dependencies: Dependencies::new(),
        }));
    }
    for (key, type_id) in [
        ("wrong_dependency_type", std::any::TypeId::of::<u8>()),
        ("undeclared_dependency", std::any::TypeId::of::<()>()),
    ] {
        actions.push(Arc::new(Factory {
            metadata: ActionMetadata::new(
                format!("core.{key}").parse().unwrap(),
                key,
                "Dependency admission fixture",
            )
            .with_effect_contract(ActionEffectContract::NoExternalEffects),
            dependencies: Dependencies::new().resource(nebula_core::ResourceRequirement::new(
                "provider.resident".parse().unwrap(),
                type_id,
                "FixtureResource",
            )),
        }));
    }
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
