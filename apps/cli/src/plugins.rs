//! Dynamic plugin loading for the CLI.
//!
//! Scans standard directories for `.so`/`.dll`/`.dylib` plugin files,
//! loads them via `PluginLoader`, and registers metadata into `PluginRegistry`.
//!
//! Search order:
//! 1. `./plugins/` (project-local)
//! 2. Platform data dir (user-global):
//!    - Linux:   `~/.local/share/nebula/plugins/`
//!    - macOS:   `~/Library/Application Support/nebula/plugins/`
//!    - Windows: `C:\Users\<user>\AppData\Roaming\nebula\plugins\`

use std::path::PathBuf;
use std::sync::Arc;

use nebula_plugin::{PluginLoader, PluginRegistry, PluginType};

/// Load plugins from standard directories and register into the given registry.
///
/// Returns the number of plugins loaded. Errors are logged and skipped
/// (a bad plugin should not prevent the CLI from starting).
pub fn load_plugins(registry: &mut PluginRegistry) -> usize {
    let dirs = plugin_directories();
    let mut count = 0;

    for dir in &dirs {
        if !dir.exists() {
            continue;
        }

        let loader = PluginLoader::new(dir.clone());
        match loader.load_all() {
            Ok(plugins) => {
                for plugin_type in plugins {
                    if let Err(e) = register_plugin_type(registry, &plugin_type) {
                        tracing::warn!(error = %e, "failed to register plugin");
                    } else {
                        count += 1;
                    }
                }
            }
            Err(e) => {
                tracing::warn!(dir = %dir.display(), error = %e, "failed to scan plugin directory");
            }
        }
    }

    count
}

/// List the directories to scan for plugins.
pub fn plugin_directories() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // Project-local plugins.
    let local = PathBuf::from("plugins");
    if local.exists() {
        dirs.push(local);
    }

    // User-global plugins (platform-specific data dir).
    if let Some(data) = dirs::data_dir() {
        let global = data.join("nebula").join("plugins");
        dirs.push(global);
    }

    dirs
}

fn register_plugin_type(
    registry: &mut PluginRegistry,
    plugin_type: &Arc<PluginType>,
) -> Result<(), nebula_plugin::PluginError> {
    // Get plugin metadata for logging.
    let plugin = plugin_type.get_plugin(None)?;

    let key = plugin.key().clone();
    let name = plugin.name().to_owned();

    // Call on_load lifecycle hook.
    plugin.on_load()?;

    // Register into the plugin registry.
    registry.register(PluginType::Single(Arc::clone(&plugin)))?;

    tracing::info!(
        plugin_key = %key,
        plugin_name = %name,
        "loaded external plugin"
    );

    Ok(())
}
