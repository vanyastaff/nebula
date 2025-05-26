#![allow(
    unsafe_code,
    reason = "unsafe code is used for dynamic library loading"
)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::read_dir;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::{env, fs, io, panic};

use libloading::{Library, Symbol};
use thiserror::Error;

use crate::node::NodeType;
use crate::types::Key;

/// Errors that can occur during node loading
#[derive(Error, Debug)]
pub enum NodeLoadError {
    /// Failed to load a node library
    #[error("Failed to load node: {0}")]
    Load(String),

    /// Failed to find the 'create_node' function in a node library
    #[error("Failed to find the 'create_node' function in node {0}: {1}")]
    CreateNode(String, String),

    /// A panic occurred while loading a node
    #[error("Panic occurred while loading node {0}")]
    Panic(String),

    /// Failed to read a directory
    #[error("Directory read error: {0}")]
    DirectoryRead(String),

    /// Failed to create a key
    #[error("Key creation error: {0}")]
    KeyCreation(#[from] crate::types::KeyParseError),
}

/// NodeLoader is responsible for loading node plugins from dynamic libraries.
/// It supports caching loaded nodes to avoid reloading the same node multiple
/// times.
pub struct NodeLoader {
    /// Path to the directory containing node libraries
    pub path: PathBuf,

    /// Cache of loaded nodes, keyed by node key
    cache: Arc<Mutex<HashMap<Key, Arc<NodeType>>>>,

