//! Multi-version plugin container.

use std::{collections::HashMap, fmt, sync::Arc};

use nebula_core::PluginKey;

use crate::{PluginError, plugin::Plugin};

/// Container that stores multiple versions of the same plugin, keyed by `u32`.
///
/// Always contains at least one version — the plugin passed to `PluginVersions::new`
/// fixes the container's key, and subsequent `add` calls must match that key.
///
/// ```
/// use nebula_plugin::{Plugin, PluginMetadata, PluginVersions};
///
/// #[derive(Debug)]
/// struct MyPlugin(PluginMetadata);
/// impl Plugin for MyPlugin {
///     fn metadata(&self) -> &PluginMetadata {
///         &self.0
///     }
/// }
///
/// let m1 = PluginMetadata::builder("slack", "Slack")
///     .version(1)
///     .build()
///     .unwrap();
/// let m2 = PluginMetadata::builder("slack", "Slack")
///     .version(2)
///     .build()
///     .unwrap();
///
/// let mut versions = PluginVersions::new(MyPlugin(m1));
/// versions.add(MyPlugin(m2)).unwrap();
///
/// assert_eq!(versions.len(), 2);
/// assert_eq!(versions.latest().unwrap().version(), 2);
/// ```
#[derive(Clone)]
pub struct PluginVersions {
    key: PluginKey,
    versions: HashMap<u32, Arc<dyn Plugin>>,
}

impl PluginVersions {
    /// Create a container seeded with `first`. Its key becomes the container's key.
    pub fn new<P: Plugin + 'static>(first: P) -> Self {
        let key = first.key().clone();
        let version = first.version();
        let mut versions = HashMap::new();
        versions.insert(version, Arc::new(first) as Arc<dyn Plugin>);
        Self { key, versions }
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

        if self.key != key {
            return Err(PluginError::KeyMismatch {
                plugin_key: key,
                container_key: self.key.clone(),
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
        self.versions
            .get(&version)
            .cloned()
            .ok_or_else(|| PluginError::VersionNotFound {
                version,
                key: self.key.clone(),
            })
    }

    /// Get the latest (highest version number) plugin.
    pub fn latest(&self) -> Result<Arc<dyn Plugin>, PluginError> {
        self.versions
            .values()
            .max_by_key(|p| p.version())
            .cloned()
            .ok_or_else(|| PluginError::NoVersionsAvailable(self.key.clone()))
    }

    /// The container's key (fixed at construction).
    pub fn key(&self) -> &PluginKey {
        &self.key
    }

    /// All version numbers present.
    pub fn version_numbers(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.versions.keys().copied().collect();
        v.sort_unstable();
        v
    }

    /// Number of versions stored (always ≥ 1 by construction).
    #[expect(
        clippy::len_without_is_empty,
        reason = "len ≥ 1 by construction — is_empty() would be meaningless"
    )]
    pub fn len(&self) -> usize {
        self.versions.len()
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
    fn new_seeds_with_first_plugin() {
        let v = PluginVersions::new(stub("slack", 1));
        assert_eq!(v.len(), 1);
        assert_eq!(v.key().as_str(), "slack");
        assert_eq!(v.latest().unwrap().version(), 1);
    }

    #[test]
    fn add_and_get() {
        let mut v = PluginVersions::new(stub("slack", 1));
        v.add(stub("slack", 2)).unwrap();

        assert_eq!(v.len(), 2);
        assert_eq!(v.get(1).unwrap().version(), 1);
        assert_eq!(v.get(2).unwrap().version(), 2);
    }

    #[test]
    fn latest_returns_highest() {
        let mut v = PluginVersions::new(stub("a", 3));
        v.add(stub("a", 1)).unwrap();
        v.add(stub("a", 5)).unwrap();

        assert_eq!(v.latest().unwrap().version(), 5);
    }

    #[test]
    fn rejects_duplicate_version() {
        let mut v = PluginVersions::new(stub("a", 1));
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
        let mut v = PluginVersions::new(stub("a", 1));
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
    fn version_numbers_sorted() {
        let mut v = PluginVersions::new(stub("x", 3));
        v.add(stub("x", 1)).unwrap();
        v.add(stub("x", 2)).unwrap();
        assert_eq!(v.version_numbers(), vec![1, 2, 3]);
    }
}
