//! PluginType — single plugin or versioned set.

use std::sync::Arc;

use nebula_core::PluginKey;
use semver::Version;

use crate::{PluginError, plugin::Plugin, versions::PluginVersions};

/// Wraps either a single plugin instance or a multi-version container.
pub enum PluginType {
    /// A single, non-versioned plugin.
    Single(Arc<dyn Plugin>),
    /// Multiple versions of the same plugin.
    Versions(PluginVersions),
}

impl PluginType {
    /// Wrap a single plugin.
    pub fn single<P: Plugin + 'static>(plugin: P) -> Self {
        Self::Single(Arc::new(plugin))
    }

    /// Create a versioned container starting with the given plugin.
    pub fn versioned<P: Plugin + 'static>(plugin: P) -> Self {
        Self::Versions(PluginVersions::new(plugin))
    }

    /// The key of the contained plugin(s).
    pub fn key(&self) -> &PluginKey {
        match self {
            Self::Single(p) => p.key(),
            Self::Versions(v) => v.key(),
        }
    }

    /// Retrieve a plugin by version, or the only / latest plugin if `version` is `None`.
    pub fn get_plugin(&self, version: Option<&Version>) -> Result<Arc<dyn Plugin>, PluginError> {
        match self {
            Self::Single(plugin) => {
                if let Some(v) = version {
                    if plugin.version() == v {
                        Ok(Arc::clone(plugin))
                    } else {
                        Err(PluginError::VersionNotFound {
                            version: v.clone(),
                            key: plugin.key().clone(),
                        })
                    }
                } else {
                    Ok(Arc::clone(plugin))
                }
            },
            Self::Versions(v) => match version {
                Some(ver) => v.get(ver),
                None => v.latest(),
            },
        }
    }

    /// Get the latest version.
    pub fn latest(&self) -> Result<Arc<dyn Plugin>, PluginError> {
        self.get_plugin(None)
    }

    /// Add a new version. If the current variant is `Single`, it is promoted
    /// to `Versions` containing both the existing and the new plugin.
    pub fn add_version<P: Plugin + 'static>(&mut self, plugin: P) -> Result<(), PluginError> {
        match self {
            Self::Single(existing) => {
                let existing_clone = Arc::clone(existing);
                let mut versions = PluginVersions::new(ArcPlugin(existing_clone));
                versions.add(plugin)?;
                *self = Self::Versions(versions);
                Ok(())
            },
            Self::Versions(versions) => {
                versions.add(plugin)?;
                Ok(())
            },
        }
    }

    /// Whether this contains multiple versions.
    pub fn is_versioned(&self) -> bool {
        matches!(self, Self::Versions(_))
    }

    /// All available versions (ascending).
    pub fn version_numbers(&self) -> Vec<Version> {
        match self {
            Self::Single(p) => vec![p.version().clone()],
            Self::Versions(v) => v.version_numbers(),
        }
    }
}

impl std::fmt::Debug for PluginType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Single(p) => f.debug_tuple("Single").field(&p.key()).finish(),
            Self::Versions(v) => f.debug_tuple("Versions").field(v).finish(),
        }
    }
}

/// Wrapper to pass an `Arc<dyn Plugin>` into APIs that accept `impl Plugin`.
#[derive(Debug, Clone)]
struct ArcPlugin(Arc<dyn Plugin>);

impl Plugin for ArcPlugin {
    fn manifest(&self) -> &crate::PluginManifest {
        self.0.manifest()
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

    fn v(major: u64, minor: u64, patch: u64) -> Version {
        Version::new(major, minor, patch)
    }

    fn stub(key: &str, version: Version) -> StubPlugin {
        StubPlugin(
            PluginManifest::builder(key, key)
                .version(version)
                .build()
                .unwrap(),
        )
    }

    #[test]
    fn single_get_plugin() {
        let pt = PluginType::single(stub("a", v(1, 0, 0)));
        assert!(!pt.is_versioned());
        assert_eq!(pt.get_plugin(None).unwrap().version(), &v(1, 0, 0));
        assert_eq!(
            pt.get_plugin(Some(&v(1, 0, 0))).unwrap().version(),
            &v(1, 0, 0)
        );
        assert!(pt.get_plugin(Some(&v(2, 0, 0))).is_err());
    }

    #[test]
    fn versioned_get_plugin() {
        let pt = PluginType::versioned(stub("a", v(1, 0, 0)));
        assert!(pt.is_versioned());
        assert_eq!(
            pt.get_plugin(Some(&v(1, 0, 0))).unwrap().version(),
            &v(1, 0, 0)
        );
    }

    #[test]
    fn add_version_promotes_single() {
        let mut pt = PluginType::single(stub("a", v(1, 0, 0)));
        assert!(!pt.is_versioned());

        pt.add_version(stub("a", v(2, 0, 0))).unwrap();
        assert!(pt.is_versioned());
        assert_eq!(pt.version_numbers().len(), 2);
        assert_eq!(pt.latest().unwrap().version(), &v(2, 0, 0));
    }

    #[test]
    fn version_numbers() {
        let mut pt = PluginType::versioned(stub("a", v(3, 0, 0)));
        pt.add_version(stub("a", v(1, 2, 0))).unwrap();
        pt.add_version(stub("a", v(5, 0, 1))).unwrap();

        let nums = pt.version_numbers();
        assert_eq!(nums, vec![v(1, 2, 0), v(3, 0, 0), v(5, 0, 1)]);
    }

    #[test]
    fn key() {
        let pt = PluginType::single(stub("slack", v(1, 0, 0)));
        assert_eq!(pt.key().as_str(), "slack");
    }
}
