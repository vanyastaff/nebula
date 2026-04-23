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

use std::{path::PathBuf, time::Duration};

use nebula_engine::ActionRegistry;
use nebula_plugin::PluginRegistry;
use nebula_sandbox::{capabilities::PluginCapabilities, discovery};

/// Default timeout for plugin actions.
const DEFAULT_PLUGIN_TIMEOUT: Duration = Duration::from_secs(30);

/// Discover and register community plugins from standard directories.
///
/// Returns the number of actions registered.
pub(crate) async fn discover_and_register(action_registry: &ActionRegistry) -> usize {
    let dirs = plugin_directories();
    let mut plugin_registry = PluginRegistry::new();
    let mut all_handlers = Vec::new();

    for dir in &dirs {
        if !dir.exists() {
            continue;
        }

        // TODO (ADR-0025 D4): load per-deployment capability policy from CLI config.
        let handlers = discovery::discover_directory(
            dir,
            &mut plugin_registry,
            DEFAULT_PLUGIN_TIMEOUT,
            PluginCapabilities::none(),
        )
        .await;
        all_handlers.extend(handlers);
    }

    // Feed discovered actions into the runtime ActionRegistry.
    let mut total = 0;
    for (metadata, handler) in all_handlers {
        let key = &metadata.base.key;
        if action_registry.get(key).is_some() {
            tracing::warn!(
                action = %key,
                "community action key collision detected, skipping registration",
            );
            continue;
        }
        tracing::info!(action = %key, "registered community action");
        action_registry.register(metadata, handler);
        total += 1;
    }

    total
}

/// List the directories that would be scanned for plugins.
pub(crate) fn plugin_directories() -> Vec<PathBuf> {
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
