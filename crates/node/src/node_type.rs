//! NodeType — single node or versioned set.

use std::sync::Arc;

use nebula_core::NodeKey;

use crate::NodeError;
use crate::node::Node;
use crate::versions::NodeVersions;

/// Wraps either a single node instance or a multi-version container.
pub enum NodeType {
    /// A single, non-versioned node.
    Single(Arc<dyn Node>),
    /// Multiple versions of the same node.
    Versions(NodeVersions),
}

impl NodeType {
    /// Wrap a single node.
    pub fn single<N: Node + 'static>(node: N) -> Self {
        Self::Single(Arc::new(node))
    }

    /// Create a versioned container starting with the given node.
    pub fn versioned<N: Node + 'static>(node: N) -> Result<Self, NodeError> {
        let mut versions = NodeVersions::new();
        versions.add(node)?;
        Ok(Self::Versions(versions))
    }

    /// The key of the contained node(s).
    pub fn key(&self) -> &NodeKey {
        match self {
            Self::Single(n) => n.key(),
            Self::Versions(v) => v.key().expect("non-empty NodeVersions always has a key"),
        }
    }

    /// Retrieve a node by version, or the only / latest node if `version` is `None`.
    pub fn get_node(&self, version: Option<u32>) -> Result<Arc<dyn Node>, NodeError> {
        match self {
            Self::Single(node) => {
                if let Some(v) = version {
                    if node.version() == v {
                        Ok(Arc::clone(node))
                    } else {
                        Err(NodeError::VersionNotFound {
                            version: v,
                            key: node.key().clone(),
                        })
                    }
                } else {
                    Ok(Arc::clone(node))
                }
            }
            Self::Versions(v) => match version {
                Some(ver) => v.get(ver),
                None => v.latest(),
            },
        }
    }

    /// Get the latest version.
    pub fn latest(&self) -> Result<Arc<dyn Node>, NodeError> {
        self.get_node(None)
    }

    /// Add a new version. If the current variant is `Single`, it is promoted
    /// to `Versions` containing both the existing and the new node.
    pub fn add_version<N: Node + 'static>(&mut self, node: N) -> Result<(), NodeError> {
        match self {
            Self::Single(existing) => {
                let mut versions = NodeVersions::new();
                // We need to move the existing Arc<dyn Node> into versions.
                // NodeVersions::add takes N: Node, but we have Arc<dyn Node>.
                // We'll re-create as Versions with just the new node, then add
                // the old one by wrapping.
                let existing_clone = Arc::clone(existing);
                versions.add(ArcNode(existing_clone))?;
                versions.add(node)?;
                *self = Self::Versions(versions);
                Ok(())
            }
            Self::Versions(versions) => {
                versions.add(node)?;
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
            Self::Single(n) => vec![n.version()],
            Self::Versions(v) => v.version_numbers(),
        }
    }
}

impl std::fmt::Debug for NodeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Single(n) => f.debug_tuple("Single").field(&n.key()).finish(),
            Self::Versions(v) => f.debug_tuple("Versions").field(v).finish(),
        }
    }
}

/// Wrapper to pass an `Arc<dyn Node>` into APIs that accept `impl Node`.
#[derive(Debug, Clone)]
struct ArcNode(Arc<dyn Node>);

impl Node for ArcNode {
    fn metadata(&self) -> &crate::NodeMetadata {
        self.0.metadata()
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
    fn single_get_node() {
        let nt = NodeType::single(stub("a", 1));
        assert!(!nt.is_versioned());
        assert_eq!(nt.get_node(None).unwrap().version(), 1);
        assert_eq!(nt.get_node(Some(1)).unwrap().version(), 1);
        assert!(nt.get_node(Some(2)).is_err());
    }

    #[test]
    fn versioned_get_node() {
        let nt = NodeType::versioned(stub("a", 1)).unwrap();
        assert!(nt.is_versioned());
        assert_eq!(nt.get_node(Some(1)).unwrap().version(), 1);
    }

    #[test]
    fn add_version_promotes_single() {
        let mut nt = NodeType::single(stub("a", 1));
        assert!(!nt.is_versioned());

        nt.add_version(stub("a", 2)).unwrap();
        assert!(nt.is_versioned());
        assert_eq!(nt.version_numbers().len(), 2);
        assert_eq!(nt.latest().unwrap().version(), 2);
    }

    #[test]
    fn version_numbers() {
        let mut nt = NodeType::versioned(stub("a", 3)).unwrap();
        nt.add_version(stub("a", 1)).unwrap();
        nt.add_version(stub("a", 5)).unwrap();

        let mut nums = nt.version_numbers();
        nums.sort();
        assert_eq!(nums, vec![1, 3, 5]);
    }

    #[test]
    fn key() {
        let nt = NodeType::single(stub("slack", 1));
        assert_eq!(nt.key().as_str(), "slack");
    }
}
