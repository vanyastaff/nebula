use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::node::{NodeError, NodeLoader, NodeType};
use crate::types::Key;

/// A registry for managing and loading nodes.
pub struct NodeRegistry {
    /// A map of node keys to their corresponding `Arc<NodeType>` instances.
    nodes: HashMap<Key, Arc<NodeType>>,

    /// The loader responsible for loading node libraries.
    loader: NodeLoader,
}

impl NodeRegistry {
    /// Creates a new `NodeRegistry` with an optional loader path.
    ///
    /// # Arguments
    ///
    /// * `loader_path` - An optional `PathBuf` specifying the directory
    ///   containing node libraries.
    ///
    /// # Returns
    ///
    /// A new instance of `NodeRegistry`.
    pub fn new(loader_path: Option<PathBuf>) -> Self {
        let loader_path = loader_path.unwrap_or_else(|| {
            NodeLoader::get_default_deps_path().expect("Failed to find default deps path")
        });

        NodeRegistry {
            nodes: HashMap::new(),
            loader: NodeLoader::new(loader_path),
        }
    }

    /// Registers a node directly.
    ///
    /// # Arguments
    ///
    /// * `node` - The `Arc<NodeType>` instance to register.
    ///
    /// # Returns
    ///
    /// `Ok(())` if registration was successful, or `Err(NodeError)` if there
    /// was an error.
    pub fn register(&mut self, node: Arc<NodeType>) -> Result<(), NodeError> {
        let key = node.key().clone();

        if self.nodes.contains_key(&key) {
            return Err(NodeError::AlreadyExists(key));
        }

        self.nodes.insert(key, node);
        Ok(())
    }

    /// Registers a node with a specific key, replacing any existing node with
    /// the same key.
    ///
    /// # Arguments
    ///
    /// * `key` - The key to register the node under.
    /// * `node` - The `Arc<NodeType>` instance to register.
    pub fn register_with_key(&mut self, key: Key, node: Arc<NodeType>) {
        self.nodes.insert(key, node);
    }

    /// Loads a node by its name string.
    ///
    /// # Arguments
    ///
    /// * `name` - A string slice representing the name of the node.
    ///
    /// # Returns
    ///
    /// A `Result` containing the loaded `Arc<NodeType>` or a `NodeError` if an
    /// error occurs.
    pub fn load_by_name(&mut self, name: &str) -> Result<Arc<NodeType>, NodeError> {
        if !self.can_load(name) {
            return Err(NodeError::NotFound(Key::new(name)?));
        }

        let node = self
            .loader
            .load_node(name)
            .map_err(|e| NodeError::LoadError(name.to_string(), e.to_string()))?;

        let key = node.key().clone();
        self.nodes.insert(key.clone(), node.clone());

        Ok(node)
    }

    /// Loads a node by its key.
    ///
    /// # Arguments
    ///
    /// * `key` - The `Key` of the node to load.
    ///
    /// # Returns
    ///
    /// A `Result` containing the loaded `Arc<NodeType>` or a `NodeError` if an
    /// error occurs.
    pub fn load_by_key(&mut self, key: &Key) -> Result<Arc<NodeType>, NodeError> {
        // Get the string representation of the key
        let name = key.as_str();

        self.load_by_name(name)
    }

    /// Loads all nodes in the loader's path.
    ///
    /// # Returns
    ///
    /// A `Result` containing a vector of loaded `Arc<NodeType>` instances or a
    /// `NodeError` if an error occurs.
    pub fn load_all(&mut self) -> Result<Vec<Arc<NodeType>>, NodeError> {
        let nodes = self
            .loader
            .load_all()
            .map_err(|e| NodeError::LoadError("all nodes".to_string(), e.to_string()))?;

        for node in &nodes {
            let key = node.key().clone();
            self.nodes.insert(key, node.clone());
        }

        Ok(nodes)
    }

    /// Checks if a node with the given name exists in the registry.
    ///
    /// # Arguments
    ///
    /// * `name` - A string slice representing the name of the node.
    ///
    /// # Returns
    ///
    /// A boolean indicating whether the node exists.
    pub fn exists_by_name(&self, name: &str) -> Result<bool, NodeError> {
        let key = Key::new(name)?;
        Ok(self.nodes.contains_key(&key))
    }

