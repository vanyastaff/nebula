//! Multi-version plugin container.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use nebula_core::PluginKey;

use crate::PluginError;
use crate::plugin::Plugin;

/// Container that stores multiple versions of the same plugin, keyed by `u32`.
///
/// The first plugin added sets the container's key; subsequent additions must
/// have a matching key.
///
/// ```
/// use nebula_plugin::{PluginVersions, PluginMetadata, Plugin, PluginComponents};
///
/// #[derive(Debug)]
/// struct MyPlugin(PluginMetadata);
/// impl Plugin for MyPlugin {
///     fn metadata(&self) -> &PluginMetadata { &self.0 }
///     fn register(&self, _components: &mut PluginComponents) {}
/// }
///
/// let mut versions = PluginVersions::new();
/// let m1 = PluginMetadata::builder("slack", "Slack").version(1).build().unwrap();
/// let m2 = PluginMetadata::builder("slack", "Slack").version(2).build().unwrap();
///
/// versions.add(MyPlugin(m1)).unwrap();
/// versions.add(MyPlugin(m2)).unwrap();
///
/// assert_eq!(versions.len(), 2);
/// assert_eq!(versions.latest().unwrap().version(), 2);
/// ```
#[derive(Clone)]
pub struct PluginVersions {
    key: Option<PluginKey>,
    versions: HashMap<u32, Arc<dyn Plugin>>,
}

impl PluginVersions {
    /// Create an empty container.
    pub fn new() -> Self {
        Self {
            key: None,
            versions: HashMap::new(),
        }
    }

    /// Add a plugin version. Returns `&mut Self` for chaining.
    ///
    /// # Errors
    ///
    /// - [`PluginError::KeyMismatch`] if the plugin's key differs from the container's.
    /// - [`PluginError::VersionAlreadyExists`] if the version number is already present.
    pub fn add<P: Plugin + 'static>(&mut self, plugin: P) -> Result<&mut Self, PluginError> {
        let version = plugin.version();
        let key = plugin.key().clone();

        if self.versions.is_empty() {
            self.key = Some(key.clone());
        } else if self.key.as_ref() != Some(&key) {
            return Err(PluginError::KeyMismatch {
                plugin_key: key,
                container_key: self.key.clone().unwrap(),
            });
        }

        if self.versions.contains_key(&version) {
            return Err(PluginError::VersionAlreadyExists { version, key });
        }

        self.versions.insert(version, Arc::new(plugin));
        Ok(self)
    }

    /// Get a specific version.
    pub fn get(&self, version: u32) -> Result<Arc<dyn Plugin>, PluginError> {
        let key = self.require_key()?;
        self.versions
            .get(&version)
            .cloned()
            .ok_or_else(|| PluginError::VersionNotFound {
                version,
                key: key.clone(),
            })
    }

    /// Get the latest (highest version number) plugin.
    pub fn latest(&self) -> Result<Arc<dyn Plugin>, PluginError> {
        let key = self.require_key()?;
        self.versions
            .values()
            .max_by_key(|p| p.version())
            .cloned()
            .ok_or_else(|| PluginError::NoVersionsAvailable(key.clone()))
    }

    /// The container's key (set by the first added plugin).
    pub fn key(&self) -> Option<&PluginKey> {
        self.key.as_ref()
    }

    /// All version numbers present.
    pub fn version_numbers(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.versions.keys().copied().collect();
        v.sort_unstable();
        v
    }

    /// Number of versions stored.
    pub fn len(&self) -> usize {
        self.versions.len()
    }

    /// Whether the container is empty.
    pub fn is_empty(&self) -> bool {
        self.versions.is_empty()
    }

    /// Helper: return the key or an error if the container is empty.
    fn require_key(&self) -> Result<&PluginKey, PluginError> {
        self.key
            .as_ref()
            .ok_or_else(|| PluginError::NoVersionsAvailable("unknown".parse().unwrap()))
    }
}

impl Default for PluginVersions {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for PluginVersions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PluginVersions")
            .field("key", &self.key)
            .field("versions", &self.version_numbers())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PluginMetadata;

    #[derive(Debug)]
    struct StubPlugin(PluginMetadata);
    impl Plugin for StubPlugin {
        fn metadata(&self) -> &PluginMetadata {
            &self.0
        }

        fn register(&self, _components: &mut crate::PluginComponents) {}
    }

    fn stub(key: &str, version: u32) -> StubPlugin {
        StubPlugin(
            PluginMetadata::builder(key, key)
                .version(version)
                .build()
                .unwrap(),
        )
    }

    #[test]
    fn add_and_get() {
        let mut v = PluginVersions::new();
        v.add(stub("slack", 1)).unwrap();
        v.add(stub("slack", 2)).unwrap();

        assert_eq!(v.len(), 2);
        assert_eq!(v.get(1).unwrap().version(), 1);
        assert_eq!(v.get(2).unwrap().version(), 2);
    }

    #[test]
    fn latest_returns_highest() {
        let mut v = PluginVersions::new();
        v.add(stub("a", 3)).unwrap();
        v.add(stub("a", 1)).unwrap();
        v.add(stub("a", 5)).unwrap();

        assert_eq!(v.latest().unwrap().version(), 5);
    }

    #[test]
    fn rejects_duplicate_version() {
        let mut v = PluginVersions::new();
        v.add(stub("a", 1)).unwrap();
        let err = v.add(stub("a", 1)).unwrap_err();
        assert_eq!(
            err,
            PluginError::VersionAlreadyExists {
                version: 1,
                key: "a".parse().unwrap(),
            }
        );
    }

    #[test]
    fn rejects_key_mismatch() {
        let mut v = PluginVersions::new();
        v.add(stub("a", 1)).unwrap();
        let err = v.add(stub("b", 2)).unwrap_err();
        assert_eq!(
            err,
            PluginError::KeyMismatch {
                plugin_key: "b".parse().unwrap(),
                container_key: "a".parse().unwrap(),
            }
        );
    }

    #[test]
    fn empty_latest_errors() {
        let v = PluginVersions::new();
        assert!(v.latest().is_err());
    }

    #[test]
    fn version_numbers_sorted() {
        let mut v = PluginVersions::new();
        v.add(stub("x", 3)).unwrap();
        v.add(stub("x", 1)).unwrap();
        v.add(stub("x", 2)).unwrap();
        assert_eq!(v.version_numbers(), vec![1, 2, 3]);
    }
}
