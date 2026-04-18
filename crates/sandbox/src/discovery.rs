//! Plugin discovery — scan directories for plugin binaries and query metadata
//! using the duplex v2 protocol.

use std::{path::Path, sync::Arc, time::Duration};

use nebula_action::{ActionHandler, ActionMetadata};
use nebula_core::ActionKey;
use nebula_plugin_sdk::protocol::{ActionDescriptor, DUPLEX_PROTOCOL_VERSION, PluginToHost};
use semver::Version;

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
///
/// The metadata probe itself runs with the given `capabilities`; in most
/// deployments this can be [`PluginCapabilities::none`] since metadata
/// enumeration does not require network or filesystem access.
pub async fn discover_plugin(
    binary: &Path,
    capabilities: PluginCapabilities,
) -> Result<DiscoveredPlugin, String> {
    let sandbox = ProcessSandbox::new(binary.to_path_buf(), Duration::from_secs(5), capabilities);

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
///
/// `default_capabilities` is applied to every discovered plugin's runtime
/// sandbox (and the metadata probe). Callers are expected to source this
/// from host configuration per deployment policy.
pub async fn discover_directory(
    dir: &Path,
    default_timeout: Duration,
    default_capabilities: PluginCapabilities,
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

        match discover_plugin(&path, default_capabilities.clone()).await {
            Ok(plugin) => {
                let sandbox = Arc::new(ProcessSandbox::new(
                    path.clone(),
                    default_timeout,
                    default_capabilities.clone(),
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
    let namespace_prefix = format!("{}.", plugin.key);
    let interface_version = parse_interface_version(&plugin.version).unwrap_or_else(|| {
        tracing::warn!(
            plugin = %plugin.key,
            version = %plugin.version,
            "invalid plugin version; defaulting action interface version to 1.0.0",
        );
        Version::new(1, 0, 0)
    });
    plugin
        .actions
        .iter()
        .filter_map(|action| {
            let full_key = if action.key.contains('.') {
                if !action.key.starts_with(&namespace_prefix) {
                    tracing::warn!(
                        plugin = %plugin.key,
                        action_key = %action.key,
                        "action key outside plugin namespace, skipping",
                    );
                    return None;
                }
                action.key.clone()
            } else {
                format!("{namespace_prefix}{}", action.key)
            };

            let action_key = match ActionKey::new(&full_key) {
                Ok(key) => key,
                Err(e) => {
                    tracing::warn!(key = %full_key, error = %e, "invalid action key, skipping");
                    return None;
                },
            };

            let metadata = ActionMetadata::new(action_key, &action.name, &action.description)
                .with_version_full(interface_version.clone());

            let handler = ActionHandler::Stateless(Arc::new(ProcessSandboxHandler::new(
                Arc::clone(&sandbox),
                metadata.clone(),
            )));

            Some((metadata, handler))
        })
        .collect()
}

fn parse_interface_version(version: &str) -> Option<Version> {
    Version::parse(version).ok()
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
    use std::{path::PathBuf, sync::Arc, time::Duration};

    use nebula_plugin_sdk::protocol::ActionDescriptor;

    use super::{DiscoveredPlugin, create_handlers};
    use crate::{capabilities::PluginCapabilities, process::ProcessSandbox};

    #[test]
    fn non_unix_executable_extension_is_case_insensitive() {
        assert!(super::can_non_unix_executable_extension("exe"));
        assert!(super::can_non_unix_executable_extension("EXE"));
        assert!(super::can_non_unix_executable_extension("ExE"));
        assert!(super::can_non_unix_executable_extension(""));
        assert!(!super::can_non_unix_executable_extension("dll"));
    }

    #[test]
    fn create_handlers_rejects_cross_namespace_fq_keys() {
        let plugin = DiscoveredPlugin {
            key: "com.good.plugin".to_owned(),
            version: "1.0.0".to_owned(),
            actions: vec![
                ActionDescriptor {
                    key: "echo".to_owned(),
                    name: "Echo".to_owned(),
                    description: "ok".to_owned(),
                },
                ActionDescriptor {
                    key: "system.exec".to_owned(),
                    name: "Exec".to_owned(),
                    description: "bad".to_owned(),
                },
            ],
        };
        let sandbox = Arc::new(ProcessSandbox::new(
            PathBuf::from("nebula-plugin-dummy"),
            Duration::from_secs(1),
            PluginCapabilities::none(),
        ));

        let handlers = create_handlers(&plugin, sandbox);
        assert_eq!(handlers.len(), 1);
        assert_eq!(handlers[0].0.base.key.as_str(), "com.good.plugin.echo");
    }

    #[test]
    fn create_handlers_use_plugin_major_minor_version() {
        let plugin = DiscoveredPlugin {
            key: "com.good.plugin".to_owned(),
            version: "2.7.3".to_owned(),
            actions: vec![ActionDescriptor {
                key: "echo".to_owned(),
                name: "Echo".to_owned(),
                description: "ok".to_owned(),
            }],
        };
        let sandbox = Arc::new(ProcessSandbox::new(
            PathBuf::from("nebula-plugin-dummy"),
            Duration::from_secs(1),
            PluginCapabilities::none(),
        ));

        let handlers = create_handlers(&plugin, sandbox);
        assert_eq!(handlers.len(), 1);
        assert_eq!(handlers[0].0.base.version.major, 2);
        assert_eq!(handlers[0].0.base.version.minor, 7);
        assert_eq!(handlers[0].0.base.version.patch, 3);
    }
}
