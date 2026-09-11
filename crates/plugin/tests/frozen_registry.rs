use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicUsize, Ordering},
};

use nebula_action::{
    Action, ActionContext, ActionError, ActionFactory, ActionMetadataDraft, ActionResult,
    InstanceFactory, StatelessAction,
};
use nebula_core::{
    ActionKey, ArtifactSetDigest, CredentialKey, Dependencies, PluginKey, ResourceKey, WorkflowId,
    WorkflowVersionId, node_key,
};
use nebula_credential::{
    AnyCredential, AuthPattern, Credential, CredentialContext, CredentialMetadataDraft,
    SecretString, SecretToken, contract::plugin_capability_report, error::CredentialError,
    resolve::StaticResolveResult,
};
use nebula_error::Classify;
use nebula_metadata::{PluginDependency, PluginManifest};
use nebula_plugin::{
    Plugin, PluginManifestBuilder, PluginRegistry, RegistryFreezeError, ResolvedPlugin,
    RuntimeContractVersion, WorkerFlavorContext,
};
use nebula_resource::{
    KindActivator, Provider, Resident, ResidentConfig, ResourceFactory, ResourceMetadataDraft,
};
use nebula_workflow::{NodeDefinition, WorkflowBuilder};
use semver::{Version, VersionReq};

#[derive(Clone)]
struct MetadataFixture<const INDEX: usize>;

impl<const INDEX: usize> nebula_resource::HasCredentialSlots for MetadataFixture<INDEX> {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }

    fn declares_credential_slots() -> bool {
        false
    }
}

impl<const INDEX: usize> nebula_core::DeclaresDependencies for MetadataFixture<INDEX> {}

#[async_trait::async_trait]
impl<const INDEX: usize> Provider for MetadataFixture<INDEX> {
    type Config = ();
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        let key = match INDEX {
            0 => "alpha.db",
            1 => "alpha.internal_resource",
            _ => panic!("resource metadata fixture index {INDEX} is not declared"),
        };
        ResourceKey::new(key).expect("valid fixture resource key")
    }

    async fn create(
        &self,
        _config: &(),
        _context: &nebula_resource::ResourceContext,
    ) -> Result<(), nebula_resource::Error> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl<const INDEX: usize> nebula_resource::ResidentProvider for MetadataFixture<INDEX> {}

fn resource_factory_at<const INDEX: usize>() -> Arc<dyn ResourceFactory> {
    let factory = KindActivator::<MetadataFixture<INDEX>, _, _>::with_metadata(
        ResourceMetadataDraft::from_key(MetadataFixture::<INDEX>::key()),
        || MetadataFixture,
        || Resident::new(ResidentConfig::default()),
    );
    Arc::new(factory)
}

fn resource_factory(key: &str) -> Arc<dyn ResourceFactory> {
    match key {
        "alpha.db" => resource_factory_at::<0>(),
        "alpha.internal_resource" => resource_factory_at::<1>(),
        _ => panic!("resource factory fixture key `{key}` is not declared"),
    }
}

fn action_factory(key: &str) -> Arc<dyn ActionFactory> {
    Arc::new(
        InstanceFactory::new(
            ActionMetadataDraft::new(
                ActionKey::new(key).expect("test action key must be valid"),
                nebula_action::MetadataName::try_from(key).expect("fixture display name"),
                "test action",
            )
            .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects),
            MetadataAction,
        )
        .expect("test action metadata admits through its structural factory"),
    )
}

struct MetadataAction;

