//! `ResolvedPlugin` — immutable component-contract snapshots and eager caches.
//!
//! `ResolvedPlugin::from` calls `plugin.actions()` / `credentials()` /
//! `resources()` exactly once, reads each erased projection once, and validates
//! captured facts before building three O(1) lookup indices. Registry freeze
//! consumes the captured keys; the remaining crate-private accessors are
//! reserved for the approved compiler phase so it need not reopen erased
//! introspection traits.
//!
//! See `docs/pitfalls.md`.

use std::{any::TypeId, collections::HashMap, sync::Arc};

use nebula_action::{ActionFactory, ActionMetadata};
use nebula_core::{ActionKey, CredentialKey, Dependencies, PluginKey, ResourceKey};
use nebula_credential::{AnyCredential, Capabilities, CredentialMetadata};
use nebula_metadata::PluginManifest;
use nebula_resource::{ResourceFactory, ResourceMetadata};
use semver::Version;

use crate::{ComponentKind, PluginError, plugin::Plugin};

pub(crate) struct ActionContractSnapshot {
    metadata: ActionMetadata,
    dependencies: Dependencies,
}

impl ActionContractSnapshot {
    pub(crate) const fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }

    #[expect(dead_code, reason = "reserved for the approved compiler phase")]
    pub(crate) const fn dependencies(&self) -> &Dependencies {
        &self.dependencies
    }
}

struct ResolvedAction {
    factory: Arc<dyn ActionFactory>,
    contract: ActionContractSnapshot,
}

pub(crate) struct CredentialContractSnapshot {
    projected_key: CredentialKey,
    metadata: CredentialMetadata,
    capabilities: Capabilities,
    type_id: TypeId,
}

impl CredentialContractSnapshot {
    pub(crate) const fn projected_key(&self) -> &CredentialKey {
        &self.projected_key
    }

    #[expect(dead_code, reason = "reserved for the approved compiler phase")]
    pub(crate) const fn metadata(&self) -> &CredentialMetadata {
        &self.metadata
    }

    #[expect(dead_code, reason = "reserved for the approved compiler phase")]
    pub(crate) const fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    #[expect(dead_code, reason = "reserved for the approved compiler phase")]
    pub(crate) const fn type_id(&self) -> TypeId {
        self.type_id
    }
}

struct ResolvedCredential {
    credential: Arc<dyn AnyCredential>,
    contract: CredentialContractSnapshot,
}

struct ProjectedCredential {
    credential: Arc<dyn AnyCredential>,
    raw_projected_key: String,
    metadata: CredentialMetadata,
    capabilities: Capabilities,
    erased_type_id: TypeId,
    downcast_type_id: TypeId,
}

pub(crate) struct ResourceContractSnapshot {
    metadata: ResourceMetadata,
    dependencies: Dependencies,
    type_id: TypeId,
}

impl ResourceContractSnapshot {
    pub(crate) const fn metadata(&self) -> &ResourceMetadata {
        &self.metadata
    }

    #[expect(dead_code, reason = "reserved for the approved compiler phase")]
    pub(crate) const fn dependencies(&self) -> &Dependencies {
        &self.dependencies
    }

    #[expect(dead_code, reason = "reserved for the approved compiler phase")]
    pub(crate) const fn type_id(&self) -> TypeId {
        self.type_id
    }
}

struct ResolvedResource {
    factory: Arc<dyn ResourceFactory>,
    contract: ResourceContractSnapshot,
}

/// Per-plugin resolved wrapper with eager O(1) component lookups.
///
/// Constructed via [`ResolvedPlugin::from`], which calls `plugin.actions()`,
/// `credentials()`, and `resources()` exactly once, validates that every
/// component key starts with `{plugin.key()}.`, and catches within-plugin
/// duplicate keys. Once constructed, component lookup is O(1).
pub struct ResolvedPlugin {
    plugin: Arc<dyn Plugin>,
    manifest: PluginManifest,
    actions: HashMap<ActionKey, ResolvedAction>,
    credentials: HashMap<CredentialKey, ResolvedCredential>,
    resources: HashMap<ResourceKey, ResolvedResource>,
}

