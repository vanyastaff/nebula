//! Integration tests for `ResolvedPlugin` — namespace enforcement and lookup.

use std::{any::TypeId, future::Future, pin::Pin, sync::Arc};

use nebula_action::{ActionContext, ActionError, ActionFactory, ActionHandle, ActionMetadata};
use nebula_core::{ActionKey, CredentialKey, Dependencies, ResourceKey};
use nebula_credential::{AnyCredential, AuthPattern, Capabilities, CredentialMetadata};
use nebula_metadata::PluginManifest;
use nebula_plugin::{ComponentKind, Plugin, PluginError, ResolvedPlugin};
use nebula_resource::{
    ResourceFactory, ResourceMetadata, SlotIdentity,
    factory::{BoxFut, RegisterRequest},
};
use nebula_schema::ValidSchema;
use nebula_workflow::NodeDefinition;

// ── Stub ActionFactory ───────────────────────────────────────────────────────
//
// The plugin contract returns `Vec<Arc<dyn ActionFactory>>`. For these tests
// we only need the metadata side — `instantiate` is never invoked because
// the tests only check namespace / dedup / registration. A stub factory that
// errors on `instantiate` is sufficient.

struct StubAction {
    metadata: ActionMetadata,
    dependencies: Dependencies,
}

impl StubAction {
    fn new(key: &str) -> Self {
        Self {
            metadata: ActionMetadata::new(
                ActionKey::new(key).expect("valid action key"),
                key,
                "stub",
            ),
            dependencies: Dependencies::new(),
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

impl ActionFactory for StubAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }

    fn dependencies(&self) -> &Dependencies {
        &self.dependencies
    }

    fn instantiate<'a>(
        &'a self,
        _node: &'a NodeDefinition,
        _ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
        Box::pin(async {
            Err(ActionError::fatal(
                "StubAction::instantiate is a test stub — should never be invoked",
            ))
        })
    }
}

// ── Stub AnyCredential ───────────────────────────────────────────────────────

struct StubCredential {
    projected_key: String,
    metadata_key: CredentialKey,
    mismatched_downcast_projection: bool,
}

impl StubCredential {
    fn new(key: &str) -> Self {
        let key = CredentialKey::new(key).expect("valid credential key");
        Self {
            projected_key: key.as_str().to_owned(),
            metadata_key: key,
            mismatched_downcast_projection: false,
        }
    }

    fn with_metadata_key(projected_key: &str, metadata_key: &str) -> Self {
        Self {
            projected_key: projected_key.to_owned(),
            metadata_key: CredentialKey::new(metadata_key).expect("valid metadata credential key"),
            mismatched_downcast_projection: false,
        }
    }

    fn with_mismatched_downcast_projection(key: &str) -> Self {
        let mut credential = Self::new(key);
        credential.mismatched_downcast_projection = true;
        credential
    }

    fn with_invalid_projected_key(projected_key: &str, metadata_key: &str) -> Self {
        Self {
            projected_key: projected_key.to_owned(),
            metadata_key: CredentialKey::new(metadata_key).expect("valid metadata credential key"),
            mismatched_downcast_projection: false,
        }
    }
}

impl std::fmt::Debug for StubCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StubCredential")
            .field("projected_key", &self.projected_key)
            .field("metadata_key", &self.metadata_key)
            .finish()
    }
}

impl AnyCredential for StubCredential {
    fn credential_key(&self) -> &str {
        &self.projected_key
    }

    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata::new(
            self.metadata_key.clone(),
            "Stub",
            "stub credential",
            ValidSchema::empty(),
            AuthPattern::SecretToken,
        )
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::empty()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        static DIFFERENT_CONCRETE_TYPE: u8 = 0;

        if self.mismatched_downcast_projection {
            &DIFFERENT_CONCRETE_TYPE
        } else {
            self
        }
    }
}

// ── Stub ResourceFactory ─────────────────────────────────────────────────────
//
// Implements the B+ merged `ResourceFactory` contract (ADR-0095 D2).
// The introspection arm (`key`, `metadata`, `validate`) is the only part
// exercised by the namespace/dedup tests; `register` is a stub that always
// returns `SlotIdentity::Unbound` because these tests never call it.

struct StubResource {
    factory_key: ResourceKey,
    metadata_key: ResourceKey,
    dependencies: Dependencies,
}

impl StubResource {
    fn new(key: &str) -> Self {
        let key = ResourceKey::new(key).expect("valid resource key");
        Self {
            factory_key: key.clone(),
            metadata_key: key,
            dependencies: Dependencies::new(),
        }
    }

    fn with_metadata_key(factory_key: &str, metadata_key: &str) -> Self {
        Self {
            factory_key: ResourceKey::new(factory_key).expect("valid factory resource key"),
            metadata_key: ResourceKey::new(metadata_key).expect("valid metadata resource key"),
            dependencies: Dependencies::new(),
        }
    }
}