impl Action for MetadataAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            nebula_core::action_key!("fixture.metadata"),
            nebula_action::metadata_name!("Fixture metadata"),
            "Plugin metadata fixture",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for MetadataAction {
    async fn execute(
        &self,
        input: serde_json::Value,
        _context: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

static OBSERVED_ACTION_METADATA_READS: AtomicUsize = AtomicUsize::new(0);
static OBSERVED_ACTION_DEPENDENCY_READS: AtomicUsize = AtomicUsize::new(0);

struct ObservedAction;

impl Action for ObservedAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        OBSERVED_ACTION_METADATA_READS.fetch_add(1, Ordering::Relaxed);
        ActionMetadataDraft::new(
            nebula_core::action_key!("alpha.run"),
            nebula_action::metadata_name!("Run"),
            "Observed action admission fixture",
        )
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
    }

    fn dependencies() -> &'static Dependencies {
        OBSERVED_ACTION_DEPENDENCY_READS.fetch_add(1, Ordering::Relaxed);
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for ObservedAction {
    async fn execute(
        &self,
        input: serde_json::Value,
        _context: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

fn observed_action_factory() -> Arc<dyn ActionFactory> {
    OBSERVED_ACTION_METADATA_READS.store(0, Ordering::Relaxed);
    OBSERVED_ACTION_DEPENDENCY_READS.store(0, Ordering::Relaxed);
    Arc::new(
        InstanceFactory::new(ObservedAction::metadata(), ObservedAction)
            .expect("observed action metadata admits through its structural factory"),
    )
}

macro_rules! credential_fixture {
    ($type:ident, $key:literal) => {
        struct $type;

        impl Credential for $type {
            type Properties = ();
            type Scheme = SecretToken;
            type State = SecretToken;

            const KEY: &'static str = $key;

            fn metadata() -> CredentialMetadataDraft {
                CredentialMetadataDraft::new(
                    nebula_core::credential_key!($key),
                    nebula_credential::metadata_name!("Test credential"),
                    "test credential",
                    AuthPattern::SecretToken,
                )
            }

            fn project(state: &SecretToken) -> SecretToken {
                state.clone()
            }

            async fn resolve(
                _properties: &(),
                _context: &CredentialContext,
            ) -> Result<StaticResolveResult<SecretToken>, CredentialError> {
                Ok(StaticResolveResult::Complete(SecretToken::new(
                    SecretString::new("fixture"),
                )))
            }
        }

        impl plugin_capability_report::IsInteractive for $type {
            const VALUE: bool = false;
        }
        impl plugin_capability_report::IsRefreshable for $type {
            const VALUE: bool = false;
        }
        impl plugin_capability_report::IsRevocable for $type {
            const VALUE: bool = false;
        }
        impl plugin_capability_report::IsTestable for $type {
            const VALUE: bool = false;
        }
        impl plugin_capability_report::IsDynamic for $type {
            const VALUE: bool = false;
        }
    };
}

credential_fixture!(AlphaAuthCredential, "alpha.auth");
credential_fixture!(AlphaInternalCredential, "alpha.internal_credential");

fn test_credential(key: &str) -> Arc<dyn AnyCredential> {
    match key {
        "alpha.auth" => Arc::new(AlphaAuthCredential),
        "alpha.internal_credential" => Arc::new(AlphaInternalCredential),
        _ => panic!("credential metadata fixture key `{key}` is not declared"),
    }
}

struct TestPlugin {
    manifest: PluginManifest,
    actions: Vec<Arc<dyn ActionFactory>>,
    credentials: Vec<Arc<dyn AnyCredential>>,
    resources: Vec<Arc<dyn ResourceFactory>>,
}

impl std::fmt::Debug for TestPlugin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TestPlugin")
            .field("key", self.manifest.key())
            .finish()
    }
}

impl TestPlugin {
    fn empty(builder: PluginManifestBuilder) -> Self {
        Self {
            manifest: builder.build().expect("test manifest must be valid"),
            actions: Vec::new(),
            credentials: Vec::new(),
            resources: Vec::new(),
        }
    }

    fn with_components(
        builder: PluginManifestBuilder,
        action_key: &str,
        credential_key: &str,
        resource_key: &str,
    ) -> Self {
        Self {
            manifest: builder.build().expect("test manifest must be valid"),
            actions: vec![action_factory(action_key)],
            credentials: vec![test_credential(credential_key)],
            resources: vec![resource_factory(resource_key)],
        }
    }
}

impl Plugin for TestPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn actions(&self) -> Vec<Arc<dyn ActionFactory>> {
        self.actions.clone()
    }

    fn credentials(&self) -> Vec<Arc<dyn AnyCredential>> {
        self.credentials.clone()
    }

    fn resources(&self) -> Vec<Arc<dyn ResourceFactory>> {
        self.resources.clone()
    }
}

fn resolved(plugin: TestPlugin) -> Arc<ResolvedPlugin> {
    Arc::new(ResolvedPlugin::from(plugin).expect("test plugin must resolve"))
}

fn register(registry: &mut PluginRegistry, plugin: TestPlugin) {
    registry
        .register(resolved(plugin))
        .expect("test plugin registration must succeed");
}

fn dependency(key: &str, requirement: &str) -> PluginDependency {
    PluginDependency::new(
        key.parse().expect("test dependency key must be valid"),
        VersionReq::parse(requirement).expect("test dependency requirement must be valid"),
    )
}

