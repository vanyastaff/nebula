//! Dynamic node loading from shared libraries.
//!
//! This module is only available with the `dynamic-loading` feature.
//!
//! Each node plugin is a shared library (`.dll` / `.so` / `.dylib`) exporting
//! a `create_node` symbol that returns a [`NodeType`].

// This module needs unsafe for FFI.
#![allow(unsafe_code, reason = "FFI calls for dynamic library loading")]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use libloading::{Library, Symbol};

use crate::node_type::NodeType;

/// Errors from the dynamic loading layer.
#[derive(Debug, thiserror::Error)]
pub enum NodeLoadError {
    /// Library file not found or failed to open.
    #[error("failed to load node library '{name}': {reason}")]
    Load {
        /// The node name that was being loaded.
        name: String,
        /// The underlying error message.
        reason: String,
    },

    /// The `create_node` symbol was not found in the library.
    #[error("symbol 'create_node' not found in node '{name}': {reason}")]
    SymbolNotFound {
        /// The node name.
        name: String,
        /// The underlying error message.
        reason: String,
    },

    /// A panic occurred in the loaded library.
    #[error("panic occurred while loading node '{0}'")]
    Panic(String),

    /// Failed to read the plugins directory.
    #[error("directory read error: {0}")]
    DirectoryRead(String),
}

/// Loads node plugins from shared libraries on disk.
///
/// Maintains a cache of already-loaded nodes and keeps loaded [`Library`]
/// handles alive for the duration.
pub struct NodeLoader {
    path: PathBuf,
    cache: Mutex<HashMap<String, Arc<NodeType>>>,
    /// Libraries must stay alive while their node instances are in use.
    #[allow(dead_code, reason = "must keep Library handles alive")]
    libraries: Mutex<Vec<Library>>,
}

impl NodeLoader {
    /// Create a new loader pointing at the given directory.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            cache: Mutex::new(HashMap::new()),
            libraries: Mutex::new(Vec::new()),
        }
    }

    /// The directory this loader scans for plugins.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether a library file for the given node name exists.
    pub fn exists(&self, name: &str) -> bool {
        self.lib_path(name).exists()
    }

    /// Load a node by name, returning a cached instance if available.
    ///
    /// # Safety
    ///
    /// This calls into an external shared library via FFI. The library must
    /// export `create_node` with the expected ABI.
    pub fn load(&self, name: &str) -> Result<Arc<NodeType>, NodeLoadError> {
        // Check cache.
        {
            let cache = self.cache.lock().unwrap();
            if let Some(cached) = cache.get(name) {
                return Ok(Arc::clone(cached));
            }
        }

        let lib_path = self.lib_path(name);
        if !lib_path.exists() {
            return Err(NodeLoadError::Load {
                name: name.to_owned(),
                reason: format!("library not found at {}", lib_path.display()),
            });
        }

        let result = std::panic::catch_unwind(|| {
            // SAFETY: We trust the plugin to export `create_node` with the correct ABI.
            unsafe {
                let lib = Library::new(&lib_path).map_err(|e| NodeLoadError::Load {
                    name: name.to_owned(),
                    reason: e.to_string(),
                })?;

                let create_fn: Symbol<fn() -> NodeType> =
                    lib.get(b"create_node")
                        .map_err(|e| NodeLoadError::SymbolNotFound {
                            name: name.to_owned(),
                            reason: e.to_string(),
                        })?;

                let node_type = create_fn();
                let node_arc = Arc::new(node_type);

                // Keep the library alive.
                self.libraries.lock().unwrap().push(lib);

                Ok::<_, NodeLoadError>(node_arc)
            }
        });

        match result {
            Ok(Ok(node)) => {
                self.cache
                    .lock()
                    .unwrap()
                    .insert(name.to_owned(), Arc::clone(&node));
                Ok(node)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(NodeLoadError::Panic(name.to_owned())),
        }
    }

    /// Load all node libraries found in the directory.
    pub fn load_all(&self) -> Result<Vec<Arc<NodeType>>, NodeLoadError> {
        let entries = std::fs::read_dir(&self.path)
            .map_err(|e| NodeLoadError::DirectoryRead(e.to_string()))?;

        let mut nodes = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|e| NodeLoadError::DirectoryRead(e.to_string()))?
                .path();

            if self.is_node_library(&path)
                && let Some(name) = self.extract_node_name(&path)
            {
                match self.load(&name) {
                    Ok(node) => nodes.push(node),
                    Err(e) => tracing::warn!(name, error = %e, "skipping node that failed to load"),
                }
            }
        }
        Ok(nodes)
    }

    /// Construct the expected library file path for a node name.
    fn lib_path(&self, name: &str) -> PathBuf {
        self.path.join(format!("{name}_node.{}", Self::lib_ext()))
    }

    /// Platform-specific library extension.
    fn lib_ext() -> &'static str {
        if cfg!(target_os = "windows") {
            "dll"
        } else if cfg!(target_os = "macos") {
            "dylib"
        } else {
            "so"
        }
    }

    /// Check if a path looks like a node library.
    fn is_node_library(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| ext == Self::lib_ext())
            && path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.ends_with("_node"))
    }

    /// Extract the node name from a library filename (e.g. `slack_node.dll` → `slack`).
    fn extract_node_name(&self, path: &Path) -> Option<String> {
        path.file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| s.ends_with("_node"))
            .map(|s| s.strip_suffix("_node").unwrap().to_owned())
    }
}

impl std::fmt::Debug for NodeLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeLoader")
            .field("path", &self.path)
            .field(
                "cached",
                &self.cache.lock().unwrap().keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lib_path_format() {
        let loader = NodeLoader::new(PathBuf::from("/plugins"));
        let path = loader.lib_path("slack");
        let expected_ext = NodeLoader::lib_ext();
        assert!(
            path.to_str()
                .unwrap()
                .ends_with(&format!("slack_node.{expected_ext}"))
        );
    }

    #[test]
    fn is_node_library_checks() {
        let loader = NodeLoader::new(PathBuf::from("/plugins"));
        let ext = NodeLoader::lib_ext();

        let valid = PathBuf::from(format!("/plugins/slack_node.{ext}"));
        assert!(loader.is_node_library(&valid));

        let not_node = PathBuf::from(format!("/plugins/utils.{ext}"));
        assert!(!loader.is_node_library(&not_node));

        let wrong_ext = PathBuf::from("/plugins/slack_node.txt");
        assert!(!loader.is_node_library(&wrong_ext));
    }

    #[test]
    fn extract_node_name_works() {
        let loader = NodeLoader::new(PathBuf::from("/plugins"));
        let ext = NodeLoader::lib_ext();

        let path = PathBuf::from(format!("/plugins/http_request_node.{ext}"));
        assert_eq!(loader.extract_node_name(&path), Some("http_request".into()));

        let not_node = PathBuf::from(format!("/plugins/utils.{ext}"));
        assert_eq!(loader.extract_node_name(&not_node), None);
    }

    #[test]
    fn load_nonexistent_returns_error() {
        let loader = NodeLoader::new(PathBuf::from("/nonexistent"));
        let result = loader.load("missing");
        assert!(result.is_err());
    }
}
