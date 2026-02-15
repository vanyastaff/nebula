//! PluginType — single plugin or versioned set.

use std::sync::Arc;

use nebula_core::PluginKey;

use crate::PluginError;
use crate::plugin::Plugin;
use crate::versions::PluginVersions;

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
    pub fn versioned<P: Plugin + 'static>(plugin: P) -> Result<Self, PluginError> {
        let mut versions = PluginVersions::new();
        versions.add(plugin)?;
        Ok(Self::Versions(versions))
    }

    /// The key of the contained plugin(s).
    pub fn key(&self) -> &PluginKey {
        match self {
            Self::Single(p) => p.key(),
            Self::Versions(v) => v.key().expect("non-empty PluginVersions always has a key"),
        }
    }

    /// Retrieve a plugin by version, or the only / latest plugin if `version` is `None`.
    pub fn get_plugin(&self, version: Option<u32>) -> Result<Arc<dyn Plugin>, PluginError> {
        match self {
            Self::Single(plugin) => {
                if let Some(v) = version {
                    if plugin.version() == v {
                        Ok(Arc::clone(plugin))
                    } else {
                        Err(PluginError::VersionNotFound {
                            version: v,
                            key: plugin.key().clone(),
                        })
                    }
                } else {
                    Ok(Arc::clone(plugin))
                }
            }
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
                let mut versions = PluginVersions::new();
                let existing_clone = Arc::clone(existing);
                versions.add(ArcPlugin(existing_clone))?;
                versions.add(plugin)?;
                *self = Self::Versions(versions);
                Ok(())
            }
            Self::Versions(versions) => {
                versions.add(plugin)?;
                Ok(())
            }
        }
    }

    /// Whether this contains multiple versions.
    pub fn is_versioned(&self) -> bool {
        matches!(self, Self::Versions(_))
    }

    /// All available version numbers.
    pub fn version_numbers(&self) -> Vec<u32> {
        match self {
            Self::Single(p) => vec![p.version()],
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
    fn metadata(&self) -> &crate::PluginMetadata {
        self.0.metadata()
    }

    fn register(&self, components: &mut crate::PluginComponents) {
        self.0.register(components)
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
    fn single_get_plugin() {
        let pt = PluginType::single(stub("a", 1));
        assert!(!pt.is_versioned());
        assert_eq!(pt.get_plugin(None).unwrap().version(), 1);
        assert_eq!(pt.get_plugin(Some(1)).unwrap().version(), 1);
        assert!(pt.get_plugin(Some(2)).is_err());
    }

    #[test]
    fn versioned_get_plugin() {
        let pt = PluginType::versioned(stub("a", 1)).unwrap();
        assert!(pt.is_versioned());
        assert_eq!(pt.get_plugin(Some(1)).unwrap().version(), 1);
    }

    #[test]
    fn add_version_promotes_single() {
        let mut pt = PluginType::single(stub("a", 1));
        assert!(!pt.is_versioned());

        pt.add_version(stub("a", 2)).unwrap();
        assert!(pt.is_versioned());
        assert_eq!(pt.version_numbers().len(), 2);
        assert_eq!(pt.latest().unwrap().version(), 2);
    }

    #[test]
    fn version_numbers() {
        let mut pt = PluginType::versioned(stub("a", 3)).unwrap();
        pt.add_version(stub("a", 1)).unwrap();
        pt.add_version(stub("a", 5)).unwrap();

        let mut nums = pt.version_numbers();
        nums.sort();
        assert_eq!(nums, vec![1, 3, 5]);
    }

    #[test]
    fn key() {
        let pt = PluginType::single(stub("slack", 1));
        assert_eq!(pt.key().as_str(), "slack");
    }
}
