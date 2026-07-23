//! In-memory plugin registry.

use std::{collections::HashMap, sync::Arc};

use nebula_core::{ArtifactSetDigest, PluginKey};

use crate::{
    PluginContractDescriptor, PluginError, PluginSet, ResolvedPlugin, RuntimeContractVersion,
    WorkerFlavorRevision,
    dependency::{self, PluginDependencyError},
};

/// In-memory registry mapping [`PluginKey`] to [`Arc<ResolvedPlugin>`].
///
/// Thread-safety is the caller's responsibility — wrap in `RwLock` if
/// shared across threads.
///
/// ```
/// use std::sync::Arc;
///
/// use nebula_plugin::{Plugin, PluginManifest, PluginRegistry, ResolvedPlugin};
///
/// #[derive(Debug)]
/// struct EchoPlugin(PluginManifest);
/// impl Plugin for EchoPlugin {
///     fn manifest(&self) -> &PluginManifest {
///         &self.0
///     }
/// }
///
/// let mut registry = PluginRegistry::new();
/// let manifest = PluginManifest::builder("echo", "Echo").build().unwrap();
/// let resolved = Arc::new(ResolvedPlugin::from(EchoPlugin(manifest)).unwrap());
/// registry.register(resolved).unwrap();
///
/// assert!(registry.contains(&"echo".parse().unwrap()));
/// ```
#[derive(Default)]
pub struct PluginRegistry {
    plugins: HashMap<PluginKey, Arc<ResolvedPlugin>>,
}

/// Failure to consume a mutable registry into a validated immutable registry.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RegistryFreezeError {
    /// A worker flavor cannot be identified without at least one plugin.
    #[error("cannot freeze an empty plugin registry")]
    EmptyRegistry,
    /// The registered dependency graph is incomplete, incompatible, or cyclic.
    #[error(transparent)]
    Dependency(#[from] PluginDependencyError),
    /// A dependency uses an operator unknown to this fingerprint protocol.
    #[error("plugin `{plugin}` dependency `{dependency}` uses an unsupported version requirement")]
    UnsupportedVersionRequirement {
        /// Plugin declaring the unsupported requirement.
        plugin: PluginKey,
        /// Dependency whose requirement cannot be canonicalized.
        dependency: PluginKey,
    },
}

/// Validated immutable plugin registry and its reproducible flavor revision.
///
/// This type preserves the mutable registry's catalog lookup and iteration
/// surface, exposes the validated load order, and provides no registration,
/// removal, or clearing operations.
pub struct FrozenPluginRegistry {
    registry: PluginRegistry,
    load_order: Vec<PluginKey>,
    plugin_set: PluginSet,
    revision: WorkerFlavorRevision,
}