fn empty_plugin(key: &str, version: &str) -> TestPlugin {
    TestPlugin::empty(
        PluginManifest::builder(key, key)
            .version(Version::parse(version).expect("test plugin version must be valid")),
    )
}

fn component_plugin(
    key: &str,
    version: &str,
    action_key: &str,
    credential_key: &str,
    resource_key: &str,
) -> TestPlugin {
    TestPlugin::with_components(
        PluginManifest::builder(key, key)
            .version(Version::parse(version).expect("test plugin version must be valid")),
        action_key,
        credential_key,
        resource_key,
    )
}

fn freeze(
    registry: PluginRegistry,
    artifact_byte: u8,
    runtime_version: &str,
) -> nebula_plugin::FrozenPluginRegistry {
    registry
        .freeze(
            ArtifactSetDigest::from_bytes([artifact_byte; 32]),
            runtime_version
                .parse::<RuntimeContractVersion>()
                .expect("test runtime version must be valid"),
        )
        .expect("test registry must freeze")
}

fn assert_action_projection_read_once() {
    assert_eq!(OBSERVED_ACTION_METADATA_READS.load(Ordering::Relaxed), 1);
    assert_eq!(OBSERVED_ACTION_DEPENDENCY_READS.load(Ordering::Relaxed), 1);
}

#[test]
fn resolved_plugin_snapshots_contracts_for_frozen_use() {
    let plugin = TestPlugin {
        manifest: PluginManifest::builder("alpha", "alpha")
            .build()
            .expect("test manifest must be valid"),
        actions: vec![observed_action_factory()],
        credentials: vec![Arc::new(AlphaAuthCredential)],
        resources: vec![resource_factory("alpha.db")],
    };
    let resolved = Arc::new(ResolvedPlugin::from(plugin).expect("test plugin must resolve"));

    assert_action_projection_read_once();

    assert_eq!(resolved.actions().count(), 1);
    assert_eq!(resolved.credentials().count(), 1);
    assert_eq!(resolved.resources().count(), 1);
    let mut registry = PluginRegistry::new();
    registry
        .register(resolved)
        .expect("test plugin registration must succeed");
    let frozen = freeze(registry, 0x33, "1.0.0");
    assert_eq!(frozen.all_actions().count(), 1);
    assert_eq!(frozen.all_credentials().count(), 1);
    assert_eq!(frozen.all_resources().count(), 1);

    let workflow = WorkflowBuilder::new("snapshot proof")
        .id(WorkflowId::from_bytes([0x34; 16]))
        .add_node(
            NodeDefinition::new(node_key!("run"), "Run", "alpha", "run")
                .expect("test node is valid"),
        )
        .build()
        .expect("test workflow is valid");
    let plan = frozen
        .compile_graph_v1(WorkflowVersionId::from_bytes([0x35; 16]), &workflow)
        .expect("the snapshot-backed workflow compiles");
    plan.validate_against(&frozen)
        .expect("compatibility consumes the same snapshots");

    assert_action_projection_read_once();
}

#[test]
fn canonical_descriptor_and_worker_revision_match_golden_vectors() {
    let mut registry = PluginRegistry::new();
    register(&mut registry, empty_plugin("base", "1.4.0+linux.x86-64"));
    register(
        &mut registry,
        TestPlugin::with_components(
            PluginManifest::builder("alpha", "alpha")
                .version(Version::parse("2.0.0-rc.1+build.7").unwrap())
                .dependency(dependency("base", ">=1.0.0-alpha.1, <2, >=1.0.0-alpha.1")),
            "alpha.run",
            "alpha.auth",
            "alpha.db",
        ),
    );

    let frozen = freeze(registry, 0x11, "1.0.0+runtime.9");

    assert_eq!(
        frozen.plugin_set().id().to_string(),
        "896bebef54fd67cb6493735e6159a6ba11fbff685025eff9627e32322b5af492"
    );
    assert_eq!(
        frozen.revision().id().to_string(),
        "db6e79172ed7b278b14ee234b94f827b80aa9fc5dda8b1f606ee5267bfcf5dd6"
    );

    let alpha = &frozen.plugin_set().plugins()[0];
    assert_eq!(alpha.key().as_str(), "alpha");
    assert_eq!(alpha.version().to_string(), "2.0.0-rc.1");
    assert_eq!(alpha.action_keys()[0].as_str(), "alpha.run");
    assert_eq!(alpha.credential_keys()[0].as_str(), "alpha.auth");
    assert_eq!(alpha.resource_keys()[0].as_str(), "alpha.db");
    assert_eq!(alpha.dependencies()[0].key().as_str(), "base");
    assert_eq!(
        alpha.dependencies()[0].req().to_string(),
        ">=1.0.0-alpha.1, <2"
    );
}

