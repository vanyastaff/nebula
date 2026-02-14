//! Multi-version node container.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use nebula_core::NodeKey;

use crate::NodeError;
use crate::node::Node;

/// Container that stores multiple versions of the same node, keyed by `u32`.
///
/// The first node added sets the container's key; subsequent additions must
/// have a matching key.
///
/// ```
/// use nebula_node::{NodeVersions, NodeMetadata, Node, NodeComponents};
///
/// #[derive(Debug)]
/// struct MyNode(NodeMetadata);
/// impl Node for MyNode {
///     fn metadata(&self) -> &NodeMetadata { &self.0 }
///     fn register(&self, _components: &mut NodeComponents) {}
/// }
///
/// let mut versions = NodeVersions::new();
/// let m1 = NodeMetadata::builder("slack", "Slack").version(1).build().unwrap();
/// let m2 = NodeMetadata::builder("slack", "Slack").version(2).build().unwrap();
///
/// versions.add(MyNode(m1)).unwrap();
/// versions.add(MyNode(m2)).unwrap();
///
/// assert_eq!(versions.len(), 2);
/// assert_eq!(versions.latest().unwrap().version(), 2);
/// ```
#[derive(Clone)]
pub struct NodeVersions {
    key: Option<NodeKey>,
    versions: HashMap<u32, Arc<dyn Node>>,
}

impl NodeVersions {
    /// Create an empty container.
    pub fn new() -> Self {
        Self {
            key: None,
            versions: HashMap::new(),
        }
    }

    /// Add a node version. Returns `&mut Self` for chaining.
    ///
    /// # Errors
    ///
    /// - [`NodeError::KeyMismatch`] if the node's key differs from the container's.
    /// - [`NodeError::VersionAlreadyExists`] if the version number is already present.
    pub fn add<N: Node + 'static>(&mut self, node: N) -> Result<&mut Self, NodeError> {
        let version = node.version();
        let key = node.key().clone();

        if self.versions.is_empty() {
            self.key = Some(key.clone());
        } else if self.key.as_ref() != Some(&key) {
            return Err(NodeError::KeyMismatch {
                node_key: key,
                container_key: self.key.clone().unwrap(),
            });
        }

        if self.versions.contains_key(&version) {
            return Err(NodeError::VersionAlreadyExists { version, key });
        }

        self.versions.insert(version, Arc::new(node));
        Ok(self)
    }

    /// Get a specific version.
    pub fn get(&self, version: u32) -> Result<Arc<dyn Node>, NodeError> {
        let key = self.require_key()?;
        self.versions
            .get(&version)
            .cloned()
            .ok_or_else(|| NodeError::VersionNotFound {
                version,
                key: key.clone(),
            })
    }

    /// Get the latest (highest version number) node.
    pub fn latest(&self) -> Result<Arc<dyn Node>, NodeError> {
        let key = self.require_key()?;
        self.versions
            .values()
            .max_by_key(|n| n.version())
            .cloned()
            .ok_or_else(|| NodeError::NoVersionsAvailable(key.clone()))
    }

    /// The container's key (set by the first added node).
    pub fn key(&self) -> Option<&NodeKey> {
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
    fn require_key(&self) -> Result<&NodeKey, NodeError> {
        self.key
            .as_ref()
            .ok_or_else(|| NodeError::NoVersionsAvailable("unknown".parse().unwrap()))
    }
}

impl Default for NodeVersions {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for NodeVersions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeVersions")
            .field("key", &self.key)
            .field("versions", &self.version_numbers())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeMetadata;

    #[derive(Debug)]
    struct StubNode(NodeMetadata);
    impl Node for StubNode {
        fn metadata(&self) -> &NodeMetadata {
            &self.0
        }

        fn register(&self, _components: &mut crate::NodeComponents) {}
    }

    fn stub(key: &str, version: u32) -> StubNode {
        StubNode(
            NodeMetadata::builder(key, key)
                .version(version)
                .build()
                .unwrap(),
        )
    }

    #[test]
    fn add_and_get() {
        let mut v = NodeVersions::new();
        v.add(stub("slack", 1)).unwrap();
        v.add(stub("slack", 2)).unwrap();

        assert_eq!(v.len(), 2);
        assert_eq!(v.get(1).unwrap().version(), 1);
        assert_eq!(v.get(2).unwrap().version(), 2);
    }

    #[test]
    fn latest_returns_highest() {
        let mut v = NodeVersions::new();
        v.add(stub("a", 3)).unwrap();
        v.add(stub("a", 1)).unwrap();
        v.add(stub("a", 5)).unwrap();

        assert_eq!(v.latest().unwrap().version(), 5);
    }

    #[test]
    fn rejects_duplicate_version() {
        let mut v = NodeVersions::new();
        v.add(stub("a", 1)).unwrap();
        let err = v.add(stub("a", 1)).unwrap_err();
        assert_eq!(
            err,
            NodeError::VersionAlreadyExists {
                version: 1,
                key: "a".parse().unwrap(),
            }
        );
    }

    #[test]
    fn rejects_key_mismatch() {
        let mut v = NodeVersions::new();
        v.add(stub("a", 1)).unwrap();
        let err = v.add(stub("b", 2)).unwrap_err();
        assert_eq!(
            err,
            NodeError::KeyMismatch {
                node_key: "b".parse().unwrap(),
                container_key: "a".parse().unwrap(),
            }
        );
    }

    #[test]
    fn empty_latest_errors() {
        let v = NodeVersions::new();
        assert!(v.latest().is_err());
    }

    #[test]
    fn version_numbers_sorted() {
        let mut v = NodeVersions::new();
        v.add(stub("x", 3)).unwrap();
        v.add(stub("x", 1)).unwrap();
        v.add(stub("x", 2)).unwrap();
        assert_eq!(v.version_numbers(), vec![1, 2, 3]);
    }
}