impl PluginRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a resolved plugin. Fails if the key already exists.
    pub fn register(&mut self, plugin: Arc<ResolvedPlugin>) -> Result<(), PluginError> {
        let key = plugin.key().clone();
        if self.plugins.contains_key(&key) {
            return Err(PluginError::AlreadyExists(key));
        }
        self.plugins.insert(key, plugin);
        Ok(())
    }

    /// Look up a resolved plugin by key.
    pub fn get(&self, key: &PluginKey) -> Option<Arc<ResolvedPlugin>> {
        self.plugins.get(key).cloned()
    }

    /// Whether a plugin with the given key exists.
    pub fn contains(&self, key: &PluginKey) -> bool {
        self.plugins.contains_key(key)
    }

    /// Remove a plugin by key. Returns the removed plugin, or `None` if not found.
    pub fn remove(&mut self, key: &PluginKey) -> Option<Arc<ResolvedPlugin>> {
        self.plugins.remove(key)
    }

    /// Remove all plugins.
    pub fn clear(&mut self) {
        self.plugins.clear();
    }

    /// Iterate all registered plugins.
    pub fn iter(&self) -> impl Iterator<Item = (&PluginKey, &Arc<ResolvedPlugin>)> {
        self.plugins.iter()
    }

    /// Number of registered plugins.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Flat iterator over every action across every registered plugin.
    ///
    /// Engine uses this at startup to bulk-register handlers into the
    /// runtime's flat `ActionRegistry`. Order follows plugin iteration
    /// order (i.e., `HashMap`-unstable) × intra-plugin cache order.
    pub fn all_actions(
        &self,
    ) -> impl Iterator<Item = (&PluginKey, &Arc<dyn nebula_action::ActionFactory>)> {
        self.plugins
            .iter()
            .flat_map(|(pk, rp)| rp.actions().map(move |(_k, a)| (pk, a)))
    }

    /// Flat iterator over every credential across every registered plugin.
    pub fn all_credentials(
        &self,
    ) -> impl Iterator<Item = (&PluginKey, &Arc<dyn nebula_credential::AnyCredential>)> {
        self.plugins
            .iter()
            .flat_map(|(pk, rp)| rp.credentials().map(move |(_k, c)| (pk, c)))
    }

    /// Flat iterator over every resource across every registered plugin.
    pub fn all_resources(
        &self,
    ) -> impl Iterator<Item = (&PluginKey, &Arc<dyn nebula_resource::ResourceFactory>)> {
        self.plugins
            .iter()
            .flat_map(|(pk, rp)| rp.resources().map(move |(_k, r)| (pk, r)))
    }

    /// Resolve an action by its full key.
    ///
    /// Walks registered plugins, probes each [`ResolvedPlugin`]'s cache by
    /// the full key; returns the first match. O(plugins) + O(1) inner.
    /// Not on engine dispatch hot path; introspection / catalog UI.
    pub fn resolve_action(
        &self,
        full: &nebula_core::ActionKey,
    ) -> Option<Arc<dyn nebula_action::ActionFactory>> {
        self.plugins
            .values()
            .find_map(|rp| rp.action(full).cloned())
    }

    /// Resolve a credential by its full key.
    pub fn resolve_credential(
        &self,
        full: &nebula_core::CredentialKey,
    ) -> Option<Arc<dyn nebula_credential::AnyCredential>> {
        self.plugins
            .values()
            .find_map(|rp| rp.credential(full).cloned())
    }

    /// Resolve a resource by its full key.
    pub fn resolve_resource(
        &self,
        full: &nebula_core::ResourceKey,
    ) -> Option<Arc<dyn nebula_resource::ResourceFactory>> {
        self.plugins
            .values()
            .find_map(|rp| rp.resource(full).cloned())
    }

    /// Compute a topological load order for all registered plugins.
    ///
    /// Returns a `Vec<PluginKey>` in which every dependency appears before
    /// its dependent. Plugins with no dependencies are included in ascending
    /// key order.
    ///
    /// # Errors
    ///
    /// - [`PluginDependencyError::MissingDependency`] — a declared dependency
    ///   key is absent from the registry.
    /// - [`PluginDependencyError::VersionMismatch`] — the registered version
    ///   does not satisfy the declared requirement.
    /// - [`PluginDependencyError::Cycle`] — the dependency graph is cyclic.
    #[tracing::instrument(skip(self), fields(plugin_count = self.plugins.len()))]
    pub fn resolve_load_order(&self) -> Result<Vec<PluginKey>, PluginDependencyError> {
        dependency::resolve(self)
    }

    /// Consumes this mutable registry after dependency validation and derives
    /// its immutable worker-flavor identity.
    ///
    /// `artifact_set_digest` and `runtime_contract_version` are trusted
    /// activation inputs: the composition root must derive them from the
    /// activated artifacts and running runtime contract, never from plugin
    /// metadata, a request payload, or another untrusted source. This method
    /// derives reproducible identifiers; it does not authenticate those inputs
    /// and [`PluginSet::id`] is not a capability proof.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryFreezeError::EmptyRegistry`] for an empty registry, or
    /// [`RegistryFreezeError::Dependency`] when dependency validation fails,
    /// or [`RegistryFreezeError::UnsupportedVersionRequirement`] when a
    /// requirement contains a semver operator this fingerprint version does
    /// not define.
    #[tracing::instrument(
        skip(self),
        fields(
            plugin_count = self.plugins.len(),
            artifact_set_digest = %artifact_set_digest,
            runtime_contract_version = %runtime_contract_version,
            outcome = tracing::field::Empty,
            error_code = tracing::field::Empty,
            plugin_set_id = tracing::field::Empty,
            worker_flavor_revision_id = tracing::field::Empty,
        )
    )]
    pub fn freeze(
        self,
        artifact_set_digest: ArtifactSetDigest,
        runtime_contract_version: RuntimeContractVersion,
    ) -> Result<FrozenPluginRegistry, RegistryFreezeError> {
        if self.plugins.is_empty() {
            let span = tracing::Span::current();
            span.record("outcome", "error");
            span.record("error_code", "PLUGIN_FREEZE:EMPTY_REGISTRY");
            return Err(RegistryFreezeError::EmptyRegistry);
        }

        let load_order = match self.resolve_load_order() {
            Ok(load_order) => load_order,
            Err(source) => {
                let span = tracing::Span::current();
                span.record("outcome", "error");
                span.record("error_code", nebula_error::Classify::code(&source).as_str());
                return Err(RegistryFreezeError::Dependency(source));
            },
        };
        let descriptors = match self
            .plugins
            .values()
            .map(|plugin| {
                PluginContractDescriptor::from_resolved(plugin).map_err(|dependency| {
                    RegistryFreezeError::UnsupportedVersionRequirement {
                        plugin: plugin.key().clone(),
                        dependency,
                    }
                })
            })
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(descriptors) => descriptors,
            Err(error) => {
                let span = tracing::Span::current();
                span.record("outcome", "error");
                span.record("error_code", nebula_error::Classify::code(&error).as_str());
                return Err(error);
            },
        };
        let plugin_set = match PluginSet::derive(descriptors) {
            Ok(plugin_set) => plugin_set,
            Err((plugin, dependency)) => {
                let error =
                    RegistryFreezeError::UnsupportedVersionRequirement { plugin, dependency };
                let span = tracing::Span::current();
                span.record("outcome", "error");
                span.record("error_code", nebula_error::Classify::code(&error).as_str());
                return Err(error);
            },
        };
        let revision = WorkerFlavorRevision::derive(
            plugin_set.id(),
            runtime_contract_version,
            artifact_set_digest,
        );

        let span = tracing::Span::current();
        span.record("outcome", "success");
        span.record("plugin_set_id", tracing::field::display(plugin_set.id()));
        span.record(
            "worker_flavor_revision_id",
            tracing::field::display(revision.id()),
        );

        Ok(FrozenPluginRegistry {
            registry: self,
            load_order,
            plugin_set,
            revision,
        })
    }
}