fn dependency_registry(
    dependency_order: &[(&str, &str)],
    registration_order: &[&str],
) -> PluginRegistry {
    let mut registry = PluginRegistry::new();
    for key in registration_order {
        match *key {
            "application" => {
                let mut application = PluginManifest::builder("application", "application");
                for (key, requirement) in dependency_order {
                    application = application.dependency(dependency(key, requirement));
                }
                register(&mut registry, TestPlugin::empty(application));
            },
            "auxiliary" => register(&mut registry, empty_plugin("auxiliary", "3.2.0")),
            "base" => register(&mut registry, empty_plugin("base", "1.5.0")),
            unexpected => panic!("unexpected test plugin key: {unexpected}"),
        }
    }
    registry
}

#[test]
fn identity_is_invariant_to_registration_dependency_and_comparator_order() {
    let first = freeze(
        dependency_registry(
            &[
                ("base", ">= 1.0.0, < 2.0.0, >=1.0.0"),
                ("auxiliary", "^3.0.0"),
                ("base", "<2.0.0, >=1.0.0"),
            ],
            &["application", "base", "auxiliary"],
        ),
        1,
        "1.0.0",
    );
    let second = freeze(
        dependency_registry(
            &[("auxiliary", "^3.0.0"), ("base", "<2.0.0, >=1.0.0")],
            &["auxiliary", "base", "application"],
        ),
        1,
        "1.0.0",
    );

    assert_eq!(first.plugin_set().id(), second.plugin_set().id());
    assert_eq!(first.revision().id(), second.revision().id());
    assert_eq!(first.load_order(), second.load_order());
    assert_eq!(
        first
            .load_order()
            .iter()
            .map(PluginKey::as_str)
            .collect::<Vec<_>>(),
        vec!["auxiliary", "base", "application"]
    );
    assert_eq!(
        first
            .plugin_set()
            .plugins()
            .iter()
            .find(|descriptor| descriptor.key().as_str() == "application")
            .unwrap()
            .dependencies()
            .iter()
            .map(|dependency| { (dependency.key().as_str(), dependency.req().to_string(),) })
            .collect::<Vec<_>>(),
        vec![
            ("auxiliary", "^3.0.0".to_owned()),
            ("base", ">=1.0.0, <2.0.0".to_owned()),
        ]
    );
}

fn changed_dependency_registry(dependency_key: &str, requirement: &str) -> PluginRegistry {
    let mut registry = PluginRegistry::new();
    register(&mut registry, empty_plugin("base", "1.5.0"));
    register(&mut registry, empty_plugin("replacement", "1.5.0"));
    register(
        &mut registry,
        TestPlugin::empty(
            PluginManifest::builder("application", "application")
                .dependency(dependency(dependency_key, requirement)),
        ),
    );
    registry
}

#[test]
fn dependency_key_and_requirement_each_change_plugin_set_identity() {
    let baseline = freeze(changed_dependency_registry("base", "^1.0.0"), 1, "1.0.0");
    let key_changed = freeze(
        changed_dependency_registry("replacement", "^1.0.0"),
        1,
        "1.0.0",
    );
    let requirement_changed = freeze(
        changed_dependency_registry("base", ">=1.4.0, <2.0.0"),
        1,
        "1.0.0",
    );

    assert_ne!(baseline.plugin_set().id(), key_changed.plugin_set().id());
    assert_ne!(
        baseline.plugin_set().id(),
        requirement_changed.plugin_set().id()
    );
}

