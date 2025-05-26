use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;

use crate::node::error::NodeError;
use crate::node::node::Node;
use crate::types::Key;

/// A container for managing multiple versions of a node.
/// This structure allows storing and retrieving different versions of the same
/// node identified by their version numbers.
#[derive(Clone)]
pub struct NodeVersions {
    /// Cached key of the node. This is populated when the first node is added
    /// or when `get_node_key()` is called.
    key: Option<Key>,

    /// Map of version numbers to node instances.
    /// The keys are version numbers, and the values are thread-safe references
    /// to nodes.
    versions: HashMap<u32, Arc<dyn Node>>,
}

impl NodeVersions {
    /// Creates a new empty `NodeVersions` container.
    ///
    /// # Returns
    ///
    /// A new empty `NodeVersions` instance.
    pub fn new() -> Self {
        Self {
            key: None,
            versions: HashMap::new(),
        }
    }

    /// Gets the node key, caching it if needed.
    ///
    /// This method ensures that the `key` field is populated with the key from
    /// one of the stored nodes. If the `key` is not yet set and there are nodes
    /// in the container, it will extract and cache the key from the first
    /// available node.
    ///
    /// # Returns
    ///
    /// * `Ok(Key)` - The node key.
    /// * `Err(NodeError::NoVersionsAvailable)` - If no versions are available
    ///   to extract the key from.
    fn get_node_key(&mut self) -> Result<Key, NodeError> {
        if self.key.is_none() {
            if let Some(node) = self.versions.values().next() {
                self.key = Some(node.key().clone());
            } else {
                return Err(NodeError::NoVersionsAvailable(
                    "unknown".try_into().unwrap(),
                ));
            }
        }

        Ok(self.key.clone().unwrap())
    }

    /// Adds a new version of a node to the container.
    ///
    /// This method validates that the node's key matches the container's key
    /// (if already set) and that the version is not yet present in the
    /// container.
    ///
    /// # Arguments
    ///
    /// * `node` - The node to add.
    ///
    /// # Returns
    ///
    /// * `Ok(&mut Self)` - Self reference for method chaining if the node was
    ///   successfully added.
    /// * `Err(NodeError::KeyMismatch)` - If the node's key doesn't match the
    ///   container's key.
    /// * `Err(NodeError::VersionAlreadyExists)` - If a node with the same
    ///   version already exists.
    ///
    /// # Type Parameters
    ///
    /// * `N` - Type that implements the `Node` trait.
    pub fn add<N>(&mut self, node: N) -> Result<&mut Self, NodeError>
    where
        N: Node + 'static,
    {
        // Extract version and key before any operations
        let version = node.version();
        let key = node.key().clone();

        // If this is the first node, set the container's key
        // Otherwise, ensure the node's key matches the container's key
        if self.versions.len() == 0 {
            self.key = Some(key.clone());
        } else if self.key.as_ref() != Some(&key) {
            return Err(NodeError::KeyMismatch(key, self.key.clone().unwrap()));
        }

        // Check if the version already exists
        if self.versions.contains_key(&version) {
            return Err(NodeError::VersionAlreadyExists { version, key });
        }

        // Add the node to the container
        self.versions.insert(version, Arc::new(node));
        Ok(self)
    }

    /// Retrieves a node with the specified version.
    ///
    /// # Arguments
    ///
    /// * `version` - The version number to retrieve.
    ///
    /// # Returns
    ///
    /// * `Ok(Arc<dyn Node>)` - The node with the specified version.
    /// * `Err(NodeError::VersionNotFound)` - If no node with the specified
    ///   version exists.
    /// * `Err(NodeError::NoVersionsAvailable)` - If no versions are available
    ///   to extract the key from.
    pub fn get(&mut self, version: u32) -> Result<Arc<dyn Node>, NodeError> {
        let key = self.get_node_key()?;

        self.versions
            .get(&version)
            .cloned()
            .ok_or_else(|| NodeError::VersionNotFound(version, key))
    }

    /// Retrieves the node with the latest (highest) version number.
    ///
    /// # Returns
    ///
    /// * `Ok(Arc<dyn Node>)` - The node with the highest version number.
    /// * `Err(NodeError::NoVersionsAvailable)` - If no versions are available.
    pub fn get_latest(&mut self) -> Result<Arc<dyn Node>, NodeError> {
        let key = self.get_node_key()?;

        self.versions
            .values()
            .max_by_key(|node| node.version())
            .cloned()
            .ok_or_else(|| NodeError::NoVersionsAvailable(key))
    }

    /// Get the key of the node versions.
    pub fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    /// Get the versions of the node.
    pub fn versions(&self) -> Vec<u32> {
        self.versions.keys().cloned().collect()
    }
}

impl Default for NodeVersions {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for NodeVersions {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeVersions")
            .field("versions", &self.versions.keys().collect::<Vec<_>>())
            .field("key", &self.key)
            .finish()
    }
}