impl FrozenPluginRegistry {
    /// Looks up a resolved plugin by key.
    pub fn get(&self, key: &PluginKey) -> Option<Arc<ResolvedPlugin>> {
        self.registry.get(key)
    }

    /// Whether a plugin with the given key exists.
    pub fn contains(&self, key: &PluginKey) -> bool {
        self.registry.contains(key)
    }

    /// Iterates registered plugins. Iteration order is unspecified.
    pub fn iter(&self) -> impl Iterator<Item = (&PluginKey, &Arc<ResolvedPlugin>)> {
        self.registry.iter()
    }

    /// Number of registered plugins.
    pub fn len(&self) -> usize {
        self.registry.len()
    }

    /// Whether the frozen registry is empty.
    ///
    /// A successfully frozen registry is never empty.
    pub fn is_empty(&self) -> bool {
        self.registry.is_empty()
    }

    /// Flat iterator over every action across every registered plugin.
    pub fn all_actions(
        &self,
    ) -> impl Iterator<Item = (&PluginKey, &Arc<dyn nebula_action::ActionFactory>)> {
        self.registry.all_actions()
    }

    /// Flat iterator over every credential across every registered plugin.
    pub fn all_credentials(
        &self,
    ) -> impl Iterator<Item = (&PluginKey, &Arc<dyn nebula_credential::AnyCredential>)> {
        self.registry.all_credentials()
    }

    /// Flat iterator over every resource across every registered plugin.
    pub fn all_resources(
        &self,
    ) -> impl Iterator<Item = (&PluginKey, &Arc<dyn nebula_resource::ResourceFactory>)> {
        self.registry.all_resources()
    }

    /// Resolves an action by its full key.
    pub fn resolve_action(
        &self,
        full: &nebula_core::ActionKey,
    ) -> Option<Arc<dyn nebula_action::ActionFactory>> {
        self.registry.resolve_action(full)
    }

