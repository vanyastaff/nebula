//! WASM plugin loader — loads `.wasm` files from disk.

use std::path::PathBuf;

/// Metadata returned by a WASM plugin's `metadata` export.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WasmPluginMetadata {
    /// Unique plugin key.
    pub key: String,
    /// Human-readable name.
    pub name: String,
    /// Plugin version.
    pub version: u32,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// Action descriptors.
    #[serde(default)]
    pub actions: Vec<WasmActionDescriptor>,
}

/// Describes an action provided by a WASM plugin.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WasmActionDescriptor {
    /// Action key.
    pub key: String,
    /// Human-readable name.
    pub name: String,
    /// Description.
    #[serde(default)]
    pub description: String,
}

/// Loads WASM plugins from a directory.
pub struct WasmPluginLoader {
    paths: Vec<PathBuf>,
}

impl WasmPluginLoader {
    /// Create a loader that scans the given directories.
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self { paths }
    }

    /// Discover all `.wasm` files in the configured directories.
    pub fn discover(&self) -> Vec<PathBuf> {
        self.paths
            .iter()
            .filter(|dir| dir.exists())
            .flat_map(|dir| std::fs::read_dir(dir).into_iter().flatten())
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("wasm"))
            .collect()
    }
}