impl std::fmt::Debug for StubResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StubResource")
            .field("factory_key", &self.factory_key)
            .field("metadata_key", &self.metadata_key)
            .finish()
    }
}

impl ResourceFactory for StubResource {
    fn key(&self) -> ResourceKey {
        self.factory_key.clone()
    }

    fn dependencies(&self) -> &Dependencies {
        &self.dependencies
    }

    fn resource_type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::from_key(&self.metadata_key)
    }

    fn validate(&self, _config_json: serde_json::Value) -> Result<(), nebula_resource::Error> {
        Ok(())
    }

    fn register<'a>(
        &'a self,
        _manager: &'a nebula_resource::Manager,
        _request: RegisterRequest<'a>,
    ) -> BoxFut<'a, Result<SlotIdentity, nebula_resource::Error>> {
        // Test stub: register is never invoked by the namespace/dedup tests.
        Box::pin(async { Ok(SlotIdentity::Unbound) })
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
        self.actions.push(Arc::new(StubAction::new(action_key)));
        self
    }

    fn with_credential(mut self, cred_key: &str) -> Self {
        self.credentials
            .push(Arc::new(StubCredential::new(cred_key)));
        self
    }

    fn with_mismatched_credential(mut self, projected_key: &str, metadata_key: &str) -> Self {
        self.credentials
            .push(Arc::new(StubCredential::with_metadata_key(
                projected_key,
                metadata_key,
            )));
        self
    }

    fn with_invalid_credential_projection(
        mut self,
        projected_key: &str,
        metadata_key: &str,
    ) -> Self {
        self.credentials
            .push(Arc::new(StubCredential::with_invalid_projected_key(
                projected_key,
                metadata_key,
            )));
        self
    }

    fn with_mismatched_credential_type(mut self, key: &str) -> Self {
        self.credentials.push(Arc::new(
            StubCredential::with_mismatched_downcast_projection(key),
        ));
        self
    }

    fn with_resource(mut self, res_key: &'static str) -> Self {
        self.resources.push(Arc::new(StubResource::new(res_key)));
        self
    }

    fn with_mismatched_resource(mut self, factory_key: &str, metadata_key: &str) -> Self {
        self.resources
            .push(Arc::new(StubResource::with_metadata_key(
                factory_key,
                metadata_key,
            )));
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
fn resolved_plugin_rejects_mismatched_credential_downcast_type() {
    let plugin = StubPlugin::new("slack").with_mismatched_credential_type("slack.oauth2");
    let error = ResolvedPlugin::from(plugin).expect_err("mismatched type projection must fail");

    assert!(matches!(
        error,
        PluginError::ComponentTypeMismatch {
            plugin,
            kind: ComponentKind::Credential,
            key,
        } if plugin.as_str() == "slack" && key == "slack.oauth2"
    ));
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

    assert_eq!(forward_error, reversed_error);
    assert!(matches!(
        forward_error,
        PluginError::InvalidComponentKey {
            plugin,
            kind: ComponentKind::Credential,
            projected_key,
        } if plugin.as_str() == "slack" && projected_key == "slack.aaa!"
    ));
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
fn resolved_plugin_rejects_resource_key_metadata_mismatch() {
    let plugin = StubPlugin::new("slack")
        .with_mismatched_resource("slack.http_client", "slack.audit_client");
    let error = ResolvedPlugin::from(plugin).expect_err("key mismatch must fail resolution");

    assert!(matches!(
        error,
        PluginError::ComponentKeyMismatch {
            plugin,
            kind: ComponentKind::Resource,
            projected_key,
            metadata_key,
        } if plugin.as_str() == "slack"
            && projected_key == "slack.http_client"
            && metadata_key == "slack.audit_client"
    ));
}

#[test]
fn resource_validation_error_is_deterministic_across_contribution_order() {
    let forward = StubPlugin::new("slack")
        .with_mismatched_resource("slack.beta", "slack.aaa")
        .with_mismatched_resource("slack.alpha", "slack.zzz")
        .with_mismatched_resource("slack.alpha", "slack.aaa");
    let reversed = StubPlugin::new("slack")
        .with_mismatched_resource("slack.alpha", "slack.aaa")
        .with_mismatched_resource("slack.alpha", "slack.zzz")
        .with_mismatched_resource("slack.beta", "slack.aaa");

    let forward_error =
        ResolvedPlugin::from(forward).expect_err("the invalid resource set must fail");
    let reversed_error =
        ResolvedPlugin::from(reversed).expect_err("the invalid resource set must fail");

    assert_eq!(forward_error, reversed_error);
    assert!(matches!(
        forward_error,
        PluginError::ComponentKeyMismatch {
            plugin,
            kind: ComponentKind::Resource,
            projected_key,
            metadata_key,
        } if plugin.as_str() == "slack"
            && projected_key == "slack.alpha"
            && metadata_key == "slack.aaa"
    ));
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
