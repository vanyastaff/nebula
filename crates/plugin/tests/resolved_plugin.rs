//! Integration tests for `ResolvedPlugin` — namespace enforcement and lookup.

use std::sync::{Arc, OnceLock};

use nebula_action::{
    Action, ActionContext, ActionError, ActionFactory, ActionMetadataDraft, ActionResult,
    InstanceFactory, StatelessAction,
};
use nebula_core::{ActionKey, CredentialKey, Dependencies, ResourceKey};
use nebula_credential::{
    AnyCredential, Credential, CredentialContext, CredentialMetadataDraft, SecretString,
    SecretToken, contract::plugin_capability_report, error::CredentialError,
    resolve::StaticResolveResult,
};
use nebula_metadata::{Metadata, PluginManifest};
use nebula_plugin::{ComponentKind, Plugin, PluginError, ResolvedPlugin};
use nebula_resource::{
    KindActivator, Provider, Resident, ResidentConfig, ResourceFactory, ResourceMetadataDraft,
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

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("MetadataFixture"),
            "",
        )
    }

    fn key() -> ResourceKey {
        let key = match INDEX {
            0 => "slack.http_client",
            1 => "slack.audit_client",
            2 => "slack.aaa",
            3 => "slack.zzz",
            4 => "api.http_client",
            5 => "http.client",
            6 => "http.pool",
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
        ResourceMetadataDraft::new(
            MetadataFixture::<INDEX>::key(),
            nebula_resource::metadata_name!("Metadata fixture"),
            "",
        ),
        || MetadataFixture,
        || Resident::new(ResidentConfig::default()),
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

fn action_factory(key: &str) -> Arc<dyn ActionFactory> {
    Arc::new(
        InstanceFactory::new(
            ActionMetadataDraft::new(
                ActionKey::new(key).expect("valid action key"),
                nebula_action::MetadataName::try_from(key).expect("fixture display name"),
                "stub",
            ),
            MetadataAction,
        )
        .expect("stub metadata admits through its structural factory"),
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

// ── Stub AnyCredential ───────────────────────────────────────────────────────

macro_rules! credential_fixture {
    ($type:ident, $projected_key:literal, $metadata_key:literal) => {
        struct $type;

        impl Credential for $type {
            type Properties = ();
            type Scheme = SecretToken;
            type State = SecretToken;

            const KEY: &'static str = $projected_key;

            fn metadata() -> CredentialMetadataDraft {
                CredentialMetadataDraft::new(
                    nebula_core::credential_key!($metadata_key),
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

credential_fixture!(SlackOauthCredential, "slack.oauth2", "slack.oauth2");
credential_fixture!(SlackBotCredential, "slack.bot_token", "slack.bot_token");
credential_fixture!(GithubOauthCredential, "github.oauth2", "github.oauth2");
credential_fixture!(MismatchedCredential, "slack.oauth2", "slack.bot_token");
credential_fixture!(InvalidBadCredential, "slack.bad!", "slack.oauth2");
credential_fixture!(InvalidLowCredential, "slack.low!", "slack.beta");
credential_fixture!(InvalidZzzCredential, "slack.zzz!", "slack.alpha");
credential_fixture!(InvalidAaaCredential, "slack.aaa!", "slack.alpha");

fn credential_fixture(projected_key: &str, metadata_key: &str) -> Arc<dyn AnyCredential> {
    match (projected_key, metadata_key) {
        ("slack.oauth2", "slack.oauth2") => Arc::new(SlackOauthCredential),
        ("slack.bot_token", "slack.bot_token") => Arc::new(SlackBotCredential),
        ("github.oauth2", "github.oauth2") => Arc::new(GithubOauthCredential),
        ("slack.oauth2", "slack.bot_token") => Arc::new(MismatchedCredential),
        ("slack.bad!", "slack.oauth2") => Arc::new(InvalidBadCredential),
        ("slack.low!", "slack.beta") => Arc::new(InvalidLowCredential),
        ("slack.zzz!", "slack.alpha") => Arc::new(InvalidZzzCredential),
        ("slack.aaa!", "slack.alpha") => Arc::new(InvalidAaaCredential),
        _ => panic!("credential fixture ({projected_key:?}, {metadata_key:?}) is not declared"),
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
        self.credentials
            .push(credential_fixture(cred_key, cred_key));
        self
    }

    fn with_mismatched_credential(mut self, projected_key: &str, metadata_key: &str) -> Self {
        self.credentials
            .push(credential_fixture(projected_key, metadata_key));
        self
    }

    fn with_invalid_credential_projection(
        mut self,
        projected_key: &str,
        metadata_key: &str,
    ) -> Self {
        self.credentials
            .push(credential_fixture(projected_key, metadata_key));
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
fn resolved_plugin_rejects_credential_key_metadata_mismatch() {
    let plugin =
        StubPlugin::new("slack").with_mismatched_credential("slack.oauth2", "slack.bot_token");
    let error = ResolvedPlugin::from(plugin).expect_err("key mismatch must fail resolution");

    assert!(matches!(
        error,
        PluginError::ComponentKeyMismatch {
            plugin,
            kind: ComponentKind::Credential,
            projected_key,
            metadata_key,
        } if plugin.as_str() == "slack"
            && projected_key == "slack.oauth2"
            && metadata_key == "slack.bot_token"
    ));
}

#[test]
fn resolved_plugin_rejects_invalid_credential_key_projection() {
    let plugin =
        StubPlugin::new("slack").with_invalid_credential_projection("slack.bad!", "slack.oauth2");
    let error = ResolvedPlugin::from(plugin).expect_err("invalid projected key must fail");

    assert!(matches!(
        error,
        PluginError::InvalidComponentKey {
            plugin,
            kind: ComponentKind::Credential,
            projected_key,
        } if plugin.as_str() == "slack" && projected_key == "slack.bad!"
    ));
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
        .with_invalid_credential_projection("slack.low!", "slack.beta")
        .with_invalid_credential_projection("slack.zzz!", "slack.alpha")
        .with_invalid_credential_projection("slack.aaa!", "slack.alpha");
    let reversed = StubPlugin::new("slack")
        .with_invalid_credential_projection("slack.aaa!", "slack.alpha")
        .with_invalid_credential_projection("slack.zzz!", "slack.alpha")
        .with_invalid_credential_projection("slack.low!", "slack.beta");

    let forward_error =
        ResolvedPlugin::from(forward).expect_err("the invalid credential set must fail");
    let reversed_error =
        ResolvedPlugin::from(reversed).expect_err("the invalid credential set must fail");

    for error in [forward_error, reversed_error] {
        std::assert_matches!(
            error,
            PluginError::InvalidComponentKey {
                plugin,
                kind: ComponentKind::Credential,
                projected_key,
            } if plugin.as_str() == "slack" && projected_key == "slack.aaa!"
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
        action.metadata().base().key().as_str(),
        "slack.send_message"
    );

    // Hits the HTTP plugin's cache.
    let http_post = reg
        .resolve_action(&ActionKey::new("http.post").unwrap())
        .expect("http post");
    assert_eq!(http_post.metadata().base().key().as_str(), "http.post");

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
        .map(|(_pk, a)| a.metadata().base().key().as_str())
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
        cred.metadata()
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
        res.metadata()
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
