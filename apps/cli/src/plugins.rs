//! Plugin discovery for the CLI.
//!
//! Scans standard directories for plugin files and loads them.
//!
//! Search order:
//! 1. `./plugins/` (project-local)
//! 2. Platform data dir (user-global):
//!    - Linux:   `~/.local/share/nebula/plugins/`
//!    - macOS:   `~/Library/Application Support/nebula/plugins/`
//!    - Windows: `C:\Users\<user>\AppData\Roaming\nebula\plugins\`
//!
//! Currently a stub — WASM plugin loading via `nebula-sandbox` is planned.

use std::path::PathBuf;

use nebula_plugin::PluginRegistry;

/// Load plugins from standard directories and register into the given registry.
///
/// Returns the number of plugins loaded.
pub fn load_plugins(_registry: &mut PluginRegistry) -> usize {
    // TODO: WASM plugin loading via nebula-sandbox.
    // For now, all actions are built-in (registered in actions.rs).
    0
}

/// List the directories that would be scanned for plugins.
pub fn plugin_directories() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    let local = PathBuf::from("plugins");
    if local.exists() {
        dirs.push(local);
    }

    if let Some(data) = dirs::data_dir() {
        let global = data.join("nebula").join("plugins");
        dirs.push(global);
    }

    dirs
}
