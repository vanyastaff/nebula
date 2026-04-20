//! `ResolvedPlugin` — per-plugin resolved wrapper with eager component caches.
//!
//! `ResolvedPlugin::from` calls `plugin.actions()` / `credentials()` /
//! `resources()` exactly once, validates the namespace invariant (every full
//! key starts with `{plugin.key()}.`), and builds three flat
//! `HashMap<FullKey, Arc<dyn …>>` indices for O(1) lookup. Within-plugin
//! duplicate keys surface as `PluginError::DuplicateComponent`; out-of-
//! namespace keys surface as `PluginError::NamespaceMismatch`.
//!
//! See ADR-0027 and `docs/pitfalls.md`.

use std::{collections::HashMap, sync::Arc};

use nebula_action::Action;
use nebula_core::{ActionKey, CredentialKey, PluginKey, ResourceKey};
use nebula_credential::AnyCredential;
use nebula_metadata::PluginManifest;
use nebula_resource::AnyResource;
use semver::Version;

use crate::{ComponentKind, PluginError, plugin::Plugin};

/// Per-plugin resolved wrapper with eager O(1) component lookups.
///
/// Constructed via [`ResolvedPlugin::from`], which calls `plugin.actions()`,
/// `credentials()`, and `resources()` exactly once, validates that every
/// component key starts with `{plugin.key()}.`, and catches within-plugin
/// duplicate keys. Once constructed, component lookup is O(1).
pub struct ResolvedPlugin {
    plugin: Arc<dyn Plugin>,
    actions: HashMap<ActionKey, Arc<dyn Action>>,
    credentials: HashMap<CredentialKey, Arc<dyn AnyCredential>>,
    resources: HashMap<ResourceKey, Arc<dyn AnyResource>>,
}

impl std::fmt::Debug for ResolvedPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedPlugin")
            .field("key", self.plugin.key())
            .field("version", self.plugin.version())
            .field("action_count", &self.actions.len())
            .field("credential_count", &self.credentials.len())
            .field("resource_count", &self.resources.len())
            .finish()
    }
}

impl ResolvedPlugin {
    /// Construct from an `impl Plugin`. Eagerly resolves component lists, checks
    /// the namespace invariant, catches within-plugin duplicate keys.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::NamespaceMismatch`] if any component key does not
    /// start with `{plugin.key()}.`; returns [`PluginError::DuplicateComponent`]
    /// if two components of the same kind share a key.
    pub fn from<P: Plugin + 'static>(plugin: P) -> Result<Self, PluginError> {
        let plugin_key = plugin.manifest().key().clone();
        let prefix = format!("{}.", plugin_key.as_str());

        let actions = Self::build_action_index(&plugin_key, &prefix, plugin.actions())?;
        let credentials = Self::build_credential_index(&plugin_key, &prefix, plugin.credentials())?;
        let resources = Self::build_resource_index(&plugin_key, &prefix, plugin.resources())?;

        Ok(Self {
            plugin: Arc::new(plugin) as Arc<dyn Plugin>,
            actions,
            credentials,
            resources,
        })
    }

    /// The underlying plugin.
    pub fn plugin(&self) -> &Arc<dyn Plugin> {
        &self.plugin
    }

    /// The plugin manifest.
    pub fn manifest(&self) -> &PluginManifest {
        self.plugin.manifest()
    }

    /// The plugin key.
    pub fn key(&self) -> &PluginKey {
        self.plugin.key()
    }

    /// The plugin version.
    pub fn version(&self) -> &Version {
        self.plugin.version()
    }

    /// Look up an action by key.
    pub fn action(&self, key: &ActionKey) -> Option<&Arc<dyn Action>> {
        self.actions.get(key)
    }

    /// Look up a credential by key.
    pub fn credential(&self, key: &CredentialKey) -> Option<&Arc<dyn AnyCredential>> {
        self.credentials.get(key)
    }

    /// Look up a resource by key.
    pub fn resource(&self, key: &ResourceKey) -> Option<&Arc<dyn AnyResource>> {
        self.resources.get(key)
    }

    /// Iterate all registered actions.
    pub fn actions(&self) -> impl Iterator<Item = (&ActionKey, &Arc<dyn Action>)> {
        self.actions.iter()
    }

    /// Iterate all registered credentials.
    pub fn credentials(&self) -> impl Iterator<Item = (&CredentialKey, &Arc<dyn AnyCredential>)> {
        self.credentials.iter()
    }

    /// Iterate all registered resources.
    pub fn resources(&self) -> impl Iterator<Item = (&ResourceKey, &Arc<dyn AnyResource>)> {
        self.resources.iter()
    }

    fn build_action_index(
        plugin_key: &PluginKey,
        prefix: &str,
        raw: Vec<Arc<dyn Action>>,
    ) -> Result<HashMap<ActionKey, Arc<dyn Action>>, PluginError> {
        let mut out = HashMap::with_capacity(raw.len());
        for action in raw {
            let key = action.metadata().base.key.clone();
            if !key.as_str().starts_with(prefix) {
                return Err(PluginError::NamespaceMismatch {
                    plugin: plugin_key.clone(),
                    offending_key: key.as_str().to_owned(),
                    kind: ComponentKind::Action,
                });
            }
            if out.contains_key(&key) {
                return Err(PluginError::DuplicateComponent {
                    plugin: plugin_key.clone(),
                    key: key.as_str().to_owned(),
                    kind: ComponentKind::Action,
                });
            }
            out.insert(key, action);
        }
        Ok(out)
    }

    fn build_credential_index(
        plugin_key: &PluginKey,
        prefix: &str,
        raw: Vec<Arc<dyn AnyCredential>>,
    ) -> Result<HashMap<CredentialKey, Arc<dyn AnyCredential>>, PluginError> {
        let mut out = HashMap::with_capacity(raw.len());
        for cred in raw {
            // AnyCredential::credential_key() returns &str (the KEY const)
            let key_str = cred.credential_key();
            if !key_str.starts_with(prefix) {
                return Err(PluginError::NamespaceMismatch {
                    plugin: plugin_key.clone(),
                    offending_key: key_str.to_owned(),
                    kind: ComponentKind::Credential,
                });
            }
            let key: CredentialKey =
                key_str
                    .parse()
                    .map_err(|_| PluginError::NamespaceMismatch {
                        plugin: plugin_key.clone(),
                        offending_key: key_str.to_owned(),
                        kind: ComponentKind::Credential,
                    })?;
            if out.contains_key(&key) {
                return Err(PluginError::DuplicateComponent {
                    plugin: plugin_key.clone(),
                    key: key.as_str().to_owned(),
                    kind: ComponentKind::Credential,
                });
            }
            out.insert(key, cred);
        }
        Ok(out)
    }

    fn build_resource_index(
        plugin_key: &PluginKey,
        prefix: &str,
        raw: Vec<Arc<dyn AnyResource>>,
    ) -> Result<HashMap<ResourceKey, Arc<dyn AnyResource>>, PluginError> {
        let mut out = HashMap::with_capacity(raw.len());
        for res in raw {
            let key = res.key();
            if !key.as_str().starts_with(prefix) {
                return Err(PluginError::NamespaceMismatch {
                    plugin: plugin_key.clone(),
                    offending_key: key.as_str().to_owned(),
                    kind: ComponentKind::Resource,
                });
            }
            if out.contains_key(&key) {
                return Err(PluginError::DuplicateComponent {
                    plugin: plugin_key.clone(),
                    key: key.as_str().to_owned(),
                    kind: ComponentKind::Resource,
                });
            }
            out.insert(key, res);
        }
        Ok(out)
    }
}
