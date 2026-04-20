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
