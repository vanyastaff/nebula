//! Multi-version plugin container.

use std::{collections::HashMap, fmt, sync::Arc};

use nebula_core::PluginKey;
use semver::Version;

use crate::{PluginError, plugin::Plugin};

/// Container that stores multiple versions of the same plugin, keyed by [`semver::Version`].
///
/// Always contains at least one version — the plugin passed to `PluginVersions::new`
/// fixes the container's key, and subsequent `add` calls must match that key.
///
/// ```
/// use nebula_plugin::{Plugin, PluginManifest, PluginVersions};
/// use semver::Version;
///
/// #[derive(Debug)]
/// struct MyPlugin(PluginManifest);
/// impl Plugin for MyPlugin {
///     fn manifest(&self) -> &PluginManifest {
///         &self.0
///     }
/// }
///
/// let m1 = PluginManifest::builder("slack", "Slack")
///     .version(Version::new(1, 0, 0))
///     .build()
///     .unwrap();
/// let m2 = PluginManifest::builder("slack", "Slack")
///     .version(Version::new(2, 0, 0))
///     .build()
///     .unwrap();
///
/// let mut versions = PluginVersions::new(MyPlugin(m1));
/// versions.add(MyPlugin(m2)).unwrap();
///
/// assert_eq!(versions.len(), 2);
/// assert_eq!(versions.latest().unwrap().version(), &Version::new(2, 0, 0));
/// ```
#[derive(Clone)]
pub struct PluginVersions {
    key: PluginKey,
    versions: HashMap<Version, Arc<dyn Plugin>>,
}

impl PluginVersions {
    /// Create a container seeded with `first`. Its key becomes the container's key.
    pub fn new<P: Plugin + 'static>(first: P) -> Self {
        let key = first.key().clone();
        let version = first.version().clone();
        let mut versions = HashMap::new();
        versions.insert(version, Arc::new(first) as Arc<dyn Plugin>);
        Self { key, versions }
    }

    /// Add a plugin version. Returns `&mut Self` for chaining.
    ///
    /// # Errors
    ///
    /// - [`PluginError::KeyMismatch`] if the plugin's key differs from the container's.
    /// - [`PluginError::VersionAlreadyExists`] if the version is already present.
    pub fn add<P: Plugin + 'static>(&mut self, plugin: P) -> Result<&mut Self, PluginError> {
        let version = plugin.version().clone();
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
    pub fn get(&self, version: &Version) -> Result<Arc<dyn Plugin>, PluginError> {
        self.versions
            .get(version)
            .cloned()
            .ok_or_else(|| PluginError::VersionNotFound {
                version: version.clone(),
                key: self.key.clone(),
            })
    }

    /// Get the latest (highest version) plugin.
    pub fn latest(&self) -> Result<Arc<dyn Plugin>, PluginError> {
        self.versions
            .values()
            .max_by(|a, b| a.version().cmp(b.version()))
            .cloned()
            .ok_or_else(|| PluginError::NoVersionsAvailable(self.key.clone()))
    }

    /// The container's key (fixed at construction).
    pub fn key(&self) -> &PluginKey {
        &self.key
    }

    /// All versions present, sorted ascending.
    pub fn version_numbers(&self) -> Vec<Version> {
        let mut v: Vec<Version> = self.versions.keys().cloned().collect();
        v.sort();
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
    use crate::PluginManifest;

    #[derive(Debug)]
    struct StubPlugin(PluginManifest);
    impl Plugin for StubPlugin {
        fn manifest(&self) -> &PluginManifest {
            &self.0
        }
    }

    fn stub(key: &str, version: Version) -> StubPlugin {
        StubPlugin(
            PluginManifest::builder(key, key)
                .version(version)
                .build()
                .unwrap(),
        )
    }

    fn v(major: u64, minor: u64, patch: u64) -> Version {
        Version::new(major, minor, patch)
    }

    #[test]
    fn new_seeds_with_first_plugin() {
        let pv = PluginVersions::new(stub("slack", v(1, 0, 0)));
        assert_eq!(pv.len(), 1);
        assert_eq!(pv.key().as_str(), "slack");
        assert_eq!(pv.latest().unwrap().version(), &v(1, 0, 0));
    }

    #[test]
    fn add_and_get() {
        let mut pv = PluginVersions::new(stub("slack", v(1, 0, 0)));
        pv.add(stub("slack", v(2, 0, 0))).unwrap();

        assert_eq!(pv.len(), 2);
        assert_eq!(pv.get(&v(1, 0, 0)).unwrap().version(), &v(1, 0, 0));
        assert_eq!(pv.get(&v(2, 0, 0)).unwrap().version(), &v(2, 0, 0));
    }

    #[test]
    fn latest_returns_highest() {
        let mut pv = PluginVersions::new(stub("a", v(3, 0, 0)));
        pv.add(stub("a", v(1, 0, 0))).unwrap();
        pv.add(stub("a", v(5, 0, 0))).unwrap();

        assert_eq!(pv.latest().unwrap().version(), &v(5, 0, 0));
    }

    #[test]
    fn rejects_duplicate_version() {
        let mut pv = PluginVersions::new(stub("a", v(1, 0, 0)));
        let err = pv.add(stub("a", v(1, 0, 0))).unwrap_err();
        assert_eq!(
            err,
            PluginError::VersionAlreadyExists {
                version: v(1, 0, 0),
                key: "a".parse().unwrap(),
            }
        );
    }

    #[test]
    fn rejects_key_mismatch() {
        let mut pv = PluginVersions::new(stub("a", v(1, 0, 0)));
        let err = pv.add(stub("b", v(2, 0, 0))).unwrap_err();
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
        let mut pv = PluginVersions::new(stub("x", v(3, 0, 0)));
        pv.add(stub("x", v(1, 2, 0))).unwrap();
        pv.add(stub("x", v(2, 0, 1))).unwrap();
        assert_eq!(
            pv.version_numbers(),
            vec![v(1, 2, 0), v(2, 0, 1), v(3, 0, 0)]
        );
    }
}
