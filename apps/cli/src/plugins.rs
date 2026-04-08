//! Plugin discovery for the CLI.
//!
//! Scans standard directories for community plugin binaries.
//! Each binary is queried for metadata, then actions are registered
//! in the ActionRegistry via ProcessSandboxHandler.
//!
//! Search order:
//! 1. `./plugins/` (project-local)
//! 2. Platform data dir (user-global):
//!    - Linux:   `~/.local/share/nebula/plugins/`
//!    - macOS:   `~/Library/Application Support/nebula/plugins/`
//!    - Windows: `C:\Users\<user>\AppData\Roaming\nebula\plugins\`

use std::path::PathBuf;
use std::time::Duration;

use nebula_runtime::ActionRegistry;
use nebula_sandbox::discovery;

/// Default timeout for plugin actions.
const DEFAULT_PLUGIN_TIMEOUT: Duration = Duration::from_secs(30);

/// Discover and register community plugins from standard directories.
///
/// Returns the number of actions registered.
pub async fn discover_and_register(registry: &ActionRegistry) -> usize {
    let dirs = plugin_directories();
    let mut total = 0;

    for dir in &dirs {
        if !dir.exists() {
            continue;
        }

        let plugins = discovery::discover_directory(dir, DEFAULT_PLUGIN_TIMEOUT).await;

        for (plugin_name, handlers) in plugins {
            for handler in handlers {
                let key = handler.metadata().key.as_str().to_owned();
                registry.register(handler);
                tracing::info!(action = %key, plugin = %plugin_name, "registered community action");
                total += 1;
            }
        }
    }

    total
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
