//! Plugin discovery — scan directories for plugin binaries and get metadata.

use nebula_action::handler::InternalHandler;
use nebula_action::metadata::ActionMetadata;
use nebula_core::ActionKey;
use nebula_plugin_protocol::{PROTOCOL_VERSION, PluginMetadata, PluginResponse};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::capabilities::PluginCapabilities;
use crate::handler::ProcessSandboxHandler;
use crate::process::ProcessSandbox;

/// Discover a plugin by running its binary and asking for metadata.
pub async fn discover_plugin(binary: &Path) -> Result<PluginMetadata, String> {
    let sandbox = ProcessSandbox::new(
        binary.to_path_buf(),
        Duration::from_secs(5),
        PluginCapabilities::none(),
    );

    let output = sandbox
        .call("__metadata__", serde_json::Value::Null)
        .await
        .map_err(|e| format!("discovery failed for {}: {e}", binary.display()))?;

    // Parse the tagged response.
    let response: PluginResponse = serde_json::from_str(&output)
        .map_err(|e| format!("invalid response from {}: {e}", binary.display()))?;

    let metadata_value = match response {
        PluginResponse::Ok { output } => output,
        PluginResponse::Error { code, message, .. } => {
            return Err(format!("{}: {code}: {message}", binary.display()));
        }
    };

    let metadata: LeftPluginMetadata = serde_json::from_value(metadata_value)
        .map_err(|e| format!("invalid metadata from {}: {e}", binary.display()))?;

    // Validate protocol version.
    if metadata.protocol_version != PROTOCOL_VERSION {
        return Err(format!(
            "{}: protocol version mismatch (plugin={}, host={})",
            binary.display(),
            metadata.protocol_version,
            PROTOCOL_VERSION,
        ));
    }

    Ok(metadata.into_protocol())
}

/// Discover all plugins in a directory and create handlers.
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

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "failed to read directory entry");
                continue;
            }
        };
        let path = entry.path();

        if !is_executable(&path) {
            continue;
        }

        match discover_plugin(&path).await {
            Ok(meta) => {
                let sandbox = Arc::new(ProcessSandbox::new(
                    path.clone(),
                    default_timeout,
                    PluginCapabilities::none(), // TODO: load from config
                ));
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
    plugin: &PluginMetadata,
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
    if !path.is_file() {
        return false;
    }

    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    if !name.starts_with("nebula-plugin-") && !name.starts_with("nebula_plugin_") {
        return false;
    }

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if matches!(
        ext,
        "toml" | "json" | "yaml" | "yml" | "md" | "txt" | "so" | "dll" | "dylib"
    ) {
        return false;
    }

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
        ext == "exe" || ext.is_empty()
    }
}

/// Internal type for parsing metadata with protocol_version field.
#[derive(serde::Deserialize)]
struct LeftPluginMetadata {
    #[serde(default)]
    protocol_version: u32,
    key: String,
    name: String,
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    description: String,
    #[serde(default)]
    actions: Vec<nebula_plugin_protocol::ActionMeta>,
}

fn default_version() -> u32 {
    1
}

impl LeftPluginMetadata {
    fn into_protocol(self) -> PluginMetadata {
        let mut meta = PluginMetadata::new(self.key, self.name)
            .version(self.version)
            .description(self.description);
        for action in self.actions {
            meta = meta.action(action.key, action.name, action.description);
        }
        meta
    }
}
