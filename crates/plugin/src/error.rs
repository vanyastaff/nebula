//! Plugin error types.

use nebula_core::PluginKey;

/// Which component kind flagged a plugin construction error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentKind {
    /// An action component.
    Action,
    /// A credential component.
    Credential,
    /// A resource component.
    Resource,
}

impl core::fmt::Display for ComponentKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Action => f.write_str("action"),
            Self::Credential => f.write_str("credential"),
            Self::Resource => f.write_str("resource"),
        }
    }
}

/// Errors from plugin operations.
#[derive(Debug, thiserror::Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum PluginError {
    /// Plugin not found in the registry.
    #[classify(category = "not_found", code = "PLUGIN:NOT_FOUND")]
    #[error("plugin not found: {0}")]
    NotFound(PluginKey),

    /// A plugin with this key already exists in the registry.
    #[classify(category = "conflict", code = "PLUGIN:ALREADY_EXISTS")]
    #[error("plugin '{0}' already exists")]
    AlreadyExists(PluginKey),

    /// Plugin manifest construction failed — wraps `nebula_metadata::ManifestError`.
    #[classify(category = "validation", code = "PLUGIN:INVALID_MANIFEST")]
    #[error("invalid plugin manifest: {0}")]
    InvalidManifest(#[from] nebula_metadata::ManifestError),

    /// Plugin declared an action/credential/resource whose full key does not
    /// start with the plugin's own prefix. Caught at `ResolvedPlugin::from`.
    #[classify(category = "validation", code = "PLUGIN:NAMESPACE_MISMATCH")]
    #[error(
        "plugin '{plugin}' declared {kind} key '{offending_key}' outside its namespace '{plugin}.*'"
    )]
    NamespaceMismatch {
        /// The plugin that declared the out-of-namespace component.
        plugin: PluginKey,
        /// The offending component key.
        offending_key: String,
        /// Which kind of component triggered the violation.
        kind: ComponentKind,
    },

    /// A contribution's erased key projection disagrees with the key carried
    /// by its metadata snapshot.
    #[classify(category = "validation", code = "PLUGIN:COMPONENT_KEY_MISMATCH")]
    #[error(
        "plugin '{plugin}' declared {kind} key '{projected_key}' but its metadata key is '{metadata_key}'"
    )]
    ComponentKeyMismatch {
        /// The plugin that declared the incoherent component.
        plugin: PluginKey,
        /// Which kind of component triggered the violation.
        kind: ComponentKind,
        /// Key returned by the erased credential or resource factory.
        projected_key: String,
        /// Key captured from the component metadata.
        metadata_key: String,
    },

    /// A contribution returned a key projection that is not valid for its
    /// component kind.
    #[classify(category = "validation", code = "PLUGIN:INVALID_COMPONENT_KEY")]
    #[error("plugin '{plugin}' declared invalid {kind} key '{projected_key}'")]
    InvalidComponentKey {
        /// The plugin that declared the invalid component key.
        plugin: PluginKey,
        /// Which kind of component triggered the violation.
        kind: ComponentKind,
        /// Invalid key returned by the erased contribution.
        projected_key: String,
    },

    /// A contribution's erased concrete type differs from the type returned
    /// by its downcast projection.
    #[classify(category = "validation", code = "PLUGIN:COMPONENT_TYPE_MISMATCH")]
    #[error("plugin '{plugin}' declared incoherent type projections for {kind} '{key}'")]
    ComponentTypeMismatch {
        /// The plugin that declared the incoherent component.
        plugin: PluginKey,
        /// Which kind of component triggered the violation.
        kind: ComponentKind,
        /// The component key whose local type projections disagreed.
        key: String,
    },

    /// Plugin declared two components of the same kind with identical full keys.
    #[classify(category = "conflict", code = "PLUGIN:DUPLICATE_COMPONENT")]
    #[error("plugin '{plugin}' declared duplicate {kind} key '{key}'")]
    DuplicateComponent {
        /// The plugin that declared the duplicate component.
        plugin: PluginKey,
        /// The duplicate key.
        key: String,
        /// Which kind of component is duplicated.
        kind: ComponentKind,
    },
}

impl PartialEq for PluginError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::NotFound(a), Self::NotFound(b)) => a == b,
            (Self::AlreadyExists(a), Self::AlreadyExists(b)) => a == b,
            (Self::InvalidManifest(a), Self::InvalidManifest(b)) => a == b,
            (
                Self::NamespaceMismatch {
                    plugin: p1,
                    offending_key: k1,
                    kind: ki1,
                },
                Self::NamespaceMismatch {
                    plugin: p2,
                    offending_key: k2,
                    kind: ki2,
                },
            ) => p1 == p2 && k1 == k2 && ki1 == ki2,
            (
                Self::ComponentKeyMismatch {
                    plugin: p1,
                    kind: ki1,
                    projected_key: pk1,
                    metadata_key: mk1,
                },
                Self::ComponentKeyMismatch {
                    plugin: p2,
                    kind: ki2,
                    projected_key: pk2,
                    metadata_key: mk2,
                },
            ) => p1 == p2 && ki1 == ki2 && pk1 == pk2 && mk1 == mk2,
            (
                Self::InvalidComponentKey {
                    plugin: p1,
                    kind: ki1,
                    projected_key: k1,
                },
                Self::InvalidComponentKey {
                    plugin: p2,
                    kind: ki2,
                    projected_key: k2,
                },
            ) => p1 == p2 && ki1 == ki2 && k1 == k2,
            (
                Self::ComponentTypeMismatch {
                    plugin: p1,
                    kind: ki1,
                    key: k1,
                },
                Self::ComponentTypeMismatch {
                    plugin: p2,
                    kind: ki2,
                    key: k2,
                },
            ) => p1 == p2 && ki1 == ki2 && k1 == k2,
            (
                Self::DuplicateComponent {
                    plugin: p1,
                    key: k1,
                    kind: ki1,
                },
                Self::DuplicateComponent {
                    plugin: p2,
                    key: k2,
                    kind: ki2,
                },
            ) => p1 == p2 && k1 == k2 && ki1 == ki2,
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
    fn already_exists_display() {
        let key: PluginKey = "slack".parse().unwrap();
        let err = PluginError::AlreadyExists(key);
        assert_eq!(err.to_string(), "plugin 'slack' already exists");
    }

    #[test]
    fn partial_eq() {
        let a = PluginError::NotFound("slack".parse().unwrap());
        let b = PluginError::NotFound("slack".parse().unwrap());
        assert_eq!(a, b);

        let c = PluginError::NotFound("http".parse().unwrap());
        assert_ne!(a, c);
    }

    #[test]
    fn namespace_mismatch_display() {
        let err = PluginError::NamespaceMismatch {
            plugin: "slack".parse().unwrap(),
            offending_key: "api.foo".into(),
            kind: ComponentKind::Action,
        };
        let s = err.to_string();
        assert!(s.contains("slack"));
        assert!(s.contains("api.foo"));
        assert!(s.contains("action"));
    }

    #[test]
    fn duplicate_component_display() {
        let err = PluginError::DuplicateComponent {
            plugin: "slack".parse().unwrap(),
            key: "slack.send".into(),
            kind: ComponentKind::Credential,
        };
        let s = err.to_string();
        assert!(s.contains("duplicate"));
        assert!(s.contains("slack.send"));
        assert!(s.contains("credential"));
    }
}
