//! In-memory plugin registry.

use std::collections::HashMap;
use std::sync::Arc;

use nebula_core::PluginKey;

use crate::PluginError;
use crate::plugin_type::PluginType;

/// In-memory registry mapping [`PluginKey`] to [`PluginType`].
///
/// Thread-safety is the caller's responsibility — wrap in `RwLock` if
/// shared across threads.
///
/// ```
/// use nebula_plugin::{PluginRegistry, PluginType, PluginMetadata, Plugin, PluginComponents};
///
/// #[derive(Debug)]
/// struct EchoPlugin(PluginMetadata);
/// impl Plugin for EchoPlugin {
///     fn metadata(&self) -> &PluginMetadata { &self.0 }
///     fn register(&self, _components: &mut PluginComponents) {}
/// }
///
/// let mut registry = PluginRegistry::new();
/// let meta = PluginMetadata::builder("echo", "Echo").build().unwrap();
/// let plugin_type = PluginType::single(EchoPlugin(meta));
/// registry.register(plugin_type).unwrap();
///
/// assert!(registry.contains(&"echo".parse().unwrap()));
/// ```
pub struct PluginRegistry {
    plugins: HashMap<PluginKey, Arc<PluginType>>,
}

impl PluginRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    /// Register a plugin type. Fails if the key already exists.
    pub fn register(&mut self, plugin_type: PluginType) -> Result<(), PluginError> {
        let key = plugin_type.key().clone();
        if self.plugins.contains_key(&key) {
            return Err(PluginError::AlreadyExists(key));
        }
        self.plugins.insert(key, Arc::new(plugin_type));
        Ok(())
    }

    /// Register or replace a plugin type under the given key.
    pub fn register_or_replace(&mut self, plugin_type: PluginType) {
        let key = plugin_type.key().clone();
        self.plugins.insert(key, Arc::new(plugin_type));
    }

    /// Look up a plugin type by key.
    pub fn get(&self, key: &PluginKey) -> Result<Arc<PluginType>, PluginError> {
        self.plugins
            .get(key)
            .cloned()
            .ok_or_else(|| PluginError::NotFound(key.clone()))
    }

    /// Look up a plugin type by raw string (normalizes the key).
    pub fn get_by_name(&self, name: &str) -> Result<Arc<PluginType>, PluginError> {
        let key: PluginKey = name.parse().map_err(PluginError::InvalidKey)?;
        self.get(&key)
    }

    /// Whether a plugin with the given key exists.
    pub fn contains(&self, key: &PluginKey) -> bool {
        self.plugins.contains_key(key)
    }

    /// Remove a plugin by key.
    pub fn remove(&mut self, key: &PluginKey) -> Result<Arc<PluginType>, PluginError> {
        self.plugins
            .remove(key)
            .ok_or_else(|| PluginError::NotFound(key.clone()))
    }

    /// Remove all plugins.
    pub fn clear(&mut self) {
        self.plugins.clear();
    }

    /// All registered keys.
    pub fn keys(&self) -> Vec<PluginKey> {
        self.plugins.keys().cloned().collect()
    }

    /// All registered plugin types.
    pub fn values(&self) -> Vec<Arc<PluginType>> {
        self.plugins.values().cloned().collect()
    }

    /// Number of registered plugins.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for PluginRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginRegistry")
            .field("count", &self.plugins.len())
            .field("keys", &self.keys())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PluginMetadata;
    use crate::plugin::Plugin;

    #[derive(Debug)]
    struct StubPlugin(PluginMetadata);
    impl Plugin for StubPlugin {
        fn metadata(&self) -> &PluginMetadata {
            &self.0
        }

        fn register(&self, _components: &mut crate::PluginComponents) {}
    }

    fn make_type(key: &str) -> PluginType {
        let meta = PluginMetadata::builder(key, key).build().unwrap();
        PluginType::single(StubPlugin(meta))
    }

    #[test]
    fn register_and_get() {
        let mut reg = PluginRegistry::new();
        reg.register(make_type("slack")).unwrap();

        let key: PluginKey = "slack".parse().unwrap();
        let pt = reg.get(&key).unwrap();
        assert_eq!(pt.key().as_str(), "slack");
    }

    #[test]
    fn get_by_name() {
        let mut reg = PluginRegistry::new();
        reg.register(make_type("http_request")).unwrap();

        let pt = reg.get_by_name("HTTP Request").unwrap();
        assert_eq!(pt.key().as_str(), "http_request");
    }

    #[test]
    fn duplicate_register_fails() {
        let mut reg = PluginRegistry::new();
        reg.register(make_type("a")).unwrap();
        let err = reg.register(make_type("a")).unwrap_err();
        assert_eq!(err, PluginError::AlreadyExists("a".parse().unwrap()));
    }

    #[test]
    fn register_or_replace() {
        let mut reg = PluginRegistry::new();
        reg.register(make_type("a")).unwrap();
        reg.register_or_replace(make_type("a")); // no error
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn remove() {
        let mut reg = PluginRegistry::new();
        reg.register(make_type("x")).unwrap();

        let key: PluginKey = "x".parse().unwrap();
        let removed = reg.remove(&key).unwrap();
        assert_eq!(removed.key().as_str(), "x");
        assert!(reg.is_empty());
    }

    #[test]
    fn remove_not_found() {
        let mut reg = PluginRegistry::new();
        let key: PluginKey = "nope".parse().unwrap();
        assert!(reg.remove(&key).is_err());
    }

    #[test]
    fn clear() {
        let mut reg = PluginRegistry::new();
        reg.register(make_type("a")).unwrap();
        reg.register(make_type("b")).unwrap();
        assert_eq!(reg.len(), 2);

        reg.clear();
        assert!(reg.is_empty());
    }

    #[test]
    fn contains() {
        let mut reg = PluginRegistry::new();
        let key: PluginKey = "foo".parse().unwrap();
        assert!(!reg.contains(&key));

        reg.register(make_type("foo")).unwrap();
        assert!(reg.contains(&key));
    }
}
