//! In-memory node registry.

use std::collections::HashMap;
use std::sync::Arc;

use nebula_core::NodeKey;

use crate::NodeError;
use crate::node_type::NodeType;

/// In-memory registry mapping [`NodeKey`] to [`NodeType`].
///
/// Thread-safety is the caller's responsibility — wrap in `RwLock` if
/// shared across threads.
///
/// ```
/// use nebula_node::{NodeRegistry, NodeType, NodeMetadata, Node};
///
/// #[derive(Debug)]
/// struct EchoNode(NodeMetadata);
/// impl Node for EchoNode {
///     fn metadata(&self) -> &NodeMetadata { &self.0 }
/// }
///
/// let mut registry = NodeRegistry::new();
/// let meta = NodeMetadata::builder("echo", "Echo").build().unwrap();
/// let node_type = NodeType::single(EchoNode(meta));
/// registry.register(node_type).unwrap();
///
/// assert!(registry.contains(&"echo".parse().unwrap()));
/// ```
pub struct NodeRegistry {
    nodes: HashMap<NodeKey, Arc<NodeType>>,
}

impl NodeRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
        }
    }

    /// Register a node type. Fails if the key already exists.
    pub fn register(&mut self, node_type: NodeType) -> Result<(), NodeError> {
        let key = node_type.key().clone();
        if self.nodes.contains_key(&key) {
            return Err(NodeError::AlreadyExists(key));
        }
        self.nodes.insert(key, Arc::new(node_type));
        Ok(())
    }

    /// Register or replace a node type under the given key.
    pub fn register_or_replace(&mut self, node_type: NodeType) {
        let key = node_type.key().clone();
        self.nodes.insert(key, Arc::new(node_type));
    }

    /// Look up a node type by key.
    pub fn get(&self, key: &NodeKey) -> Result<Arc<NodeType>, NodeError> {
        self.nodes
            .get(key)
            .cloned()
            .ok_or_else(|| NodeError::NotFound(key.clone()))
    }

    /// Look up a node type by raw string (normalizes the key).
    pub fn get_by_name(&self, name: &str) -> Result<Arc<NodeType>, NodeError> {
        let key: NodeKey = name.parse().map_err(NodeError::InvalidKey)?;
        self.get(&key)
    }

    /// Whether a node with the given key exists.
    pub fn contains(&self, key: &NodeKey) -> bool {
        self.nodes.contains_key(key)
    }

    /// Remove a node by key.
    pub fn remove(&mut self, key: &NodeKey) -> Result<Arc<NodeType>, NodeError> {
        self.nodes
            .remove(key)
            .ok_or_else(|| NodeError::NotFound(key.clone()))
    }

    /// Remove all nodes.
    pub fn clear(&mut self) {
        self.nodes.clear();
    }

    /// All registered keys.
    pub fn keys(&self) -> Vec<NodeKey> {
        self.nodes.keys().cloned().collect()
    }

    /// All registered node types.
    pub fn values(&self) -> Vec<Arc<NodeType>> {
        self.nodes.values().cloned().collect()
    }

    /// Number of registered nodes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

impl Default for NodeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for NodeRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeRegistry")
            .field("count", &self.nodes.len())
            .field("keys", &self.keys())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeMetadata;
    use crate::node::Node;

    #[derive(Debug)]
    struct StubNode(NodeMetadata);
    impl Node for StubNode {
        fn metadata(&self) -> &NodeMetadata {
            &self.0
        }
    }

    fn make_type(key: &str) -> NodeType {
        let meta = NodeMetadata::builder(key, key).build().unwrap();
        NodeType::single(StubNode(meta))
    }

    #[test]
    fn register_and_get() {
        let mut reg = NodeRegistry::new();
        reg.register(make_type("slack")).unwrap();

        let key: NodeKey = "slack".parse().unwrap();
        let nt = reg.get(&key).unwrap();
        assert_eq!(nt.key().as_str(), "slack");
    }

    #[test]
    fn get_by_name() {
        let mut reg = NodeRegistry::new();
        reg.register(make_type("http_request")).unwrap();

        let nt = reg.get_by_name("HTTP Request").unwrap();
        assert_eq!(nt.key().as_str(), "http_request");
    }

    #[test]
    fn duplicate_register_fails() {
        let mut reg = NodeRegistry::new();
        reg.register(make_type("a")).unwrap();
        let err = reg.register(make_type("a")).unwrap_err();
        assert_eq!(err, NodeError::AlreadyExists("a".parse().unwrap()));
    }

    #[test]
    fn register_or_replace() {
        let mut reg = NodeRegistry::new();
        reg.register(make_type("a")).unwrap();
        reg.register_or_replace(make_type("a")); // no error
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn remove() {
        let mut reg = NodeRegistry::new();
        reg.register(make_type("x")).unwrap();

        let key: NodeKey = "x".parse().unwrap();
        let removed = reg.remove(&key).unwrap();
        assert_eq!(removed.key().as_str(), "x");
        assert!(reg.is_empty());
    }

    #[test]
    fn remove_not_found() {
        let mut reg = NodeRegistry::new();
        let key: NodeKey = "nope".parse().unwrap();
        assert!(reg.remove(&key).is_err());
    }

    #[test]
    fn clear() {
        let mut reg = NodeRegistry::new();
        reg.register(make_type("a")).unwrap();
        reg.register(make_type("b")).unwrap();
        assert_eq!(reg.len(), 2);

        reg.clear();
        assert!(reg.is_empty());
    }

    #[test]
    fn contains() {
        let mut reg = NodeRegistry::new();
        let key: NodeKey = "foo".parse().unwrap();
        assert!(!reg.contains(&key));

        reg.register(make_type("foo")).unwrap();
        assert!(reg.contains(&key));
    }
}