    /// Checks if a node with the given key exists in the registry.
    ///
    /// # Arguments
    ///
    /// * `key` - The `Key` of the node to check.
    ///
    /// # Returns
    ///
    /// A boolean indicating whether the node exists.
    pub fn exists(&self, key: &Key) -> bool {
        self.nodes.contains_key(key)
    }

    /// Checks if a node with the given name can be loaded.
    ///
    /// # Arguments
    ///
    /// * `name` - A string slice representing the name of the node.
    ///
    /// # Returns
    ///
    /// A boolean indicating whether the node can be loaded.
    pub fn can_load(&self, name: &str) -> bool {
        self.loader.check_node(name)
    }

    /// Retrieves a node by its name, loading it if necessary.
    ///
    /// # Arguments
    ///
    /// * `name` - A string slice representing the name of the node.
    ///
    /// # Returns
    ///
    /// A `Result` containing the `Arc<NodeType>` instance or a `NodeError` if
    /// an error occurs.
    pub fn get_by_name(&mut self, name: &str) -> Result<Arc<NodeType>, NodeError> {
        let key = Key::new(name)?;

        if !self.exists(&key) {
            self.load_by_name(name)?;
        }

        self.nodes
            .get(&key)
            .cloned()
            .ok_or_else(|| NodeError::NotFound(key))
    }

    /// Retrieves a node by its key, loading it if necessary.
    ///
    /// # Arguments
    ///
    /// * `key` - The `Key` of the node to retrieve.
    ///
    /// # Returns
    ///
    /// A `Result` containing the `Arc<NodeType>` instance or a `NodeError` if
    /// an error occurs.
    pub fn get(&mut self, key: &Key) -> Result<Arc<NodeType>, NodeError> {
        if !self.exists(key) {
            self.load_by_key(key)?;
        }

        self.nodes
            .get(key)
            .cloned()
            .ok_or_else(|| NodeError::NotFound(key.clone()))
    }

    /// Retrieves a node by its key without loading it if it doesn't exist.
    ///
    /// # Arguments
    ///
    /// * `key` - The `Key` of the node to retrieve.
    ///
    /// # Returns
    ///
    /// A `Result` containing the `Arc<NodeType>` instance or a `NodeError` if
    /// an error occurs.
    pub fn get_if_exists(&self, key: &Key) -> Result<Arc<NodeType>, NodeError> {
        self.nodes
            .get(key)
            .cloned()
            .ok_or_else(|| NodeError::NotFound(key.clone()))
    }

    /// Retrieves all nodes in the registry.
    ///
    /// # Returns
    ///
    /// A vector of `Arc<NodeType>` instances.
    pub fn get_all(&self) -> Vec<Arc<NodeType>> {
        self.nodes.values().cloned().collect()
    }

    /// Retrieves all node keys in the registry.
    ///
    /// # Returns
    ///
    /// A vector of node keys.
    pub fn get_keys(&self) -> Vec<Key> {
        self.nodes.keys().cloned().collect()
    }

    /// Removes a node by its key.
    ///
    /// # Arguments
    ///
    /// * `key` - The `Key` of the node to remove.
    ///
    /// # Returns
    ///
    /// A `Result` containing the removed `Arc<NodeType>` instance or a
    /// `NodeError` if an error occurs.
    pub fn remove(&mut self, key: &Key) -> Result<Arc<NodeType>, NodeError> {
        self.nodes
            .remove(key)
            .ok_or_else(|| NodeError::NotFound(key.clone()))
    }

    /// Clears all nodes from the registry.
    pub fn clear(&mut self) {
        self.nodes.clear();
    }

    /// Returns the number of nodes in the registry.
    ///
    /// # Returns
    ///
    /// The number of nodes in the registry.
    pub fn count(&self) -> usize {
        self.nodes.len()
    }

    /// Checks if the registry is empty.
    ///
    /// # Returns
    ///
    /// A boolean indicating whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Gets a reference to the underlying NodeLoader.
    ///
    /// # Returns
    ///
    /// A reference to the NodeLoader instance.
    pub fn loader(&self) -> &NodeLoader {
        &self.loader
    }

    /// Gets a mutable reference to the underlying NodeLoader.
    ///
    /// # Returns
    ///
    /// A mutable reference to the NodeLoader instance.
    pub fn loader_mut(&mut self) -> &mut NodeLoader {
        &mut self.loader
    }
}

impl Default for NodeRegistry {
    fn default() -> Self {
        NodeRegistry::new(None)
    }
}