    /// Loaded libraries that must be kept alive for the nodes to function
    #[allow(dead_code, reason = "Not using but need to keep a pointer save ref")]
    // The library references need to be kept alive
    libraries: Arc<Mutex<Vec<Library>>>,
}

impl NodeLoader {
    /// Creates a new NodeLoader with the specified path.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the directory containing node libraries
    pub fn new(path: PathBuf) -> Self {
        NodeLoader {
            path,
            cache: Arc::new(Mutex::new(HashMap::new())),
            libraries: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Gets the default path for node dependencies.
    /// This is typically the "deps" directory in the target build directory.
    ///
    /// # Returns
    ///
    /// The path to the deps directory if it exists, or None otherwise.
    pub fn get_default_deps_path() -> Option<PathBuf> {
        let current_dir = get_project_root().ok()?;
        let target_dir = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        let deps_path = current_dir.join("target").join(target_dir).join("deps");
        deps_path.try_exists().ok().map(|_| deps_path)
    }

    /// Loads a node by name.
    /// If the node is already cached, returns the cached version.
    /// Otherwise, loads the node from its library file.
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the node to load
    ///
    /// # Returns
    ///
    /// The loaded node is wrapped in Arc, or an error if loading failed.
    pub fn load_node(&self, name: &str) -> Result<Arc<NodeType>, NodeLoadError> {
        // Convert name to Key for cache lookup
        let key = Key::new(name)?;

        // Check cache first
        {
            let cache = self.cache.lock().unwrap();
            if let Some(cached_node) = cache.get(&key) {
                return Ok(Arc::clone(cached_node));
            }
        }

        // Construct the node library path
        let node_path = self.path.join(self.get_lib_filename(name));
        if !node_path.exists() {
            return Err(NodeLoadError::Load(format!(
                "Node library for {} not found at {:?}",
                name, node_path
            )));
        }

        // Try to load the node, catching any panics
        let result = panic::catch_unwind(|| unsafe {
            // Load the library
            let lib = Library::new(&node_path)
                .map_err(|e| NodeLoadError::Load(format!("Failed to load node: {}", e)))?;

            // Get the create_node function
            let fn_create_node: Symbol<fn() -> NodeType> = lib
                .get(b"create_node")
                .map_err(|e| NodeLoadError::CreateNode(name.to_string(), format!("{}", e)))?;

            // Create the node and wrap it in Arc
            let node = fn_create_node();
            let node_arc = Arc::new(node);

            // Keep the library alive
            {
                let mut libraries = self.libraries.lock().unwrap();
                libraries.push(lib);
            }

            Ok::<_, NodeLoadError>(node_arc)
        });

        // Handle the result
        match result {
            Ok(Ok(node)) => {
                // Cache the node using its actual key
                let node_key = node.key().clone();
                let mut cache = self.cache.lock().unwrap();
                cache.insert(node_key, Arc::clone(&node));
                Ok(node)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(NodeLoadError::Panic(name.to_string())),
        }
    }

    /// Loads all nodes from the node directory.
    ///
    /// # Returns
    ///
    /// A vector of all successfully loaded nodes, or an error if reading the
    /// directory failed.
    pub fn load_all(&self) -> Result<Vec<Arc<NodeType>>, NodeLoadError> {
        let entries =
            read_dir(&self.path).map_err(|e| NodeLoadError::DirectoryRead(e.to_string()))?;

        let mut nodes = Vec::new();

        for entry in entries {
            let path = entry
                .map_err(|e| NodeLoadError::DirectoryRead(e.to_string()))?
                .path();

            if self.is_node_library(&path) {
                if let Some(node_name) = self.extract_node_name(&path) {
                    match self.load_node(&node_name) {
                        Ok(node) => nodes.push(node),
                        Err(err) => {
                            eprintln!("Failed to load node '{}': {}", node_name, err);
                        }
                    }
                }
            }
        }

        Ok(nodes)
    }

    /// Checks if a node with the specified name exists in the node directory.
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the node to check
    ///
    /// # Returns
    ///
    /// true if the node exists, false otherwise
    pub fn check_node(&self, name: &str) -> bool {
        self.path.join(self.get_lib_filename(name)).exists()
    }

    /// Gets all currently cached nodes.
    ///
    /// # Returns
    ///
    /// A vector of all cached nodes.
    pub fn get_cached_nodes(&self) -> Vec<Arc<NodeType>> {
        let cache = self.cache.lock().unwrap();
        cache.values().cloned().collect()
    }

    /// Gets a cached node by its key.
    ///
    /// # Arguments
    ///
    /// * `key` - Key of the node to get
    ///
    /// # Returns
    ///
    /// The cached node if found, or None otherwise.
    pub fn get_cached_node(&self, key: &Key) -> Option<Arc<NodeType>> {
        let cache = self.cache.lock().unwrap();
        cache.get(key).cloned()
    }

    /// Gets all cached node keys.
    ///
    /// # Returns
    ///
    /// A vector of all cached node keys.
    pub fn get_cached_keys(&self) -> Vec<Key> {
        let cache = self.cache.lock().unwrap();
        cache.keys().cloned().collect()
    }

    /// Constructs the filename for a node library.
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the node
    ///
    /// # Returns
    ///
    /// The filename for the node library
    fn get_lib_filename(&self, name: &str) -> String {
        format!("{}_node.{}", name, Self::get_lib_extension())
    }

    /// Gets the library extension for the current platform.
    ///
    /// # Returns
    ///
    /// The library extension (dll, dylib, or so)
    fn get_lib_extension() -> &'static str {
        if cfg!(target_os = "windows") {
            "dll"
        } else if cfg!(target_os = "macos") {
            "dylib"
        } else {
            "so"
        }
    }

    /// Checks if a path is a node library.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to check
    ///
    /// # Returns
    ///
    /// true if the path is a node library, false otherwise
    fn is_node_library(&self, path: &Path) -> bool {
        path.extension() == Some(Self::get_lib_extension().as_ref())
            && path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.ends_with("_node"))
                .unwrap_or(false)
    }

    /// Extracts the node name from a node library path.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the node library
    ///
    /// # Returns
    ///
    /// The node name, or None if the path is not a valid node library
    fn extract_node_name(&self, path: &Path) -> Option<String> {
        path.file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| s.ends_with("_node"))
            .map(|s| s.trim_end_matches("_node").to_string())
    }
}

pub fn get_project_root() -> io::Result<PathBuf> {
    let path = env::current_dir()?;
    let mut path_ancestors = path.as_path().ancestors();

    while let Some(p) = path_ancestors.next() {
        let has_cargo = read_dir(p)?
            .into_iter()
            .any(|p| p.unwrap().file_name() == OsString::from("Cargo.lock"));
        if has_cargo {
            return Ok(PathBuf::from(p));
        }
    }
    Err(io::Error::new(
        ErrorKind::NotFound,
        "Ran out of places to find Cargo.toml",
    ))
}