impl std::fmt::Debug for ResolvedPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedPlugin")
            .field("key", self.manifest.key())
            .field("version", self.manifest.version())
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
    /// Returns:
    ///
    /// - [`PluginError::InvalidComponentKey`] when a credential's captured raw
    ///   key projection is not a valid [`CredentialKey`].
    /// - [`PluginError::ComponentKeyMismatch`] when a credential or resource
    ///   key projection disagrees with its metadata key.
    /// - [`PluginError::ComponentTypeMismatch`] when a credential's inherited
    ///   erased type identity disagrees with its `as_any()` projection.
    /// - [`PluginError::NamespaceMismatch`] when a component key is outside
    ///   `{plugin.key()}.*`.
    /// - [`PluginError::DuplicateComponent`] when two components of the same
    ///   kind share a key.
    pub fn from<P: Plugin + 'static>(plugin: P) -> Result<Self, PluginError> {
        let manifest = plugin.manifest().clone();
        let plugin_key = manifest.key().clone();
        let prefix = format!("{}.", plugin_key.as_str());

        let actions = Self::build_action_index(&plugin_key, &prefix, plugin.actions())?;
        let credentials = Self::build_credential_index(&plugin_key, &prefix, plugin.credentials())?;
        let resources = Self::build_resource_index(&plugin_key, &prefix, plugin.resources())?;

        Ok(Self {
            plugin: Arc::new(plugin) as Arc<dyn Plugin>,
            manifest,
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
        &self.manifest
    }

    /// The plugin key.
    pub fn key(&self) -> &PluginKey {
        self.manifest.key()
    }

    /// The plugin version.
    pub fn version(&self) -> &Version {
        self.manifest.version()
    }

    /// Look up an action by key.
    pub fn action(&self, key: &ActionKey) -> Option<&Arc<dyn ActionFactory>> {
        self.actions.get(key).map(|entry| &entry.factory)
    }

    /// Look up a credential by key.
    pub fn credential(&self, key: &CredentialKey) -> Option<&Arc<dyn AnyCredential>> {
        self.credentials.get(key).map(|entry| &entry.credential)
    }

    /// Look up a resource by key.
    pub fn resource(&self, key: &ResourceKey) -> Option<&Arc<dyn ResourceFactory>> {
        self.resources.get(key).map(|entry| &entry.factory)
    }

    /// Iterate all registered actions.
    pub fn actions(&self) -> impl Iterator<Item = (&ActionKey, &Arc<dyn ActionFactory>)> {
        self.actions
            .iter()
            .map(|(key, entry)| (key, &entry.factory))
    }

    /// Iterate all registered credentials.
    pub fn credentials(&self) -> impl Iterator<Item = (&CredentialKey, &Arc<dyn AnyCredential>)> {
        self.credentials
            .iter()
            .map(|(key, entry)| (key, &entry.credential))
    }

    /// Iterate all registered resources.
    pub fn resources(&self) -> impl Iterator<Item = (&ResourceKey, &Arc<dyn ResourceFactory>)> {
        self.resources
            .iter()
            .map(|(key, entry)| (key, &entry.factory))
    }

    pub(crate) fn action_contracts(&self) -> impl Iterator<Item = &ActionContractSnapshot> {
        self.actions.values().map(|entry| &entry.contract)
    }

    pub(crate) fn credential_contracts(&self) -> impl Iterator<Item = &CredentialContractSnapshot> {
        self.credentials.values().map(|entry| &entry.contract)
    }

    pub(crate) fn resource_contracts(&self) -> impl Iterator<Item = &ResourceContractSnapshot> {
        self.resources.values().map(|entry| &entry.contract)
    }

    fn build_action_index(
        plugin_key: &PluginKey,
        prefix: &str,
        contributions: Vec<Arc<dyn ActionFactory>>,
    ) -> Result<HashMap<ActionKey, ResolvedAction>, PluginError> {
        let mut projected = contributions
            .into_iter()
            .map(|factory| {
                let metadata = factory.metadata().clone();
                let dependencies = factory.dependencies().clone();
                let key = metadata.base.key.clone();
                (
                    key,
                    ResolvedAction {
                        factory,
                        contract: ActionContractSnapshot {
                            metadata,
                            dependencies,
                        },
                    },
                )
            })
            .collect::<Vec<_>>();
        projected.sort_by(|(left, _), (right, _)| left.cmp(right));

        let mut index = HashMap::with_capacity(projected.len());
        for (key, action) in projected {
            if !key.as_str().starts_with(prefix) {
                return Err(PluginError::NamespaceMismatch {
                    plugin: plugin_key.clone(),
                    offending_key: key.as_str().to_owned(),
                    kind: ComponentKind::Action,
                });
            }
            if index.contains_key(&key) {
                return Err(PluginError::DuplicateComponent {
                    plugin: plugin_key.clone(),
                    key: key.as_str().to_owned(),
                    kind: ComponentKind::Action,
                });
            }
            index.insert(key, action);
        }
        Ok(index)
    }

    fn build_credential_index(
        plugin_key: &PluginKey,
        prefix: &str,
        contributions: Vec<Arc<dyn AnyCredential>>,
    ) -> Result<HashMap<CredentialKey, ResolvedCredential>, PluginError> {
        let mut projected = contributions
            .into_iter()
            .map(|credential| {
                let raw_projected_key = credential.credential_key().to_owned();
                let metadata = credential.metadata();
                let capabilities = credential.capabilities();
                let erased_type_id = std::any::Any::type_id(credential.as_ref());
                let downcast_type_id = credential.as_any().type_id();
                ProjectedCredential {
                    credential,
                    raw_projected_key,
                    metadata,
                    capabilities,
                    erased_type_id,
                    downcast_type_id,
                }
            })
            .collect::<Vec<_>>();
        projected.sort_by(|left, right| {
            left.metadata
                .base
                .key
                .cmp(&right.metadata.base.key)
                .then_with(|| left.raw_projected_key.cmp(&right.raw_projected_key))
        });

        let mut index = HashMap::with_capacity(projected.len());
        for projected in projected {
            let typed_projected_key =
                CredentialKey::new(&projected.raw_projected_key).map_err(|_| {
                    PluginError::InvalidComponentKey {
                        plugin: plugin_key.clone(),
                        kind: ComponentKind::Credential,
                        projected_key: projected.raw_projected_key.clone(),
                    }
                })?;
            let metadata_key = &projected.metadata.base.key;
            if typed_projected_key != *metadata_key {
                return Err(PluginError::ComponentKeyMismatch {
                    plugin: plugin_key.clone(),
                    kind: ComponentKind::Credential,
                    projected_key: projected.raw_projected_key,
                    metadata_key: metadata_key.as_str().to_owned(),
                });
            }
            if projected.erased_type_id != projected.downcast_type_id {
                return Err(PluginError::ComponentTypeMismatch {
                    plugin: plugin_key.clone(),
                    kind: ComponentKind::Credential,
                    key: metadata_key.as_str().to_owned(),
                });
            }
            if !metadata_key.as_str().starts_with(prefix) {
                return Err(PluginError::NamespaceMismatch {
                    plugin: plugin_key.clone(),
                    offending_key: metadata_key.as_str().to_owned(),
                    kind: ComponentKind::Credential,
                });
            }
            if index.contains_key(metadata_key) {
                return Err(PluginError::DuplicateComponent {
                    plugin: plugin_key.clone(),
                    key: metadata_key.as_str().to_owned(),
                    kind: ComponentKind::Credential,
                });
            }
            let key = metadata_key.clone();
            index.insert(
                key,
                ResolvedCredential {
                    credential: projected.credential,
                    contract: CredentialContractSnapshot {
                        projected_key: typed_projected_key,
                        metadata: projected.metadata,
                        capabilities: projected.capabilities,
                        type_id: projected.erased_type_id,
                    },
                },
            );
        }
        Ok(index)
    }

    fn build_resource_index(
        plugin_key: &PluginKey,
        prefix: &str,
        contributions: Vec<Arc<dyn ResourceFactory>>,
    ) -> Result<HashMap<ResourceKey, ResolvedResource>, PluginError> {
        let mut projected = contributions
            .into_iter()
            .map(|factory| {
                let key = factory.key();
                let metadata = factory.metadata();
                let dependencies = factory.dependencies().clone();
                let type_id = factory.resource_type_id();
                (
                    key,
                    ResolvedResource {
                        factory,
                        contract: ResourceContractSnapshot {
                            metadata,
                            dependencies,
                            type_id,
                        },
                    },
                )
            })
            .collect::<Vec<_>>();
        projected.sort_by(|(left_key, left), (right_key, right)| {
            left_key.cmp(right_key).then_with(|| {
                left.contract
                    .metadata
                    .base
                    .key
                    .cmp(&right.contract.metadata.base.key)
            })
        });

        let mut index = HashMap::with_capacity(projected.len());
        for (key, resource) in projected {
            let metadata_key = &resource.contract.metadata.base.key;
            if &key != metadata_key {
                return Err(PluginError::ComponentKeyMismatch {
                    plugin: plugin_key.clone(),
                    kind: ComponentKind::Resource,
                    projected_key: key.as_str().to_owned(),
                    metadata_key: metadata_key.as_str().to_owned(),
                });
            }
            if !key.as_str().starts_with(prefix) {
                return Err(PluginError::NamespaceMismatch {
                    plugin: plugin_key.clone(),
                    offending_key: key.as_str().to_owned(),
                    kind: ComponentKind::Resource,
                });
            }
            if index.contains_key(&key) {
                return Err(PluginError::DuplicateComponent {
                    plugin: plugin_key.clone(),
                    key: key.as_str().to_owned(),
                    kind: ComponentKind::Resource,
                });
            }
            index.insert(key, resource);
        }
        Ok(index)
    }
}