#[test]
fn component_keys_and_prerelease_are_logical_but_build_metadata_is_not() {
    let mut plain = PluginRegistry::new();
    register(
        &mut plain,
        component_plugin("alpha", "1.2.3", "alpha.run", "alpha.auth", "alpha.db"),
    );
    let mut build = PluginRegistry::new();
    register(
        &mut build,
        component_plugin(
            "alpha",
            "1.2.3+linux.x86-64",
            "alpha.run",
            "alpha.auth",
            "alpha.db",
        ),
    );
    let mut prerelease = PluginRegistry::new();
    register(
        &mut prerelease,
        component_plugin("alpha", "1.2.3-rc.1", "alpha.run", "alpha.auth", "alpha.db"),
    );
    let mut component_changed = PluginRegistry::new();
    register(
        &mut component_changed,
        component_plugin("alpha", "1.2.3", "alpha.execute", "alpha.auth", "alpha.db"),
    );

    let plain = freeze(plain, 1, "1.0.0");
    let build = freeze(build, 1, "1.0.0");
    let prerelease = freeze(prerelease, 1, "1.0.0");
    let component_changed = freeze(component_changed, 1, "1.0.0");

    assert_eq!(plain.plugin_set().id(), build.plugin_set().id());
    assert_ne!(plain.plugin_set().id(), prerelease.plugin_set().id());
    assert_ne!(plain.plugin_set().id(), component_changed.plugin_set().id());
}

#[test]
fn trusted_artifact_and_runtime_inputs_each_change_worker_revision() {
    let registry = || {
        let mut registry = PluginRegistry::new();
        register(&mut registry, empty_plugin("alpha", "1.0.0"));
        registry
    };

    let baseline = freeze(registry(), 1, "1.0.0");
    let artifact_changed = freeze(registry(), 2, "1.0.0");
    let runtime_changed = freeze(registry(), 1, "2.0.0");
    let runtime_build = freeze(registry(), 1, "1.0.0+build.7");

    assert_ne!(baseline.revision().id(), artifact_changed.revision().id());
    assert_ne!(baseline.revision().id(), runtime_changed.revision().id());
    assert_eq!(baseline.revision().id(), runtime_build.revision().id());
    assert_eq!(
        runtime_build
            .revision()
            .runtime_contract_version()
            .to_string(),
        "1.0.0"
    );
}

#[test]
fn freeze_preserves_dependency_validation_failures() {
    let empty = PluginRegistry::new()
        .freeze(
            ArtifactSetDigest::from_bytes([0; 32]),
            "1.0.0".parse().unwrap(),
        )
        .unwrap_err();
    assert!(matches!(empty, RegistryFreezeError::EmptyRegistry));
    assert_eq!(empty.code().as_str(), "PLUGIN_FREEZE:EMPTY_REGISTRY");

    let mut missing = PluginRegistry::new();
    register(
        &mut missing,
        TestPlugin::empty(
            PluginManifest::builder("application", "application")
                .dependency(dependency("missing", "^1.0.0")),
        ),
    );
    assert!(matches!(
        missing
            .freeze(
                ArtifactSetDigest::from_bytes([0; 32]),
                "1.0.0".parse().unwrap(),
            )
            .unwrap_err(),
        RegistryFreezeError::Dependency(
            nebula_plugin::PluginDependencyError::MissingDependency { .. }
        )
    ));

    let mut mismatch = PluginRegistry::new();
    register(&mut mismatch, empty_plugin("base", "1.0.0"));
    register(
        &mut mismatch,
        TestPlugin::empty(
            PluginManifest::builder("application", "application")
                .dependency(dependency("base", "^2.0.0")),
        ),
    );
    assert!(matches!(
        mismatch
            .freeze(
                ArtifactSetDigest::from_bytes([0; 32]),
                "1.0.0".parse().unwrap(),
            )
            .unwrap_err(),
        RegistryFreezeError::Dependency(nebula_plugin::PluginDependencyError::VersionMismatch(_))
    ));

    let mut cycle = PluginRegistry::new();
    register(
        &mut cycle,
        TestPlugin::empty(
            PluginManifest::builder("alpha", "alpha").dependency(dependency("beta", "*")),
        ),
    );
    register(
        &mut cycle,
        TestPlugin::empty(
            PluginManifest::builder("beta", "beta").dependency(dependency("alpha", "*")),
        ),
    );
    assert!(matches!(
        cycle
            .freeze(
                ArtifactSetDigest::from_bytes([0; 32]),
                "1.0.0".parse().unwrap(),
            )
            .unwrap_err(),
        RegistryFreezeError::Dependency(nebula_plugin::PluginDependencyError::Cycle { .. })
    ));
}

#[test]
fn unsupported_fingerprint_operator_error_is_stable_and_classified() {
    let error = RegistryFreezeError::UnsupportedVersionRequirement {
        plugin: "application".parse().unwrap(),
        dependency: "base".parse().unwrap(),
    };

    assert_eq!(error.category(), nebula_error::ErrorCategory::Validation);
    assert_eq!(
        error.code().as_str(),
        "PLUGIN_FREEZE:UNSUPPORTED_VERSION_REQUIREMENT"
    );
    assert_eq!(
        error.to_string(),
        "plugin `application` dependency `base` uses an unsupported version requirement"
    );
}

