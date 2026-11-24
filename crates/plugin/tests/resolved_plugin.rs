//! Integration tests for `ResolvedPlugin` — namespace enforcement and lookup.

use std::sync::{Arc, OnceLock};

use nebula_action::{
    Action, ActionContext, ActionError, ActionFactory, ActionMetadata, ActionResult,
    InstanceFactory, StatelessAction,
};
use nebula_core::{ActionKey, CredentialKey, Dependencies, ResourceKey};
use nebula_credential::{
    AnyCredential, Credential, CredentialContext, CredentialMetadata, SecretString, SecretToken,
    contract::plugin_capability_report, error::CredentialError, resolve::StaticResolveResult,
};
use nebula_metadata::PluginManifest;
use nebula_plugin::{ComponentKind, Plugin, PluginError, ResolvedPlugin};
use nebula_resource::{
    KindActivator, Provider, Resident, ResidentConfig, ResourceFactory, ResourceMetadata,
};

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

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::new(nebula_resource::metadata_name!("Metadata fixture"), "")
    }

    const KEY: &'static str = match INDEX {
        0 => "slack.http_client",
        1 => "slack.audit_client",
        2 => "slack.aaa",
        3 => "slack.zzz",
        4 => "api.http_client",
        5 => "http.client",
        6 => "http.pool",
        _ => panic!("resource metadata fixture index is not declared"),
    };

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
    let factory = KindActivator::<MetadataFixture<INDEX>, _, _>::new(
        || MetadataFixture,
        nebula_resource::topology::fixed(|| Resident::new(ResidentConfig::default())),
    );
    Arc::new(factory)
}

fn resource_factory(key: &str) -> Arc<dyn ResourceFactory> {
    match key {
        "slack.http_client" => resource_factory_at::<0>(),
        "slack.audit_client" => resource_factory_at::<1>(),
        "slack.aaa" => resource_factory_at::<2>(),
        "slack.zzz" => resource_factory_at::<3>(),
        "api.http_client" => resource_factory_at::<4>(),
        "http.client" => resource_factory_at::<5>(),
        "http.pool" => resource_factory_at::<6>(),
        _ => panic!("resource factory fixture key `{key}` is not declared"),
    }
}

/// One fixture action type per key: a key is its type's `KEY`.
fn action_factory_at<const INDEX: usize>() -> Arc<dyn ActionFactory> {
    Arc::new(
        InstanceFactory::new(MetadataAction::<INDEX>)
            .expect("stub metadata admits through its structural factory"),
    )
}

fn action_factory(key: &str) -> Arc<dyn ActionFactory> {
    match key {
        "api.foo" => action_factory_at::<0>(),
        "github.send" => action_factory_at::<1>(),
        "http.get" => action_factory_at::<2>(),
        "http.post" => action_factory_at::<3>(),
        "http.send" => action_factory_at::<4>(),
        "slack.send" => action_factory_at::<5>(),
        "slack.send_message" => action_factory_at::<6>(),
        _ => panic!("action factory fixture key `{key}` is not declared"),
    }
}

struct MetadataAction<const INDEX: usize>;

