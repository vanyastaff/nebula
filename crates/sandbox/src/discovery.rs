//! Plugin discovery — scan directories for plugin binaries and get metadata.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use nebula_action::handler::InternalHandler;
use nebula_action::metadata::ActionMetadata;
use nebula_core::ActionKey;

use crate::handler::ProcessSandboxHandler;
use crate::process::ProcessSandbox;

/// Metadata returned by a plugin binary.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiscoveredPlugin {
    /// Plugin key.
    pub key: String,
    /// Human-readable name.
    pub name: String,
    /// Plugin version.
    #[serde(default = "default_version")]
    pub version: u32,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// Actions this plugin provides.
    #[serde(default)]
    pub actions: Vec<DiscoveredAction>,
}

/// An action discovered from a plugin.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiscoveredAction {
    /// Action key.
    pub key: String,
    /// Human-readable name.
    pub name: String,
    /// Description.
    #[serde(default)]
    pub description: String,
}

fn default_version() -> u32 {
    1
}

/// Discover a plugin by running its binary and asking for metadata.
///
/// Sends `{"action_key":"__metadata__","input":{}}` to stdin,
/// expects `{"output":{...}}` on stdout with plugin metadata.
pub async fn discover_plugin(binary: &Path) -> Result<DiscoveredPlugin, String> {
    let sandbox = ProcessSandbox::new(binary.to_path_buf(), Duration::from_secs(5));

    let output = sandbox
        .call("__metadata__", "{}")
        .await
        .map_err(|e| format!("discovery failed for {}: {e}", binary.display()))?;

    // Parse the response — could be wrapped in {"output": ...} or direct.
    let value: serde_json::Value =
        serde_json::from_str(&output).map_err(|e| format!("invalid metadata JSON: {e}"))?;

    let metadata_value = if value.get("output").is_some() {
        value["output"].clone()
    } else {
        value
    };

    serde_json::from_value(metadata_value).map_err(|e| format!("invalid plugin metadata: {e}"))
}

/// Discover all plugins in a directory and create handlers.
///
/// Returns a list of `(plugin_name, Vec<handlers>)` ready to register.
pub async fn discover_directory(
    dir: &Path,
    default_timeout: Duration,
) -> Vec<(String, Vec<Arc<dyn InternalHandler>>)> {
    let mut results = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "failed to read plugin directory");
            return results;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();

        if !is_executable(&path) {
            continue;
        }

        match discover_plugin(&path).await {
            Ok(meta) => {
                let sandbox = Arc::new(ProcessSandbox::new(path.clone(), default_timeout));
                let handlers = create_handlers(&meta, sandbox);

                tracing::info!(
                    plugin = %meta.key,
                    actions = handlers.len(),
                    binary = %path.display(),
                    "discovered community plugin"
                );

                results.push((meta.key.clone(), handlers));
            }
            Err(e) => {
                tracing::warn!(binary = %path.display(), error = %e, "skipping plugin");
            }
        }
    }

    results
}

/// Create InternalHandler instances for each action in a discovered plugin.
fn create_handlers(
    plugin: &DiscoveredPlugin,
    sandbox: Arc<ProcessSandbox>,
) -> Vec<Arc<dyn InternalHandler>> {
    plugin
        .actions
        .iter()
        .filter_map(|action| {
            let full_key = if action.key.contains('.') {
                action.key.clone()
            } else {
                format!("{}.{}", plugin.key, action.key)
            };

            let action_key = match ActionKey::new(&full_key) {
                Ok(key) => key,
                Err(e) => {
                    tracing::warn!(key = %full_key, error = %e, "invalid action key, skipping");
                    return None;
                }
            };

            let metadata = ActionMetadata::new(action_key, &action.name, &action.description);

            let handler: Arc<dyn InternalHandler> =
                Arc::new(ProcessSandboxHandler::new(Arc::clone(&sandbox), metadata));

            Some(handler)
        })
        .collect()
}

/// Check if a file looks like an executable plugin binary.
fn is_executable(path: &Path) -> bool {
    // Skip non-files.
    if !path.is_file() {
        return false;
    }

    // Must start with "nebula-plugin-" or "nebula_plugin_".
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    if !name.starts_with("nebula-plugin-") && !name.starts_with("nebula_plugin_") {
        return false;
    }

    // Skip common non-executable extensions.
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if matches!(
        ext,
        "toml" | "json" | "yaml" | "yml" | "md" | "txt" | "so" | "dll" | "dylib"
    ) {
        return false;
    }

    // On Unix, check executable bit.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = path.metadata() {
            return meta.permissions().mode() & 0o111 != 0;
        }
        false
    }

    #[cfg(not(unix))]
    {
        // On Windows, check for .exe extension.
        ext == "exe" || ext.is_empty()
    }
}
