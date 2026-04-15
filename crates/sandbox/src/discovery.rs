//! Plugin discovery — scan directories for plugin binaries and query metadata
//! using the duplex v2 protocol.

use std::{path::Path, sync::Arc, time::Duration};

use nebula_action::{ActionHandler, ActionMetadata};
use nebula_core::ActionKey;
use nebula_plugin_sdk::protocol::{ActionDescriptor, DUPLEX_PROTOCOL_VERSION, PluginToHost};

use crate::{
    capabilities::PluginCapabilities, handler::ProcessSandboxHandler, process::ProcessSandbox,
};

/// Plugin metadata returned by [`discover_plugin`].
///
/// Host-side projection of [`PluginToHost::MetadataResponse`] — drops the
/// correlation `id` field and keeps the rest.
#[derive(Debug, Clone)]
pub struct DiscoveredPlugin {
    /// Unique plugin key (e.g., `"com.author.telegram"`).
    pub key: String,
    /// Plugin version string (semver).
    pub version: String,
    /// Actions this plugin provides.
    pub actions: Vec<ActionDescriptor>,
}

/// Discover a plugin by spawning its binary and sending a `MetadataRequest`
/// envelope.
pub async fn discover_plugin(binary: &Path) -> Result<DiscoveredPlugin, String> {
    let sandbox = ProcessSandbox::new(
        binary.to_path_buf(),
        Duration::from_secs(5),
        PluginCapabilities::none(),
    );

    let envelope = sandbox
        .get_metadata()
        .await
        .map_err(|e| format!("discovery failed for {}: {e}", binary.display()))?;

    match envelope {
        PluginToHost::MetadataResponse {
            protocol_version,
            plugin_key,
            plugin_version,
            actions,
            ..
        } => {
            if protocol_version != DUPLEX_PROTOCOL_VERSION {
                return Err(format!(
                    "{}: protocol version mismatch (plugin={}, host={})",
                    binary.display(),
                    protocol_version,
                    DUPLEX_PROTOCOL_VERSION,
                ));
            }
            Ok(DiscoveredPlugin {
                key: plugin_key,
                version: plugin_version,
                actions,
            })
        },
        other => Err(format!(
            "{}: unexpected envelope from plugin: {}",
            binary.display(),
            response_kind(&other),
        )),
    }
}

fn response_kind(env: &PluginToHost) -> &'static str {
    match env {
        PluginToHost::ActionResultOk { .. } => "action_result_ok",
        PluginToHost::ActionResultError { .. } => "action_result_error",
        PluginToHost::RpcCall { .. } => "rpc_call",
        PluginToHost::Log { .. } => "log",
        PluginToHost::MetadataResponse { .. } => "metadata_response",
    }
}

/// Discover all plugins in a directory and create handlers.
pub async fn discover_directory(
    dir: &Path,
    default_timeout: Duration,
) -> Vec<(String, Vec<(ActionMetadata, ActionHandler)>)> {
    let mut results = Vec::new();

    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "failed to read plugin directory");
            return results;
        },
    };

    loop {
        let entry = match entries.next_entry().await {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(e) => {
                tracing::warn!(error = %e, "failed to read directory entry");
                continue;
            },
        };
        let path = entry.path();

        if !is_executable(&path) {
            continue;
        }

        match discover_plugin(&path).await {
            Ok(plugin) => {
                let sandbox = Arc::new(ProcessSandbox::new(
                    path.clone(),
                    default_timeout,
                    PluginCapabilities::none(), // TODO: load from config
                ));
                let handlers = create_handlers(&plugin, sandbox);

                tracing::info!(
                    plugin = %plugin.key,
                    version = %plugin.version,
                    actions = handlers.len(),
                    binary = %path.display(),
                    "discovered community plugin"
                );

                results.push((plugin.key.clone(), handlers));
            },
            Err(e) => {
                tracing::warn!(binary = %path.display(), error = %e, "skipping plugin");
            },
        }
    }

    results
}

/// Create `ActionHandler` instances for each action in a discovered plugin.
fn create_handlers(
    plugin: &DiscoveredPlugin,
    sandbox: Arc<ProcessSandbox>,
) -> Vec<(ActionMetadata, ActionHandler)> {
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
                },
            };

            let metadata = ActionMetadata::new(action_key, &action.name, &action.description);

            let handler = ActionHandler::Stateless(Arc::new(ProcessSandboxHandler::new(
                Arc::clone(&sandbox),
                metadata.clone(),
            )));

            Some((metadata, handler))
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
        can_non_unix_executable_extension(ext)
    }
}

/// Returns true when `extension` is valid for a Windows plugin binary (`.exe`, any ASCII case, or
/// empty).
#[cfg(any(test, not(unix)))]
fn can_non_unix_executable_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case("exe") || extension.is_empty()
}

#[cfg(test)]
mod tests {
    #[test]
    fn non_unix_executable_extension_is_case_insensitive() {
        assert!(super::can_non_unix_executable_extension("exe"));
        assert!(super::can_non_unix_executable_extension("EXE"));
        assert!(super::can_non_unix_executable_extension("ExE"));
        assert!(super::can_non_unix_executable_extension(""));
        assert!(!super::can_non_unix_executable_extension("dll"));
    }
}