#[test]
fn frozen_registry_matches_mutable_read_surface_for_every_component_kind() {
    let mut registry = PluginRegistry::new();
    register(
        &mut registry,
        component_plugin("alpha", "1.0.0", "alpha.run", "alpha.auth", "alpha.db"),
    );

    let plugin_key: PluginKey = "alpha".parse().unwrap();
    let action_key: ActionKey = "alpha.run".parse().unwrap();
    let credential_key: CredentialKey = "alpha.auth".parse().unwrap();
    let resource_key: ResourceKey = "alpha.db".parse().unwrap();
    let mutable_action = registry.resolve_action(&action_key).unwrap();
    let mutable_credential = registry.resolve_credential(&credential_key).unwrap();
    let mutable_resource = registry.resolve_resource(&resource_key).unwrap();
    assert!(registry.contains(&plugin_key));
    assert_eq!(registry.all_actions().count(), 1);
    assert_eq!(registry.all_credentials().count(), 1);
    assert_eq!(registry.all_resources().count(), 1);

    let frozen = freeze(registry, 1, "1.0.0");

    assert!(frozen.contains(&plugin_key));
    assert_eq!(frozen.get(&plugin_key).unwrap().key(), &plugin_key);
    assert_eq!(frozen.iter().count(), 1);
    assert_eq!(frozen.len(), 1);
    assert!(!frozen.is_empty());
    assert_eq!(frozen.all_actions().count(), 1);
    assert_eq!(frozen.all_credentials().count(), 1);
    assert_eq!(frozen.all_resources().count(), 1);
    assert!(Arc::ptr_eq(
        &mutable_action,
        &frozen.resolve_action(&action_key).unwrap()
    ));
    assert!(Arc::ptr_eq(
        &mutable_credential,
        &frozen.resolve_credential(&credential_key).unwrap()
    ));
    assert!(Arc::ptr_eq(
        &mutable_resource,
        &frozen.resolve_resource(&resource_key).unwrap()
    ));
    assert_eq!(frozen.revision().plugin_set_id(), frozen.plugin_set().id());
}

#[test]
fn frozen_registry_debug_is_stable_and_omits_registry_contents() {
    let mut registry = PluginRegistry::new();
    register(
        &mut registry,
        component_plugin(
            "alpha",
            "1.0.0",
            "alpha.internal_action",
            "alpha.internal_credential",
            "alpha.internal_resource",
        ),
    );
    let frozen = freeze(registry, 1, "1.0.0");

    let debug = format!("{frozen:?}");

    assert_eq!(
        debug,
        format!(
            "FrozenPluginRegistry {{ count: 1, plugin_set_id: {:?}, revision_id: {:?} }}",
            frozen.plugin_set().id(),
            frozen.revision().id()
        )
    );
    assert!(!debug.contains("alpha"));
    assert!(!debug.contains("internal_action"));
    assert!(!debug.contains("internal_credential"));
    assert!(!debug.contains("internal_resource"));
}

#[test]
fn worker_flavor_context_is_derived_from_the_frozen_registry() {
    let mut registry = PluginRegistry::new();
    register(&mut registry, empty_plugin("beta", "1.0.0"));
    register(&mut registry, empty_plugin("alpha", "1.0.0"));
    let frozen = freeze(registry, 7, "1.0.0");

    let context = WorkerFlavorContext::from_registry(&frozen);

    assert_eq!(context.revision_id(), frozen.revision().id());
    assert_eq!(
        context
            .plugin_keys()
            .iter()
            .map(PluginKey::as_str)
            .collect::<Vec<_>>(),
        vec!["alpha", "beta"]
    );
}

#[test]
fn runtime_contract_parse_error_is_stable_and_classified() {
    let error = "not-semver"
        .parse::<RuntimeContractVersion>()
        .expect_err("invalid semantic version must fail");

    assert_eq!(error.category(), nebula_error::ErrorCategory::Validation);
    assert_eq!(
        error.code().as_str(),
        "PLUGIN_FLAVOR:INVALID_RUNTIME_CONTRACT_VERSION"
    );
    assert!(!error.source_error().to_string().is_empty());
    assert_eq!(error.to_string(), "invalid runtime contract version");
}
