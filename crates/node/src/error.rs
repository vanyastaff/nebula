//! Node error types.

use nebula_core::NodeKey;

/// Errors from node operations.
#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    /// Node not found in the registry.
    #[error("node not found: {0}")]
    NotFound(NodeKey),

    /// A specific version was not found.
    #[error("version {version} not found for node '{key}'")]
    VersionNotFound {
        /// The requested version.
        version: u32,
        /// The node key.
        key: NodeKey,
    },

    /// The node is not versioned (it is a single instance).
    #[error("node '{0}' is not versioned")]
    NotVersioned(NodeKey),

    /// A node with this key already exists in the registry.
    #[error("node '{0}' already exists")]
    AlreadyExists(NodeKey),

    /// No versions are available in a `NodeVersions` container.
    #[error("no versions available for node '{0}'")]
    NoVersionsAvailable(NodeKey),

    /// The key of a node being added doesn't match the container's key.
    #[error("key mismatch: node has key '{node_key}', container has key '{container_key}'")]
    KeyMismatch {
        /// The incoming node's key.
        node_key: NodeKey,
        /// The container's existing key.
        container_key: NodeKey,
    },

    /// A version already exists in the container.
    #[error("version {version} already exists for node '{key}'")]
    VersionAlreadyExists {
        /// The conflicting version.
        version: u32,
        /// The node key.
        key: NodeKey,
    },

    /// A required field was missing during node construction.
    #[error("missing required field '{field}' for node")]
    MissingRequiredField {
        /// The missing field name.
        field: &'static str,
    },

    /// Node key validation failed.
    #[error("invalid node key: {0}")]
    InvalidKey(#[from] nebula_core::NodeKeyError),
}

impl PartialEq for NodeError {
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
                    node_key: n1,
                    container_key: c1,
                },
                Self::KeyMismatch {
                    node_key: n2,
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

impl Eq for NodeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_display() {
        let key: NodeKey = "slack".parse().unwrap();
        let err = NodeError::NotFound(key);
        assert_eq!(err.to_string(), "node not found: slack");
    }

    #[test]
    fn version_not_found_display() {
        let key: NodeKey = "http_request".parse().unwrap();
        let err = NodeError::VersionNotFound { version: 3, key };
        assert_eq!(
            err.to_string(),
            "version 3 not found for node 'http_request'"
        );
    }

    #[test]
    fn already_exists_display() {
        let key: NodeKey = "slack".parse().unwrap();
        let err = NodeError::AlreadyExists(key);
        assert_eq!(err.to_string(), "node 'slack' already exists");
    }

    #[test]
    fn key_mismatch_display() {
        let nk: NodeKey = "foo".parse().unwrap();
        let ck: NodeKey = "bar".parse().unwrap();
        let err = NodeError::KeyMismatch {
            node_key: nk,
            container_key: ck,
        };
        assert!(err.to_string().contains("foo"));
        assert!(err.to_string().contains("bar"));
    }

    #[test]
    fn partial_eq() {
        let a = NodeError::NotFound("slack".parse().unwrap());
        let b = NodeError::NotFound("slack".parse().unwrap());
        assert_eq!(a, b);

        let c = NodeError::NotFound("http".parse().unwrap());
        assert_ne!(a, c);
    }
}
