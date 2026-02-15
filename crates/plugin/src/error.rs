//! Plugin error types.

use nebula_core::PluginKey;

/// Errors from plugin operations.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    /// Plugin not found in the registry.
    #[error("plugin not found: {0}")]
    NotFound(PluginKey),

    /// A specific version was not found.
    #[error("version {version} not found for plugin '{key}'")]
    VersionNotFound {
        /// The requested version.
        version: u32,
        /// The plugin key.
        key: PluginKey,
    },

    /// The plugin is not versioned (it is a single instance).
    #[error("plugin '{0}' is not versioned")]
    NotVersioned(PluginKey),

    /// A plugin with this key already exists in the registry.
    #[error("plugin '{0}' already exists")]
    AlreadyExists(PluginKey),

    /// No versions are available in a `PluginVersions` container.
    #[error("no versions available for plugin '{0}'")]
    NoVersionsAvailable(PluginKey),

    /// The key of a plugin being added doesn't match the container's key.
    #[error("key mismatch: plugin has key '{plugin_key}', container has key '{container_key}'")]
    KeyMismatch {
        /// The incoming plugin's key.
        plugin_key: PluginKey,
        /// The container's existing key.
        container_key: PluginKey,
    },

    /// A version already exists in the container.
    #[error("version {version} already exists for plugin '{key}'")]
    VersionAlreadyExists {
        /// The conflicting version.
        version: u32,
        /// The plugin key.
        key: PluginKey,
    },

    /// A required field was missing during plugin construction.
    #[error("missing required field '{field}' for plugin")]
    MissingRequiredField {
        /// The missing field name.
        field: &'static str,
    },

    /// Plugin key validation failed.
    #[error("invalid plugin key: {0}")]
    InvalidKey(#[from] nebula_core::PluginKeyError),
}

impl PartialEq for PluginError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::NotFound(a), Self::NotFound(b)) => a == b,
            (
                Self::VersionNotFound {
                    version: v1,
                    key: k1,
                },
                Self::VersionNotFound {
                    version: v2,
                    key: k2,
                },
            ) => v1 == v2 && k1 == k2,
            (Self::NotVersioned(a), Self::NotVersioned(b)) => a == b,
            (Self::AlreadyExists(a), Self::AlreadyExists(b)) => a == b,
            (Self::NoVersionsAvailable(a), Self::NoVersionsAvailable(b)) => a == b,
            (
                Self::KeyMismatch {
                    plugin_key: n1,
                    container_key: c1,
                },
                Self::KeyMismatch {
                    plugin_key: n2,
                    container_key: c2,
                },
            ) => n1 == n2 && c1 == c2,
            (
                Self::VersionAlreadyExists {
                    version: v1,
                    key: k1,
                },
                Self::VersionAlreadyExists {
                    version: v2,
                    key: k2,
                },
            ) => v1 == v2 && k1 == k2,
            (
                Self::MissingRequiredField { field: f1 },
                Self::MissingRequiredField { field: f2 },
            ) => f1 == f2,
            (Self::InvalidKey(a), Self::InvalidKey(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for PluginError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_display() {
        let key: PluginKey = "slack".parse().unwrap();
        let err = PluginError::NotFound(key);
        assert_eq!(err.to_string(), "plugin not found: slack");
    }

    #[test]
    fn version_not_found_display() {
        let key: PluginKey = "http_request".parse().unwrap();
        let err = PluginError::VersionNotFound { version: 3, key };
        assert_eq!(
            err.to_string(),
            "version 3 not found for plugin 'http_request'"
        );
    }

    #[test]
    fn already_exists_display() {
        let key: PluginKey = "slack".parse().unwrap();
        let err = PluginError::AlreadyExists(key);
        assert_eq!(err.to_string(), "plugin 'slack' already exists");
    }

    #[test]
    fn key_mismatch_display() {
        let pk: PluginKey = "foo".parse().unwrap();
        let ck: PluginKey = "bar".parse().unwrap();
        let err = PluginError::KeyMismatch {
            plugin_key: pk,
            container_key: ck,
        };
        assert!(err.to_string().contains("foo"));
        assert!(err.to_string().contains("bar"));
    }

    #[test]
    fn partial_eq() {
        let a = PluginError::NotFound("slack".parse().unwrap());
        let b = PluginError::NotFound("slack".parse().unwrap());
        assert_eq!(a, b);

        let c = PluginError::NotFound("http".parse().unwrap());
        assert_ne!(a, c);
    }
}
