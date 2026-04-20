//! In-memory plugin registry.

use std::{collections::HashMap, sync::Arc};

use nebula_core::PluginKey;

use crate::{PluginError, ResolvedPlugin};

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
    ) -> impl Iterator<Item = (&PluginKey, &Arc<dyn nebula_action::Action>)> {
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
    ) -> impl Iterator<Item = (&PluginKey, &Arc<dyn nebula_resource::AnyResource>)> {
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
    ) -> Option<Arc<dyn nebula_action::Action>> {
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
    ) -> Option<Arc<dyn nebula_resource::AnyResource>> {
        self.plugins
            .values()
            .find_map(|rp| rp.resource(full).cloned())
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
        let keys: Vec<_> = reg.iter().map(|(k, _)| k.as_str().to_owned()).collect();
        assert_eq!(keys.len(), 2);
    }
}