    /// Resolves a credential by its full key.
    pub fn resolve_credential(
        &self,
        full: &nebula_core::CredentialKey,
    ) -> Option<Arc<dyn nebula_credential::AnyCredential>> {
        self.registry.resolve_credential(full)
    }

    /// Resolves a resource by its full key.
    pub fn resolve_resource(
        &self,
        full: &nebula_core::ResourceKey,
    ) -> Option<Arc<dyn nebula_resource::ResourceFactory>> {
        self.registry.resolve_resource(full)
    }

    /// Deterministic dependency-first plugin load order.
    pub fn load_order(&self) -> &[PluginKey] {
        &self.load_order
    }

    /// Canonical registered plugin-set descriptor.
    pub const fn plugin_set(&self) -> &PluginSet {
        &self.plugin_set
    }

    /// Immutable worker-flavor revision descriptor.
    pub const fn revision(&self) -> &WorkerFlavorRevision {
        &self.revision
    }
}

impl nebula_error::Classify for RegistryFreezeError {
    fn category(&self) -> nebula_error::ErrorCategory {
        match self {
            Self::EmptyRegistry | Self::UnsupportedVersionRequirement { .. } => {
                nebula_error::ErrorCategory::Validation
            },
            Self::Dependency(source) => nebula_error::Classify::category(source),
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        match self {
            Self::EmptyRegistry => nebula_error::ErrorCode::new("PLUGIN_FREEZE:EMPTY_REGISTRY"),
            Self::Dependency(source) => nebula_error::Classify::code(source),
            Self::UnsupportedVersionRequirement { .. } => {
                nebula_error::ErrorCode::new("PLUGIN_FREEZE:UNSUPPORTED_VERSION_REQUIREMENT")
            },
        }
    }
}

impl std::fmt::Debug for FrozenPluginRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FrozenPluginRegistry")
            .field("count", &self.registry.len())
            .field("plugin_set_id", &self.plugin_set.id())
            .field("revision_id", &self.revision.id())
            .finish()
    }
}

impl std::fmt::Debug for PluginRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginRegistry")
            .field("count", &self.plugins.len())
            .field("keys", &self.plugins.keys().cloned().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nebula_metadata::PluginManifest;

    use super::*;
    use crate::{ResolvedPlugin, plugin::Plugin};

    #[derive(Debug)]
    struct StubPlugin(PluginManifest);
    impl Plugin for StubPlugin {
        fn manifest(&self) -> &PluginManifest {
            &self.0
        }
    }

    fn make(key: &str) -> Arc<ResolvedPlugin> {
        let manifest = PluginManifest::builder(key, key).build().unwrap();
        Arc::new(ResolvedPlugin::from(StubPlugin(manifest)).unwrap())
    }

    #[test]
    fn register_and_get() {
        let mut reg = PluginRegistry::new();
        reg.register(make("slack")).unwrap();
        let key: PluginKey = "slack".parse().unwrap();
        assert_eq!(reg.get(&key).unwrap().key().as_str(), "slack");
    }

    #[test]
    fn duplicate_register_fails() {
        let mut reg = PluginRegistry::new();
        reg.register(make("a")).unwrap();
        let err = reg.register(make("a")).unwrap_err();
        assert_eq!(err, PluginError::AlreadyExists("a".parse().unwrap()));
    }

    #[test]
    fn remove_and_contains() {
        let mut reg = PluginRegistry::new();
        reg.register(make("x")).unwrap();
        let key: PluginKey = "x".parse().unwrap();
        assert!(reg.contains(&key));
        let removed = reg.remove(&key).unwrap();
        assert_eq!(removed.key().as_str(), "x");
        assert!(!reg.contains(&key));
    }

    #[test]
    fn clear_empties() {
        let mut reg = PluginRegistry::new();
        reg.register(make("a")).unwrap();
        reg.register(make("b")).unwrap();
        reg.clear();
        assert!(reg.is_empty());
    }

    #[test]
    fn iter_visits_all() {
        let mut reg = PluginRegistry::new();
        reg.register(make("a")).unwrap();
        reg.register(make("b")).unwrap();
        assert_eq!(reg.iter().count(), 2);
    }
}
