//! Integration tests for `ResolvedPlugin` — namespace enforcement and lookup.

use std::sync::Arc;

use nebula_action::{Action, ActionDependencies, ActionMetadata};
use nebula_core::{ActionKey, AuthPattern, CredentialKey, ResourceKey};
use nebula_credential::{AnyCredential, CredentialMetadata};
use nebula_metadata::PluginManifest;
use nebula_plugin::{ComponentKind, Plugin, PluginError, ResolvedPlugin};
use nebula_resource::{AnyResource, ResourceMetadata};
use nebula_schema::ValidSchema;

// ── Stub Action ──────────────────────────────────────────────────────────────

struct StubAction {
    metadata: ActionMetadata,
}

impl StubAction {
    fn new(key: &str) -> Self {
        Self {
            metadata: ActionMetadata::new(
                ActionKey::new(key).expect("valid action key"),
                key,
                "stub",
            ),
        }
    }
}

impl std::fmt::Debug for StubAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StubAction")
            .field("key", &self.metadata.base.key)
            .finish()
    }
}

impl ActionDependencies for StubAction {}

impl Action for StubAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

// ── Stub AnyCredential ───────────────────────────────────────────────────────

struct StubCredential {
    key: CredentialKey,
}

impl StubCredential {
    fn new(key: &str) -> Self {
        Self {
            key: CredentialKey::new(key).expect("valid credential key"),
        }
    }
}

impl std::fmt::Debug for StubCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StubCredential")
            .field("key", &self.key)
            .finish()
    }
}

impl AnyCredential for StubCredential {
    fn credential_key(&self) -> &str {
        self.key.as_str()
    }

    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata::new(
            self.key.clone(),
            "Stub",
            "stub credential",
            ValidSchema::empty(),
            AuthPattern::SecretToken,
        )
    }
}

// ── Stub AnyResource ─────────────────────────────────────────────────────────

struct StubResource {
    key: ResourceKey,
}

impl StubResource {
    fn new(key: &str) -> Self {
        Self {
            key: ResourceKey::new(key).expect("valid resource key"),
        }
    }
}

impl std::fmt::Debug for StubResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StubResource")
            .field("key", &self.key)
            .finish()
    }
}

impl AnyResource for StubResource {
    fn key(&self) -> ResourceKey {
        self.key.clone()
    }

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::from_key(&self.key)
    }
}

// ── Stub Plugin ──────────────────────────────────────────────────────────────

struct StubPlugin {
    manifest: PluginManifest,
    actions: Vec<Arc<dyn Action>>,
    credentials: Vec<Arc<dyn AnyCredential>>,
    resources: Vec<Arc<dyn AnyResource>>,
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
        self.actions.push(Arc::new(StubAction::new(action_key)));
        self
    }

    fn with_credential(mut self, cred_key: &str) -> Self {
        self.credentials
            .push(Arc::new(StubCredential::new(cred_key)));
        self
    }

    fn with_resource(mut self, res_key: &'static str) -> Self {
        self.resources.push(Arc::new(StubResource::new(res_key)));
        self
    }
}

impl Plugin for StubPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn actions(&self) -> Vec<Arc<dyn Action>> {
        self.actions.clone()
    }

    fn credentials(&self) -> Vec<Arc<dyn AnyCredential>> {
        self.credentials.clone()
    }

    fn resources(&self) -> Vec<Arc<dyn AnyResource>> {
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
    assert_eq!(action.metadata().base.key.as_str(), "slack.send_message");

    // Hits the HTTP plugin's cache.
    let http_post = reg
        .resolve_action(&ActionKey::new("http.post").unwrap())
        .expect("http post");
    assert_eq!(http_post.metadata().base.key.as_str(), "http.post");

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
        .map(|(_pk, a)| a.metadata().base.key.as_str())
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
    assert_eq!(cred.metadata().base.key.as_str(), "slack.oauth2");

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
    assert_eq!(res.metadata().base.key.as_str(), "http.client");
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