impl<const INDEX: usize> Action for MetadataAction<INDEX> {
    const KEY: &'static str = match INDEX {
        0 => "api.foo",
        1 => "github.send",
        2 => "http.get",
        3 => "http.post",
        4 => "http.send",
        5 => "slack.send",
        6 => "slack.send_message",
        _ => panic!("action metadata fixture index is not declared"),
    };
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            nebula_action::MetadataName::try_from(Self::KEY).expect("fixture display name"),
            "stub",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl<const INDEX: usize> StatelessAction for MetadataAction<INDEX> {
    async fn execute(
        &self,
        input: serde_json::Value,
        _context: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

// ── Stub AnyCredential ───────────────────────────────────────────────────────

macro_rules! credential_fixture {
    ($type:ident, $key:literal) => {
        struct $type;

        impl Credential for $type {
            type Properties = ();
            type Scheme = SecretToken;
            type State = SecretToken;

            const KEY: &'static str = $key;

            fn metadata() -> CredentialMetadata {
                CredentialMetadata::new(
                    nebula_credential::metadata_name!("Stub"),
                    "stub credential",
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

credential_fixture!(SlackOauthCredential, "slack.oauth2");
credential_fixture!(SlackBotCredential, "slack.bot_token");
credential_fixture!(GithubOauthCredential, "github.oauth2");
credential_fixture!(GithubBotCredential, "github.bot_token");

fn credential_fixture(key: &str) -> Arc<dyn AnyCredential> {
    match key {
        "slack.oauth2" => Arc::new(SlackOauthCredential),
        "slack.bot_token" => Arc::new(SlackBotCredential),
        "github.oauth2" => Arc::new(GithubOauthCredential),
        "github.bot_token" => Arc::new(GithubBotCredential),
        _ => panic!("credential fixture {key:?} is not declared"),
    }
}

// ── Stub Plugin ──────────────────────────────────────────────────────────────

struct StubPlugin {
    manifest: PluginManifest,
    actions: Vec<Arc<dyn ActionFactory>>,
    credentials: Vec<Arc<dyn AnyCredential>>,
    resources: Vec<Arc<dyn ResourceFactory>>,
}

impl std::fmt::Debug for StubPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StubPlugin")
            .field("key", self.manifest.key())
            .finish()
    }
}

impl StubPlugin {
    fn new(key: &str) -> Self {
        Self {
            manifest: PluginManifest::builder(key, key).build().unwrap(),
            actions: vec![],
            credentials: vec![],
            resources: vec![],
        }
    }

    fn with_action(mut self, action_key: &'static str) -> Self {
        self.actions.push(action_factory(action_key));
        self
    }

    fn with_credential(mut self, cred_key: &str) -> Self {
        self.credentials.push(credential_fixture(cred_key));
        self
    }

    fn with_resource(mut self, res_key: &'static str) -> Self {
        self.resources.push(resource_factory(res_key));
        self
    }
}

impl Plugin for StubPlugin {
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

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn resolved_plugin_accepts_well_namespaced_action() {
    let plugin = StubPlugin::new("slack").with_action("slack.send_message");
    let resolved = ResolvedPlugin::from(plugin).expect("should resolve");

    let action_key = ActionKey::new("slack.send_message").unwrap();
    assert!(
        resolved.action(&action_key).is_some(),
        "action should be findable by key"
    );
    assert_eq!(resolved.actions().count(), 1);
}

#[test]
fn resolved_plugin_rejects_out_of_namespace_action() {
    let plugin = StubPlugin::new("slack").with_action("api.foo");
    let err = ResolvedPlugin::from(plugin).expect_err("should reject out-of-namespace key");

    assert!(
        matches!(
            err,
            PluginError::NamespaceMismatch {
                kind: ComponentKind::Action,
                ..
            }
        ),
        "expected NamespaceMismatch for action, got: {err}"
    );
}

#[test]
fn resolved_plugin_rejects_duplicate_action_keys() {
    // Two distinct StubAction objects with the same key string.
    let plugin = StubPlugin::new("slack")
        .with_action("slack.send")
        .with_action("slack.send");
    let err = ResolvedPlugin::from(plugin).expect_err("should reject duplicate key");

    assert!(
        matches!(
            err,
            PluginError::DuplicateComponent {
                kind: ComponentKind::Action,
                ..
            }
        ),
        "expected DuplicateComponent for action, got: {err}"
    );
}

#[test]
fn resolved_plugin_accepts_well_namespaced_credential() {
    let plugin = StubPlugin::new("slack").with_credential("slack.oauth2");
    let resolved = ResolvedPlugin::from(plugin).expect("should resolve");

    let key = CredentialKey::new("slack.oauth2").unwrap();
    assert!(resolved.credential(&key).is_some());
}

#[test]
fn typed_credential_blanket_preserves_downcast_identity() {
    let credential = SlackOauthCredential;
    let erased: &dyn AnyCredential = &credential;

    assert!(
        erased
            .as_any()
            .downcast_ref::<SlackOauthCredential>()
            .is_some()
    );
}

#[test]
fn credential_validation_error_is_deterministic_across_contribution_order() {
    let forward = StubPlugin::new("slack")
        .with_credential("slack.oauth2")
        .with_credential("github.oauth2")
        .with_credential("github.bot_token");
    let reversed = StubPlugin::new("slack")
        .with_credential("github.bot_token")
        .with_credential("github.oauth2")
        .with_credential("slack.oauth2");

    let forward_error =
        ResolvedPlugin::from(forward).expect_err("the invalid credential set must fail");
    let reversed_error =
        ResolvedPlugin::from(reversed).expect_err("the invalid credential set must fail");

    for error in [forward_error, reversed_error] {
        std::assert_matches!(
            error,
            PluginError::NamespaceMismatch {
                plugin,
                kind: ComponentKind::Credential,
                offending_key,
            } if plugin.as_str() == "slack" && offending_key == "github.bot_token"
        );
    }
}

#[test]
fn resolved_plugin_rejects_out_of_namespace_credential() {
    let plugin = StubPlugin::new("slack").with_credential("github.oauth2");
    let err = ResolvedPlugin::from(plugin).expect_err("should reject");

    assert!(matches!(
        err,
        PluginError::NamespaceMismatch {
            kind: ComponentKind::Credential,
            ..
        }
    ));
}

#[test]
fn resolved_plugin_rejects_duplicate_credentials() {
    let plugin = StubPlugin::new("slack")
        .with_credential("slack.oauth2")
        .with_credential("slack.oauth2");
    let err = ResolvedPlugin::from(plugin).expect_err("should reject duplicate");

    assert!(matches!(
        err,
        PluginError::DuplicateComponent {
            kind: ComponentKind::Credential,
            ..
        }
    ));
}

#[test]
fn resolved_plugin_accepts_well_namespaced_resource() {
    let plugin = StubPlugin::new("slack").with_resource("slack.http_client");
    let resolved = ResolvedPlugin::from(plugin).expect("should resolve");

    let key = ResourceKey::new("slack.http_client").unwrap();
    assert!(resolved.resource(&key).is_some());
}

#[test]
fn resolved_plugin_rejects_out_of_namespace_resource() {
    let plugin = StubPlugin::new("slack").with_resource("api.http_client");
    let err = ResolvedPlugin::from(plugin).expect_err("should reject");

    assert!(matches!(
        err,
        PluginError::NamespaceMismatch {
            kind: ComponentKind::Resource,
            ..
        }
    ));
}

#[test]
fn resolved_plugin_rejects_duplicate_resources() {
    let plugin = StubPlugin::new("slack")
        .with_resource("slack.http_client")
        .with_resource("slack.http_client");
    let err = ResolvedPlugin::from(plugin).expect_err("should reject duplicate");

    assert!(matches!(
        err,
        PluginError::DuplicateComponent {
            kind: ComponentKind::Resource,
            ..
        }
    ));
}

#[test]
fn resolved_plugin_with_no_components_is_valid() {
    let plugin = StubPlugin::new("empty");
    let resolved = ResolvedPlugin::from(plugin).expect("empty plugin should be valid");
    assert_eq!(resolved.actions().count(), 0);
    assert_eq!(resolved.credentials().count(), 0);
    assert_eq!(resolved.resources().count(), 0);
    assert_eq!(resolved.key().as_str(), "empty");
}

// ============================================================
// PluginRegistry aggregate accessors (PR 5)
// ============================================================

use nebula_plugin::PluginRegistry;

#[test]
fn registry_resolve_action_finds_across_plugins() {
    let mut reg = PluginRegistry::new();

    reg.register(Arc::new(
        ResolvedPlugin::from(StubPlugin::new("slack").with_action("slack.send_message")).unwrap(),
    ))
    .unwrap();
    reg.register(Arc::new(
        ResolvedPlugin::from(
            StubPlugin::new("http")
                .with_action("http.get")
                .with_action("http.post"),
        )
        .unwrap(),
    ))
    .unwrap();

    // Hits the Slack plugin's cache.
    let action = reg
        .resolve_action(&ActionKey::new("slack.send_message").unwrap())
        .expect("slack action");
    assert_eq!(
        action.definition().base().key().as_str(),
        "slack.send_message"
    );

    // Hits the HTTP plugin's cache.
    let http_post = reg
        .resolve_action(&ActionKey::new("http.post").unwrap())
        .expect("http post");
    assert_eq!(http_post.definition().base().key().as_str(), "http.post");

    // Unknown key: no match.
    assert!(
        reg.resolve_action(&ActionKey::new("unknown.key").unwrap())
            .is_none()
    );
}

#[test]
fn registry_all_actions_yields_every_action() {
    let mut reg = PluginRegistry::new();
    reg.register(Arc::new(
        ResolvedPlugin::from(StubPlugin::new("slack").with_action("slack.send_message")).unwrap(),
    ))
    .unwrap();
    reg.register(Arc::new(
        ResolvedPlugin::from(StubPlugin::new("http").with_action("http.get")).unwrap(),
    ))
    .unwrap();

    assert_eq!(reg.all_actions().count(), 2);

    let keys: Vec<&str> = reg
        .all_actions()
        .map(|(_pk, a)| a.definition().base().key().as_str())
        .collect();
    assert!(keys.contains(&"slack.send_message"));
    assert!(keys.contains(&"http.get"));
}

#[test]
fn registry_resolve_credential_finds_across_plugins() {
    let mut reg = PluginRegistry::new();
    reg.register(Arc::new(
        ResolvedPlugin::from(StubPlugin::new("slack").with_credential("slack.oauth2")).unwrap(),
    ))
    .unwrap();

    let cred = reg
        .resolve_credential(&CredentialKey::new("slack.oauth2").unwrap())
        .expect("oauth2");
    assert_eq!(
        cred.definition()
            .expect("fixture metadata is valid")
            .base()
            .key()
            .as_str(),
        "slack.oauth2"
    );

    assert!(
        reg.resolve_credential(&CredentialKey::new("nope.x").unwrap())
            .is_none()
    );
}

#[test]
fn registry_all_credentials_yields_every_credential() {
    let mut reg = PluginRegistry::new();
    reg.register(Arc::new(
        ResolvedPlugin::from(
            StubPlugin::new("slack")
                .with_credential("slack.oauth2")
                .with_credential("slack.bot_token"),
        )
        .unwrap(),
    ))
    .unwrap();
    assert_eq!(reg.all_credentials().count(), 2);
}

#[test]
fn registry_resolve_resource_finds_across_plugins() {
    let mut reg = PluginRegistry::new();
    reg.register(Arc::new(
        ResolvedPlugin::from(StubPlugin::new("http").with_resource("http.client")).unwrap(),
    ))
    .unwrap();

    let res = reg
        .resolve_resource(&ResourceKey::new("http.client").unwrap())
        .expect("client");
    assert_eq!(
        res.definition()
            .expect("fixture metadata is valid")
            .base()
            .key()
            .as_str(),
        "http.client"
    );
}

#[test]
fn registry_all_resources_yields_every_resource() {
    let mut reg = PluginRegistry::new();
    reg.register(Arc::new(
        ResolvedPlugin::from(
            StubPlugin::new("http")
                .with_resource("http.client")
                .with_resource("http.pool"),
        )
        .unwrap(),
    ))
    .unwrap();
    assert_eq!(reg.all_resources().count(), 2);
}

#[test]
fn mutable_and_frozen_accessors_preserve_component_ownership_across_plugins() {
    let slack = Arc::new(
        ResolvedPlugin::from(
            StubPlugin::new("slack")
                .with_action("slack.send")
                .with_credential("slack.oauth2")
                .with_resource("slack.http_client"),
        )
        .unwrap(),
    );
    let github = Arc::new(
        ResolvedPlugin::from(
            StubPlugin::new("github")
                .with_action("github.send")
                .with_credential("github.oauth2"),
        )
        .unwrap(),
    );
    let http = Arc::new(
        ResolvedPlugin::from(
            StubPlugin::new("http")
                .with_action("http.send")
                .with_resource("http.client"),
        )
        .unwrap(),
    );
    let mut registry = PluginRegistry::new();
    for plugin in [&http, &github, &slack] {
        registry.register(Arc::clone(plugin)).unwrap();
    }

    let credential_key = CredentialKey::new("github.oauth2").unwrap();
    let resource_key = ResourceKey::new("http.client").unwrap();
    let action_key = ActionKey::new("slack.send").unwrap();
    assert!(Arc::ptr_eq(
        &registry.resolve_credential(&credential_key).unwrap(),
        github.credential(&credential_key).unwrap(),
    ));
    assert!(Arc::ptr_eq(
        &registry.resolve_resource(&resource_key).unwrap(),
        http.resource(&resource_key).unwrap(),
    ));
    assert!(Arc::ptr_eq(
        &registry.resolve_action(&action_key).unwrap(),
        slack.action(&action_key).unwrap(),
    ));

    let frozen = registry
        .freeze(
            nebula_core::ArtifactSetDigest::from_bytes([0x31; 32]),
            "1.0.0".parse().unwrap(),
        )
        .unwrap();
    let mut actions: Vec<_> = frozen
        .all_actions()
        .map(|(plugin, action)| {
            (
                plugin.to_string(),
                action.definition().base().key().to_string(),
            )
        })
        .collect();
    actions.sort();
    assert_eq!(
        actions,
        [
            ("github".to_owned(), "github.send".to_owned()),
            ("http".to_owned(), "http.send".to_owned()),
            ("slack".to_owned(), "slack.send".to_owned()),
        ],
    );
    let mut credentials: Vec<_> = frozen
        .all_credentials()
        .map(|(plugin, credential)| (plugin.to_string(), credential.key().to_string()))
        .collect();
    credentials.sort();
    assert_eq!(
        credentials,
        [
            ("github".to_owned(), "github.oauth2".to_owned()),
            ("slack".to_owned(), "slack.oauth2".to_owned()),
        ],
    );
    let mut resources: Vec<_> = frozen
        .all_resources()
        .map(|(plugin, resource)| (plugin.to_string(), resource.key().to_string()))
        .collect();
    resources.sort();
    assert_eq!(
        resources,
        [
            ("http".to_owned(), "http.client".to_owned()),
            ("slack".to_owned(), "slack.http_client".to_owned()),
        ],
    );
    assert!(Arc::ptr_eq(
        &frozen.resolve_credential(&credential_key).unwrap(),
        github.credential(&credential_key).unwrap(),
    ));
    assert!(Arc::ptr_eq(
        &frozen.resolve_resource(&resource_key).unwrap(),
        http.resource(&resource_key).unwrap(),
    ));
    assert!(Arc::ptr_eq(
        &frozen.resolve_action(&action_key).unwrap(),
        slack.action(&action_key).unwrap(),
    ));
    assert!(
        frozen
            .resolve_credential(&CredentialKey::new("unknown.oauth2").unwrap())
            .is_none(),
    );
    assert!(
        frozen
            .resolve_resource(&ResourceKey::new("unknown.client").unwrap())
            .is_none(),
    );
    assert!(
        frozen
            .resolve_action(&ActionKey::new("unknown.send").unwrap())
            .is_none(),
    );
}
