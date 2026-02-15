//! Dynamic plugin loading from shared libraries.
//!
//! This module is only available with the `dynamic-loading` feature.
//!
//! Each plugin is a shared library (`.dll` / `.so` / `.dylib`) exporting
//! a `create_plugin` symbol that returns a [`PluginType`].

// This module needs unsafe for FFI.
#![allow(unsafe_code, reason = "FFI calls for dynamic library loading")]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use libloading::{Library, Symbol};

use crate::plugin_type::PluginType;

/// Errors from the dynamic loading layer.
#[derive(Debug, thiserror::Error)]
pub enum PluginLoadError {
    /// Library file not found or failed to open.
    #[error("failed to load plugin library '{name}': {reason}")]
    Load {
        /// The plugin name that was being loaded.
        name: String,
        /// The underlying error message.
        reason: String,
    },

    /// The `create_plugin` symbol was not found in the library.
    #[error("symbol 'create_plugin' not found in plugin '{name}': {reason}")]
    SymbolNotFound {
        /// The plugin name.
        name: String,
        /// The underlying error message.
        reason: String,
    },

    /// A panic occurred in the loaded library.
    #[error("panic occurred while loading plugin '{0}'")]
    Panic(String),

    /// Failed to read the plugins directory.
    #[error("directory read error: {0}")]
    DirectoryRead(String),
}

/// Loads plugins from shared libraries on disk.
///
/// Maintains a cache of already-loaded plugins and keeps loaded [`Library`]
/// handles alive for the duration.
pub struct PluginLoader {
    path: PathBuf,
    cache: Mutex<HashMap<String, Arc<PluginType>>>,
    /// Libraries must stay alive while their plugin instances are in use.
    #[allow(dead_code, reason = "must keep Library handles alive")]
    libraries: Mutex<Vec<Library>>,
}

impl PluginLoader {
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

    /// Whether a library file for the given plugin name exists.
    pub fn exists(&self, name: &str) -> bool {
        self.lib_path(name).exists()
    }

    /// Load a plugin by name, returning a cached instance if available.
    ///
    /// # Safety
    ///
    /// This calls into an external shared library via FFI. The library must
    /// export `create_plugin` with the expected ABI.
    pub fn load(&self, name: &str) -> Result<Arc<PluginType>, PluginLoadError> {
        // Check cache.
        {
            let cache = self.cache.lock().unwrap();
            if let Some(cached) = cache.get(name) {
                return Ok(Arc::clone(cached));
            }
        }

        let lib_path = self.lib_path(name);
        if !lib_path.exists() {
            return Err(PluginLoadError::Load {
                name: name.to_owned(),
                reason: format!("library not found at {}", lib_path.display()),
            });
        }

        let result = std::panic::catch_unwind(|| {
            // SAFETY: We trust the plugin to export `create_plugin` with the correct ABI.
            unsafe {
                let lib = Library::new(&lib_path).map_err(|e| PluginLoadError::Load {
                    name: name.to_owned(),
                    reason: e.to_string(),
                })?;

                let create_fn: Symbol<fn() -> PluginType> =
                    lib.get(b"create_plugin")
                        .map_err(|e| PluginLoadError::SymbolNotFound {
                            name: name.to_owned(),
                            reason: e.to_string(),
                        })?;

                let plugin_type = create_fn();
                let plugin_arc = Arc::new(plugin_type);

                // Keep the library alive.
                self.libraries.lock().unwrap().push(lib);

                Ok::<_, PluginLoadError>(plugin_arc)
            }
        });

        match result {
            Ok(Ok(plugin)) => {
                self.cache
                    .lock()
                    .unwrap()
                    .insert(name.to_owned(), Arc::clone(&plugin));
                Ok(plugin)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(PluginLoadError::Panic(name.to_owned())),
        }
    }

    /// Load all plugin libraries found in the directory.
    pub fn load_all(&self) -> Result<Vec<Arc<PluginType>>, PluginLoadError> {
        let entries = std::fs::read_dir(&self.path)
            .map_err(|e| PluginLoadError::DirectoryRead(e.to_string()))?;

        let mut plugins = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|e| PluginLoadError::DirectoryRead(e.to_string()))?
                .path();

            if self.is_plugin_library(&path)
                && let Some(name) = self.extract_plugin_name(&path)
            {
                match self.load(&name) {
                    Ok(plugin) => plugins.push(plugin),
                    Err(e) => {
                        tracing::warn!(name, error = %e, "skipping plugin that failed to load")
                    }
                }
            }
        }
        Ok(plugins)
    }

    /// Construct the expected library file path for a plugin name.
    fn lib_path(&self, name: &str) -> PathBuf {
        self.path.join(format!("nebula_{name}.{}", Self::lib_ext()))
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

    /// Check if a path looks like a plugin library.
    fn is_plugin_library(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| ext == Self::lib_ext())
            && path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with("nebula_"))
    }

    /// Extract the plugin name from a library filename (e.g. `nebula_slack.dll` → `slack`).
    fn extract_plugin_name(&self, path: &Path) -> Option<String> {
        path.file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| s.starts_with("nebula_"))
            .map(|s| s.strip_prefix("nebula_").unwrap().to_owned())
    }
}

impl std::fmt::Debug for PluginLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginLoader")
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
        let loader = PluginLoader::new(PathBuf::from("/plugins"));
        let path = loader.lib_path("slack");
        let expected_ext = PluginLoader::lib_ext();
        assert!(
            path.to_str()
                .unwrap()
                .ends_with(&format!("nebula_slack.{expected_ext}"))
        );
    }

    #[test]
    fn is_plugin_library_checks() {
        let loader = PluginLoader::new(PathBuf::from("/plugins"));
        let ext = PluginLoader::lib_ext();

        let valid = PathBuf::from(format!("/plugins/nebula_slack.{ext}"));
        assert!(loader.is_plugin_library(&valid));

        let not_plugin = PathBuf::from(format!("/plugins/utils.{ext}"));
        assert!(!loader.is_plugin_library(&not_plugin));

        let wrong_ext = PathBuf::from("/plugins/nebula_slack.txt");
        assert!(!loader.is_plugin_library(&wrong_ext));
    }

    #[test]
    fn extract_plugin_name_works() {
        let loader = PluginLoader::new(PathBuf::from("/plugins"));
        let ext = PluginLoader::lib_ext();

        let path = PathBuf::from(format!("/plugins/nebula_http_request.{ext}"));
        assert_eq!(
            loader.extract_plugin_name(&path),
            Some("http_request".into())
        );

        let not_plugin = PathBuf::from(format!("/plugins/utils.{ext}"));
        assert_eq!(loader.extract_plugin_name(&not_plugin), None);
    }

    #[test]
    fn load_nonexistent_returns_error() {
        let loader = PluginLoader::new(PathBuf::from("/nonexistent"));
        let result = loader.load("missing");
        assert!(result.is_err());
    }
}
